param(
    [string]$Exe = (Join-Path $PSScriptRoot '../target/release/bks-app.exe'),
    [string]$Artifacts = (Join-Path $PSScriptRoot '../target/accessibility-check'),
    [ValidateSet('PowerShell', 'Cmd')]
    [string]$Shell = 'PowerShell',
    [switch]$BlocksOnly,
    [switch]$NewTabOnly
)

# UI Automation and keyboard smoke only. This does not test NVDA speech.
# All UIA queries and input target a new process with its own APPDATA.
# The clipboard is never read or changed.
$ErrorActionPreference = 'Stop'
if ($BlocksOnly -and $NewTabOnly) { throw 'Select one focused check' }
# Use the system .NET Framework UIA client and its standard Win32 proxies.
# The PowerShell 7 WPF distribution can otherwise expose HWND fallback Panes.
if ($PSVersionTable.PSEdition -eq 'Core') {
    $childArguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath, '-Exe', $Exe, '-Artifacts', $Artifacts, '-Shell', $Shell)
    if ($BlocksOnly) { $childArguments += '-BlocksOnly' }
    if ($NewTabOnly) { $childArguments += '-NewTabOnly' }
    & "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" @childArguments
    exit $LASTEXITCODE
}
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName UIAutomationClientsideProviders
if (-not ('BksA11ySmoke' -as [type])) {
    $uiaReferences = @([Windows.Automation.AutomationElement].Assembly.Location,
        [Windows.Automation.AutomationPattern].Assembly.Location,
        [UIAutomationClientsideProviders.UIAutomationClientSideProviders].Assembly.Location)
    Add-Type -ReferencedAssemblies $uiaReferences -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Text;
using System.Runtime.InteropServices;
public static class BksA11ySmoke {
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.NoInlining)]
    public static void InitializeUia() {
        // UIA's default proxy loader inspects ReflectedType on stack frames.
        // A compiled entry point avoids PowerShell dynamic frames with no type.
        var table = UIAutomationClientsideProviders.UIAutomationClientSideProviders.ClientSideProviderDescriptionTable;
        try {
            System.Windows.Automation.ClientSettings.RegisterClientSideProviders(table);
        } catch (NullReferenceException) {
            // PowerShell dynamic frames can still make LoadDefaultProxies fail.
            // It clears its one-shot flag before loading; explicitly register
            // the same standard table after that failed automatic attempt.
            System.Windows.Automation.ClientSettings.RegisterClientSideProviders(table);
        }
    }
    private delegate bool EnumProc(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassNameW(IntPtr window, StringBuilder text, int capacity);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr window, uint command);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern IntPtr SetFocus(IntPtr window);
    [DllImport("user32.dll")] private static extern bool BringWindowToTop(IntPtr window);
    [DllImport("user32.dll")] private static extern bool AttachThreadInput(uint first, uint second, bool attach);
    [DllImport("kernel32.dll")] private static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", EntryPoint="SendMessageW")] private static extern IntPtr NativeMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", EntryPoint="SendMessageW")] private static extern IntPtr ReadSelection(IntPtr window, uint message, out uint start, out uint end);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)] private static extern IntPtr ReadText(IntPtr window, uint message, IntPtr capacity, StringBuilder text);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)] private static extern IntPtr WriteText(IntPtr window, uint message, IntPtr unused, string text);
    [DllImport("user32.dll", EntryPoint="GetWindowLongPtrW")] private static extern IntPtr GetWindowLongPtrW(IntPtr window, int index);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct GUIINFO {
        public uint Size, Flags;
        public IntPtr Active, Focus, Capture, Menu, MoveSize, Caret;
        public RECT CaretRect;
    }
    [DllImport("user32.dll")] private static extern bool GetGUIThreadInfo(uint thread, ref GUIINFO info);
    [StructLayout(LayoutKind.Sequential)] private struct KEYBDINPUT {
        public ushort Key, Scan; public uint Flags, Time; public UIntPtr Extra;
    }
    [StructLayout(LayoutKind.Sequential)] private struct MOUSEINPUT {
        public int X, Y; public uint Data, Flags, Time; public UIntPtr Extra;
    }
    [StructLayout(LayoutKind.Explicit)] private struct INPUTUNION {
        [FieldOffset(0)] public KEYBDINPUT Keyboard;
        [FieldOffset(0)] public MOUSEINPUT Mouse;
    }
    [StructLayout(LayoutKind.Sequential)] private struct INPUT { public uint Type; public INPUTUNION Data; }
    [DllImport("user32.dll", SetLastError=true)] private static extern uint SendInput(uint count, INPUT[] input, int size);
    public static uint ProcessId(IntPtr window) {
        uint process; GetWindowThreadProcessId(window, out process); return process;
    }
    public static bool Activate(uint process, IntPtr window) {
        if (ProcessId(window) != process) throw new InvalidOperationException("Activation target is not owned by test process");
        uint foregroundProcess;
        uint foregroundThread = GetWindowThreadProcessId(GetForegroundWindow(), out foregroundProcess);
        uint currentThread = GetCurrentThreadId();
        bool attached = foregroundThread != 0 && foregroundThread != currentThread && AttachThreadInput(currentThread, foregroundThread, true);
        try {
            ShowWindow(window, 9);
            BringWindowToTop(window);
            SetForegroundWindow(window);
            return ProcessId(GetForegroundWindow()) == process;
        } finally {
            if (attached) AttachThreadInput(currentThread, foregroundThread, false);
        }
    }
    public static void FocusTerminal(uint process, IntPtr window) {
        uint owner;
        uint targetThread = GetWindowThreadProcessId(window, out owner);
        if (owner != process || ClassName(window) != "BksTerminalSurface")
            throw new InvalidOperationException("Native focus fallback is restricted to this test's VT surface");
        uint currentThread = GetCurrentThreadId();
        bool attached = currentThread != targetThread && AttachThreadInput(currentThread, targetThread, true);
        try { SetFocus(window); }
        finally { if (attached) AttachThreadInput(currentThread, targetThread, false); }
    }
    public static IntPtr ProcessWindow(uint process, string className) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((window, parameter) => {
            if (ProcessId(window) != process) return true;
            var name = new StringBuilder(128); GetClassNameW(window, name, name.Capacity);
            if (name.ToString() != className) return true;
            result = window; return false;
        }, IntPtr.Zero);
        return result;
    }
    public static GUIINFO ThreadInfo(IntPtr window) {
        uint process; uint thread = GetWindowThreadProcessId(window, out process);
        var info = new GUIINFO(); info.Size = (uint)Marshal.SizeOf(typeof(GUIINFO));
        if (!GetGUIThreadInfo(thread, ref info)) throw new InvalidOperationException("GetGUIThreadInfo failed");
        return info;
    }
    public static string ClassName(IntPtr window) {
        var name = new StringBuilder(128); GetClassNameW(window, name, name.Capacity); return name.ToString();
    }
    public static bool NativeReadOnly(IntPtr window) { return (GetWindowLongPtrW(window, -16).ToInt64() & 0x800) != 0; }
    private static void OwnedEdit(uint process, IntPtr window) {
        if (ProcessId(window) != process || !ClassName(window).Equals("Edit", StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("Native text query refused outside test process EDIT controls");
    }
    public static uint[] Selection(uint process, IntPtr window) {
        OwnedEdit(process, window);
        uint start, end; ReadSelection(window, 0x00B0, out start, out end);
        return new[] { start, end };
    }
    public static void SetCaret(uint process, IntPtr window, uint position) {
        OwnedEdit(process, window);
        NativeMessage(window, 0x00B1, new IntPtr(position), new IntPtr(position));
    }
    public static void SetSelection(uint process, IntPtr window, uint start, uint end) {
        OwnedEdit(process, window);
        NativeMessage(window, 0x00B1, new IntPtr(start), new IntPtr(end));
    }
    public static void SetEditText(uint process, IntPtr window, string text) {
        OwnedEdit(process, window);
        WriteText(window, 0x000C, IntPtr.Zero, text);
    }
    public static string EditText(uint process, IntPtr window) {
        OwnedEdit(process, window);
        int length = NativeMessage(window, 0x000E, IntPtr.Zero, IntPtr.Zero).ToInt32();
        if (length < 0 || length > 2 * 1024 * 1024) throw new InvalidOperationException("Unexpected test journal size");
        var text = new StringBuilder(length + 1);
        ReadText(window, 0x000D, new IntPtr(text.Capacity), text);
        return text.ToString();
    }
    private static INPUT Keyboard(ushort key, ushort scan, uint flags) {
        return new INPUT { Type=1, Data=new INPUTUNION { Keyboard=new KEYBDINPUT { Key=key, Scan=scan, Flags=flags } } };
    }
    private static void Send(uint process, INPUT[] input) {
        // Refuse global input if the user has moved to another application's window.
        if (ProcessId(GetForegroundWindow()) != process) throw new InvalidOperationException("Test process lost foreground; input refused");
        if (SendInput((uint)input.Length, input, Marshal.SizeOf(typeof(INPUT))) != input.Length)
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
    }
    public static void Key(uint process, ushort key, bool control, bool shift, bool alt) {
        var down = new List<INPUT>();
        var up = new List<INPUT>();
        if (control) down.Add(Keyboard(0x11, 0, 0));
        if (shift) down.Add(Keyboard(0x10, 0, 0));
        if (alt) down.Add(Keyboard(0x12, 0, 0));
        if (alt) up.Add(Keyboard(0x12, 0, 2));
        if (shift) up.Add(Keyboard(0x10, 0, 2));
        if (control) up.Add(Keyboard(0x11, 0, 2));
        try {
            if (down.Count != 0) {
                Send(process, down.ToArray());
                System.Threading.Thread.Sleep(40);
            }
            Send(process, new[] { Keyboard(key, 0, 0), Keyboard(key, 0, 2) });
            if (down.Count != 0) System.Threading.Thread.Sleep(40);
        } finally {
            // Always release our modifiers, including when a PID guard aborts.
            if (up.Count != 0) SendInput((uint)up.Count, up.ToArray(), Marshal.SizeOf(typeof(INPUT)));
        }
    }
    public static void Unicode(uint process, string text) {
        foreach (char unit in text) Send(process, new[] { Keyboard(0, unit, 4), Keyboard(0, unit, 6) });
    }
}
'@
}
[BksA11ySmoke]::InitializeUia()

