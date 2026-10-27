param(
    [string]$Exe = (Join-Path $PSScriptRoot '../target/release/bks-app.exe'),
    [string]$Artifacts = (Join-Path $PSScriptRoot '../target/stage7-check'),
    [switch]$OutputOnly,
    [switch]$LifecycleOnly,
    [switch]$ProfilesOnly,
    [switch]$GitBashOnly,
    [switch]$MtaDiagnostics,
    [switch]$PerformanceTrace
)

# One isolated process; only its HWNDs and APPDATA are queried or changed.
# No global keys, clipboard, SSH connection, environment setup or NVDA claims.
$ErrorActionPreference = 'Stop'
if ((@($OutputOnly,$LifecycleOnly,$ProfilesOnly,$GitBashOnly) | Where-Object { $_ }).Count -ne 1) { throw 'Select one targeted stage7 mode' }
if ($PSVersionTable.PSEdition -eq 'Core') {
    $frameworkArguments = @('-NoProfile')
    if ($MtaDiagnostics) { $frameworkArguments += '-Mta' }
    $frameworkArguments += @('-ExecutionPolicy','Bypass','-File',$PSCommandPath,'-Exe',$Exe,'-Artifacts',$Artifacts)
    if ($OutputOnly) { $frameworkArguments += '-OutputOnly' }
    if ($LifecycleOnly) { $frameworkArguments += '-LifecycleOnly' }
    if ($ProfilesOnly) { $frameworkArguments += '-ProfilesOnly' }
    if ($GitBashOnly) { $frameworkArguments += '-GitBashOnly' }
    if ($MtaDiagnostics) { $frameworkArguments += '-MtaDiagnostics' }
    if ($PerformanceTrace) { $frameworkArguments += '-PerformanceTrace' }
    & "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" @frameworkArguments
    exit $LASTEXITCODE
}
if ($MtaDiagnostics -and [Threading.Thread]::CurrentThread.GetApartmentState() -ne 'MTA') { throw 'Diagnostic requires powershell.exe -Mta' }
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName UIAutomationClientsideProviders
$references = @([Windows.Automation.AutomationElement].Assembly.Location,
    [Windows.Automation.AutomationPattern].Assembly.Location,
    [UIAutomationClientsideProviders.UIAutomationClientSideProviders].Assembly.Location)
