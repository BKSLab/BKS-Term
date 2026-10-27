param(
    [string]$Exe = (Join-Path $PSScriptRoot '../target/stage9/bks-app.exe'),
    [ValidateSet('Tui','Notifications','All')][string]$Mode = 'Tui',
    [string]$Artifacts = (Join-Path $PSScriptRoot '../target/stage89-check'),
    [switch]$PrepareOnly
)

# Only newly created processes, their HWNDs and their private APPDATA are used.
# No SendInput, clipboard, real Codex, account settings or network operations.
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSEdition -eq 'Core' -or [Threading.Thread]::CurrentThread.GetApartmentState() -ne 'MTA') {
    $arguments = @('-NoProfile','-Mta','-ExecutionPolicy','Bypass','-File',$PSCommandPath,'-Exe',$Exe,'-Mode',$Mode,'-Artifacts',$Artifacts)
    if ($PrepareOnly) { $arguments += '-PrepareOnly' }
    & "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" @arguments
    exit $LASTEXITCODE
}

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName UIAutomationClientsideProviders
$references = @([Windows.Automation.AutomationElement].Assembly.Location,
    [Windows.Automation.AutomationPattern].Assembly.Location,
    [UIAutomationClientsideProviders.UIAutomationClientSideProviders].Assembly.Location)
# Compile just the existing native helper; do not evaluate stage7's scenario.
$stage7Source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'check-stage7.ps1'),[Text.Encoding]::UTF8)
$nativeSource = [regex]::Match($stage7Source,"(?s)using System;.*?(?=\r?\n'@)").Value
if (-not $nativeSource.Contains('public static class BksStage7')) { throw 'Stage7 own-PID helper unavailable' }
$extensionSource = @'
public static class BksStage89 {
    [DllImport("user32.dll")] private static extern bool PostMessageW(IntPtr window,uint message,IntPtr wparam,IntPtr lparam);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr window,uint flags);
    [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern bool ShowWindow(IntPtr window,int command);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window,out uint process);
    public static uint ProcessId(IntPtr window) { uint process; GetWindowThreadProcessId(window,out process); return process; }
    [StructLayout(LayoutKind.Sequential)] private struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] private struct GuiInfo {
        public uint Size,Flags; public IntPtr Active,Focus,Capture,Menu,MoveSize,Caret; public Rect CaretRect;
    }
    [DllImport("user32.dll")] private static extern bool GetGUIThreadInfo(uint thread,ref GuiInfo info);
    [DllImport("user32.dll",EntryPoint="SendMessageTimeoutW",SetLastError=true)]
    private static extern IntPtr MessageTimeout(IntPtr window,uint message,IntPtr wparam,IntPtr lparam,uint flags,uint timeout,out UIntPtr result);
    public static void Activate(uint process,IntPtr window) {
        BksStage7.Own(process,window); ShowWindow(window,9); SetForegroundWindow(window);
    }
    public static IntPtr Focus(uint process,IntPtr window) {
        BksStage7.Own(process,window); uint actual; uint thread=GetWindowThreadProcessId(window,out actual);
        var info=new GuiInfo(); info.Size=(uint)Marshal.SizeOf(typeof(GuiInfo));
        if(!GetGUIThreadInfo(thread,ref info))throw new InvalidOperationException("Own GetGUIThreadInfo failed");
        return info.Focus;
    }
    public static void Post(uint process,IntPtr window,uint message,int value) {
        BksStage7.Own(process,window);
        if(!PostMessageW(window,message,new IntPtr(value),IntPtr.Zero))throw new InvalidOperationException("Own PostMessage failed");
    }
    public static void Key(uint process,IntPtr window,int key) {
        BksStage7.Own(process,window);
        if(Focus(process,window)!=window)throw new InvalidOperationException("Own key target is not focused");
        Post(process,window,0x100,key); Post(process,window,0x101,key);
    }
    public static int Selected(uint process,IntPtr list) {
        BksStage7.Own(process,list); UIntPtr result;
        if(MessageTimeout(list,0x188,IntPtr.Zero,IntPtr.Zero,2,1500,out result)==IntPtr.Zero)
            throw new TimeoutException("Own LB_GETCURSEL exceeded 1500 ms");
        return unchecked((int)result.ToUInt64());
    }
    public sealed class Notice {
        public string Text{get;set;} public string Activity{get;set;} public double AtMs{get;set;}
    }
    private static readonly object noticeLock=new object();
    private static readonly List<Notice> notices=new List<Notice>();
    private static System.Windows.Automation.AutomationElement source;
    private static System.Windows.Automation.AutomationEventHandler handler;
    public static void Watch(uint process,IntPtr window) {
        Stop(); BksStage7.Own(process,window); lock(noticeLock)notices.Clear();
        source=System.Windows.Automation.AutomationElement.FromHandle(window);
        handler=(sender,args)=>{
            var notification=args as System.Windows.Automation.NotificationEventArgs;
            if(notification==null)return;
            lock(noticeLock)notices.Add(new Notice{Text=notification.DisplayString,Activity=notification.ActivityId,
                AtMs=System.Diagnostics.Stopwatch.GetTimestamp()*1000.0/System.Diagnostics.Stopwatch.Frequency});
        };
        System.Windows.Automation.Automation.AddAutomationEventHandler(
            System.Windows.Automation.AutomationElementIdentifiers.NotificationEvent,source,
            System.Windows.Automation.TreeScope.Element,handler);
    }
    public static Notice[] Notices(){lock(noticeLock)return notices.ToArray();}
    public static void Stop(){
        if(source==null||handler==null)return;
        System.Windows.Automation.Automation.RemoveAutomationEventHandler(
            System.Windows.Automation.AutomationElementIdentifiers.NotificationEvent,source,handler);
        source=null; handler=null;
    }
}
'@

