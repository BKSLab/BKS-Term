[CmdletBinding()]
param(
    [string]$Exe,
    [string]$Artifacts,
    [switch]$PrepareOnly,
    [switch]$BaselineOnly
)

# The only input recipients are HWNDs of the application started below. No
# SendInput, global keys, clipboard, real CLI, accounts, network or approvals.
$ErrorActionPreference = 'Stop'
# Windows PowerShell can evaluate parameter defaults before PSScriptRoot is
# populated. Resolve script-relative defaults only after entering the body.
if (-not $Exe) { $Exe=Join-Path $PSScriptRoot '../target/release/bks-app.exe' }
if (-not $Artifacts) { $Artifacts=Join-Path $PSScriptRoot '../target/direct-accessibility-check' }
if ($PSVersionTable.PSEdition -eq 'Core' -or [Threading.Thread]::CurrentThread.GetApartmentState() -ne 'MTA') {
    $arguments = @('-NoProfile','-Mta','-ExecutionPolicy','Bypass','-File',$PSCommandPath,'-Exe',$Exe,'-Artifacts',$Artifacts)
    if ($PrepareOnly) { $arguments += '-PrepareOnly' }
    if ($BaselineOnly) { $arguments += '-BaselineOnly' }
    & "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" @arguments
    exit $LASTEXITCODE
}
. (Join-Path $PSScriptRoot '../packaging/common.ps1')
$artifactRoot = Get-BksFullPath $Artifacts
Assert-BksNoReparse $artifactRoot
$runDirectory = Join-Path $artifactRoot ('run-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0,8))
$null = [IO.Directory]::CreateDirectory($runDirectory)
$checks = New-Object 'System.Collections.Generic.List[object]'
$observations = New-Object 'System.Collections.Generic.List[object]'
$script:Case = $null
$script:ExpectedInput = ''
$failure = $null

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, UIAutomationClientsideProviders
$references = @([Windows.Automation.AutomationElement].Assembly.Location,
    [Windows.Automation.AutomationPattern].Assembly.Location,
    [UIAutomationClientsideProviders.UIAutomationClientSideProviders].Assembly.Location)
# Compile only the existing own-PID helpers; never execute another QA scenario.
$shared = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'check-stage7.ps1'), [Text.Encoding]::UTF8)
$nativeSource = [regex]::Match($shared, "(?s)using System;.*?(?=\r?\n'@)").Value
if (-not $nativeSource.Contains('public static class BksStage7')) { throw 'Owned HWND helper unavailable.' }
$extension = @'
public static class BksDirect {
    public const string DirectReturnName="\u0412\u0435\u0440\u043d\u0443\u0442\u044c\u0441\u044f \u043a \u0443\u043f\u0440\u0430\u0432\u043b\u0435\u043d\u0438\u044e TUI";
    public const string OverviewName="\u041e\u0431\u0437\u043e\u0440 \u044d\u043a\u0440\u0430\u043d\u0430 TUI";
    public const string StaleOverview="\u042d\u043a\u0440\u0430\u043d \u043e\u0431\u043d\u043e\u0432\u0438\u043b\u0441\u044f. \u0421\u043d\u0438\u043c\u043e\u043a \u0441\u043e\u0445\u0440\u0430\u043d\u0451\u043d \u0434\u043b\u044f \u0447\u0442\u0435\u043d\u0438\u044f.";
    public const string CaptureOnNotice="\u041f\u0440\u044f\u043c\u043e\u0439 \u0432\u0432\u043e\u0434. \u041a\u043b\u0430\u0432\u0438\u0448\u0438 \u043f\u0435\u0440\u0435\u0434\u0430\u044e\u0442\u0441\u044f \u043f\u0440\u0438\u043b\u043e\u0436\u0435\u043d\u0438\u044e. Ctrl+Shift+Space \u2014 \u0447\u0438\u0442\u0430\u0442\u044c \u044d\u043a\u0440\u0430\u043d, F1 \u2014 \u0441\u043f\u0440\u0430\u0432\u043a\u0430";
    public const string CaptureOffNotice="\u0417\u0430\u0445\u0432\u0430\u0442 \u043a\u043b\u0430\u0432\u0438\u0430\u0442\u0443\u0440\u044b \u0432\u044b\u043a\u043b\u044e\u0447\u0435\u043d. Tab \u043f\u0435\u0440\u0435\u043a\u043b\u044e\u0447\u0430\u0435\u0442 \u044d\u043b\u0435\u043c\u0435\u043d\u0442\u044b \u043e\u043a\u043d\u0430";
    public const string DirectName="\u0422\u0435\u0440\u043c\u0438\u043d\u0430\u043b \u2014 \u043f\u0440\u044f\u043c\u043e\u0439 \u0432\u0432\u043e\u0434";
    private const int KeyUpData=unchecked((int)0xc0000001);
    [DllImport("user32.dll")] private static extern bool PostMessageW(IntPtr window,uint message,IntPtr wparam,IntPtr lparam);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window,out uint process);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr window,uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr window);
    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] private static extern bool GetGUIThreadInfo(uint thread,ref Gui info);
    [DllImport("user32.dll",EntryPoint="GetWindowLongPtrW")] private static extern IntPtr WindowStyle(IntPtr window,int index);
    [StructLayout(LayoutKind.Sequential)] private struct Message {
        public IntPtr Hwnd; public uint Id; public UIntPtr Wparam; public IntPtr Lparam;
        public uint Time; public int X,Y; public uint Private;
    }
    [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern IntPtr CreateWindowExW(uint ex,string cls,string title,uint style,int x,int y,int width,int height,IntPtr parent,IntPtr menu,IntPtr instance,IntPtr param);
    [DllImport("user32.dll")] private static extern bool DestroyWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern bool TranslateMessage(ref Message message);
    [DllImport("user32.dll")] private static extern bool PeekMessageW(out Message message,IntPtr window,uint first,uint last,uint remove);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] private struct Gui {
        public uint Size,Flags; public IntPtr Active,Focus,Capture,Menu,MoveSize,Caret; public Rect CaretRect;
    }
    public static IntPtr Focus(uint process,IntPtr window) {
        BksStage7.Own(process,window); uint actual; uint thread=GetWindowThreadProcessId(window,out actual);
        var info=new Gui(); info.Size=(uint)Marshal.SizeOf(typeof(Gui));
        if(!GetGUIThreadInfo(thread,ref info))throw new InvalidOperationException("Owned focus query failed");
        return info.Focus;
    }
    public static uint ForegroundProcess() {
        uint process; GetWindowThreadProcessId(GetForegroundWindow(),out process); return process;
    }
    public static Rect Bounds(uint process,IntPtr window) {
        BksStage7.Own(process,window); Rect value;
        if(!GetWindowRect(window,out value))throw new InvalidOperationException("Owned bounds query failed");
        return value;
    }
    public static bool Multiline(uint process,IntPtr window) {
        BksStage7.Own(process,window); return (WindowStyle(window,-16).ToInt64() & 4) != 0;
    }
    public static void Post(uint process,IntPtr window,uint message,int value) {
        BksStage7.Own(process,window);
        if(!PostMessageW(window,message,new IntPtr(value),IntPtr.Zero))throw new InvalidOperationException("Owned post failed");
    }
    public static void Key(uint process,IntPtr window,int key) {
        BksStage7.Own(process,window);
        if(Focus(process,window)!=window)throw new InvalidOperationException("Owned key target lacks focus");
        // WM_KEYUP needs its previous/transition bits. LPARAM=0 is interpreted
        // by TranslateMessage as another key press and generates a spurious CR.
        if(!PostMessageW(window,0x100,new IntPtr(key),new IntPtr(1)) ||
           !PostMessageW(window,0x101,new IntPtr(key),new IntPtr(KeyUpData)))throw new InvalidOperationException("Owned keyboard post failed");
    }
    public static void Character(uint process,IntPtr window,char value) {
        BksStage7.Own(process,window);
        if(Focus(process,window)!=window)throw new InvalidOperationException("Owned character target lacks focus");
        if(!PostMessageW(window,0x102,new IntPtr(value),new IntPtr(1)))throw new InvalidOperationException("Owned character post failed");
    }
    public sealed class KeyUpObservation {
        public bool MalformedKeyUpGeneratedReturn { get; set; }
        public bool NativeKeyUpGeneratedReturn { get; set; }
    }
    public static KeyUpObservation ProbeKeyUp() {
        // Invisible message-only HWND in this helper process. No application
        // launch, focus, physical/global keyboard state or other HWND changes.
        var window=CreateWindowExW(0,"STATIC","owned-keyup-contract",0,0,0,0,0,new IntPtr(-3),IntPtr.Zero,IntPtr.Zero,IntPtr.Zero);
        if(window==IntPtr.Zero)throw new InvalidOperationException("Owned message-only HWND creation failed");
        try {
            var result=new KeyUpObservation();
            foreach(int data in new[]{0,KeyUpData}) {
                Message discard;
                while(PeekMessageW(out discard,window,0x102,0x102,1)){}
                var keyup=new Message{Hwnd=window,Id=0x101,Wparam=new UIntPtr(13),Lparam=new IntPtr(data)};
                TranslateMessage(ref keyup);
                Message character;
                bool generated=PeekMessageW(out character,window,0x102,0x102,1) && character.Wparam.ToUInt64()==13;
                if(data==0)result.MalformedKeyUpGeneratedReturn=generated;
                else result.NativeKeyUpGeneratedReturn=generated;
            }
            return result;
        } finally {DestroyWindow(window);}
    }
    public sealed class Caret {
        public bool Collapsed { get; set; } public string Prefix { get; set; }
    }
    public static Caret UiaCaret(uint process,IntPtr window) {
        BksStage7.Own(process,window);
        var element=System.Windows.Automation.AutomationElement.FromHandle(window);
        object candidate;
        if(!element.TryGetCurrentPattern(System.Windows.Automation.TextPattern.Pattern,out candidate))
            throw new InvalidOperationException("Owned EDIT lacks TextPattern");
        var pattern=(System.Windows.Automation.TextPattern)candidate;
        var ranges=pattern.GetSelection();
        if(ranges.Length!=1)throw new InvalidOperationException("Owned EDIT must expose one selection/caret");
        var prefix=pattern.DocumentRange.Clone();
        prefix.MoveEndpointByRange(System.Windows.Automation.Text.TextPatternRangeEndpoint.End,ranges[0],System.Windows.Automation.Text.TextPatternRangeEndpoint.Start);
        return new Caret { Collapsed=ranges[0].CompareEndpoints(System.Windows.Automation.Text.TextPatternRangeEndpoint.Start,ranges[0],System.Windows.Automation.Text.TextPatternRangeEndpoint.End)==0, Prefix=prefix.GetText(-1) };
    }
    public sealed class Notice {
        public int Sequence { get; set; } public string ReceivedUtc { get; set; }
        public string Text { get; set; } public string Kind { get; set; }
        public string Processing { get; set; } public string Activity { get; set; }
    }
    private static readonly object noticeLock=new object();
    private static readonly List<Notice> notices=new List<Notice>();
    private static System.Windows.Automation.AutomationElement noticeSource;
    private static System.Windows.Automation.AutomationEventHandler noticeHandler;
    public static void Watch(uint process,IntPtr window) {
        BksStage7.Own(process,window);
        noticeSource=System.Windows.Automation.AutomationElement.FromHandle(window);
        noticeHandler=(sender,args)=>{
            var notification=args as System.Windows.Automation.NotificationEventArgs;
            if(notification==null)return;
            lock(noticeLock)notices.Add(new Notice { Sequence=notices.Count+1,ReceivedUtc=DateTime.UtcNow.ToString("o"),Text=notification.DisplayString,Kind=notification.NotificationKind.ToString(),Processing=notification.NotificationProcessing.ToString(),Activity=notification.ActivityId });
        };
        System.Windows.Automation.Automation.AddAutomationEventHandler(System.Windows.Automation.AutomationElementIdentifiers.NotificationEvent,noticeSource,System.Windows.Automation.TreeScope.Element,noticeHandler);
    }
    public static Notice[] Notices() { lock(noticeLock)return notices.ToArray(); }
    public static void Unwatch() {
        if(noticeSource==null || noticeHandler==null)return;
        System.Windows.Automation.Automation.RemoveAutomationEventHandler(System.Windows.Automation.AutomationElementIdentifiers.NotificationEvent,noticeSource,noticeHandler);
        noticeSource=null; noticeHandler=null;
    }
}
'@