Add-Type -ReferencedAssemblies $references -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class BksStage7 {
    public static int ResponseError { get; private set; }
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.NoInlining)]
    public static void InitializeUia() {
        var table = UIAutomationClientsideProviders.UIAutomationClientSideProviders.ClientSideProviderDescriptionTable;
        try { System.Windows.Automation.ClientSettings.RegisterClientSideProviders(table); }
        catch (NullReferenceException) {
            // Framework's proxy loader can encounter a PowerShell dynamic
            // frame; its one-shot flag is cleared before this retry.
            System.Windows.Automation.ClientSettings.RegisterClientSideProviders(table);
        }
    }
    private delegate bool EnumProc(IntPtr window, IntPtr unused);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr unused);
    [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr parent,EnumProc callback,IntPtr unused);
    [DllImport("user32.dll")] private static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll")] private static extern IntPtr GetParent(IntPtr window);
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr window,uint command);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] private static extern int GetClassNameW(IntPtr window, StringBuilder text, int capacity);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll",EntryPoint="GetWindowLongPtrW")] private static extern IntPtr WindowStyle(IntPtr window, int index);
    [DllImport("user32.dll")] private static extern bool PostMessageW(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll",EntryPoint="SendMessageTimeoutW",SetLastError=true)]
    private static extern IntPtr MessageTimeout(IntPtr window, uint message, IntPtr wparam, IntPtr lparam, uint flags, uint timeout, out UIntPtr result);
    [DllImport("user32.dll",EntryPoint="SendMessageTimeoutW",CharSet=CharSet.Unicode,SetLastError=true)]
    private static extern IntPtr ReadTimeout(IntPtr window, uint message, IntPtr capacity, StringBuilder text, uint flags, uint timeout, out UIntPtr result);
    [DllImport("user32.dll",EntryPoint="SendMessageTimeoutW",CharSet=CharSet.Unicode,SetLastError=true)]
    private static extern IntPtr WriteTimeout(IntPtr window, uint message, IntPtr unused, string text, uint flags, uint timeout, out UIntPtr result);
    [DllImport("user32.dll",EntryPoint="SendMessageTimeoutW",SetLastError=true)]
    private static extern IntPtr SelectionTimeout(IntPtr window, uint message, out uint start, out uint end, uint flags, uint timeout, out UIntPtr result);
    public static void Own(uint process, IntPtr window) {
        uint actual; GetWindowThreadProcessId(window, out actual);
        if (window == IntPtr.Zero || actual != process) throw new InvalidOperationException("HWND is not owned by the isolated test process");
    }
    public static string ClassName(IntPtr window) {
        var text = new StringBuilder(128); GetClassNameW(window, text, text.Capacity); return text.ToString();
    }
    public static IntPtr MainWindow(uint process) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((window, unused) => {
            uint actual; GetWindowThreadProcessId(window, out actual);
            if (actual != process || ClassName(window) != "BksTermWindow") return true;
            result = window; return false;
        }, IntPtr.Zero);
        return result;
    }
    public static IntPtr VisibleClass(uint process, string className) {
        IntPtr result=IntPtr.Zero;
        EnumWindows((window,unused)=>{
            uint actual; GetWindowThreadProcessId(window,out actual);
            if(actual!=process || !IsWindowVisible(window) || ClassName(window)!=className)return true;
            result=window; return false;
        },IntPtr.Zero);
        return result;
    }
    public sealed class WindowInfo {
        public long Handle{get;set;} public long Parent{get;set;} public long Owner{get;set;}
        public int ControlId{get;set;} public string Class{get;set;} public bool Visible{get;set;}
    }
    public static WindowInfo[] DialogTree(uint process,IntPtr dialog) {
        Own(process,dialog); var result=new List<WindowInfo>();
        EnumProc add=(window,unused)=>{
            uint actual; GetWindowThreadProcessId(window,out actual); if(actual!=process)return true;
            result.Add(new WindowInfo{Handle=window.ToInt64(),Parent=GetParent(window).ToInt64(),Owner=GetWindow(window,4).ToInt64(),ControlId=GetDlgCtrlID(window),Class=ClassName(window),Visible=IsWindowVisible(window)});
            return true;
        };
        add(dialog,IntPtr.Zero); EnumChildWindows(dialog,add,IntPtr.Zero); return result.ToArray();
    }
    public static bool Responsive(uint process, IntPtr window, uint timeout, uint flags) {
        Own(process, window); UIntPtr result;
        bool ok = MessageTimeout(window, 0, IntPtr.Zero, IntPtr.Zero, flags, timeout, out result) != IntPtr.Zero;
        ResponseError = ok ? 0 : Marshal.GetLastWin32Error();
        return ok;
    }
    public static string Text(uint process, IntPtr window) {
        Own(process, window); UIntPtr length;
        if (MessageTimeout(window, 0x000E, IntPtr.Zero, IntPtr.Zero, 3, 1500, out length) == IntPtr.Zero)
            throw new TimeoutException("Own WM_GETTEXTLENGTH exceeded 1500 ms");
        if (length.ToUInt64() > 8 * 1024 * 1024) throw new InvalidOperationException("Own control exceeded the bounded test read size");
        var text = new StringBuilder((int)length.ToUInt64() + 1); UIntPtr copied;
        if (ReadTimeout(window, 0x000D, new IntPtr(text.Capacity), text, 3, 1500, out copied) == IntPtr.Zero)
            throw new TimeoutException("Own WM_GETTEXT exceeded 1500 ms");
        return text.ToString();
    }
    public static void SetText(uint process, IntPtr window, string text) {
        Own(process, window);
        if (!ClassName(window).Equals("Edit", StringComparison.OrdinalIgnoreCase)) throw new InvalidOperationException("Fixture target must be EDIT");
        UIntPtr result;
        if (WriteTimeout(window, 0x000C, IntPtr.Zero, text, 3, 1500, out result) == IntPtr.Zero)
            throw new TimeoutException("Own WM_SETTEXT exceeded 1500 ms");
    }
    public static bool ReadOnly(uint process, IntPtr window) { Own(process, window); return (WindowStyle(window,-16).ToInt64() & 0x800) != 0; }
    public static bool Password(uint process, IntPtr window) { Own(process, window); return (WindowStyle(window,-16).ToInt64() & 0x20) != 0; }
    public static int Length(uint process, IntPtr window) { return ReadMessage(process,window,0x000E,0); }
    public static void ComboChoice(uint process, IntPtr editor, IntPtr combo, int id, int index) {
        Own(process,editor); Own(process,combo); UIntPtr result;
        if(MessageTimeout(combo,0x014E,new IntPtr(index),IntPtr.Zero,2,1500,out result)==IntPtr.Zero)
            throw new TimeoutException("Own CB_SETCURSEL exceeded 1500 ms");
        if(!PostMessageW(editor,0x0111,new IntPtr(id | (1 << 16)),combo))
            throw new InvalidOperationException("Own CBN_SELCHANGE failed");
    }
    private static int ReadMessage(uint process, IntPtr window, uint message, int value) {
        Own(process,window); UIntPtr result;
        if(MessageTimeout(window,message,new IntPtr(value),IntPtr.Zero,2,1500,out result)==IntPtr.Zero)
            throw new TimeoutException("Own native reading message exceeded 1500 ms");
        return unchecked((int)result.ToUInt64());
    }
    public static uint[] Selection(uint process, IntPtr window) {
        Own(process,window); uint start,end; UIntPtr result;
        if(SelectionTimeout(window,0x00B0,out start,out end,2,1500,out result)==IntPtr.Zero)
            throw new TimeoutException("Own EM_GETSEL exceeded 1500 ms");
        return new[]{start,end};
    }
    public static void Select(uint process, IntPtr window, int start, int end) {
        Own(process,window); UIntPtr result;
        if(MessageTimeout(window,0x00B1,new IntPtr(start),new IntPtr(end),2,1500,out result)==IntPtr.Zero)
            throw new TimeoutException("Own EM_SETSEL exceeded 1500 ms");
    }
    public static int FirstLine(uint process, IntPtr window) { return ReadMessage(process,window,0x00CE,0); }
    public static int LineIndex(uint process, IntPtr window, int line) { return ReadMessage(process,window,0x00BB,line); }
    public static void ScrollTo(uint process, IntPtr window, int character) {
        int line=ReadMessage(process,window,0x00C9,character);
        int first=FirstLine(process,window); UIntPtr result;
        if(MessageTimeout(window,0x00B6,IntPtr.Zero,new IntPtr(line-first),2,1500,out result)==IntPtr.Zero)
            throw new TimeoutException("Own EM_LINESCROLL exceeded 1500 ms");
        if(MessageTimeout(window,0x00B7,IntPtr.Zero,IntPtr.Zero,2,1500,out result)==IntPtr.Zero)
            throw new TimeoutException("Own EM_SCROLLCARET exceeded 1500 ms");
    }
    public static int LineFeeds(string text) { int count=0; foreach(char ch in text) if(ch=='\n') count++; return count; }
    public static void Post(uint process, IntPtr window, uint message) {
        Own(process, window);
        if (!PostMessageW(window, message, IntPtr.Zero, IntPtr.Zero)) throw new InvalidOperationException("Own PostMessage failed");
    }
}
'@
[BksStage7]::InitializeUia()
if ($LifecycleOnly -or $ProfilesOnly) {
    # Reuse only the compiled, own-PID-guarded UIA event/menu helper; the
    # stage6 application scenario is never evaluated or launched here.
    $stage6Source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'check-stage6.ps1'),[Text.Encoding]::UTF8)
    $sharedCode = [regex]::Match($stage6Source,"(?s)using System;.*?(?=\r?\n'@)").Value
    if (-not $sharedCode.Contains('public static class BksStage6')) { throw 'Shared UIA helper unavailable' }
    Add-Type -ReferencedAssemblies $references -TypeDefinition $sharedCode
}

$mode = if($LifecycleOnly){'LifecycleOnly'}elseif($ProfilesOnly){'ProfilesOnly'}elseif($GitBashOnly){'GitBashOnly'}else{'OutputOnly'}
$runName = $mode.ToLowerInvariant() + '-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0,8)
$runDirectory = (New-Item -ItemType Directory -Force -Path (Join-Path $Artifacts $runName)).FullName
$appData = (New-Item -ItemType Directory -Path (Join-Path $runDirectory 'appdata')).FullName
$checks = [Collections.Generic.List[string]]::new()
$observations = [Collections.Generic.List[object]]::new()
$process = $null
$main = [IntPtr]::Zero
$failure = $null
$maxPingMs = 0
$pingCount = 0
$pingFailures = 0
$responseFlags = if ($MtaDiagnostics) { 2 } else { 3 }
$performancePath = Join-Path $runDirectory 'performance.log'