$fixtureSource = @'
using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
public static class Stage89Fixture {
    [DllImport("kernel32.dll")] private static extern IntPtr GetStdHandle(int kind);
    [DllImport("kernel32.dll")] private static extern bool GetConsoleMode(IntPtr handle,out uint mode);
    [DllImport("kernel32.dll")] private static extern bool SetConsoleMode(IntPtr handle,uint mode);
    [DllImport("kernel32.dll")] private static extern bool ReadFile(IntPtr handle,byte[] data,uint size,out uint read,IntPtr unused);
    private static Stream output;
    private static void Write(string text){var bytes=new UTF8Encoding(false).GetBytes(text);output.Write(bytes,0,bytes.Length);output.Flush();}
    private static void Frame(string text){Write("\x1b[2J\x1b[H"+text.Replace("\r\n","\n").Replace("\n","\r\n"));}
    public static int Main(string[] args){
        if(args.Length==1&&args[0]=="--version"){Console.WriteLine("codex-cli 0.164.0");return 0;}
        bool quiet=args.Length==1&&args[0]=="--quiet";
        if(args.Length!=0&&!quiet)return 3;
        string root=Path.Combine(Environment.GetEnvironmentVariable("APPDATA"),"BKSStage89");
        Directory.CreateDirectory(root);
        string id=System.Diagnostics.Process.GetCurrentProcess().Id.ToString();
        string inputPath=Path.Combine(root,"input-"+id+".bin");
        File.WriteAllBytes(inputPath,new byte[0]);
        Console.OutputEncoding=new UTF8Encoding(false); output=Console.OpenStandardOutput();
        uint mode; IntPtr input=GetStdHandle(-10),stdout=GetStdHandle(-11);
        if(!GetConsoleMode(input,out mode)||!SetConsoleMode(input,(mode&~7u)|0x200u))return 4;
        if(GetConsoleMode(stdout,out mode))SetConsoleMode(stdout,mode|4u);
        var reader=new Thread(()=>{
            var buffer=new byte[256]; uint read;
            while(ReadFile(input,buffer,(uint)buffer.Length,out read,IntPtr.Zero)&&read>0){
                using(var file=new FileStream(inputPath,FileMode.Append,FileAccess.Write,FileShare.ReadWrite))file.Write(buffer,0,(int)read);
            }
        });
        reader.IsBackground=true; reader.Start();
        string frame=File.ReadAllText(Path.Combine(root,"frame.txt"),Encoding.UTF8);
        if(quiet)Write("BKS89_QUIET_READY\r\n");else Frame(frame);
        File.WriteAllText(Path.Combine(root,"ready-"+id+".txt"),quiet?"quiet":"dialog");
        var seen=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        while(true){
            var commands=Directory.GetFiles(root,"command-"+id+"-*.txt");Array.Sort(commands,StringComparer.Ordinal);
            foreach(string path in commands){
                if(!seen.Add(path))continue;
                string command=File.ReadAllText(path,Encoding.ASCII).Trim();
                if(command=="change")Frame("BKS89_CHANGED_SCREEN\r\nThe original fixture dialog has ended.\r\n");
                else if(command=="reset")Frame(frame);
                else if(command=="redraw")for(int i=0;i<60;i++){Frame(frame);Thread.Sleep(20);}
                else if(command.StartsWith("append:",StringComparison.Ordinal)){
                    string token=command.Substring(7);
                    Write("\r\n");
                    for(int i=1;i<=12;i++)Write("BKS89_"+token+"_ROW_"+i.ToString("D3")+"_Ж_"+new string('x',48)+"\r\n");
                }
                else if(command=="exit"){File.WriteAllText(path+".ack","exit");return 0;}
                else if(command=="exit7"){File.WriteAllText(path+".ack","exit7");return 7;}
                else return 5;
                File.WriteAllText(path+".ack",command);
            }
            Thread.Sleep(20);
        }
    }
}
'@