$checks = [Collections.Generic.List[string]]::new()
$patterns = [Collections.Generic.List[object]]::new()
$cycles = [Collections.Generic.List[object]]::new()
$runName = 'run-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Artifacts $runName)
$runDirectory = (Resolve-Path -LiteralPath (Join-Path $Artifacts $runName)).Path
$appData = Join-Path $runDirectory 'appdata'
$null = New-Item -ItemType Directory -Path $appData
if ($Shell -eq 'Cmd') {
    $configDirectory = Join-Path $appData 'BKS Term'
    $null = New-Item -ItemType Directory -Path $configDirectory
    $configuration = [ordered]@{
        schema_version=1
        default_profile_id='a11y-cmd'
        profiles=@([ordered]@{ id='a11y-cmd'; name='Accessibility CMD'; kind='local'; executable=$env:ComSpec; args=@('/Q'); working_directory=$null })
    } | ConvertTo-Json -Depth 5
    [IO.File]::WriteAllText((Join-Path $configDirectory 'profiles.json'), $configuration, [Text.UTF8Encoding]::new($false))
}
$process = $null
$mainWindow = [IntPtr]::Zero
$failure = $null

function Check([string]$Name, [bool]$Condition, [bool]$Fatal = $true) {
    if (-not $Condition) {
        $checks.Add("failed: $Name")
        if ($Fatal) { throw "FAILED: $Name" }
        if (-not $script:failure) { $script:failure = "FAILED: $Name" }
        return
    }
    $checks.Add("passed: $Name")
}
function Wait-For([scriptblock]$Condition, [string]$Name, [int]$Timeout = 10000) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.ElapsedMilliseconds -lt $Timeout) {
        if (& $Condition) { return }
        if ($process -and $process.HasExited) { throw "Process exited while waiting for $Name" }
        Start-Sleep -Milliseconds 40
    }
    throw "Timeout: $Name"
}
function Element([IntPtr]$Window) {
    if ($Window -eq [IntPtr]::Zero -or [BksA11ySmoke]::ProcessId($Window) -ne $process.Id) {
        throw 'UIA query refused: handle is not owned by the test process'
    }
    [Windows.Automation.AutomationElement]::FromHandle($Window)
}
function Pattern($Element, $Id) {
    $pattern = $null
    if ($Element.TryGetCurrentPattern($Id, [ref]$pattern)) { return $pattern }
    return $null
}
function Uia-Text([IntPtr]$Window) {
    $element = Element $Window
    $text = Pattern $element ([Windows.Automation.TextPattern]::Pattern)
    if ($text) { return $text.DocumentRange.GetText(-1) }
    $value = Pattern $element ([Windows.Automation.ValuePattern]::Pattern)
    if ($value) { return $value.Current.Value }
    throw 'Native text control exposes neither UIA TextPattern nor ValuePattern'
}
function Record-Control([IntPtr]$Window, [string]$Label) {
    if ($Window -eq [IntPtr]::Zero) { return }
    $element = Element $Window
    $current = $element.Current
    $patterns.Add([ordered]@{
        ExpectedName=$Label
        NativeHandle=$Window.ToInt64()
        Name=$current.Name
        Class=$current.ClassName
        Framework=$current.FrameworkId
        ControlType=$current.ControlType.ProgrammaticName
        KeyboardFocusable=$current.IsKeyboardFocusable
        SupportedPatterns=@($element.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName })
    })
}
function Check-Edit([IntPtr]$Window, [string]$Name, [bool]$ReadOnly) {
    $element = Element $Window
    $current = $element.Current
    $text = Pattern $element ([Windows.Automation.TextPattern]::Pattern)
    $value = Pattern $element ([Windows.Automation.ValuePattern]::Pattern)
    $patterns.Add([ordered]@{ ExpectedName=$Name; Name=$current.Name; Class=$current.ClassName; ControlType=$current.ControlType.ProgrammaticName; TextPattern=($null -ne $text); ValuePattern=($null -ne $value); ReadOnly=$ReadOnly })
    Check "$Name has a native EDIT window" ([BksA11ySmoke]::ClassName($Window) -ieq 'Edit')
    Check "$Name has an accessible name" ($current.Name -like "*$Name*") $false
    Check "$Name has an editable-text UIA role" ($current.ControlType -in @(
        [Windows.Automation.ControlType]::Edit, [Windows.Automation.ControlType]::Document)) $false
    Check "$Name is enabled and keyboard focusable" ($current.IsEnabled -and $current.IsKeyboardFocusable) $false
    Check "$Name native read-only state" ([BksA11ySmoke]::NativeReadOnly($Window) -eq $ReadOnly)
    Check "$Name exposes a readable UIA text pattern" ($null -ne $text -or $null -ne $value)
    if ($value) { Check "$Name ValuePattern read-only state" ($value.Current.IsReadOnly -eq $ReadOnly) }
    else { $checks.Add("skipped: $Name ValuePattern is not supported by this native EDIT provider") }
    if ($text) {
        Check "$Name TextPattern provides selection ranges" ($null -ne $text.GetSelection())
        $attribute = $text.DocumentRange.GetAttributeValue([Windows.Automation.TextPattern]::IsReadOnlyAttribute)
        if ($attribute -is [bool]) { Check "$Name TextPattern read-only attribute" ($attribute -eq $ReadOnly) }
        else { $checks.Add("skipped: $Name TextPattern IsReadOnlyAttribute is not supported") }
    } else { $checks.Add("skipped: $Name TextPattern/selection is not supported by this native EDIT provider") }
}
function Focus([IntPtr]$Window) {
    $element = Element $Window
    [void][BksA11ySmoke]::Activate($process.Id, $mainWindow)
    Wait-For { [BksA11ySmoke]::ProcessId([BksA11ySmoke]::GetForegroundWindow()) -eq $process.Id } 'test foreground'
    if ([BksA11ySmoke]::ClassName($Window) -eq 'BksTerminalSurface') {
        # The graphical, opt-in surface has no text UIA provider. Native text
        # controls above still use UIA SetFocus and strict UIA assertions.
        [BksA11ySmoke]::FocusTerminal($process.Id, $Window)
    } else { $element.SetFocus() }
    Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $Window } 'requested control focus'
    Start-Sleep -Milliseconds 50
}
function Key([int]$Code, [bool]$Control = $false, [bool]$Shift = $false, [bool]$Alt = $false) {
    [BksA11ySmoke]::Key($process.Id, [uint16]$Code, $Control, $Shift, $Alt)
}
function Type-Text([string]$Text) { [BksA11ySmoke]::Unicode($process.Id, $Text) }
function Select-AllText {
    Key 0x41 $true
}
function Replace-Text([IntPtr]$Window, [string]$Text) {
    if ($BlocksOnly) {
        # Fixture setup is native; this mode tests block shortcuts, not text entry.
        [BksA11ySmoke]::SetEditText($process.Id, $Window, $Text)
        Wait-For { (Uia-Text $Window).TrimEnd("`r", "`n") -eq $Text } 'native fixture text through UIA'
        return
    }
    Focus $Window
    Select-AllText
    Type-Text $Text
    Wait-For { (Uia-Text $Window).TrimEnd("`r", "`n") -eq $Text } 'typed replacement text'
}
function Check-Cycle([string]$Name, [IntPtr]$Start, [IntPtr[]]$Required) {
    Focus $Start
    $seen = [Collections.Generic.List[long]]::new()
    $names = [Collections.Generic.List[string]]::new()
    $seen.Add($Start.ToInt64())
    $names.Add((Element $Start).Current.Name)
    do {
        $before = [BksA11ySmoke]::ThreadInfo($mainWindow).Focus
        Key 9
        Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -ne $before } "$Name Tab moves focus" 2500
        $next = [BksA11ySmoke]::ThreadInfo($mainWindow).Focus
        Check "$Name Tab target is visible and enabled" ([BksA11ySmoke]::IsWindowVisible($next) -and [BksA11ySmoke]::IsWindowEnabled($next))
        $element = Element $next
        if ($next -eq $Start) { break }
        Check "$Name does not enter a smaller focus loop" (-not $seen.Contains($next.ToInt64()) -and $seen.Count -lt 64)
        $seen.Add($next.ToInt64())
        $names.Add($element.Current.Name)
    } while ($true)
    foreach ($window in $Required) { Check "$Name reaches $((Element $window).Current.Name)" ($seen.Contains($window.ToInt64())) }
    for ($index = $seen.Count - 1; $index -ge 0; $index--) {
        Key 9 $false $true
        $expected = [IntPtr]$seen[$index]
        Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $expected } "$Name Shift+Tab reverses forward order" 2500
    }
    Check "$Name Shift+Tab completes the reverse cycle" ([BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $Start)
    $cycles.Add([ordered]@{ Name=$Name; Controls=$names.ToArray() })
}
function Menu-Command([int]$Id) {
    [void][BksA11ySmoke]::PostMessageW($mainWindow, 0x111, [IntPtr]$Id, [IntPtr]::Zero)
}
function New-ProfileTab {
    $draftBeforePicker = Uia-Text $command
    $selectionBeforePicker = [BksA11ySmoke]::Selection($process.Id, $command)
    $focusBeforePicker = [BksA11ySmoke]::ThreadInfo($mainWindow).Focus
    Key 0x54 $true $true
    Wait-For { Require-PickerForeground; [BksA11ySmoke]::ThreadInfo($mainWindow).Menu -ne [IntPtr]::Zero } 'new-tab profile picker'
    Check 'Ctrl+Shift+T asks for a profile before creating a tab' ([BksA11ySmoke]::GetDlgItem($mainWindow, 2002) -eq [IntPtr]::Zero)
    Key 27
    Wait-For { Require-PickerForeground; [BksA11ySmoke]::ThreadInfo($mainWindow).Menu -eq [IntPtr]::Zero } 'cancel new-tab profile picker'
    Check 'cancelled profile picker creates no tab' ([BksA11ySmoke]::GetDlgItem($mainWindow, 2002) -eq [IntPtr]::Zero)
    Check 'cancelled profile picker preserves the draft' ((Uia-Text $command) -eq $draftBeforePicker)
    $selectionAfterPicker = [BksA11ySmoke]::Selection($process.Id, $command)
    Check 'cancelled profile picker preserves native selection' ($selectionAfterPicker[0] -eq $selectionBeforePicker[0] -and $selectionAfterPicker[1] -eq $selectionBeforePicker[1])
    Check 'cancelled profile picker restores focus' ([BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $focusBeforePicker)
    Key 0x54 $true $true
    $expectedProfile = if ($Shell -eq 'Cmd') { 'Accessibility CMD' } else { 'Windows PowerShell' }
    $resolved = @{ Item=$null; Popup=[IntPtr]::Zero }
    Wait-For {
        Require-PickerForeground
        $popup = [BksA11ySmoke]::ProcessWindow($process.Id, '#32768')
        if ($popup -eq [IntPtr]::Zero -or -not [BksA11ySmoke]::IsWindowVisible($popup)) { return $false }
        $menu = Element $popup
        $condition = [Windows.Automation.AndCondition]::new(
            [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::MenuItem),
            [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $expectedProfile))
        $resolved.Item = $menu.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
        $resolved.Popup = $popup
        $null -ne $resolved.Item
    } 'own visible popup and explicit accessible profile'
    Require-PickerForeground
    $invoke = Pattern $resolved.Item ([Windows.Automation.InvokePattern]::Pattern)
    Check 'profile choice exposes native UIA InvokePattern' ($null -ne $invoke)
    $patterns.Add([ordered]@{ Observation='explicit profile choice'; Name=$resolved.Item.Current.Name; Popup=$resolved.Popup.ToInt64(); ProcessId=$process.Id })
    $invoke.Invoke()
    Wait-For { Require-PickerForeground; [BksA11ySmoke]::GetDlgItem($mainWindow, 2002) -ne [IntPtr]::Zero } 'selected profile creates second tab'
}
function Require-PickerForeground {
    if ([BksA11ySmoke]::ProcessId([BksA11ySmoke]::GetForegroundWindow()) -ne $process.Id) {
        throw 'Test process lost foreground; input refused'
    }
}
function Block-Caret([string]$Label) {
    # Let the key and any native edit processing complete before testing clamped edges.
    Start-Sleep -Milliseconds 100
    try {
        Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $journal } "$Label journal focus" 2500
    } catch {
        $patterns.Add([ordered]@{ Observation=$Label; ActualFocus=[BksA11ySmoke]::ThreadInfo($mainWindow).Focus.ToInt64(); ExpectedJournal=$journal.ToInt64(); NativeSelection=[BksA11ySmoke]::Selection($process.Id, $journal) })
        throw
    }
    Check "$Label focuses the native journal" ([BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $journal)
    $selection = [BksA11ySmoke]::Selection($process.Id, $journal)
    Check "$Label collapses the native selection to a caret" ($selection[0] -eq $selection[1])
    $textPattern = Pattern (Element $journal) ([Windows.Automation.TextPattern]::Pattern)
    if ($textPattern) {
        $ranges = $textPattern.GetSelection()
        Check "$Label exposes the caret through UIA selection" ($ranges.Count -gt 0 -and $ranges[0].GetText(-1) -eq '')
        $line = $ranges[0].Clone()
        $line.ExpandToEnclosingUnit([Windows.Automation.Text.TextUnit]::Line)
        $patterns.Add([ordered]@{ Observation=$Label; NativeCaret=$selection[0]; UiaCaretLine=$line.GetText(-1) })
    }
    return [uint32]$selection[0]
}
function Submit-Block([string]$Text, [string]$Marker, [string]$Tail = '') {
    Replace-Text $command $Text
    if ($BlocksOnly) {
        [void][BksA11ySmoke]::PostMessageW($execute, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) # BM_CLICK
    } else { Key 13 }
    Wait-For { [string]::IsNullOrWhiteSpace((Uia-Text $command)) } 'explicit command accepted'
    try {
        Wait-For { (Uia-Text $journal).Contains($Marker) -and (!$Tail -or (Uia-Text $journal).Contains($Tail)) } 'block output through UIA' 15000
    } catch {
        $patterns.Add([ordered]@{ Observation='block output timeout'; ExpectedMarker=$Marker; ExpectedTail=$Tail; NativeJournal=[BksA11ySmoke]::EditText($process.Id, $journal) })
        throw
    }
    # The final output can precede the next OSC prompt by one polling interval.
    Start-Sleep -Milliseconds 150
}
function Check-BlockNavigation {
    $secondMarker = 'BKS_UIA_NAV_TWO_Ж'
    $thirdMarker = 'BKS_UIA_NAV_THREE_Я'
    if ($Shell -eq 'Cmd') {
        Submit-Block 'chcp 65001>nul & echo BKS_UIA_NAV_TW^O_Ж & echo строка_д^ва_中' $secondMarker 'строка_два_中'
        Submit-Block 'echo BKS_UIA_NAV_TH^REE_Я & echo строка_т^ри_中' $thirdMarker 'строка_три_中'
    } else {
        Submit-Block "[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); 'BKS_UIA_NAV_'+'TWO_Ж'; 'строка_'+'два_中'" $secondMarker 'строка_два_中'
        Submit-Block "'BKS_UIA_NAV_'+'THREE_Я'; 'строка_'+'три_中'" $thirdMarker 'строка_три_中'
    }
    $draft = 'draft_navigation_Ж中'
    Replace-Text $command $draft
    Focus $command
    Key 0x26 $true
    $last = Block-Caret 'Ctrl+Up from command editor reaches last block'
    Key 0x26 $true
    $middle = Block-Caret 'second Ctrl+Up reaches previous block'
    Key 0x26 $true
    $first = Block-Caret 'third Ctrl+Up reaches first block'
    Check 'three block starts have strict UTF-16 order' ($first -lt $middle -and $middle -lt $last)
    $rawText = [BksA11ySmoke]::EditText($process.Id, $journal)
    Check 'first block contains its own Unicode output' ($rawText.Substring($first, $middle - $first).Contains($marker))
    Check 'second block contains its Unicode and multiline output' ($rawText.Substring($middle, $last - $middle).Contains($secondMarker) -and $rawText.Substring($middle, $last - $middle).Contains('строка_два_中'))
    Check 'last block contains its Unicode and multiline output' ($rawText.Substring($last).Contains($thirdMarker) -and $rawText.Substring($last).Contains('строка_три_中'))
    Key 0x26 $true
    Check 'Ctrl+Up clamps at the first block' ((Block-Caret 'first block boundary') -eq $first)
    Key 0x28 $true
    Check 'Ctrl+Down reaches the middle block' ((Block-Caret 'forward middle block') -eq $middle)
    Key 0x28 $true
    Check 'Ctrl+Down reaches the last block' ((Block-Caret 'forward last block') -eq $last)
    Key 0x28 $true
    Check 'Ctrl+Down clamps at the last block' ((Block-Caret 'last block boundary') -eq $last)
    $inside = $rawText.IndexOf($thirdMarker, $last, [StringComparison]::Ordinal) + 1
    Check 'test caret setup is inside the last output marker' ($inside -gt $last)
    [BksA11ySmoke]::SetCaret($process.Id, $journal, $inside)
    Wait-For { [BksA11ySmoke]::Selection($process.Id, $journal)[0] -eq $inside } 'caret inside last block'
    Key 0x26 $true
    Check 'Ctrl+Up from inside a block reaches its own start' ((Block-Caret 'current block start') -eq $last)
    Focus $command
    Key 0x28 $true
    Check 'Ctrl+Down from command editor reaches the first block' ((Block-Caret 'external forward navigation') -eq $first)
    Key 0x28 $false $false $true
    Check 'Alt+Down remains a next-block alias' ((Block-Caret 'Alt+Down alias') -eq $middle)
    Key 0x26 $false $false $true
    Check 'Alt+Up remains a previous-block alias' ((Block-Caret 'Alt+Up alias') -eq $first)
    Check 'block navigation preserves the command draft' ((Uia-Text $command).TrimEnd("`r", "`n") -eq $draft)
    return [ordered]@{ First=$first; Middle=$middle; Last=$last; Draft=$draft }
}
function Check-Help([IntPtr]$Previous, [bool]$UseCloseButton = $false) {
    Focus $Previous
    Key 0x70
    Wait-For { [BksA11ySmoke]::ProcessWindow($process.Id, 'BksTermHelpWindow') -ne [IntPtr]::Zero } 'F1 help window'
    $help = [BksA11ySmoke]::ProcessWindow($process.Id, 'BksTermHelpWindow')
    Wait-For { [BksA11ySmoke]::GetDlgItem($help, 1) -ne [IntPtr]::Zero -and [BksA11ySmoke]::GetDlgItem($help, 2) -ne [IntPtr]::Zero } 'help controls initialized'
    $text = [BksA11ySmoke]::GetDlgItem($help, 1)
    $close = [BksA11ySmoke]::GetDlgItem($help, 2)
    Wait-For { [BksA11ySmoke]::IsWindowVisible($text) -and [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $text } 'help displayed and focused'
    Check-Edit $text 'Справка BKS Term' $true
    Check 'help instructions can be read through UIA' ((Uia-Text $text).Contains('Ctrl+Shift+Space'))
    $closeElement = Element $close
    Record-Control $close 'help close button'
    Check 'help close button has an accessible button role and name' ($closeElement.Current.ControlType -eq [Windows.Automation.ControlType]::Button -and $closeElement.Current.Name -in @('Закрыть', 'Закрыть справку'))
    Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $text } 'F1 focuses readable help text'
    $helpTextPattern = Pattern (Element $text) ([Windows.Automation.TextPattern]::Pattern)
    if ($helpTextPattern) {
        Key 0x41 $true
        Wait-For { (($helpTextPattern.GetSelection() | ForEach-Object { $_.GetText(-1) }) -join '').TrimEnd("`r", "`n") -eq (Uia-Text $text).TrimEnd("`r", "`n") } 'help Ctrl+A selection through UIA'
        Check 'help Ctrl+A exposes the selected instructions through UIA' $true
    }
    Key 9
    Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $close } 'help Tab reaches close button'
    Key 9 $false $true
    Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $text } 'help Shift+Tab returns to text'
    if ($UseCloseButton) {
        Key 9
        Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $close } 'help close button focus before Enter'
        Key 13
    } else { Key 27 }
    Wait-For { -not [BksA11ySmoke]::IsWindow($help) } 'help closes by keyboard'
    Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $Previous } 'help restores previous visible focus'
    Check 'help closes without closing the terminal window' ([BksA11ySmoke]::IsWindow($mainWindow))
}