function Check([string]$Name,[bool]$Condition) {
    if (-not $Condition) { throw "FAILED: $Name" }
    $checks.Add("passed: $Name")
}
function Wait-For([scriptblock]$Condition,[string]$Name,[int]$Timeout=15000) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.ElapsedMilliseconds -lt $Timeout) {
        if (& $Condition) { return }
        if ($process.HasExited) { throw "Isolated process exited: $Name" }
        Start-Sleep -Milliseconds 40
    }
    throw "Timeout: $Name"
}
function Element([IntPtr]$Window) {
    [BksStage7]::Own($process.Id,$Window)
    [Windows.Automation.AutomationElement]::FromHandle($Window)
}
function Uia-Text([IntPtr]$Window) {
    $pattern = $null
    if (-not (Element $Window).TryGetCurrentPattern([Windows.Automation.TextPattern]::Pattern,[ref]$pattern)) { throw 'Native EDIT does not expose TextPattern' }
    $pattern.DocumentRange.GetText(-1)
}
function Click-Own([IntPtr]$Parent,[int]$Id) {
    [BksStage7]::Own($process.Id,$Parent)
    Wait-For { $child=[BksStage7]::GetDlgItem($Parent,$Id); $child -ne [IntPtr]::Zero -and [BksStage7]::IsWindowVisible($child) -and [BksStage7]::IsWindowEnabled($child) } "own button $Id ready"
    [BksStage7]::Post($process.Id,[BksStage7]::GetDlgItem($Parent,$Id),0x00F5)
}
function Open-OwnDialog([int]$Command,[string]$Class) {
    [BksStage6]::Post($process.Id,$main,0x111,$Command)
    Wait-For {
        $dialog=[BksStage6]::ProcessClass($process.Id,$Class)
        $last=if($Class -eq 'BksTermSshProfileEditor'){4107}else{4303}
        $dialog -ne [IntPtr]::Zero -and [BksStage7]::IsWindowVisible($dialog) -and [BksStage7]::GetDlgItem($dialog,$last) -ne [IntPtr]::Zero
    } "own $Class opens with initialized controls"
    [BksStage6]::ProcessClass($process.Id,$Class)
}
function Invoke-Profiles {
    Wait-For { (Uia-Text $journal).Contains('BKS7_HIDDEN_READY') } 'local hidden-input fixture ready'
    $draft='stage7_protected_draft_Ж'
    [BksStage7]::SetText($process.Id,$inputBox,$draft)
    $profilePath=Join-Path $appData 'BKS Term/profiles.json'
    $identityPath=Join-Path $runDirectory 'fixture identity path'
    [IO.File]::WriteAllText($identityPath,'fixture_identity_marker_only',[Text.Encoding]::ASCII)
    $editor=Open-OwnDialog 60009 'BksTermSshProfileEditor'
    foreach($id in 4101..4105){
        $field=[BksStage7]::GetDlgItem($editor,$id)
        Check "SSH field $id is a named native editable field" ([BksStage7]::ClassName($field) -ieq 'Edit' -and (Element $field).Current.Name.Length -gt 0)
    }
    foreach($pair in @(@(4101,'Stage7 SSH Ж'),@(4102,'stage7-fixture-alias'),@(4103,'fixture-user'),@(4104,'70000'),@(4105,$identityPath))){[BksStage7]::SetText($process.Id,[BksStage7]::GetDlgItem($editor,$pair[0]),$pair[1])}
    Click-Own $editor 4106
    $validation=@{Button=$null;Invoke=$null;Dialog=[IntPtr]::Zero}
    Wait-For {
        $dialog=[BksStage7]::VisibleClass($process.Id,'#32770')
        if($dialog -eq [IntPtr]::Zero){return $false}
        $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Button)
        $validation.Button=(Element $dialog).FindFirst([Windows.Automation.TreeScope]::Descendants,$condition)
        $pattern=$null
        if($validation.Button -and $validation.Button.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern,[ref]$pattern)){
            $validation.Invoke=$pattern; $validation.Dialog=$dialog; return $true
        }
        return $false
    } 'visible own validation dialog and accessible acknowledgement button'
    $observations.Add([ordered]@{Label='SSH validation';ButtonName=$validation.Button.Current.Name;ButtonAutomationId=$validation.Button.Current.AutomationId;NativeTree=[BksStage7]::DialogTree($process.Id,$validation.Dialog)})
    $validation.Invoke.Invoke()
    Wait-For { [BksStage7]::VisibleClass($process.Id,'#32770') -eq [IntPtr]::Zero } 'validation message dismissed'
    Check 'invalid SSH port leaves the editor open and profile unsaved' ([BksStage7]::IsWindowVisible($editor) -and -not ([IO.File]::ReadAllText($profilePath).Contains('Stage7 SSH Ж')))
    [BksStage7]::SetText($process.Id,[BksStage7]::GetDlgItem($editor,4104),'2222')
    Click-Own $editor 4106
    Wait-For { [BksStage6]::ProcessClass($process.Id,'BksTermSshProfileEditor') -eq [IntPtr]::Zero -and [IO.File]::ReadAllText($profilePath).Contains('Stage7 SSH Ж') } 'SSH profile saved atomically'
    $saved=([IO.File]::ReadAllText($profilePath,[Text.Encoding]::UTF8) | ConvertFrom-Json).profiles | Where-Object {$_.kind -eq 'ssh'} | Select-Object -First 1
    Check 'saved SSH profile keeps alias, port, user and identity path as arguments' ($saved.name -eq 'Stage7 SSH Ж' -and $saved.args -contains 'stage7-fixture-alias' -and $saved.args -contains '2222' -and $saved.args -contains 'fixture-user' -and $saved.args -contains $identityPath)
    Check 'saving SSH metadata never launches an SSH tab or replaces the current draft' ([BksStage7]::GetDlgItem($main,2002) -eq [IntPtr]::Zero -and [BksStage7]::Text($process.Id,$inputBox) -eq $draft)
    $editor=Open-OwnDialog 60009 'BksTermSshProfileEditor'
    [BksStage7]::ComboChoice($process.Id,$editor,[BksStage7]::GetDlgItem($editor,4100),4100,1)
    Wait-For { [BksStage7]::Text($process.Id,[BksStage7]::GetDlgItem($editor,4101)) -eq 'Stage7 SSH Ж' } 'saved SSH fields reload'
    Check 'SSH editor reloads the saved identity and port' ([BksStage7]::Text($process.Id,[BksStage7]::GetDlgItem($editor,4105)) -eq $identityPath -and [BksStage7]::Text($process.Id,[BksStage7]::GetDlgItem($editor,4104)) -eq '2222')
    [BksStage7]::SetText($process.Id,[BksStage7]::GetDlgItem($editor,4101),'Stage7 SSH edited Ж')
    Click-Own $editor 4106
    Wait-For { [IO.File]::ReadAllText($profilePath).Contains('Stage7 SSH edited Ж') } 'existing SSH profile edited and saved'
    $fileBeforeCancel=[IO.File]::ReadAllText($profilePath)
    $editor=Open-OwnDialog 60009 'BksTermSshProfileEditor'
    [BksStage7]::SetText($process.Id,[BksStage7]::GetDlgItem($editor,4101),'discarded_profile_edit')
    Click-Own $editor 4107
    Wait-For { [BksStage6]::ProcessClass($process.Id,'BksTermSshProfileEditor') -eq [IntPtr]::Zero } 'SSH editor cancel'
    Check 'SSH cancel preserves persisted metadata and restores the owner' ([IO.File]::ReadAllText($profilePath) -eq $fileBeforeCancel -and [BksStage7]::IsWindowEnabled($main))

    $secret=Open-OwnDialog 60010 'BksTermSecretInput'
    $password=[BksStage7]::GetDlgItem($secret,4301)
    Check 'protected input exposes ES_PASSWORD and UIA IsPassword' ([BksStage7]::Password($process.Id,$password) -and (Element $password).Current.IsPassword)
    [BksStage7]::SetText($process.Id,$password,$canary)
    Click-Own $secret 4303
    Wait-For { [BksStage6]::ProcessClass($process.Id,'BksTermSecretInput') -eq [IntPtr]::Zero } 'protected input cancelled'
    Check 'protected cancel sends no completed input to the local hidden fixture' (-not (Uia-Text $journal).Contains('BKS7_HIDDEN_ACCEPTED') -and -not (Uia-Text $journal).Contains('BKS7_HIDDEN_REJECTED'))
    $secret=Open-OwnDialog 60010 'BksTermSecretInput'
    $password=[BksStage7]::GetDlgItem($secret,4301)
    Check 'reopened protected input is empty' ([BksStage7]::Length($process.Id,$password) -eq 0)
    [BksStage7]::SetText($process.Id,$password,$canary)
    Click-Own $secret 4302
    Wait-For { (Uia-Text $journal).Contains('BKS7_HIDDEN_ACCEPTED') -or (Uia-Text $journal).Contains('BKS7_HIDDEN_REJECTED') } 'protected UTF8 input reaches the local fixture'
    $transcript=Uia-Text $journal
    Check 'protected input sends one exact canary plus Enter after cancel' ($transcript.Contains('BKS7_HIDDEN_ACCEPTED') -and -not $transcript.Contains('BKS7_HIDDEN_REJECTED'))
    Check 'protected input never enters the command draft, profile or transcript' ([BksStage7]::Text($process.Id,$inputBox) -eq $draft -and -not $transcript.Contains($canary) -and -not [IO.File]::ReadAllText($profilePath).Contains($canary) -and -not [IO.File]::ReadAllText($profilePath).Contains('fixture_identity_marker_only'))
    Check 'protected submit restores the native owner without launching SSH' ([BksStage7]::IsWindowEnabled($main) -and [BksStage7]::GetDlgItem($main,2002) -eq [IntPtr]::Zero)
    $observations.Add([ordered]@{Label='SSH metadata and protected fixture';SavedProfileId=$saved.id;Host='stage7-fixture-alias';Port=2222;PasswordRole=$true;CancelledInputIgnored=$true;CanaryHashAccepted=$true;PlaintextLogged=$false;RemoteConnectionAttempted=$false})
}
function Invoke-GitBash {
    $profilePath=Join-Path $appData 'BKS Term/profiles.json'
    $before=[IO.File]::ReadAllText($profilePath)
    $glyphs='Ж'+[char]0x754c+'e'+[char]0x301+[char]::ConvertFromUtf32(0x1f600)
    $command="printf '%s\n' 'BKS7_GIT_''PWD'; pwd; printf '%s\n' 'BKS7_GIT_''UNI_$glyphs'; printf '%b\n' 'BKS7_GIT_''GENERATED_\u0416\u754ce\u0301\U0001f600'; printf '%s ' 'BKS7_GIT_''HEX'; printf '%b' '\u0416\u754ce\u0301\U0001f600' | od -An -tx1; git --version; printf '%s\n' 'BKS7_GIT_''DONE'"
    [BksStage7]::SetText($process.Id,$inputBox,$command)
    $nativeInput=[BksStage7]::Text($process.Id,$inputBox)
    $controlledInput=[regex]::Match($nativeInput,"UNI_[^']*").Value
    $observations.Add([ordered]@{Label='controlled Git Bash native command input';SupplementaryUnicodePresent=$nativeInput.Contains($glyphs);Utf16Units=@($controlledInput.ToCharArray() | ForEach-Object {'U+{0:X4}' -f [int]$_});SourceHasNoLineBreaks=($command -notmatch '[\r\n]')})
    [BksStage7]::Post($process.Id,$execute,0x00F5)
    Wait-For { (Uia-Text $journal).Contains('BKS7_GIT_DONE') } 'real Git Bash command output' 20000
    $text=Uia-Text $journal
    $actualUnicode=[regex]::Match($text,'BKS7_GIT_UNI_[^\r\n]*').Value
    $observations.Add([ordered]@{Label='controlled Git Bash Unicode output';Actual=$actualUnicode;Utf16Units=@($actualUnicode.ToCharArray() | ForEach-Object {'U+{0:X4}' -f [int]$_});Expected='BKS7_GIT_UNI_'+$glyphs;Generated=[regex]::Match($text,'BKS7_GIT_GENERATED_[^\r\n]*').Value;GeneratedHex=[regex]::Match($text,'BKS7_GIT_HEX[^\r\n]*').Value;GitVersion=[regex]::Match($text,'git version \d+[^\r\n]*').Value})
    Check 'Git Bash executes pwd with a Unix path' ($text -match 'BKS7_GIT_PWD\r?\n/[^\r\n]*')
    Check 'Git Bash preserves generated Unicode output through ConPTY and native UIA' ($text.Contains('BKS7_GIT_GENERATED_'+$glyphs))
    Check 'Git Bash executes the installed Git client' ($text -match 'git version \d+\.')
    Check 'legacy builtin Git Bash alias is corrected only at runtime' ([IO.File]::ReadAllText($profilePath) -eq $before)
    Check 'Git Bash preserves supplementary Unicode supplied literally in command input' ($text.Contains('BKS7_GIT_UNI_'+$glyphs))
    $observations.Add([ordered]@{Label='real Git Bash smoke';LegacyExecutable=$legacyBash;Pwd=$true;Unicode=$true;GitVersion=$true;ProfileFileUnchanged=$true})
}
function Invoke-Lifecycle {
    $status = [BksStage7]::GetDlgItem($main,1005)
    $notice = 'Stage7 CMD: сессия завершена, код 7'
    $tail = 'BKS7_LIFE_FINAL_TAIL'
    Wait-For { [BksStage7]::Text($process.Id,$status).Contains('Завершено, код 7') } 'CMD exit code 7 after final output'
    $oldText = Uia-Text $journal
    $oldInstance = [regex]::Match($oldText,'BKS7_LIFE_INSTANCE_\d+_\d+').Value
    Check 'exited CMD retains its final stdout tail through UIA' ($oldText.Contains($tail) -and $oldInstance.Length -gt 0)
    Check 'exit status and tab accessible name expose code 7' ((Element $status).Current.Name.Contains('Завершено, код 7') -and (Element ([BksStage7]::GetDlgItem($main,2001))).Current.Name.Contains('7'))
    Check 'exit notice is journaled once after the final tail' ([regex]::Matches($oldText,[regex]::Escape($notice)).Count -eq 1 -and $oldText.IndexOf($tail) -lt $oldText.IndexOf($notice))
    $quiet = [Diagnostics.Stopwatch]::StartNew()
    while($quiet.ElapsedMilliseconds -lt 500){Start-Sleep -Milliseconds 100}
    Check 'polling does not duplicate the exit notice' ([regex]::Matches((Uia-Text $journal),[regex]::Escape($notice)).Count -eq 1)
    [BksStage7]::SetText($process.Id,$inputBox,'old_exited_draft_Ж')
    $announcement = [BksStage7]::GetDlgItem($main,1026)
    [BksStage6]::WatchNotifications($process.Id,$announcement)
    try {
        [BksStage6]::Activate($process.Id,$main)
        [BksStage6]::Post($process.Id,$main,0x111,60008)
        Wait-For { [BksStage7]::GetDlgItem($main,2002) -ne [IntPtr]::Zero -and [BksStage7]::Text($process.Id,$status).Contains('Завершено, код 7') -and (Uia-Text $journal).Contains($tail) } 'restart creates a fresh completed CMD tab'
        $freshText = Uia-Text $journal
        $freshInstance = [regex]::Match($freshText,'BKS7_LIFE_INSTANCE_\d+_\d+').Value
        Check 'restart launches a fresh PTY with an independent output instance' ($freshInstance.Length -gt 0 -and $freshInstance -ne $oldInstance -and -not $freshText.Contains($oldInstance))
        Check 'fresh session journals one completion and starts with an empty draft' ([regex]::Matches($freshText,[regex]::Escape($notice)).Count -eq 1 -and [BksStage7]::Text($process.Id,$inputBox).Length -eq 0)
        $foreground = [BksStage6]::ProcessId([BksStage6]::GetForegroundWindow()) -eq $process.Id
        if($foreground){
            Wait-For { [BksStage6]::ProcessId([BksStage6]::GetForegroundWindow()) -ne $process.Id -or [BksStage6]::Notifications().Length -gt 0 } 'foreground lifecycle notification'
            if([BksStage6]::ProcessId([BksStage6]::GetForegroundWindow()) -eq $process.Id){
                $events = [BksStage6]::Notifications()
                Check 'foreground restart emits one concise completion Notification' ($events.Length -eq 1 -and $events[0].Text -eq $notice -and $events[0].Activity -eq 'bks.feedback')
            }else{$checks.Add('skipped: own foreground lost before lifecycle Notification verification')}
        }else{$checks.Add('skipped: own foreground unavailable for lifecycle Notification verification')}
        $observations.Add([ordered]@{ Label='lifecycle notifications'; Events=[BksStage6]::Notifications() })
    }finally{[BksStage6]::StopNotifications()}
    [BksStage6]::Post($process.Id,$main,0x111,2001)
    Wait-For { (Uia-Text $journal).Contains($oldInstance) } 'old exited tab restored'
    Check 'restart preserves the old output and draft' ((Uia-Text $journal).Contains($tail) -and [BksStage7]::Text($process.Id,$inputBox) -eq 'old_exited_draft_Ж')

    # Create a live local CMD by invoking its own native profile menu item.
    [BksStage7]::Post($process.Id,[BksStage7]::GetDlgItem($main,1010),0x00F5)
    $resolved = @{ Item=$null }
    Wait-For {
        $popup=[BksStage6]::ProcessClass($process.Id,'#32768')
        if($popup -eq [IntPtr]::Zero -or -not [BksStage7]::IsWindowVisible($popup)){return $false}
        $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty,'Stage7 Live CMD')
        $resolved.Item=(Element $popup).FindFirst([Windows.Automation.TreeScope]::Descendants,$condition)
        $null -ne $resolved.Item
    } 'own profile popup exposes live CMD'
    $invoke=$null
    Check 'live CMD profile choice has native InvokePattern' ($resolved.Item.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern,[ref]$invoke))
    $invoke.Invoke()
    Wait-For { [BksStage7]::GetDlgItem($main,2003) -ne [IntPtr]::Zero -and (Uia-Text $journal).Contains('BKS7_LIVE_READY') } 'live local CMD created'
    Check 'live CMD status remains connected' ([BksStage7]::Text($process.Id,$status).Contains('Подключено'))
    [BksStage7]::Post($process.Id,[BksStage7]::GetDlgItem($main,3003),0x00F5)
    Wait-For { [BksStage6]::ProcessClass($process.Id,'#32770') -ne [IntPtr]::Zero } 'own live-close confirmation'
    $dialog=[BksStage6]::ProcessClass($process.Id,'#32770')
    $yes=[BksStage7]::GetDlgItem($dialog,6)
    [BksStage7]::Own($process.Id,$yes)
    $closeTimer=[Diagnostics.Stopwatch]::StartNew()
    [BksStage7]::Post($process.Id,$yes,0x00F5)
    Wait-For {
        if(-not [BksStage7]::Responsive($process.Id,$main,1000,2)){throw 'FAILED: live tab close blocked UI beyond 1000 ms'}
        [BksStage7]::GetDlgItem($main,2003) -eq [IntPtr]::Zero
    } 'live tab retires without blocking UI'
    Check 'live tab closes within 1000 ms after confirmation' ($closeTimer.ElapsedMilliseconds -lt 1000)
    Check 'closing the live tab preserves both completed tabs' ([BksStage7]::GetDlgItem($main,2001) -ne [IntPtr]::Zero -and [BksStage7]::GetDlgItem($main,2002) -ne [IntPtr]::Zero)
    $observations.Add([ordered]@{ Label='lifecycle'; OldInstance=$oldInstance; NewInstance=$freshInstance; ExitCode=7; Tail=$tail; OldNoticeCount=1; FreshNoticeCount=1; LiveCloseMs=$closeTimer.ElapsedMilliseconds })
}