$runName = $Mode.ToLowerInvariant() + '-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0,8)
$runDirectory = (New-Item -ItemType Directory -Force -Path (Join-Path $Artifacts $runName)).FullName
$checks = [Collections.Generic.List[object]]::new()
$observations = [Collections.Generic.List[object]]::new()
$script:Case = $null
$script:CommandNumber = 0
$failure = $null
$templateExe = Join-Path $runDirectory 'fixture-template.exe'
$sourceFixture = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../crates/bks-app/tests/fixtures/tui/codex-0.164.0/official-folder-trust-sandbox-70x14.txt')).Path

function Check([string]$Name,[bool]$Condition) {
    $status = if($Condition){'passed'}else{'failed'}
    $checks.Add([ordered]@{Status=$status;Name=$Name;Case=$(if($script:Case){$script:Case.Name}else{'prepare'})})
    if (-not $Condition) { throw "FAILED: $Name" }
}
function Wait-For([scriptblock]$Condition,[string]$Name,[int]$Timeout=15000) {
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while($timer.ElapsedMilliseconds -lt $Timeout){
        if(& $Condition){return}
        if($script:Case -and $script:Case.Process.HasExited){throw "Own application exited: $Name"}
        Start-Sleep -Milliseconds 40
    }
    if($script:Case){
        $state=Foreground-State
        $observations.Add([ordered]@{Label='timeout foreground';Condition=$Name;Foreground=$state})
        throw ("Timeout: $Name; foreground pid="+$state.Pid+' class='+$state.Class+' root='+$state.Root+' expectedMain='+$state.ExpectedMain)
    }
    throw "Timeout: $Name"
}
function Foreground-State {
    $foreground=[BksStage89]::GetForegroundWindow()
    $root=[BksStage89]::GetAncestor($foreground,2)
    return @{Hwnd=$foreground.ToInt64();Root=$root.ToInt64();Pid=[BksStage89]::ProcessId($foreground);Class=[BksStage7]::ClassName($foreground);RootClass=[BksStage7]::ClassName($root);ExpectedMain=$script:Case.Main.ToInt64();ExpectedPid=$script:Case.Process.Id}
}
function Own-Foreground {
    return $script:Case.Main -ne [IntPtr]::Zero -and [BksStage89]::GetAncestor([BksStage89]::GetForegroundWindow(),2) -eq $script:Case.Main
}
function Own-Element([IntPtr]$Window) {
    [BksStage7]::Own($script:Case.Process.Id,$Window)
    [Windows.Automation.AutomationElement]::FromHandle($Window)
}
function Uia-Text([IntPtr]$Window) {
    $pattern=$null
    if(-not (Own-Element $Window).TryGetCurrentPattern([Windows.Automation.TextPattern]::Pattern,[ref]$pattern)){throw 'Own context has no TextPattern'}
    $pattern.DocumentRange.GetText(-1)
}
function Invoke-Own([IntPtr]$Window) {
    $pattern=$null
    if(-not (Own-Element $Window).TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern,[ref]$pattern)){throw 'Own action has no InvokePattern'}
    $pattern.Invoke()
}
function Focus-Own([IntPtr]$Window) {
    (Own-Element $Window).SetFocus()
    Wait-For { [BksStage89]::Focus($script:Case.Process.Id,$Window) -eq $Window } 'own native focus'
}
function Post-Command([int]$Command) {
    [BksStage89]::Post($script:Case.Process.Id,$script:Case.Main,0x111,$Command)
}
function Start-Case([string]$Name) {
    $directory=(New-Item -ItemType Directory -Force -Path (Join-Path $runDirectory $Name)).FullName
    $appData=(New-Item -ItemType Directory -Force -Path (Join-Path $directory 'appdata')).FullName
    $task=(New-Item -ItemType Directory -Force -Path (Join-Path $appData 'BKSStage89')).FullName
    $bin=(New-Item -ItemType Directory -Force -Path (Join-Path $appData 'npm/node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin')).FullName
    $mockExe=Join-Path $bin 'codex.exe'
    if(-not (Test-Path -LiteralPath $mockExe)){Copy-Item -LiteralPath $templateExe -Destination $mockExe}
    Copy-Item -LiteralPath $sourceFixture -Destination (Join-Path $task 'frame.txt') -Force
    $profilesDirectory=(New-Item -ItemType Directory -Force -Path (Join-Path $appData 'BKS Term')).FullName
    $profiles=@(
        [ordered]@{id='codex-tui-0.164.0';name='Stage89 Codex fixture';kind='local';executable=$mockExe;args=@();working_directory=$task},
        [ordered]@{id='stage89-quiet';name='Stage89 quiet fixture';kind='local';executable=$mockExe;args=@('--quiet');working_directory=$task})
    $json=[ordered]@{schema_version=1;default_profile_id='codex-tui-0.164.0';profiles=$profiles}|ConvertTo-Json -Depth 5
    [IO.File]::WriteAllText((Join-Path $profilesDirectory 'profiles.json'),$json,[Text.UTF8Encoding]::new($false))
    $start=[Diagnostics.ProcessStartInfo]::new($Exe)
    $start.UseShellExecute=$false
    $start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.WorkingDirectory=$directory
    $start.Arguments='--profile codex-tui-0.164.0'
    $start.EnvironmentVariables['APPDATA']=$appData
    $process=[Diagnostics.Process]::Start($start)
    $script:Case=@{Name=$Name;AppData=$appData;Task=$task;MockExe=$mockExe;Process=$process;Main=[IntPtr]::Zero;Mocks=[Collections.Generic.List[object]]::new();Primary=0}
    Wait-For { [BksStage7]::MainWindow($process.Id) -ne [IntPtr]::Zero } 'own main window'
    $script:Case.Main=[BksStage7]::MainWindow($process.Id)
    Wait-For { [BksStage7]::GetDlgItem($script:Case.Main,1002) -ne [IntPtr]::Zero } 'own controls initialized'
    [BksStage89]::Activate($process.Id,$script:Case.Main)
    Wait-For { Own-Foreground } 'own foreground root for UIA notifications'
    Wait-For {
        $new=@(Get-ChildItem -LiteralPath $task -Filter 'ready-*.txt' | Where-Object {
            $id=[int]($_.BaseName.Substring(6))
            try{$candidate=[Diagnostics.Process]::GetProcessById($id); -not $candidate.HasExited -and $candidate.MainModule.FileName -eq $mockExe -and [IO.File]::ReadAllText($_.FullName) -eq 'dialog'}catch{$false}
        })
        $new.Count -gt 0
    } 'own compiled CLI fixture ready'
    $ready=Get-ChildItem -LiteralPath $task -Filter 'ready-*.txt' | Where-Object { [IO.File]::ReadAllText($_.FullName) -eq 'dialog' } | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    $script:Case.Primary=[int]($ready.BaseName.Substring(6))
    Register-Mock $script:Case.Primary
    Wait-For { [BksStage7]::Text($process.Id,[BksStage7]::GetDlgItem($script:Case.Main,1001)).Contains('Folder access') } 'source-derived fixture reaches own terminal'
    $observations.Add([ordered]@{Case=$Name;ApplicationPid=$process.Id;FixturePid=$script:Case.Primary;MockExecutable=$mockExe;AppData=$appData})
}
function Register-Mock([int]$FixturePid) {
    $mock=[Diagnostics.Process]::GetProcessById($FixturePid)
    if($mock.MainModule.FileName -ne $script:Case.MockExe){throw 'Fixture process is outside own APPDATA'}
    if(-not @($script:Case.Mocks|Where-Object{$_.Id -eq $FixturePid}).Count){$script:Case.Mocks.Add($mock)}
}
function Send-Fixture([string]$Command,[int]$FixturePid=0) {
    if($FixturePid -eq 0){$FixturePid=$script:Case.Primary}
    Register-Mock $FixturePid
    $script:CommandNumber++
    $path=Join-Path $script:Case.Task ('command-{0}-{1:D5}.txt' -f $FixturePid,$script:CommandNumber)
    [IO.File]::WriteAllText(($path+'.pending'),$Command,[Text.Encoding]::ASCII)
    [IO.File]::Move(($path+'.pending'),$path)
    Wait-For { Test-Path -LiteralPath ($path+'.ack') } "own fixture acknowledgement: $Command"
}
function Input-Hex {
    $path=Join-Path $script:Case.Task ('input-'+$script:Case.Primary+'.bin')
    $file=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
    try{$bytes=[byte[]]::new($file.Length);[void]$file.Read($bytes,0,$bytes.Length)}finally{$file.Dispose()}
    ([BitConverter]::ToString($bytes)).Replace('-','').ToLowerInvariant()
}
function Stop-Case {
    if(-not $script:Case){return}
    [BksStage89]::Stop()
    $case=$script:Case
    foreach($mock in $case.Mocks){
        if($mock.HasExited){continue}
        if($mock.MainModule.FileName -ne $case.MockExe){throw 'Cleanup refused a fixture outside own APPDATA'}
        $script:CommandNumber++
        $path=Join-Path $case.Task ('command-{0}-{1:D5}.txt' -f $mock.Id,$script:CommandNumber)
        [IO.File]::WriteAllText(($path+'.pending'),'exit',[Text.Encoding]::ASCII)
        [IO.File]::Move(($path+'.pending'),$path)
    }
    if(-not $case.Process.HasExited){
        $ownMain=[BksStage7]::MainWindow($case.Process.Id)
        if($ownMain -ne [IntPtr]::Zero){[BksStage89]::Post($case.Process.Id,$ownMain,0x10,0)}
        if(-not $case.Process.WaitForExit(10000)){$case.Process.Kill();$checks.Add([ordered]@{Status='failed';Name='Own application required forced cleanup';Case=$case.Name})}
    }
    foreach($mock in $case.Mocks){
        if(-not $mock.WaitForExit(3000)){
            if($mock.MainModule.FileName -ne $case.MockExe){throw 'Cleanup refused an unknown executable'}
            $mock.Kill();$checks.Add([ordered]@{Status='failed';Name='Own fixture required forced cleanup';Case=$case.Name})
        }
    }
    Check 'own application and known fixture processes closed' ($case.Process.HasExited -and @($case.Mocks|Where-Object{-not $_.HasExited}).Count -eq 0)
    $script:Case=$null
}
function Open-Tui {
    Post-Command 60014
    Wait-For {
        $dialog=[BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermTuiDialog')
        $dialog -ne [IntPtr]::Zero -and [BksStage7]::GetDlgItem($dialog,4403) -ne [IntPtr]::Zero
    } 'explicit native TUI dialog'
    [BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermTuiDialog')
}
function Wait-TuiClosed {
    Wait-For { [BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermTuiDialog') -eq [IntPtr]::Zero } 'own TUI dialog closes'
}
function Invoke-Tui {
    Start-Case 'tui-cancel-stale'
    $origin=[BksStage89]::Focus($script:Case.Process.Id,$script:Case.Main)
    Start-Sleep -Milliseconds 3200
    Check 'recognition opens no window and preserves focus' ([BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermTuiDialog') -eq [IntPtr]::Zero -and [BksStage89]::Focus($script:Case.Process.Id,$script:Case.Main) -eq $origin)
    Check 'detection sends no CLI input' ((Input-Hex) -eq '')
    $dialog=Open-Tui
    $context=[BksStage7]::GetDlgItem($dialog,4400)
    $options=[BksStage7]::GetDlgItem($dialog,4401)
    $contextText=Uia-Text $context
    $observations.Add([ordered]@{Case=$script:Case.Name;Label='native TUI context';Dialog=$dialog.ToInt64();ContextHandle=$context.ToInt64();Context=$contextText})
    Check 'context is named read-only native EDIT with TextPattern' ([BksStage7]::ClassName($context) -ieq 'Edit' -and [BksStage7]::ReadOnly($script:Case.Process.Id,$context) -and (Own-Element $context).Current.Name.Length -gt 0)
    foreach($part in @('Folder access','/workspace/project','Folder settings can run code','Trust and continue','Quit','enter continue and create sandbox')){Check "full context retains $part" ($contextText.Contains($part))}
    $selection=$null
    Check 'native option list exposes SelectionPattern' ([BksStage7]::ClassName($options) -ieq 'ListBox' -and (Own-Element $options).TryGetCurrentPattern([Windows.Automation.SelectionPattern]::Pattern,[ref]$selection))
    $items=(Own-Element $options).FindAll([Windows.Automation.TreeScope]::Children,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::ListItem))
    Check 'both native choices and original selection are accessible' ($items.Count -eq 2 -and $items[0].Current.Name -eq 'Trust and continue' -and $items[1].Current.Name -eq 'Quit' -and [BksStage89]::Selected($script:Case.Process.Id,$options) -eq 0)
    Check 'opening focuses the reading context' ([BksStage89]::Focus($script:Case.Process.Id,$dialog) -eq $context)
    [BksStage89]::Key($script:Case.Process.Id,$context,13)
    Start-Sleep -Milliseconds 200
    Check 'Enter while reading does not apply or close' ([BksStage7]::IsWindowVisible($dialog) -and (Input-Hex) -eq '')
    Focus-Own $options
    [BksStage89]::Key($script:Case.Process.Id,$options,40)
    Wait-For { [BksStage89]::Selected($script:Case.Process.Id,$options) -eq 1 } 'native Down changes selection'
    Check 'selection is exposed through UIA without writing bytes' ($selection.Current.GetSelection()[0].Current.Name -eq 'Quit' -and (Input-Hex) -eq '')
    [BksStage89]::Key($script:Case.Process.Id,$options,27)
    Wait-TuiClosed
    $observations.Add([ordered]@{Case=$script:Case.Name;Label='focus immediately after hidden dialog';Origin=$origin.ToInt64();AfterClose=[BksStage89]::Focus($script:Case.Process.Id,$script:Case.Main).ToInt64();InputHex=Input-Hex})
    # DestroyWindow makes the HWND invisible before finish() restores focus.
    # Wait for that operation to complete instead of sampling its midpoint.
    Wait-For { [BksStage89]::Focus($script:Case.Process.Id,$script:Case.Main) -eq $origin } 'source focus restored after native dialog destruction'
    Check 'Escape returns focus and sends no CLI bytes' ((Input-Hex) -eq '' -and [BksStage89]::Focus($script:Case.Process.Id,$script:Case.Main) -eq $origin)
    $dialog=Open-Tui
    Invoke-Own ([BksStage7]::GetDlgItem($dialog,4403))
    Wait-TuiClosed
    Check 'explicit Close sends no CLI bytes' ((Input-Hex) -eq '')
    $dialog=Open-Tui
    $context=[BksStage7]::GetDlgItem($dialog,4400)
    $oldContext=Uia-Text $context
    $apply=[BksStage7]::GetDlgItem($dialog,4402)
    $options=[BksStage7]::GetDlgItem($dialog,4401)
    $status=[BksStage7]::GetDlgItem($dialog,4404)
    [BksStage89]::Watch($script:Case.Process.Id,$status)
    Send-Fixture 'change'
    Wait-For { -not [BksStage7]::IsWindowEnabled($apply) -and -not [BksStage7]::IsWindowEnabled($options) } 'changed source disables stale actions'
    Wait-For { @([BksStage89]::Notices()|Where-Object{$_.Text.Contains('устарел')}).Count -gt 0 } 'stale reason raises an actual UIA Notification'
    Check 'stale context is preserved and apply is disabled through UIA' ((Uia-Text $context) -eq $oldContext -and -not (Own-Element $apply).Current.IsEnabled -and (Own-Element $status).Current.Name.Contains('устарел'))
    # Simulate a button command already queued before the control was disabled.
    [BksStage89]::Post($script:Case.Process.Id,$dialog,0x111,4402)
    Start-Sleep -Milliseconds 300
    Check 'queued stale apply cannot produce CLI input or close the window' ((Input-Hex) -eq '' -and [BksStage7]::IsWindowVisible($dialog))
    $observations.Add([ordered]@{Case=$script:Case.Name;Context=$oldContext;StaleNotifications=[BksStage89]::Notices();InputHex=Input-Hex})
    [BksStage89]::Stop()
    Invoke-Own ([BksStage7]::GetDlgItem($dialog,4403))
    Wait-TuiClosed
    Stop-Case

    Start-Case 'tui-apply'
    Start-Sleep -Milliseconds 3200
    $dialog=Open-Tui
    Check 'fresh dialog starts with no automatic input' ((Input-Hex) -eq '')
    Invoke-Own ([BksStage7]::GetDlgItem($dialog,4402))
    Wait-TuiClosed
    Wait-For { (Input-Hex) -eq '310d' } 'explicit first choice reaches only the fake CLI as 1 plus CR'
    Start-Sleep -Milliseconds 500
    Check 'explicit choice is sent once with no extra bytes' ((Input-Hex) -eq '310d')
    $observations.Add([ordered]@{Case=$script:Case.Name;InputHex=Input-Hex;Meaning='ASCII 1 followed by carriage return; mock CLI only'})
    Stop-Case
}
function Watch-Output {
    [BksStage89]::Watch($script:Case.Process.Id,[BksStage7]::GetDlgItem($script:Case.Main,1026))
}
function Token-Notices([string]$Token) {
    @([BksStage89]::Notices()|Where-Object{$_.Text.Contains('BKS89_'+$Token+'_')})
}
function Set-Speech([int]$Command,[string]$Value) {
    Post-Command $Command
    $path=Join-Path $script:Case.AppData 'BKS Term/settings.json'
    Wait-For { (Test-Path -LiteralPath $path) -and (([IO.File]::ReadAllText($path)|ConvertFrom-Json).speech_mode -eq $Value) } "speech mode $Value saved"
    Check "menu $Command persists speech_mode=$Value" (([IO.File]::ReadAllText($path)|ConvertFrom-Json).speech_mode -eq $Value)
    Wait-OutputReady
}
function Wait-OutputReady {
    # Explicit UI feedback mutes automatic output for 1000 ms in production.
    # Start the fixture only after the actual caption is stable for that span,
    # and require the foreground condition the output policy intentionally uses.
    [BksStage89]::Activate($script:Case.Process.Id,$script:Case.Main)
    $readyState=@{Caption='';Since=[DateTime]::UtcNow}
    Wait-For {
        if(-not (Own-Foreground)){return $false}
        $caption=[BksStage7]::Text($script:Case.Process.Id,[BksStage7]::GetDlgItem($script:Case.Main,1026))
        if($caption -ne $readyState.Caption){$readyState.Caption=$caption;$readyState.Since=[DateTime]::UtcNow;return $false}
        ([DateTime]::UtcNow-$readyState.Since).TotalMilliseconds -ge 1200
    } 'own foreground and stable explicit feedback before automatic output'
}
function Append-Check([string]$Token,[bool]$Expected) {
    Wait-OutputReady
    Send-Fixture ('append:'+$Token)
    if($Expected){Wait-For { (Token-Notices $Token).Count -gt 0 } "UIA reports appended $Token rows"}
    Start-Sleep -Milliseconds 2800
    $events=@(Token-Notices $Token)
    Check "automatic output $Token matches persisted mode" (($events.Count -gt 0) -eq $Expected)
    if($Expected){
        Check 'UIA output notifications have bounded count and length' ($events.Count -le 4 -and @($events|Where-Object{$_.Text.Length -gt 480}).Count -eq 0)
        $gaps=@(for($i=1;$i -lt $events.Count;$i++){$events[$i].AtMs-$events[$i-1].AtMs})
        Check 'UIA output batches are spaced by at least 800 ms' (@($gaps|Where-Object{$_ -lt 800}).Count -eq 0)
    }
    $observations.Add([ordered]@{Case=$script:Case.Name;Token=$Token;ExpectedAutomatic=$Expected;Events=$events})
}
function Open-QuietTab {
    # Post BM_CLICK so the popup's nested menu loop cannot block this client.
    [BksStage89]::Post($script:Case.Process.Id,[BksStage7]::GetDlgItem($script:Case.Main,1010),0xF5,0)
    $choice=@{Element=$null}
    Wait-For {
        $popup=[BksStage7]::VisibleClass($script:Case.Process.Id,'#32768')
        if($popup -eq [IntPtr]::Zero){return $false}
        $choice.Element=(Own-Element $popup).FindFirst([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty,'Stage89 quiet fixture'))
        $null -ne $choice.Element
    } 'own profile popup exposes quiet fixture'
    $pattern=$null
    if(-not $choice.Element.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern,[ref]$pattern)){throw 'Quiet fixture menu has no InvokePattern'}
    $pattern.Invoke()
    Wait-For { [BksStage7]::GetDlgItem($script:Case.Main,2002) -ne [IntPtr]::Zero -and [BksStage7]::Text($script:Case.Process.Id,[BksStage7]::GetDlgItem($script:Case.Main,1001)).Contains('BKS89_QUIET_READY') } 'own second tab ready'
    $ready=Get-ChildItem -LiteralPath $script:Case.Task -Filter 'ready-*.txt'|Where-Object{[IO.File]::ReadAllText($_.FullName) -eq 'quiet'}|Sort-Object LastWriteTime -Descending|Select-Object -First 1
    Register-Mock ([int]$ready.BaseName.Substring(6))
}
function Invoke-Notifications {
    foreach($modeSpec in @(@(60011,'all',$true),@(60012,'important',$false),@(60013,'manual',$false))){
        Start-Case 'notifications-persistence'
        Watch-Output
        Set-Speech $modeSpec[0] $modeSpec[1]
        Stop-Case
        Start-Case 'notifications-persistence'
        Watch-Output
        Append-Check ('PERSIST_'+$modeSpec[1].ToUpperInvariant()) $modeSpec[2]
        Check 'mode remains serialized after a fresh application launch' (([IO.File]::ReadAllText((Join-Path $script:Case.AppData 'BKS Term/settings.json'))|ConvertFrom-Json).speech_mode -eq $modeSpec[1])
        Stop-Case
    }
    Start-Case 'notifications-redraw-background'
    Watch-Output
    Set-Speech 60011 'all'
    Append-Check 'NEWLINES' $true
    # Establish the same frame first. Returning from a log to Folder access is
    # a genuine semantic transition in stage9, separate from repeated redraws.
    Send-Fixture 'reset'
    Start-Sleep -Milliseconds 1400
    $before=[BksStage89]::Notices().Length
    Send-Fixture 'redraw'
    Start-Sleep -Milliseconds 1800
    Check 'sixty identical fixture redraws emit no new UIA Notifications' ([BksStage89]::Notices().Length -eq $before)
    Open-QuietTab
    Start-Sleep -Milliseconds 800
    $before=[BksStage89]::Notices().Length
    Send-Fixture 'append:BACKGROUND' $script:Case.Primary
    Start-Sleep -Milliseconds 2200
    Check 'background tab output produces no automatic UIA Notification' ([BksStage89]::Notices().Length -eq $before -and (Token-Notices 'BACKGROUND').Count -eq 0)
    Post-Command 2001
    Wait-For { [BksStage7]::Text($script:Case.Process.Id,[BksStage7]::GetDlgItem($script:Case.Main,1001)).Contains('BKS89_BACKGROUND_ROW_') } 'background output remains available for explicit reading'
    Start-Sleep -Milliseconds 1400
    Check 'returning to a tab does not replay its background backlog' ((Token-Notices 'BACKGROUND').Count -eq 0)
    Set-Speech 60012 'important'
    Send-Fixture 'exit7'
    Wait-For { @([BksStage89]::Notices()|Where-Object{$_.Text.Contains('сессия завершена') -and $_.Text.Contains('7')}).Count -gt 0 } 'important mode reports fixture exit code 7'
    Start-Sleep -Milliseconds 1400
    Check 'important completion raises one UIA Notification' (@([BksStage89]::Notices()|Where-Object{$_.Text.Contains('сессия завершена') -and $_.Text.Contains('7')}).Count -eq 1)
    $observations.Add([ordered]@{Case=$script:Case.Name;Events=[BksStage89]::Notices();InputHex=Input-Hex})
    Stop-Case
}

try {
    Add-Type -ReferencedAssemblies $references -TypeDefinition ($nativeSource+"`r`n"+$extensionSource)
    [BksStage7]::InitializeUia()
    [IO.File]::WriteAllText((Join-Path $runDirectory 'fixture-source.cs'),$fixtureSource,[Text.UTF8Encoding]::new($false))
    Add-Type -TypeDefinition $fixtureSource -OutputAssembly $templateExe -OutputType ConsoleApplication
    Check 'native/UIA helpers and hermetic mock compile' (Test-Path -LiteralPath $templateExe)
    if($PrepareOnly){
        $checks.Add([ordered]@{Status='skipped';Name='PrepareOnly: no application or fixture process launched';Case='prepare'})
    }else{
        $Exe=(Resolve-Path -LiteralPath $Exe).Path
        if($Mode -in @('Tui','All')){Invoke-Tui}
        if($Mode -in @('Notifications','All')){Invoke-Notifications}
    }
}catch{
    $failure=$_.Exception.Message
    if(-not $failure.StartsWith('FAILED:')){$checks.Add([ordered]@{Status='error';Name=$failure;Case=$(if($script:Case){$script:Case.Name}else{'prepare'})})}
    $observations.Add([ordered]@{Label='failure location';Stack=$_.ScriptStackTrace;Foreground=$(if($script:Case){Foreground-State}else{$null})})
    if($script:Case -and -not $script:Case.Process.HasExited){
        try{
            $dialog=[BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermTuiDialog')
            $journal=[BksStage7]::Text($script:Case.Process.Id,[BksStage7]::GetDlgItem($script:Case.Main,1001))
            $observations.Add([ordered]@{Label='own HWND state before cleanup';ApplicationPid=$script:Case.Process.Id;Main=$script:Case.Main.ToInt64();Tui=$dialog.ToInt64();ForegroundIsMain=([BksStage89]::GetForegroundWindow() -eq $script:Case.Main);ForegroundIsTui=([BksStage89]::GetForegroundWindow() -eq $dialog);Focus=[BksStage89]::Focus($script:Case.Process.Id,$script:Case.Main).ToInt64();Connection=[BksStage7]::Text($script:Case.Process.Id,[BksStage7]::GetDlgItem($script:Case.Main,1005));Announcement=[BksStage7]::Text($script:Case.Process.Id,[BksStage7]::GetDlgItem($script:Case.Main,1026));KnownFixtureTail=$journal.Substring([Math]::Max(0,$journal.Length-2000));InputHex=Input-Hex;DialogTree=$(if($dialog -ne [IntPtr]::Zero){[BksStage7]::DialogTree($script:Case.Process.Id,$dialog)}else{@()});Events=[BksStage89]::Notices()})
        }catch{$observations.Add([ordered]@{Label='own failure evidence unavailable';Reason=$_.Exception.Message})}
    }
}finally{
    try{Stop-Case}catch{$checks.Add([ordered]@{Status='error';Name=('Own cleanup: '+$_.Exception.Message);Case='cleanup'})}
    $counts=[ordered]@{passed=@($checks|Where-Object{$_.Status -eq 'passed'}).Count;failed=@($checks|Where-Object{$_.Status -eq 'failed'}).Count;skipped=@($checks|Where-Object{$_.Status -eq 'skipped'}).Count;error=@($checks|Where-Object{$_.Status -eq 'error'}).Count}
    [ordered]@{Scope='Own PID/APPDATA; hermetic source-derived fixture only; actual UIA events, not NVDA speech or live Codex acceptance';Mode=$Mode;PreparedOnly=[bool]$PrepareOnly;Executable=$Exe;Counts=$counts;Checks=$checks.ToArray();Observations=$observations.ToArray()}|ConvertTo-Json -Depth 8|Set-Content -LiteralPath (Join-Path $runDirectory 'summary.json') -Encoding utf8
    $checks|ForEach-Object{"$($_.Status): $($_.Name)"}|Set-Content -LiteralPath (Join-Path $runDirectory 'results.txt') -Encoding utf8
}
Write-Output "Stage89 ${Mode}: passed=$($counts.passed) failed=$($counts.failed) skipped=$($counts.skipped) error=$($counts.error)"
Write-Output "Artifacts: $runDirectory"
if($failure -or $counts.failed -or $counts.error){throw $(if($failure){$failure}else{'Stage89 checks or cleanup failed'})}