try {
    $Exe = (Resolve-Path -LiteralPath $Exe).Path
    $startInfo = [Diagnostics.ProcessStartInfo]::new($Exe)
    $startInfo.UseShellExecute = $false
    $startInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $startInfo.EnvironmentVariables['APPDATA'] = $appData
    $process = [Diagnostics.Process]::Start($startInfo)
    Wait-For { [BksA11ySmoke]::ProcessWindow($process.Id, 'BksTermWindow') -ne [IntPtr]::Zero } 'main window'
    $mainWindow = [BksA11ySmoke]::ProcessWindow($process.Id, 'BksTermWindow')
    Wait-For { [BksA11ySmoke]::GetDlgItem($mainWindow, 1002) -ne [IntPtr]::Zero -and [BksA11ySmoke]::GetDlgItem($mainWindow, 1003) -ne [IntPtr]::Zero } 'native controls'
    $journal = [BksA11ySmoke]::GetDlgItem($mainWindow, 1001)
    $command = [BksA11ySmoke]::GetDlgItem($mainWindow, 1002)
    $execute = [BksA11ySmoke]::GetDlgItem($mainWindow, 1003)
    $surface = [BksA11ySmoke]::GetDlgItem($mainWindow, 1017)
    $newTab = [BksA11ySmoke]::GetDlgItem($mainWindow, 1010)
    [void][BksA11ySmoke]::ShowWindow($mainWindow, 5)
    [void][BksA11ySmoke]::Activate($process.Id, $mainWindow)
    Wait-For { [BksA11ySmoke]::IsWindowVisible($command) -and [BksA11ySmoke]::IsWindowVisible($journal) } 'accessible startup view'
    Check 'startup exposes native command editor and journal' ([BksA11ySmoke]::IsWindowVisible($command) -and [BksA11ySmoke]::IsWindowVisible($journal))
    Check 'graphical VT surface is opt-in' (-not [BksA11ySmoke]::IsWindowVisible($surface))
    foreach ($entry in @(
        @($journal, 'journal'), @($command, 'command'),
        @([BksA11ySmoke]::GetParent($journal), 'journal parent'),
        @([BksA11ySmoke]::GetWindow($journal, 3), 'journal preceding sibling'),
        @([BksA11ySmoke]::GetWindow($command, 3), 'command preceding sibling')
    )) { Record-Control ([IntPtr]$entry[0]) $entry[1] }
    Check-Edit $journal 'Журнал терминала' $true
    Check-Edit $command 'Команда' $false
    Wait-For { (Uia-Text $journal).Contains('>') } "$Shell prompt through UIA" 20000
    Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $command } 'startup command focus'
    if ($NewTabOnly) {
        Focus $command
        $pickerDraft = 'profile_picker_draft_Ж'
        [BksA11ySmoke]::SetEditText($process.Id, $command, $pickerDraft)
        [BksA11ySmoke]::SetSelection($process.Id, $command, 8, 14)
        New-ProfileTab
        Wait-For { [BksA11ySmoke]::GetDlgItem($mainWindow, 2002) -ne [IntPtr]::Zero } 'selected profile creates second tab'
        Wait-For { (Uia-Text $journal).Contains('>') } 'selected profile prompt' 20000
        Check 'selected profile creates a tab with a fresh draft' ((Uia-Text $command).TrimEnd("`r", "`n") -eq '')
        Check 'selected profile starts with native input focus' ([BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $command)
        Check 'selected profile has closed search initially' (-not [BksA11ySmoke]::IsWindowVisible([BksA11ySmoke]::GetDlgItem($mainWindow, 1020)))
        Key 9 $true $true
        Wait-For { (Uia-Text $command).TrimEnd("`r", "`n") -eq $pickerDraft } 'original tab draft restored'
        Check 'profile choice preserves the original tab draft' ((Uia-Text $command).TrimEnd("`r", "`n") -eq $pickerDraft)
    } elseif ($BlocksOnly) {
        $marker = 'BKS_UIA_READY_Ж'
        $testCommand = if ($Shell -eq 'Cmd') { 'echo BKS_UIA_REA^DY_Ж' } else { "[Console]::WriteLine('BKS_UIA_'+'READY_Ж')" }
        Submit-Block $testCommand $marker
        $navigation = Check-BlockNavigation
        Focus $command
        New-ProfileTab
        Wait-For { [BksA11ySmoke]::GetDlgItem($mainWindow, 2002) -ne [IntPtr]::Zero } 'second tab'
        Wait-For { (Uia-Text $journal).Contains('>') -and -not (Uia-Text $journal).Contains($marker) } 'independent second tab' 20000
        $tabMarker = 'BKS_UIA_TAB_SECOND_Ж'
        $tabCommand = if ($Shell -eq 'Cmd') { 'echo BKS_UIA_TAB_SEC^OND_Ж' } else { "'BKS_UIA_TAB_'+'SECOND_Ж'" }
        Submit-Block $tabCommand $tabMarker
        Replace-Text $command 'draft_second_Ж'
        Focus $command
        Key 0x26 $true
        $secondTabAnchor = Block-Caret 'second tab navigates its own block'
        Check 'second tab block contains only its own output' ([BksA11ySmoke]::EditText($process.Id, $journal).Substring($secondTabAnchor).Contains($tabMarker) -and -not (Uia-Text $journal).Contains($marker))
        Check 'second tab navigation preserves its draft' ((Uia-Text $command).TrimEnd("`r", "`n") -eq 'draft_second_Ж')
        Key 9 $true
        Wait-For { (Uia-Text $journal).Contains($marker) } 'return to first tab'
        Check 'first tab draft remains independent' ((Uia-Text $command).TrimEnd("`r", "`n") -eq $navigation.Draft)
        Check 'first tab block caret remains independent' ([BksA11ySmoke]::Selection($process.Id, $journal)[0] -eq $navigation.First)
        Key 0x26 $true
        Check 'first tab keeps its last block after switching tabs' ((Block-Caret 'first tab last block after switch') -eq $navigation.Last)
        Key 9 $true
        Wait-For { (Uia-Text $journal).Contains($tabMarker) -and -not (Uia-Text $journal).Contains($marker) } 'return to second tab'
        Check 'second tab keeps its independent block caret' ([BksA11ySmoke]::Selection($process.Id, $journal)[0] -eq $secondTabAnchor)
        Check 'second tab keeps its independent command draft' ((Uia-Text $command).TrimEnd("`r", "`n") -eq 'draft_second_Ж')
    } else {
    Check-Cycle 'startup' $command @($journal, $execute, $newTab, [BksA11ySmoke]::GetDlgItem($mainWindow, 2001))

    Focus $command
    Type-Text 'draftЖ'
    Wait-For { (Uia-Text $command).TrimEnd("`r", "`n") -eq 'draftЖ' } 'initial Unicode draft'
    Key 8
    Wait-For { (Uia-Text $command).TrimEnd("`r", "`n") -eq 'draft' } 'Backspace removes the Unicode character'
    Type-Text 'Я'
    Start-Sleep -Milliseconds 150
    $typedText = Uia-Text $command
    $patterns.Add([ordered]@{ Observation='Unicode draft after Backspace'; UiaText=$typedText; Utf16Units=@($typedText.ToCharArray() | ForEach-Object { 'U+{0:X4}' -f [int]$_ }) })
    Wait-For { (Uia-Text $command).TrimEnd("`r", "`n") -eq 'draftЯ' } 'Unicode typing and Backspace editing'
    Check 'native editor accepts Unicode typing and editing' $true
    Select-AllText
    $commandTextPattern = Pattern (Element $command) ([Windows.Automation.TextPattern]::Pattern)
    if ($commandTextPattern) {
        Wait-For { (($commandTextPattern.GetSelection() | ForEach-Object { $_.GetText(-1) }) -join '').TrimEnd("`r", "`n") -eq 'draftЯ' } 'editor selection through UIA'
        Check 'Ctrl+A editor selection is readable through UIA' $true
    }
    $marker = 'BKS_UIA_READY_Ж'
    $testCommand = if ($Shell -eq 'Cmd') { 'echo BKS_UIA_REA^DY_Ж' } else { "[Console]::WriteLine('BKS_UIA_'+'READY_Ж')" }
    Replace-Text $command $testCommand
    Key 13
    Wait-For { (Uia-Text $journal).Contains($marker) } 'executed command output through UIA' 15000
    Check 'native journal exposes executed Unicode output through UIA' $true
    $journalTextPattern = Pattern (Element $journal) ([Windows.Automation.TextPattern]::Pattern)
    if ($journalTextPattern) {
        Focus $journal
        Key 0x41 $true
        Wait-For { (($journalTextPattern.GetSelection() | ForEach-Object { $_.GetText(-1) }) -join '').TrimEnd("`r", "`n") -eq (Uia-Text $journal).TrimEnd("`r", "`n") } 'journal Ctrl+A selection through UIA'
        Check 'journal Ctrl+A exposes selected output through UIA' $true
        $range = $journalTextPattern.DocumentRange.FindText($marker, $false, $false)
        Check 'journal TextPattern can locate output' ($null -ne $range)
        $range.Select()
        Check 'journal selection returns selected output through UIA' ((($journalTextPattern.GetSelection() | ForEach-Object { $_.GetText(-1) }) -join '') -eq $marker)
    }
    Check-Help $command $true
    $navigation = Check-BlockNavigation

    Focus $command
    New-ProfileTab
    Wait-For { [BksA11ySmoke]::GetDlgItem($mainWindow, 2002) -ne [IntPtr]::Zero } 'second tab'
    Wait-For { [BksA11ySmoke]::IsWindowVisible($journal) -and [BksA11ySmoke]::IsWindowVisible($command) } 'new tab accessible defaults'
    Wait-For { (Uia-Text $journal).Contains('>') -and -not (Uia-Text $journal).Contains($marker) } 'independent second tab journal' 20000
    Check 'new tab retains accessible defaults and its own output' (-not (Uia-Text $journal).Contains($marker))
    $tabMarker = 'BKS_UIA_TAB_SECOND_Ж'
    $tabCommand = if ($Shell -eq 'Cmd') { 'echo BKS_UIA_TAB_SEC^OND_Ж' } else { "[Console]::WriteLine('BKS_UIA_TAB_'+'SECOND_Ж')" }
    Submit-Block $tabCommand $tabMarker
    Replace-Text $command 'draft_second_Ж'
    Check-Cycle 'two tabs' $command @($journal, $execute, $newTab,
        [BksA11ySmoke]::GetDlgItem($mainWindow, 2001), [BksA11ySmoke]::GetDlgItem($mainWindow, 2002),
        [BksA11ySmoke]::GetDlgItem($mainWindow, 3001), [BksA11ySmoke]::GetDlgItem($mainWindow, 3002))
    Key 0x26 $true
    $secondTabAnchor = Block-Caret 'second tab navigates its own block'
    Check 'second tab block contains only its own output' ([BksA11ySmoke]::EditText($process.Id, $journal).Substring($secondTabAnchor).Contains($tabMarker) -and -not (Uia-Text $journal).Contains($marker))
    Check 'second tab navigation preserves its draft' ((Uia-Text $command).TrimEnd("`r", "`n") -eq 'draft_second_Ж')
    Key 9 $true
    Wait-For { (Uia-Text $journal).Contains($marker) } 'Ctrl+Tab returns to first tab'
    Check 'first tab draft remains independent' ((Uia-Text $command).TrimEnd("`r", "`n") -eq $navigation.Draft)
    Check 'first tab block caret remains independent' ([BksA11ySmoke]::Selection($process.Id, $journal)[0] -eq $navigation.First)

    Menu-Command 1009
    Wait-For { [BksA11ySmoke]::IsWindowVisible($surface) -and -not [BksA11ySmoke]::IsWindowVisible($command) } 'opt-in direct mode'
    Check-Cycle 'direct mode without capture' $surface @($newTab,
        [BksA11ySmoke]::GetDlgItem($mainWindow, 2001), [BksA11ySmoke]::GetDlgItem($mainWindow, 2002))
    Key 9 $true
    Wait-For { [BksA11ySmoke]::IsWindowVisible($command) } 'second tab preserves command mode'
    Check 'mode switch preserves another tab draft' ((Uia-Text $command).TrimEnd("`r", "`n") -eq 'draft_second_Ж')
    Key 9 $true
    Wait-For { [BksA11ySmoke]::IsWindowVisible($surface) } 'first tab preserves direct mode'

    Focus $surface
    Key 0x50 $true $true
    Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Menu -ne [IntPtr]::Zero } 'profiles menu'
    Key 27
    Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Menu -eq [IntPtr]::Zero } 'Escape dismisses menu'
    Focus $surface
    if ($Shell -eq 'PowerShell') {
        Type-Text "[Console]::WriteLine('BKS_CAPTURE_'+[int][Console]::ReadKey(`$true).KeyChar)"
        Key 13
        # ReadKey is pending in this isolated shell; captured Tab must supply character 9.
    }
    Key 0x49 $true $true
    Start-Sleep -Milliseconds 150
    Key 9
    Start-Sleep -Milliseconds 150
    Check 'explicit capture keeps Tab in the terminal' ([BksA11ySmoke]::ThreadInfo($mainWindow).Focus -eq $surface)
    Key 0x20 $true $true
    Wait-For { [BksA11ySmoke]::IsWindowVisible($journal) } 'capture escape reveals native journal'
    if ($Shell -eq 'PowerShell') {
        Wait-For { (Uia-Text $journal).Contains('BKS_CAPTURE_9') } 'captured Tab reaches PTY ReadKey' 10000
        Check 'captured Tab reaches PTY only after explicit opt-in' $true
    } else { $checks.Add('skipped: CMD has no PowerShell ReadKey capture byte assertion; capture focus and escape checks still run') }
    Check-Cycle 'journal after capture escape' $journal @($newTab,
        [BksA11ySmoke]::GetDlgItem($mainWindow, 2001), [BksA11ySmoke]::GetDlgItem($mainWindow, 2002))

    Key 0x20 $true $true
    Wait-For { [BksA11ySmoke]::IsWindowVisible($surface) } 'return to direct mode'
    Focus $surface
    Key 9
    Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -ne $surface } 'Tab navigates after capture escape' 2500
    Check 'Ctrl+Shift+Space releases capture' $true
    Focus $surface
    Key 0x49 $true $true
    Check-Help $surface
    Key 9
    Wait-For { [BksA11ySmoke]::ThreadInfo($mainWindow).Focus -ne $surface } 'Tab navigates after F1 releases capture' 2500
    Check 'F1 releases capture and keeps keyboard navigation available' $true

    Menu-Command 1009
    Wait-For { [BksA11ySmoke]::IsWindowVisible($command) -and [BksA11ySmoke]::IsWindowVisible($journal) } 'return to accessible command mode'
    Check-Cycle 'command mode restored' $command @($journal, $execute, $newTab,
        [BksA11ySmoke]::GetDlgItem($mainWindow, 2001), [BksA11ySmoke]::GetDlgItem($mainWindow, 2002))
    }
} catch {
    $failure = $_
    if ($_.Exception.Message.Contains('Test process lost foreground; input refused')) {
        $checks.Add('error: isolated process lost foreground; PID guard refused keyboard input')
        $checks.Add('skipped: remaining UI scenarios after external foreground loss')
    } else { $checks.Add('failed: ' + $_.Exception.Message) }
    if ($process -and -not $process.HasExited -and $mainWindow -ne [IntPtr]::Zero) {
        try {
            $patterns.Add([ordered]@{
                Observation='failure diagnostics for isolated process'
                Focus=[BksA11ySmoke]::ThreadInfo($mainWindow).Focus.ToInt64()
                ForegroundProcess=[BksA11ySmoke]::ProcessId([BksA11ySmoke]::GetForegroundWindow())
                NativeCommandText=[BksA11ySmoke]::EditText($process.Id, $command)
                NativeJournalSelection=[BksA11ySmoke]::Selection($process.Id, $journal)
            })
        } catch { $patterns.Add([ordered]@{ Observation='failure diagnostics unavailable'; Error=$_.Exception.Message }) }
    }
} finally {
    if ($process -and -not $process.HasExited) {
        $ownedMain = [BksA11ySmoke]::ProcessWindow($process.Id, 'BksTermWindow')
        if ($ownedMain -ne [IntPtr]::Zero) {
            [void][BksA11ySmoke]::PostMessageW($ownedMain, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
        }
        if (-not $process.WaitForExit(15000)) {
            $process.Kill()
            $checks.Add('failed: test process needed forced cleanup after 15 seconds')
            if (-not $failure) { $failure = 'Test process did not close within 15 seconds' }
        } else { $checks.Add('passed: isolated test process closes cleanly') }
    }
    [ordered]@{
        Scope='UIA/keyboard smoke only; NVDA speech not tested; clipboard untouched'
        Executable=$Exe
        Shell=$Shell
        BlocksOnly=[bool]$BlocksOnly
        NewTabOnly=[bool]$NewTabOnly
        ProcessId=$(if ($process) { $process.Id } else { $null })
        AppData=$appData
        Checks=$checks.ToArray()
        Patterns=$patterns.ToArray()
        FocusCycles=$cycles.ToArray()
    } | ConvertTo-Json -Depth 7 | Set-Content -LiteralPath (Join-Path $runDirectory 'results.json') -Encoding utf8
    $checks | Set-Content -LiteralPath (Join-Path $runDirectory 'results.txt') -Encoding utf8
}
$passed = @($checks | Where-Object { $_.StartsWith('passed:') }).Count
$skipped = @($checks | Where-Object { $_.StartsWith('skipped:') }).Count
$failed = @($checks | Where-Object { $_.StartsWith('failed:') }).Count
$environmentErrors = @($checks | Where-Object { $_.StartsWith('error:') }).Count
Write-Output "UIA/keyboard smoke ($Shell): passed=$passed skipped=$skipped failed=$failed errors=$environmentErrors"
Write-Output "Artifacts: $runDirectory"
if ($failed -gt 0 -and -not $failure) { $failure = "$failed accessibility checks failed" }
if ($environmentErrors -gt 0 -and -not $failure) { $failure = "$environmentErrors UI environment errors" }
if ($failure) { throw $failure }