try {
    if($ProfilesOnly -or $GitBashOnly){
        $profileDirectory=(New-Item -ItemType Directory -Force -Path (Join-Path $appData 'BKS Term')).FullName
        if($ProfilesOnly){
            $canary='stage7_fake_Ж_72'
            $hasher=[Security.Cryptography.SHA256]::Create()
            try{$digest=([BitConverter]::ToString($hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($canary)))).Replace('-','').ToLowerInvariant()}finally{$hasher.Dispose()}
            $fixture="[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); [Console]::WriteLine('BKS7_'+'HIDDEN_READY'); `$b=[Text.StringBuilder]::new(); while(`$true){`$k=[Console]::ReadKey(`$true); if(`$k.Key -eq [ConsoleKey]::Enter){break}; [void]`$b.Append(`$k.KeyChar)}; `$h=[Security.Cryptography.SHA256]::Create(); `$s=([BitConverter]::ToString(`$h.ComputeHash([Text.Encoding]::UTF8.GetBytes(`$b.ToString())))).Replace('-','').ToLowerInvariant(); `$h.Dispose(); if(`$s -eq '$digest'){[Console]::WriteLine('BKS7_'+'HIDDEN_ACCEPTED')}else{[Console]::WriteLine('BKS7_'+'HIDDEN_REJECTED')}"
            $testProfile=[ordered]@{id='stage7-hidden';name='Stage7 Hidden Local';kind='local';executable='powershell.exe';args=@('-NoLogo','-NoProfile','-NoExit','-Command',$fixture);working_directory=$null}
        }else{
            $legacyBash=Join-Path $env:LOCALAPPDATA 'Microsoft/WindowsApps/bash.exe'
            $testProfile=[ordered]@{id='git-bash';name='Stage7 Git Bash';kind='local';executable=$legacyBash;args=@('--login','-i');working_directory=$null}
        }
        $profileJson=[ordered]@{schema_version=1;default_profile_id=$testProfile.id;profiles=@($testProfile)} | ConvertTo-Json -Depth 5
        [IO.File]::WriteAllText((Join-Path $profileDirectory 'profiles.json'),$profileJson,[Text.UTF8Encoding]::new($false))
    }
    if($LifecycleOnly){
        $profileDirectory=(New-Item -ItemType Directory -Force -Path (Join-Path $appData 'BKS Term')).FullName
        $profiles=@(
            [ordered]@{id='stage7-cmd';name='Stage7 CMD';kind='local';executable='cmd.exe';args=@('/D','/Q','/C','echo BKS7_LIFE_INSTANCE_%RANDOM%_%RANDOM%&<nul set /p =BKS7_LIFE_FINAL_TAIL&exit /b 7');working_directory=$null},
            [ordered]@{id='stage7-live';name='Stage7 Live CMD';kind='local';executable='cmd.exe';args=@('/D','/Q','/K','echo BKS7_LIVE_READY');working_directory=$null})
        $profileJson=[ordered]@{schema_version=1;default_profile_id='stage7-cmd';profiles=$profiles} | ConvertTo-Json -Depth 5
        [IO.File]::WriteAllText((Join-Path $profileDirectory 'profiles.json'),$profileJson,[Text.UTF8Encoding]::new($false))
    }
    $Exe = (Resolve-Path -LiteralPath $Exe).Path
    $start = [Diagnostics.ProcessStartInfo]::new($Exe)
    $start.UseShellExecute = $false
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.EnvironmentVariables['APPDATA'] = $appData
    if ($PerformanceTrace) { $start.EnvironmentVariables['BKS_TERM_PERF_LOG'] = $performancePath }
    $process = [Diagnostics.Process]::Start($start)
    Wait-For { [BksStage7]::MainWindow($process.Id) -ne [IntPtr]::Zero } 'main window'
    $main = [BksStage7]::MainWindow($process.Id)
    Wait-For { [BksStage7]::GetDlgItem($main,1003) -ne [IntPtr]::Zero } 'execute control initialized'
    $journal = [BksStage7]::GetDlgItem($main,1001)
    $inputBox = [BksStage7]::GetDlgItem($main,1002)
    $execute = [BksStage7]::GetDlgItem($main,1003)
    Wait-For { [BksStage7]::Text($process.Id,$journal).Length -gt 0 } 'native startup text available'
    Check 'startup shows native journal and command editor' ([BksStage7]::IsWindowVisible($journal) -and [BksStage7]::IsWindowVisible($inputBox))
    Check 'journal exposes named read-only native EDIT and TextPattern' ([BksStage7]::ClassName($journal) -ieq 'Edit' -and [BksStage7]::ReadOnly($process.Id,$journal) -and (Element $journal).Current.Name.Contains('Журнал') -and (Uia-Text $journal).Length -gt 0)
    Check 'command editor is named and editable' ([BksStage7]::ClassName($inputBox) -ieq 'Edit' -and -not [BksStage7]::ReadOnly($process.Id,$inputBox) -and (Element $inputBox).Current.Name.Contains('Команда'))
    if($ProfilesOnly){
        Invoke-Profiles
    }elseif($GitBashOnly){
        Invoke-GitBash
    }elseif($LifecycleOnly){
        Invoke-Lifecycle
    }else{
    Wait-For { [BksStage7]::Text($process.Id,$journal).Contains('PS ') } 'PowerShell ready'

    $glyphs = 'Ж' + [char]0x754c + 'e' + [char]0x301 + [char]::ConvertFromUtf32(0x1f600)
    $anchor = 'BKS7_ANCHOR_' + $glyphs
    $anchorGate = Join-Path $appData 'output-anchor.ready'
    $bookmark = $null
    $fixture = "[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); `$g=''+[char]0x416+[char]0x754c+'e'+[char]0x301+[char]::ConvertFromUtf32(0x1f600); [Console]::WriteLine('BKS7_'+'ARMED'); Start-Sleep -Seconds 1; 1..30000|%{[Console]::WriteLine(('BKS7_ROW_{0:D6}_' -f `$_)+`$g+'_'+('x'*56)); if(`$_ -eq 28000){[Console]::WriteLine('BKS7_ANCHOR_'+`$g); `$gate=[IO.Path]::Combine(`$env:APPDATA,'output-anchor.ready'); `$wait=[Diagnostics.Stopwatch]::StartNew(); while((-not [IO.File]::Exists(`$gate)) -and `$wait.ElapsedMilliseconds -lt 10000){Start-Sleep -Milliseconds 50}}}; [Console]::WriteLine('BKS7_'+'FLOOD_FINISHED')"
    [BksStage7]::SetText($process.Id,$inputBox,$fixture)
    [BksStage7]::Post($process.Id,$execute,0x00F5) # BM_CLICK: explicit harmless fixture.
    Wait-For { [BksStage7]::Text($process.Id,$inputBox).Length -eq 0 -and [BksStage7]::Text($process.Id,$journal).Contains('BKS7_ARMED') } 'finite producer accepted and armed'
    $draft = 'stage7_draft_Ж中'
    [BksStage7]::SetText($process.Id,$inputBox,$draft)

    $timer = [Diagnostics.Stopwatch]::StartNew()
    $lastRead = -500
    $lastText = ''
    while ($timer.ElapsedMilliseconds -lt 45000) {
        $ping = [Diagnostics.Stopwatch]::StartNew()
        if (-not [BksStage7]::Responsive($process.Id,$main,1000,$responseFlags)) {
            $observations.Add([ordered]@{ Label='failed own WM_NULL'; ElapsedMs=$ping.ElapsedMilliseconds; TimeoutMs=1000; Win32Error=[BksStage7]::ResponseError; LastNativeTextUtf16=$lastText.Length; NativeReadAgeMs=($timer.ElapsedMilliseconds-$lastRead) })
            if (-not $PerformanceTrace) { throw 'FAILED: own WM_NULL exceeded 1000 ms during finite output' }
            $pingFailures++
            $checks.Add('failed: own WM_NULL exceeded 1000 ms during finite output')
            Write-Output 'Observed WM_NULL timeout; retaining failed result and collecting final output/performance trace'
        } else {
            $maxPingMs = [Math]::Max($maxPingMs,$ping.ElapsedMilliseconds)
            $pingCount++
        }
        if ($timer.ElapsedMilliseconds - $lastRead -ge 500) {
            $lastRead = $timer.ElapsedMilliseconds
            try { $lastText = [BksStage7]::Text($process.Id,$journal) }
            catch {
                if (-not $PerformanceTrace -or -not ($_.Exception.InnerException -is [TimeoutException])) { throw }
                $checks.Add('failed: timed native text read during finite output')
                $observations.Add([ordered]@{ Label='timed native text read'; Error=$_.Exception.Message; LastNativeTextUtf16=$lastText.Length })
            }
            if (-not $bookmark -and $lastText.Contains($anchor)) {
                Check 'retained anchor is captured before the producer finishes' (-not $lastText.Contains('BKS7_FLOOD_FINISHED'))
                $position = $lastText.IndexOf($anchor,[StringComparison]::Ordinal)
                [BksStage7]::Select($process.Id,$journal,$position,$position+$anchor.Length)
                [BksStage7]::ScrollTo($process.Id,$journal,$position)
                $top = [BksStage7]::LineIndex($process.Id,$journal,[BksStage7]::FirstLine($process.Id,$journal))
                $logicalStart = if($top -gt 0){$lastText.LastIndexOf("`n",$top-1)+1}else{0}
                $logicalEnd = $lastText.IndexOf("`r`n",$logicalStart)
                if($logicalEnd -lt 0){$logicalEnd=$lastText.Length}
                $bookmark = [pscustomobject]@{ Start=$position; End=$position+$anchor.Length; LogicalText=$lastText.Substring($logicalStart,$logicalEnd-$logicalStart); WithinLogicalLine=$top-$logicalStart }
                $observations.Add([ordered]@{ Label='retained reading bookmark before trim'; Start=$bookmark.Start; End=$bookmark.End; ViewportLine=$bookmark.LogicalText; ViewportWithinLine=$bookmark.WithinLogicalLine })
                [IO.File]::WriteAllText($anchorGate,'ready',[Text.Encoding]::ASCII)
            }
            if ($lastText.Contains('BKS7_FLOOD_FINISHED')) { break }
        }
        if ($process.HasExited) { throw 'Isolated process exited during finite output' }
        Start-Sleep -Milliseconds 80
    }
    if ($lastText.Contains('BKS7_FLOOD_FINISHED')) {
        $idle = @{ Text=$lastText; Timer=[Diagnostics.Stopwatch]::StartNew() }
        Wait-For {
            $current = [BksStage7]::Text($process.Id,$journal)
            if ($current -ne $idle.Text) { $idle.Text=$current; $idle.Timer.Restart() }
            $marker = $current.LastIndexOf('BKS7_FLOOD_FINISHED',[StringComparison]::Ordinal)
            $prompt = $current.LastIndexOf('PS ',[StringComparison]::Ordinal)
            $idle.Timer.ElapsedMilliseconds -ge 200 -and $prompt -gt $marker -and $current.IndexOf('> ',$prompt,[StringComparison]::Ordinal) -ge 0
        } 'final prompt and journal idle before reading bookmarks' 5000
        $lastText = $idle.Text
    }
    Check 'finite 30000-line producer retains its final output marker' $lastText.Contains('BKS7_FLOOD_FINISHED')
    if ($pingFailures -eq 0) { Check 'own window answers WM_NULL during output within 1000 ms' ($pingCount -gt 1 -and $maxPingMs -lt 1000) }

    $lastRow = 'BKS7_ROW_030000_' + $glyphs + '_'
    $tailPosition = $lastText.LastIndexOf('BKS7_ROW_030000_',[StringComparison]::Ordinal)
    $observations.Add([ordered]@{ Label='finite output before semantic assertions'; NativeUtf16=$lastText.Length; NativeUtf8Bytes=[Text.Encoding]::UTF8.GetByteCount($lastText); NativeLineFeeds=[BksStage7]::LineFeeds($lastText); LastNumberedRowOffset=$tailPosition; LastNumberedRowPrefix=$(if($tailPosition -ge 0){$lastText.Substring($tailPosition,[Math]::Min(32,$lastText.Length-$tailPosition))}else{$null}) })
    Check 'retained tail contains intact wide, combining and supplementary Unicode' ($lastText.Contains($lastRow) -and -not $lastText.Contains([string][char]0xfffd))
    $strictUtf8 = [Text.UTF8Encoding]::new($false,$true)
    $journalBytes = $strictUtf8.GetByteCount($lastText) # Throws on split UTF-16 surrogate.
    $journalLines = [BksStage7]::LineFeeds($lastText)
    Check 'native journal remains within 512 KiB UTF8 and 6000 LF' ($journalBytes -le 512*1024 -and $journalLines -le 6000)
    Check 'old rows were trimmed while newer rows remain' (-not $lastText.Contains('BKS7_ROW_000001_') -and $lastText -match '^BKS7_ROW_\d{6}_')
    $firstLine = ($lastText -split "`r`n",2)[0]
    Check 'trimmed prefix preserves a complete numbered Unicode line' ($firstLine -match ('^BKS7_ROW_\d{6}_'+[regex]::Escape($glyphs)+'_x+$'))
    $uiaText = Uia-Text $journal
    Check 'UIA TextPattern exposes the retained final Unicode tail' ($uiaText.Contains($lastRow) -and $uiaText.Contains('BKS7_FLOOD_FINISHED'))
    Check 'finite producer installed the retained reading bookmark' ($null -ne $bookmark)
    $selection = [BksStage7]::Selection($process.Id,$journal)
    $finalAnchor = $lastText.IndexOf($anchor,[StringComparison]::Ordinal)
    Check 'trim preserves the exact native Unicode selection' ($finalAnchor -ge 0 -and $selection[0] -eq $finalAnchor -and $selection[1] -eq $finalAnchor+$anchor.Length)
    Check 'trim rebases retained selection positions after deleting the prefix' ($selection[0] -lt $bookmark.Start)
    $topLine = [BksStage7]::FirstLine($process.Id,$journal)
    $top = [BksStage7]::LineIndex($process.Id,$journal,$topLine)
    $next = [BksStage7]::LineIndex($process.Id,$journal,$topLine+1)
    $expectedViewport = $lastText.IndexOf($bookmark.LogicalText,[StringComparison]::Ordinal)
    $observations.Add([ordered]@{ Label='final retained viewport before assertion'; ActualTopUtf16=$top; NextVisualRowUtf16=$next; ExpectedLogicalLineUtf16=$expectedViewport; ExpectedLogicalLine=$bookmark.LogicalText })
    Check 'retained viewport line still exists after trimming' ($expectedViewport -ge 0)
    $expectedViewport += $bookmark.WithinLogicalLine
    Check 'trim keeps the reading anchor in the top native visual row' ($top -le $expectedViewport -and ($next -lt 0 -or $expectedViewport -lt $next))
    $observations.Add([ordered]@{ Label='retained reading bookmark after trim'; Start=$selection[0]; End=$selection[1]; Selected=$lastText.Substring($selection[0],$selection[1]-$selection[0]); ViewportUtf16=$top; ExpectedViewportUtf16=$expectedViewport; NextVisualRowUtf16=$next })
    Check 'output and trimming preserve the independent editable draft' ((Uia-Text $inputBox).TrimEnd("`r","`n") -eq $draft)
    [BksStage7]::SetText($process.Id,$inputBox,$draft+'_edited')
    Check 'native command draft remains editable after the flood' ((Uia-Text $inputBox).TrimEnd("`r","`n") -eq $draft+'_edited')
    $observations.Add([ordered]@{ ProducerLines=30000; NativeUtf16=$lastText.Length; NativeUtf8Bytes=$journalBytes; NativeLineFeeds=$journalLines; FirstLine=$firstLine; LastUnicodeRowPresent=$true; WmNullSamples=$pingCount; WmNullFailures=$pingFailures; MaxSuccessfulWmNullMs=$maxPingMs; Scope='queue cap/backpressure and global poll budget are covered by unit tests; this GUI run checks native cap, retained output and responsiveness' })
    }
} catch {
    $failure = $_.Exception.Message
    $checks.Add('failed: '+$failure)
    $observations.Add([ordered]@{Label='failure location';Stack=$_.ScriptStackTrace})
    if ($process -and $process.HasExited) { $observations.Add([ordered]@{ ExitCode=('0x{0:X8}' -f $process.ExitCode) }) }
    $observations.Add([ordered]@{ WmNullSamples=$pingCount; WmNullFailures=$pingFailures; MaxSuccessfulWmNullMs=$maxPingMs })
} finally {
    if ($process -and -not $process.HasExited) {
        $ownMain = [BksStage7]::MainWindow($process.Id)
        if ($ownMain -ne [IntPtr]::Zero) { [BksStage7]::Post($process.Id,$ownMain,0x10) }
        if ($process.WaitForExit(15000)) { $checks.Add('passed: isolated test process closes cleanly') }
        else { $process.Kill(); $checks.Add('failed: isolated test process required forced cleanup') }
    }
    [ordered]@{ Scope="Stage7 $mode; own PID/APPDATA; no global keyboard or clipboard; NVDA speech not tested"; Apartment=[Threading.Thread]::CurrentThread.GetApartmentState().ToString(); WmNullFlags=$responseFlags; PerformanceLog=$(if($PerformanceTrace){$performancePath}else{$null}); Executable=$Exe; ProcessId=$(if($process){$process.Id}else{$null}); AppData=$appData; Checks=$checks.ToArray(); Observations=$observations.ToArray() } |
        ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $runDirectory 'results.json') -Encoding utf8
    $checks | Set-Content -LiteralPath (Join-Path $runDirectory 'results.txt') -Encoding utf8
}
$passed = @($checks | Where-Object { $_.StartsWith('passed:') }).Count
$failed = @($checks | Where-Object { $_.StartsWith('failed:') }).Count
$skipped = @($checks | Where-Object { $_.StartsWith('skipped:') }).Count
Write-Output "Stage7 ${mode}: passed=$passed failed=$failed skipped=$skipped errors=0"
Write-Output "Artifacts: $runDirectory"
if ($failure -or $failed) { throw $(if($failure){$failure}else{'Stage7 output checks failed'}) }