function Check([string]$Name, [bool]$Condition, [switch]$ContinueOnFailure) {
    $status = if ($Condition) { 'passed' } else { 'failed' }
    $checks.Add([ordered]@{Status=$status;Name=$Name})
    if (-not $Condition -and -not $ContinueOnFailure) { throw "FAILED: $Name" }
}
function Wait-For([scriptblock]$Condition, [string]$Name, [int]$Timeout=12000) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.ElapsedMilliseconds -lt $Timeout) {
        if (& $Condition) { return }
        if ($script:Case -and $script:Case.Process.HasExited) { throw "Owned app exited: $Name" }
        Start-Sleep -Milliseconds 40
    }
    if ($script:Case) { Assert-ExpectedInput $script:ExpectedInput ('timeout: '+$Name) }
    throw "Timeout: $Name"
}
function Field([IntPtr]$Parent, [int]$Id) {
    [BksStage7]::Own($script:Case.Process.Id,$Parent)
    $window = [BksStage7]::GetDlgItem($Parent,$Id)
    [BksStage7]::Own($script:Case.Process.Id,$window)
    return $window
}
function Element([IntPtr]$Window) {
    [BksStage7]::Own($script:Case.Process.Id,$Window)
    return [Windows.Automation.AutomationElement]::FromHandle($Window)
}
function Own-Text([IntPtr]$Window) { return [BksStage7]::Text($script:Case.Process.Id,$Window) }
function Normalize-Text([string]$Text) { return $Text.Replace("`r`n","`n").Replace("`r","`n") }
function Same-UiaText([string]$Uia, [string]$Native) { return (Normalize-Text $Uia).TrimEnd("`n") -ceq (Normalize-Text $Native).TrimEnd("`n") }
function Rect-Key($Rect) { return @($Rect.Left,$Rect.Top,$Rect.Right,$Rect.Bottom) -join ',' }
function Uia-Text([IntPtr]$Window) {
    $pattern = $null
    if (-not (Element $Window).TryGetCurrentPattern([Windows.Automation.TextPattern]::Pattern,[ref]$pattern)) { throw 'Owned EDIT lacks TextPattern.' }
    return $pattern.DocumentRange.GetText(-1)
}
function Focus-Own([IntPtr]$Window, [switch]$ForceUia, [IntPtr]$Root=[IntPtr]::Zero) {
    if ($Root -eq [IntPtr]::Zero) { $Root=$script:Case.Main }
    [BksStage7]::Own($script:Case.Process.Id,$Root)
    if ($ForceUia -or [BksDirect]::Focus($script:Case.Process.Id,$Window) -ne $Window -or -not (Element $Window).Current.HasKeyboardFocus) { (Element $Window).SetFocus() }
    # UIA.SetFocus may make a child the foreground HWND. It is a supported
    # interaction when that HWND's GA_ROOT is still the owned app root.
    Wait-For { [BksDirect]::GetAncestor([BksDirect]::GetForegroundWindow(),2) -eq $Root -and [BksDirect]::Focus($script:Case.Process.Id,$Window) -eq $Window -and (Element $Window).Current.HasKeyboardFocus } 'owned foreground ancestor plus native and UIA keyboard focus'
}
function Focus-Diagnostics([IntPtr]$Direct) {
    $foreground=[BksDirect]::GetForegroundWindow()
    $foregroundRoot=[BksDirect]::GetAncestor($foreground,2)
    return @{ApplicationPid=$script:Case.Process.Id;MainHwnd=$script:Case.Main.ToInt64();MainClass=[BksStage7]::ClassName($script:Case.Main);DirectHwnd=$Direct.ToInt64();DirectClass=[BksStage7]::ClassName($Direct);DirectParent=[BksDirect]::GetParent($Direct).ToInt64();ThreadFocus=[BksDirect]::Focus($script:Case.Process.Id,$script:Case.Main).ToInt64();ForegroundHwnd=$foreground.ToInt64();ForegroundClass=[BksStage7]::ClassName($foreground);ForegroundRootHwnd=$foregroundRoot.ToInt64();ForegroundRootClass=[BksStage7]::ClassName($foregroundRoot);ForegroundPid=[BksDirect]::ForegroundProcess()}
}
function Menu([int]$Command) {
    Assert-ExpectedInput $script:ExpectedInput ('before MENU '+$Command)
    [BksDirect]::Post($script:Case.Process.Id,$script:Case.Main,0x111,$Command)
}
function Wait-CaptureNotice([int]$NoticeStart, [bool]$Enabled, [string]$Name) {
    $expected=if ($Enabled) { [BksDirect]::CaptureOnNotice } else { [BksDirect]::CaptureOffNotice }
    Wait-For {
        @([BksDirect]::Notices() | Select-Object -Skip $NoticeStart | Where-Object {
            $_.Text -ceq $expected -and $_.Activity -ceq 'bks.feedback' -and $_.Kind -ceq 'Other' -and $_.Processing -ceq 'MostRecent'
        }).Count -gt 0
    } ($Name+' delivers its explicit capture Notification')
    $observations.Add(@{Case=$Name;ExpectedCaptureNotice=$expected;NotificationStart=$NoticeStart;DeliveredNotifications=@([BksDirect]::Notices() | Select-Object -Skip $NoticeStart)})
}
function Menu-Capture([bool]$Enabled, [string]$Name) {
    # A return to the direct EDIT now enables capture. Establish a known state
    # through own focus changes before testing the explicit toggle itself.
    Focus-Own (Field $script:Case.Main 1007)
    if (-not $Enabled) { Focus-Own (Field $script:Case.Main 1011) }
    $noticeStart=[BksDirect]::Notices().Length
    Menu 60004
    Wait-CaptureNotice $noticeStart $Enabled $Name
    return $noticeStart
}
function Shared-Text([string]$Path) {
    if (-not [IO.File]::Exists($Path)) { return '' }
    $stream = [IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
    if ($stream.Length -gt 256*1024) { $stream.Dispose(); throw 'Fixture input log exceeded the bounded read size.' }
    $reader = New-Object IO.StreamReader($stream,[Text.Encoding]::UTF8)
    try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
}
function Raw-Input {
    $text = Shared-Text (Join-Path $script:Case.Task 'input.jsonl')
    # A concurrent append may expose the final JSON line before its newline.
    # Wait for that complete record rather than interpreting a partial chunk.
    if (-not $text.EndsWith("`n")) { $text=$text.Substring(0,$text.LastIndexOf("`n")+1) }
    $chunks = @($text -split "`n" | Where-Object { $_.Trim().Length -gt 0 } | ForEach-Object { ($_ | ConvertFrom-Json).Bytes })
    return $chunks -join ''
}
function Assert-ExpectedInput([string]$Expected, [string]$Phase) {
    $actual=Raw-Input
    if ($actual -cne $Expected) {
        $observations.Add(@{Case='unexpected fixture input; scenario stopped without a product verdict';Phase=$Phase;ExpectedRawInput=$Expected;ActualRawInput=$actual})
        throw ("Environment input or unexpected fixture bytes at ${Phase}: expected=$Expected actual=$actual")
    }
}
function Confirm-Input([string]$Before, [string]$Bytes, [string]$Name) {
    $expected=$Before+$Bytes
    $actual=Raw-Input
    if ($actual -cne $expected) {
        # An exact repetition of the synthetic key still fails the product
        # exact-once check. Unrelated bytes contaminate the shared desktop run.
        $suffix=if ($actual.StartsWith($Before,[StringComparison]::Ordinal)) { $actual.Substring($Before.Length) } else { '' }
        if ($suffix.Length -eq 0 -or $suffix.Replace($Bytes,'').Length -ne 0) { Assert-ExpectedInput $expected $Name }
    }
    Check $Name ($actual -ceq $expected)
    $script:ExpectedInput=$expected
}
function Uia-Selected([IntPtr]$Window) {
    $pattern=$null
    if (-not (Element $Window).TryGetCurrentPattern([Windows.Automation.SelectionPattern]::Pattern,[ref]$pattern)) { throw 'Owned LISTBOX lacks UIA SelectionPattern.' }
    return @($pattern.Current.GetSelection() | ForEach-Object { $_.Current.Name }) -join "`n"
}
function Uia-Rows([IntPtr]$Window) {
    return @((Element $Window).FindAll([Windows.Automation.TreeScope]::Children,[Windows.Automation.Condition]::TrueCondition) |
        Where-Object { $_.Current.ControlType -eq [Windows.Automation.ControlType]::ListItem } |
        ForEach-Object { $_.Current.Name })
}
function Invoke-Own([IntPtr]$Window) {
    $pattern=$null
    if (-not (Element $Window).TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern,[ref]$pattern)) { throw 'Owned button lacks UIA InvokePattern.' }
    $pattern.Invoke()
}
function Request-Scene([string]$Scene) {
    Assert-ExpectedInput $script:ExpectedInput ('before fixture scene '+$Scene)
    $done=Join-Path $script:Case.Task 'scene.done'
    if ([IO.File]::Exists($done)) { [IO.File]::Delete($done) }
    $request=Join-Path $script:Case.Task 'scene.request'
    [IO.File]::WriteAllText(($request+'.pending'),$Scene,(New-Object Text.UTF8Encoding $false))
    [IO.File]::Move(($request+'.pending'),$request)
    Wait-For { [IO.File]::Exists($done) -and (Shared-Text $done) -ceq $Scene } ('fixture scene '+$Scene+' acknowledged')
    Assert-ExpectedInput $script:ExpectedInput ('fixture scene '+$Scene)
}
function Check-Caret([IntPtr]$Direct, [string]$Label) {
    $row = '> ' + [char]0x754c + [char]::ConvertFromUtf32(0x1f600) + 'e' + [char]0x301 + ' ' + $Label
    Wait-For {
        $text=Own-Text $Direct
        $start=$text.LastIndexOf($row,[StringComparison]::Ordinal)
        $selection=[BksStage7]::Selection($script:Case.Process.Id,$Direct)
        $start -ge 0 -and $selection[0] -eq $start+5 -and $selection[1] -eq $selection[0]
    } "live $Label cursor mapped through wide/combining UTF16 cells"
    $text=Own-Text $Direct
    $expected=$text.LastIndexOf($row,[StringComparison]::Ordinal)+5
    $caret=[BksDirect]::UiaCaret($script:Case.Process.Id,$Direct)
    Check "UIA collapsed caret follows the live $Label row and Unicode column" ($caret.Collapsed -and (Normalize-Text $caret.Prefix) -ceq (Normalize-Text $text.Substring(0,$expected)))
    $observations.Add(@{Label=$Label;NativeCaret=$expected;UiaCaretNormalized=(Normalize-Text $caret.Prefix).Length})
}
function Check-Key([IntPtr]$Direct, [int]$Key, [string]$Bytes, [string]$Name) {
    Assert-ExpectedInput $script:ExpectedInput ('before '+$Name)
    $before=$script:ExpectedInput
    [BksDirect]::Key($script:Case.Process.Id,$Direct,$Key)
    Wait-For { (Raw-Input).Length -ge $before.Length+$Bytes.Length } "$Name reaches the fixture"
    Start-Sleep -Milliseconds 250
    Confirm-Input $before $Bytes "$Name reaches only the fixture exactly once"
}
function Check-Feedback([int]$NoticeStart, [string]$Label, [string]$Name) {
    Wait-For {
        @([BksDirect]::Notices() | Select-Object -Skip $NoticeStart | Where-Object { $_.Text.EndsWith($Label,[StringComparison]::Ordinal) -and $_.Activity -ceq 'bks.feedback' }).Count -gt 0
    } ($Name+' raises a real selected-row Notification')
    Start-Sleep -Milliseconds 150
    $delivered=@([BksDirect]::Notices() | Select-Object -Skip $NoticeStart)
    $observations.Add(@{Case=$Name;NotificationStart=$NoticeStart;Notifications=$delivered})
    Check ($Name+' emits exactly one selected-row Notification') ($delivered.Count -eq 1 -and $delivered[0].Text.EndsWith($Label,[StringComparison]::Ordinal) -and $delivered[0].Activity -ceq 'bks.feedback' -and $delivered[0].Kind -ceq 'Other')
}
function Start-Case {
    $directory=Join-Path $runDirectory 'direct'
    $appData=Join-Path $directory 'appdata'
    $configuration=Join-Path $appData 'BKS Term'
    $task=Join-Path $appData 'BKSDirectFixture'
    $bin=Join-Path $directory 'bin'
    foreach ($path in @($configuration,$task,$bin)) { $null=[IO.Directory]::CreateDirectory($path) }
    $mock=Join-Path $bin 'direct-fixture.exe'
    Copy-Item -LiteralPath $templateExe -Destination $mock
    Write-BksJson (Join-Path $configuration 'profiles.json') @{
        schema_version=1;default_profile_id='direct-fixture';profiles=@(@{
            id='direct-fixture';name='Direct accessibility fixture';kind='local';executable=$mock;args=@('--session');working_directory=$directory
        })
    }
    Write-BksJson (Join-Path $configuration 'settings.json') @{
        speech_mode='important';last_tab='keep_window';remember_workspace=$false;shortcuts=@{}
    }
    $start=New-Object Diagnostics.ProcessStartInfo($Exe)
    $start.UseShellExecute=$false
    $start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.WorkingDirectory=$directory
    $start.Arguments='--profile direct-fixture'
    $start.EnvironmentVariables['APPDATA']=$appData
    $start.EnvironmentVariables['LOCALAPPDATA']=(Join-Path $directory 'localappdata')
    $start.EnvironmentVariables['USERPROFILE']=(Join-Path $directory 'private-user')
    $start.EnvironmentVariables['CODEX_HOME']=(Join-Path $directory 'private-codex-home')
    $start.EnvironmentVariables['PATH']=$bin+';'+$env:SystemRoot+'\System32;'+$env:SystemRoot+'\System32\WindowsPowerShell\v1.0'
    $process=[Diagnostics.Process]::Start($start)
    $script:Case=@{Process=$process;Main=[IntPtr]::Zero;Task=$task;Mock=$mock;Fixture=$null;AppData=$appData}
    Wait-For { [BksStage7]::MainWindow($process.Id) -ne [IntPtr]::Zero } 'owned main window'
    $script:Case.Main=[BksStage7]::MainWindow($process.Id)
    Wait-For { [IO.File]::Exists((Join-Path $task 'identity.json')) -and [BksStage7]::GetDlgItem($script:Case.Main,1011) -ne [IntPtr]::Zero } 'native fixture and direct EDIT initialized'
    $identity=Read-BksJson (Join-Path $task 'identity.json')
    $fixture=[Diagnostics.Process]::GetProcessById($identity.Pid)
    if ($identity.Executable -ine $mock -or $fixture.MainModule.FileName -ine $mock) { throw 'Refusing an unknown fixture PID.' }
    $script:Case.Fixture=$fixture
    $observations.Add(@{ApplicationPid=$process.Id;FixturePid=$fixture.Id;AppData=$appData;Fixture=$mock;ExecutableSha256=(Get-BksHash $Exe);ProductVersion=(Get-Item -LiteralPath $Exe).VersionInfo.ProductVersion})
}
function Stop-Case {
    if (-not $script:Case) { return }
    $forced=$false
    [IO.File]::WriteAllText((Join-Path $script:Case.Task 'exit'),'exit')
    if (-not $script:Case.Fixture -and [IO.File]::Exists((Join-Path $script:Case.Task 'identity.json'))) {
        $identity=Read-BksJson (Join-Path $script:Case.Task 'identity.json')
        try {
            $fixture=[Diagnostics.Process]::GetProcessById($identity.Pid)
            if ($fixture.MainModule.FileName -ieq $script:Case.Mock) { $script:Case.Fixture=$fixture }
        } catch [ArgumentException] { }
    }
    if ($script:Case.Fixture -and -not $script:Case.Fixture.WaitForExit(5000)) {
        if ($script:Case.Fixture.MainModule.FileName -ine $script:Case.Mock) { throw 'Unknown cleanup executable refused.' }
        $script:Case.Fixture.Kill()
        $forced=$true
        $checks.Add(@{Status='failed';Name='Owned fixture required forced cleanup'})
    }
    if (-not $script:Case.Process.HasExited) {
        $tui=[BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermTuiDialog')
        if ($tui -ne [IntPtr]::Zero) { [BksDirect]::Post($script:Case.Process.Id,$tui,0x10,0) }
        $help=[BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermHelpWindow')
        if ($help -ne [IntPtr]::Zero) { [BksDirect]::Post($script:Case.Process.Id,$help,0x10,0) }
        $ownedMain=[BksStage7]::MainWindow($script:Case.Process.Id)
        if ($ownedMain -ne [IntPtr]::Zero) { [BksDirect]::Post($script:Case.Process.Id,$ownedMain,0x10,0) }
        if (-not $script:Case.Process.WaitForExit(8000)) {
            $script:Case.Process.Kill()
            $forced=$true
            $checks.Add(@{Status='failed';Name='Owned application required forced cleanup'})
        }
    }
    Check 'only the owned application and fixture closed without forced cleanup' (-not $forced -and $script:Case.Process.HasExited -and (-not $script:Case.Fixture -or $script:Case.Fixture.HasExited))
    $script:Case=$null
}

try {
    Add-Type -ReferencedAssemblies $references -TypeDefinition ($nativeSource+"`r`n"+$extension)
    [BksStage7]::InitializeUia()
    $keyUpProbe=[BksDirect]::ProbeKeyUp()
    $observations.Add(@{Case='own invisible message-only KEYUP contract';MalformedKeyUpGeneratedReturn=$keyUpProbe.MalformedKeyUpGeneratedReturn;NativeKeyUpGeneratedReturn=$keyUpProbe.NativeKeyUpGeneratedReturn})
    Check 'native KEYUP flags do not synthesize a second WM_CHAR Return' (-not $keyUpProbe.NativeKeyUpGeneratedReturn)
    $templateExe=Join-Path $runDirectory 'direct-fixture-template.exe'
    $csc=Join-Path $env:SystemRoot 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
    & $csc /nologo /target:exe /platform:x64 /warnaserror+ /reference:System.Web.Extensions.dll ('/out:'+$templateExe) (Join-Path $PSScriptRoot 'fixtures/direct-accessibility.cs')
    if ($LASTEXITCODE -ne 0) { throw 'Local direct fixture compilation failed.' }
    Check 'own-PID UIA helpers and the native fixture compile' (Test-Path -LiteralPath $templateExe -PathType Leaf)
    & $templateExe --self-test
    Check 'fixture parser handles split arrows, standalone Esc, Enter, UTF8 and text-invariant style scenes' ($LASTEXITCODE -eq 0)
    if ($PrepareOnly) {
        $checks.Add(@{Status='skipped';Name='PrepareOnly: the application and ConPTY scenario were not launched'})
    } else {
        $Exe=Get-BksFullPath $Exe
        Assert-BksNoReparse $Exe
        Start-Case
        $main=$script:Case.Main
        $pidOwn=$script:Case.Process.Id
        $command=Field $main 1002
        $reader=Field $main 1018
        $direct=Field $main 1011
        $canvas=Field $main 1017
        Wait-For { [BksStage7]::IsWindowVisible($direct) -and -not [BksStage7]::IsWindowVisible($command) -and [BksDirect]::Focus($pidOwn,$main) -eq $direct } 'new profile starts in native direct input without a command field'
        Check 'new profiles default to the full-area direct surface' ([BksStage7]::IsWindowVisible($direct) -and -not [BksStage7]::IsWindowVisible($command))
        $draft='BKS_DIRECT_DRAFT_'+[char]0x0416+'_never_submitted'
        [BksStage7]::SetText($pidOwn,$command,$draft)
        Menu 60005
        Wait-For { [BksStage7]::IsWindowVisible($reader) -and (Own-Text $reader).Contains('BKS_DIRECT_READY') } 'separate VT reader ready'
        $readerText=Own-Text $reader
        $bookmark=$readerText.IndexOf('BKS_DIRECT_HISTORY_010',[StringComparison]::Ordinal)
        Check 'fixture exposes scrollback for an independent reader bookmark' ($bookmark -ge 0)
        [BksStage7]::Select($pidOwn,$reader,$bookmark,$bookmark+22)
        [BksStage7]::ScrollTo($pidOwn,$reader,$bookmark)
        $bookmarkSelection=[BksStage7]::Selection($pidOwn,$reader)
        $bookmarkLine=[BksStage7]::FirstLine($pidOwn,$reader)
        $bookmarkText=$readerText.Substring($bookmarkSelection[0],$bookmarkSelection[1]-$bookmarkSelection[0])
        [BksDirect]::Watch($pidOwn,(Field $main 1026))
        Menu 60004
        Wait-For { [BksStage7]::IsWindowVisible($direct) -and [BksDirect]::Focus($pidOwn,$main) -eq $direct -and (Own-Text $direct).Contains('BKS_DIRECT_READY') } 'capture focuses the native direct EDIT'
        Check 'direct mode exposes a read-only multiline EDIT and hides the command field' ([BksStage7]::ClassName($direct) -ieq 'Edit' -and [BksStage7]::ReadOnly($pidOwn,$direct) -and [BksDirect]::Multiline($pidOwn,$direct) -and -not [BksStage7]::IsWindowVisible($command))
        $directBounds=[BksDirect]::Bounds($pidOwn,$direct)
        $canvasBounds=[BksDirect]::Bounds($pidOwn,$canvas)
        Check 'native direct EDIT covers the full terminal geometry and replaces the separate canvas' (-not [BksStage7]::IsWindowVisible($canvas) -and (Rect-Key $directBounds) -ceq (Rect-Key $canvasBounds) -and $directBounds.Bottom -gt $directBounds.Top)
        # A newly started app can have thread focus while another process still
        # owns foreground. Acquire focus through the owned native UIA provider.
        Focus-Own $direct
        $directElement=Element $direct
        $observations.Add((Focus-Diagnostics $direct))
        $observations.Add(@{DirectName=$directElement.Current.Name;ExpectedDirectName=[BksDirect]::DirectName;KeyboardFocusable=$directElement.Current.IsKeyboardFocusable;HasKeyboardFocus=$directElement.Current.HasKeyboardFocus;ApplicationPid=$pidOwn;ThreadFocus=[BksDirect]::Focus($pidOwn,$main).ToInt64();DirectHwnd=$direct.ToInt64();ForegroundHwnd=[BksDirect]::GetForegroundWindow().ToInt64();ForegroundPid=[BksDirect]::ForegroundProcess()})
        Check 'direct EDIT exposes its exact UIA name' ($directElement.Current.Name -ceq [BksDirect]::DirectName)
        Check 'direct EDIT exposes keyboard focusability and actual UIA keyboard focus' ($directElement.Current.IsKeyboardFocusable -and $directElement.Current.HasKeyboardFocus)
        if ((Raw-Input).Length -gt 0) { throw ('Environment input arrived before the first synthetic key: '+(Raw-Input)) }
        # Compare one coherent native/UIA/native observation. A resize repaint
        # can otherwise change the document between independent provider reads.
        $documentProbe=@{Before='';Uia='';After=''}
        Wait-For {
            $documentProbe.Before=Own-Text $direct
            $documentProbe.Uia=Uia-Text $direct
            $documentProbe.After=Own-Text $direct
            $documentProbe.Before -ceq $documentProbe.After
        } 'unchanged native text around a UIA TextPattern read'
        $observations.Add(@{Case='coherent native/UIA/native document';NativeBefore=$documentProbe.Before;TextPatternText=$documentProbe.Uia;NativeAfter=$documentProbe.After})
        Check 'UIA TextPattern reads the current VT document and Unicode' ((Normalize-Text $documentProbe.Uia).TrimEnd("`n") -ceq (Normalize-Text $documentProbe.After).TrimEnd("`n") -and $documentProbe.Uia.Contains([char]::ConvertFromUtf32(0x1f600)))
        $valuePattern=$null
        $hasValuePattern=$directElement.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern,[ref]$valuePattern)
        $observations.Add(@{DirectPatterns=@($directElement.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName });HasValuePattern=$hasValuePattern;ValuePatternReadOnly=$(if($hasValuePattern){$valuePattern.Current.IsReadOnly}else{$null});ValuePatternText=$(if($hasValuePattern){$valuePattern.Current.Value}else{$null});NativeDirectText=(Own-Text $direct);TextPatternText=(Uia-Text $direct)})
        # Native multiline EDIT requires TextPattern and does not implement
        # ValuePattern: https://learn.microsoft.com/en-us/dotnet/framework/ui-automation/implementing-the-ui-automation-value-control-pattern
        # Record that provider capability above; TextPattern/native read-only,
        # focus and the live UIA caret remain mandatory checks.
        if ($hasValuePattern) {
            Check 'optional ValuePattern is read-only and agrees with the native document' ($valuePattern.Current.IsReadOnly -and (Normalize-Text $valuePattern.Current.Value) -ceq (Normalize-Text (Own-Text $direct)))
        }
        Check-Caret $direct 'Alpha'
        $openingInput=Raw-Input
        if ($openingInput.Length -gt 0) { throw ('Environment input arrived before the first synthetic key: '+$openingInput) }
        Check 'opening readers and capture sends no input or approval to the fixture' ($openingInput.Length -eq 0)
        Focus-Own $direct -ForceUia
        $observations.Add(@{Case='Down immediately after native UIA.SetFocus';FocusDiagnostics=(Focus-Diagnostics $direct)})
        $noticeStart=[BksDirect]::Notices().Length
        Check-Key $direct 0x28 '1B5B42' 'Down'
        Check-Caret $direct 'Bravo'
        Wait-For { @([BksDirect]::Notices() | Select-Object -Skip $noticeStart | Where-Object { $_.Text.Contains('Bravo') }).Count -gt 0 } 'navigation raises a changed-row UIA notification'
        Check 'Down feedback uses a real announcement Notification event' (@([BksDirect]::Notices() | Select-Object -Skip $noticeStart | Where-Object { $_.Text.Contains('Bravo') }).Count -eq 1)
        Check-Key $direct 0x26 '1B5B41' 'Up'
        Check-Caret $direct 'Alpha'
        Check-Key $direct 13 '0D' 'Enter'
        Check-Key $direct 27 '1B' 'Escape'
        foreach ($character in @([char]'x',[char]0x0416)) {
            Assert-ExpectedInput $script:ExpectedInput 'before explicit WM_CHAR'
            $before=$script:ExpectedInput
            $bytes=([BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes([string]$character))).Replace('-','')
            [BksDirect]::Character($pidOwn,$direct,$character)
            Wait-For { (Raw-Input).Length -ge $before.Length+$bytes.Length } 'direct WM_CHAR reaches the fixture'
            Start-Sleep -Milliseconds 200
            Confirm-Input $before $bytes ('WM_CHAR U+'+([int]$character).ToString('X4')+' reaches PTY exactly once')
        }
        Check 'direct document contains the fixture response to typed Unicode' ((Own-Text $direct).Contains('Input: x'+[char]0x0416))
        $before=Raw-Input
        Focus-Own (Field $main 1007)
        Focus-Own $direct
        Check-Key $direct 9 '09' 'Tab after returning focus to the direct EDIT'
        Check 'returning focus to direct EDIT restores capture without an extra toggle' ([BksDirect]::Focus($pidOwn,$main) -eq $direct -and (Raw-Input) -ceq ($before+'09'))
        $directPtySize=Own-Text (Field $main 1015)
        $directCanvasRect=Rect-Key ([BksDirect]::Bounds($pidOwn,$canvas))
        Check 'direct canvas has a reported PTY size before full reading' ($directPtySize.Length -gt 0)
        # MENU_READING invokes the same full-reading action as Ctrl+Shift+Space;
        # target it directly without modifying global keyboard modifier state.
        Menu 60003
        Wait-For { [BksStage7]::IsWindowVisible($reader) -and [BksDirect]::Focus($pidOwn,$main) -eq $reader } 'reading mode focuses the independent native reader'
        $readingPtySize=Own-Text (Field $main 1015)
        $readingCanvasRect=Rect-Key ([BksDirect]::Bounds($pidOwn,$canvas))
        Check 'full reading preserves the direct canvas geometry and PTY size' ($readingPtySize -ceq $directPtySize -and $readingCanvasRect -ceq $directCanvasRect)
        $observations.Add(@{Action='MENU_READING / Ctrl+Shift+Space action';DirectPtySize=$directPtySize;ReadingPtySize=$readingPtySize;DirectCanvasRect=$directCanvasRect;ReadingCanvasRect=$readingCanvasRect;ExpandedReaderRect=(Rect-Key ([BksDirect]::Bounds($pidOwn,$reader)))})
        $restored=[BksStage7]::Selection($pidOwn,$reader)
        $readerText=Own-Text $reader
        Check 'direct caret updates preserve the separate reader selection and viewport bookmark' ($restored[0] -eq $bookmarkSelection[0] -and $restored[1] -eq $bookmarkSelection[1] -and $readerText.Substring($restored[0],$restored[1]-$restored[0]) -ceq $bookmarkText -and [BksStage7]::FirstLine($pidOwn,$reader) -eq $bookmarkLine)
        $before=Raw-Input
        foreach ($key in @(0x26,0x28,13)) { [BksDirect]::Key($pidOwn,$reader,$key) }
        [BksDirect]::Character($pidOwn,$reader,[char]'r')
        Start-Sleep -Milliseconds 350
        Assert-ExpectedInput $script:ExpectedInput 'reading arrows, Enter and WM_CHAR'
        Check 'reading arrows, Enter and WM_CHAR never reach PTY or change its document' ((Raw-Input) -ceq $before -and (Own-Text $reader) -ceq $readerText)
        # Return from independent reading restores direct capture itself.
        Menu 60003
        Wait-For { [BksDirect]::Focus($pidOwn,$main) -eq $direct } 'capture restored before F1'
        Check 'returning from full reading preserves the same canvas geometry and PTY size' ((Own-Text (Field $main 1015)) -ceq $directPtySize -and (Rect-Key ([BksDirect]::Bounds($pidOwn,$canvas))) -ceq $directCanvasRect)
        $before=Raw-Input
        [BksDirect]::Key($pidOwn,$direct,0x70)
        Wait-For { [BksStage7]::VisibleClass($pidOwn,'BksTermHelpWindow') -ne [IntPtr]::Zero } 'F1 opens owned native help'
        $help=[BksStage7]::VisibleClass($pidOwn,'BksTermHelpWindow')
        Wait-For { [BksStage7]::GetDlgItem($help,1) -ne [IntPtr]::Zero } 'native help EDIT initialized'
        $helpText=Field $help 1
        Check 'F1 help exposes a named read-only native EDIT and TextPattern' ([BksStage7]::ReadOnly($pidOwn,$helpText) -and (Element $helpText).Current.Name.Length -gt 0 -and (Uia-Text $helpText).Length -gt 0)
        Assert-ExpectedInput $script:ExpectedInput 'F1 help'
        Check 'F1 never reaches the fixture or performs an approval' ((Raw-Input) -ceq $before)
        [BksDirect]::Post($pidOwn,$help,0x10,0)
        Wait-For { [BksStage7]::VisibleClass($pidOwn,'BksTermHelpWindow') -eq [IntPtr]::Zero } 'owned help closes'
        Focus-Own $direct
        Check-Key $direct 9 '09' 'Tab after help returns focus to direct EDIT'
        Check 'help return restores capture while F1 itself adds no PTY bytes' ((Raw-Input) -ceq ($before+'09'))
        foreach ($cycle in 1..2) {
            $before=Raw-Input
            $null=Menu-Capture $true "capture on, cycle $cycle"
            Wait-For { [BksDirect]::Focus($pidOwn,$main) -eq $direct } "capture on focuses native direct EDIT, cycle $cycle"
            $null=Menu-Capture $false "capture off, cycle $cycle"
            [BksDirect]::Key($pidOwn,$direct,9)
            Wait-For { [BksDirect]::Focus($pidOwn,$main) -ne $direct } "second capture toggle releases GUI Tab, cycle $cycle"
            Start-Sleep -Milliseconds 200
            Assert-ExpectedInput $script:ExpectedInput ("capture on/off cycle $cycle")
            Check "capture on/off cycle $cycle sends no Tab or other bytes to PTY" ((Raw-Input) -ceq $before)
        }
        $captureNoticeStart=Menu-Capture $true 'capture before bounded repaint stream'
        Wait-For { [BksDirect]::Focus($pidOwn,$main) -eq $direct } 'direct focus before bounded repaint stream'
        # Delivery of the explicit capture notice is asynchronous. First wait
        # for that exact notice above, then observe a quiet second before the
        # fixture starts repainting. Retain every intervening notice: settling
        # must not swallow unexpected passive feedback or duplicate captures.
        $baseline=@{Count=[BksDirect]::Notices().Length;Quiet=[Diagnostics.Stopwatch]::StartNew()}
        Wait-For {
            $count=[BksDirect]::Notices().Length
            if ($count -ne $baseline.Count) { $baseline.Count=$count; $baseline.Quiet.Restart() }
            $baseline.Quiet.ElapsedMilliseconds -ge 1000
        } 'capture Notification baseline remains quiet for one second'
        $before=Raw-Input
        $baselineNotices=[BksDirect]::Notices()
        $noticesBefore=$baselineNotices.Length
        $setupNotices=@($baselineNotices | Select-Object -Skip $captureNoticeStart)
        $repaintObservation=@{
            Case='bounded passive repaint';RepaintFrames=100;VisualFlicker='not tested';Nvda='not tested'
            Before=@{AtUtc=[DateTime]::UtcNow.ToString('o');Focus=(Focus-Diagnostics $direct);UiaHasKeyboardFocus=(Element $direct).Current.HasKeyboardFocus;RawInput=$before;NotificationCount=$noticesBefore;Notifications=$baselineNotices;CaptureNotificationStart=$captureNoticeStart;SetupNotifications=$setupNotices;QuietMilliseconds=$baseline.Quiet.ElapsedMilliseconds}
            After=$null
        }
        $observations.Add($repaintObservation)
        Check 'repaint baseline contains only the requested explicit capture Notification' ($setupNotices.Count -eq 1 -and $setupNotices[0].Text -ceq [BksDirect]::CaptureOnNotice)
        $pings=New-Object 'System.Collections.Generic.List[double]'
        try {
            [IO.File]::WriteAllText((Join-Path $script:Case.Task 'repaint.request'),'100 frames')
            $streamTimer=[Diagnostics.Stopwatch]::StartNew()
            while (-not [IO.File]::Exists((Join-Path $script:Case.Task 'repaint.done')) -and $streamTimer.ElapsedMilliseconds -lt 10000) {
                $ping=[Diagnostics.Stopwatch]::StartNew()
                $responsive=[BksStage7]::Responsive($pidOwn,$main,1000,2)
                $pings.Add($ping.Elapsed.TotalMilliseconds)
                if (-not $responsive) { throw 'Owned main window stopped responding during bounded fixture repaint.' }
                Start-Sleep -Milliseconds 50
            }
            Check 'owned window responds during the complete bounded 100-frame repaint stream' ([IO.File]::Exists((Join-Path $script:Case.Task 'repaint.done')) -and $pings.Count -gt 3)
            Wait-For { (Own-Text $direct).Contains('Tick: 000100') } 'final repaint arrives in the native direct document'
            Check-Caret $direct 'Alpha'
            Start-Sleep -Milliseconds 300
        } finally {
            $afterNotices=[BksDirect]::Notices()
            $repaintObservation.After=@{AtUtc=[DateTime]::UtcNow.ToString('o');Focus=(Focus-Diagnostics $direct);UiaHasKeyboardFocus=(Element $direct).Current.HasKeyboardFocus;RawInput=(Raw-Input);NotificationCount=$afterNotices.Length;Notifications=$afterNotices;NewNotifications=@($afterNotices | Select-Object -Skip $noticesBefore);FinalFramePresent=(Own-Text $direct).Contains('Tick: 000100')}
            $repaintObservation.PingCount=$pings.Count
            $repaintObservation.MaxPingMilliseconds=($pings | Measure-Object -Maximum).Maximum
        }
        $focusPreserved=$repaintObservation.After.Focus.ThreadFocus -eq $direct.ToInt64() -and $repaintObservation.After.UiaHasKeyboardFocus
        $rawUnchanged=$repaintObservation.After.RawInput -ceq $before
        $noPassiveFeedback=$repaintObservation.After.NotificationCount -eq $noticesBefore
        Assert-ExpectedInput $script:ExpectedInput 'bounded passive repaint'
        Check 'passive repaint preserves native and UIA direct EDIT focus' $focusPreserved -ContinueOnFailure
        Check 'passive repaint sends no PTY input' $rawUnchanged -ContinueOnFailure
        Check 'passive repaint raises no extra feedback Notification' $noPassiveFeedback -ContinueOnFailure
        if (-not ($focusPreserved -and $rawUnchanged -and $noPassiveFeedback)) { throw 'FAILED: passive repaint assertions; see separate checks and before/after observations' }
        Assert-ExpectedInput $script:ExpectedInput 'complete Unicode/Enter/Esc/default-capture baseline'
        Check 'fixture received exactly the explicit synthetic input sequence' ((Raw-Input) -ceq $script:ExpectedInput)
        if ($BaselineOnly) {
            $checks.Add(@{Status='skipped';Name='BaselineOnly: generic overview and style-only scene checks require the new preview'})
        } else {
            # MENU_TUI_DIALOG is the Ctrl+Shift+D action. Use the owned menu
            # command rather than changing global keyboard modifier state.
            Assert-ExpectedInput $script:ExpectedInput 'before generic TUI overview'
            Menu 60014
            Wait-For { [BksStage7]::VisibleClass($pidOwn,'BksTermTuiDialog') -ne [IntPtr]::Zero } 'Ctrl+Shift+D action opens the unknown-tool native overview'
            $overview=[BksStage7]::VisibleClass($pidOwn,'BksTermTuiDialog')
            Wait-For { [BksStage7]::GetDlgItem($overview,4400) -ne [IntPtr]::Zero -and [BksStage7]::GetDlgItem($overview,4401) -ne [IntPtr]::Zero } 'overview context and native list initialized'
            $overviewContext=Field $overview 4400
            $overviewList=Field $overview 4401
            $overviewReturn=Field $overview 4402
            $overviewStatus=Field $overview 4404
            Focus-Own $overviewContext -Root $overview -ForceUia
            $overviewText=Own-Text $overviewContext
            $overviewRows=@(Uia-Rows $overviewList)
            $overviewObservation=@{Case='unknown TUI read-only overview';Before=@{RawInput=(Raw-Input);Context=$overviewText;TextPattern=(Uia-Text $overviewContext);Rows=$overviewRows;Selection=(Uia-Selected $overviewList);ReturnName=(Element $overviewReturn).Current.Name;OwnedTree=[BksStage7]::DialogTree($pidOwn,$overview)};After=$null}
            $observations.Add($overviewObservation)
            Check 'unknown TUI overview exposes named read-only native context with Unicode TextPattern' ((Own-Text $overview).Contains([BksDirect]::OverviewName) -and [BksStage7]::ReadOnly($pidOwn,$overviewContext) -and [BksStage7]::ClassName($overviewContext) -ieq 'Edit' -and (Element $overviewContext).Current.Name.Length -gt 0 -and (Same-UiaText (Uia-Text $overviewContext) $overviewText) -and $overviewText.Contains('BKS_DIRECT_READY') -and $overviewText.Contains([char]::ConvertFromUtf32(0x1f600)))
            Check 'unknown TUI overview exposes readable native rows and an explicit direct-return button' ([BksStage7]::ClassName($overviewList) -ieq 'ListBox' -and $overviewRows.Count -ge 3 -and (Element $overviewList).Current.IsEnabled -and (Element $overviewReturn).Current.IsEnabled -and (Element $overviewReturn).Current.Name -ceq [BksDirect]::DirectReturnName)
            Focus-Own $overviewList -Root $overview -ForceUia
            $selectionBefore=Uia-Selected $overviewList
            [BksDirect]::Key($pidOwn,$overviewList,0x28)
            Wait-For { (Uia-Selected $overviewList) -cne $selectionBefore } 'overview Down changes the reading selection'
            $selectionDown=Uia-Selected $overviewList
            [BksDirect]::Key($pidOwn,$overviewList,0x26)
            Wait-For { (Uia-Selected $overviewList) -ceq $selectionBefore } 'overview Up restores the reading selection'
            [BksDirect]::Key($pidOwn,$overviewList,0x28)
            Wait-For { (Uia-Selected $overviewList) -ceq $selectionDown } 'overview Down selects the same readable row again'
            [BksDirect]::Key($pidOwn,$overviewList,13)
            Start-Sleep -Milliseconds 250
            Assert-ExpectedInput $script:ExpectedInput 'arrows and Enter in read-only overview list'
            Check 'overview list arrows and Enter only navigate reading and keep the window open' ([BksStage7]::VisibleClass($pidOwn,'BksTermTuiDialog') -eq $overview -and (Uia-Selected $overviewList).Length -gt 0 -and (Uia-Selected $overviewList) -cne $selectionBefore)
            $selectionBeforeRepaint=Uia-Selected $overviewList
            Request-Scene 'passive'
            Wait-For { (Own-Text $overviewStatus).Contains([BksDirect]::StaleOverview) } 'passive output marks the generic overview snapshot stale'
            $overviewObservation.After=@{RawInput=(Raw-Input);Context=(Own-Text $overviewContext);TextPattern=(Uia-Text $overviewContext);Rows=@(Uia-Rows $overviewList);Selection=(Uia-Selected $overviewList);ListEnabled=(Element $overviewList).Current.IsEnabled;ReturnEnabled=(Element $overviewReturn).Current.IsEnabled;Status=(Own-Text $overviewStatus)}
            Assert-ExpectedInput $script:ExpectedInput 'passive update under read-only overview'
            Check 'passive output preserves the overview context, selected reading row and readable list' ($overviewObservation.After.Context -ceq $overviewText -and (Same-UiaText $overviewObservation.After.TextPattern $overviewText) -and $overviewObservation.After.Selection -ceq $selectionBeforeRepaint -and ($overviewObservation.After.Rows -join "`n") -ceq ($overviewRows -join "`n") -and $overviewObservation.After.ListEnabled)
            Check 'stale generic overview keeps the explicit direct-return button enabled' ($overviewObservation.After.ReturnEnabled -and (Element $overviewReturn).Current.Name -ceq [BksDirect]::DirectReturnName)
            $returnNoticeStart=[BksDirect]::Notices().Length
            Invoke-Own $overviewReturn
            Wait-For { [BksStage7]::VisibleClass($pidOwn,'BksTermTuiDialog') -eq [IntPtr]::Zero -and [BksDirect]::Focus($pidOwn,$main) -eq $direct } 'explicit overview return focuses the native direct EDIT'
            Wait-CaptureNotice $returnNoticeStart $true 'explicit overview return'
            Focus-Own $direct -ForceUia
            Assert-ExpectedInput $script:ExpectedInput 'explicit overview return without a CLI action'
            Check 'explicit overview return enables capture with native and UIA direct EDIT focus' ((Element $direct).Current.HasKeyboardFocus -and @([BksDirect]::Notices() | Select-Object -Skip $returnNoticeStart | Where-Object { $_.Text -ceq [BksDirect]::CaptureOnNotice }).Count -eq 1)
            $noticeStart=[BksDirect]::Notices().Length
            Check-Key $direct 0x28 '1B5B42' 'Down after explicit overview return'
            Check-Caret $direct 'Bravo'
            Check-Feedback $noticeStart 'Bravo' 'navigation after explicit overview return'
            foreach ($scene in @('background','inverse')) {
                Request-Scene $scene
                Wait-For { (Own-Text $direct).Contains('Mode: '+$scene) } ('native direct document reaches '+$scene+' scene')
                $sceneText=Own-Text $direct
                $sceneUiaText=Uia-Text $direct
                $noticeStart=[BksDirect]::Notices().Length
                $sceneObservation=@{Case=($scene+'-only navigation with hidden cursor');Before=@{NativeText=$sceneText;TextPattern=$sceneUiaText;RawInput=(Raw-Input);NotificationStart=$noticeStart};After=$null}
                $observations.Add($sceneObservation)
                Check-Key $direct 0x28 '1B5B42' ($scene+'-only Down')
                Check-Feedback $noticeStart 'Bravo' ($scene+'-only navigation')
                $sceneObservation.After=@{NativeText=(Own-Text $direct);TextPattern=(Uia-Text $direct);RawInput=(Raw-Input);Notifications=@([BksDirect]::Notices() | Select-Object -Skip $noticeStart);Focus=(Focus-Diagnostics $direct)}
                Assert-ExpectedInput $script:ExpectedInput ($scene+'-only navigation')
                Check ($scene+'-only selection transfer keeps the complete native and UIA text unchanged') ($sceneObservation.After.NativeText -ceq $sceneText -and (Same-UiaText $sceneUiaText $sceneText) -and (Same-UiaText $sceneObservation.After.TextPattern $sceneText))
            }
        }
        # The popup-only menu selection is not a WM_COMMAND handler. Invoke the
        # real mode button command, which is also used by the Settings harness.
        Menu 1009
        Wait-For { [BksStage7]::IsWindowVisible($command) -and -not [BksStage7]::IsWindowVisible($direct) } 'field input mode restored'
        Check 'direct input, reading, help and repaint preserve the unsubmitted command draft' ((Own-Text $command) -ceq $draft)
        Assert-ExpectedInput $script:ExpectedInput 'complete direct accessibility scenario'
        Check 'fixture received only the complete explicit synthetic input sequence' ((Raw-Input) -ceq $script:ExpectedInput)
        $observations.Add(@{DirectUiName=$directElement.Current.Name;DirectPatterns=@($directElement.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName });Notifications=[BksDirect]::Notices();RawInput=Raw-Input})
    }
} catch {
    $failure=$_.Exception.Message
    if (-not $failure.StartsWith('FAILED:')) { $checks.Add(@{Status='error';Name=$failure}) }
    $observations.Add(@{Failure=$failure;Stack=$_.ScriptStackTrace})
    if ($script:Case -and -not $script:Case.Process.HasExited) {
        try {
            $directDiagnostic=Field $script:Case.Main 1011
            $observations.Add((Focus-Diagnostics $directDiagnostic))
            $observations.Add(@{OwnedTree=[BksStage7]::DialogTree($script:Case.Process.Id,$script:Case.Main);RawInput=Raw-Input;Notifications=[BksDirect]::Notices();DirectNativeSelection=[BksStage7]::Selection($script:Case.Process.Id,$directDiagnostic);DirectUiaCaret=[BksDirect]::UiaCaret($script:Case.Process.Id,$directDiagnostic)})
            foreach ($id in @(1011,1018,1002,1026)) {
                $window=Field $script:Case.Main $id
                [IO.File]::WriteAllText((Join-Path $runDirectory ('owned-control-'+$id+'.txt')),(Own-Text $window),(New-Object Text.UTF8Encoding $false))
            }
        } catch { $observations.Add(@{DiagnosticError=$_.Exception.Message}) }
    }
} finally {
    try { if ('BksDirect' -as [type]) { [BksDirect]::Unwatch() } } catch { $checks.Add(@{Status='error';Name=('Notification cleanup: '+$_.Exception.Message)}) }
    try { Stop-Case } catch { $checks.Add(@{Status='error';Name=('Owned cleanup: '+$_.Exception.Message)}) }
    $counts=@{passed=@($checks | Where-Object {$_.Status -eq 'passed'}).Count;failed=@($checks | Where-Object {$_.Status -eq 'failed'}).Count;skipped=@($checks | Where-Object {$_.Status -eq 'skipped'}).Count;error=@($checks | Where-Object {$_.Status -eq 'error'}).Count}
    Write-BksJson (Join-Path $runDirectory 'summary.json') @{
        Scope='Own PIDs/HWNDs/private APPDATA; targeted posted messages and explicit UIA Invoke only; no global input/real CLI/accounts/network/approvals; UIA is not human NVDA or a visual flicker check';PreparedOnly=[bool]$PrepareOnly;BaselineOnly=[bool]$BaselineOnly;Executable=$Exe;Counts=$counts;Checks=$checks.ToArray();Observations=$observations.ToArray()
    }
    [IO.File]::WriteAllLines((Join-Path $runDirectory 'results.txt'),[string[]]@($checks | ForEach-Object { $_.Status+': '+$_.Name }), (New-Object Text.UTF8Encoding $false))
}
Write-Output "Direct accessibility: passed=$($counts.passed) failed=$($counts.failed) skipped=$($counts.skipped) error=$($counts.error)"
Write-Output "Artifacts: $runDirectory"
if ($failure -or $counts.failed -or $counts.error) { throw $(if ($failure) {$failure} else {'Direct accessibility checks or cleanup failed.'}) }
