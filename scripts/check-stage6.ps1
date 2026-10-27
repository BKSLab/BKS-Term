param(
    [string]$Exe = (Join-Path $PSScriptRoot '../target/release/bks-app.exe'),
    [string]$Artifacts = (Join-Path $PSScriptRoot '../target/stage6-check'),
    [switch]$Iteration2,
    [switch]$Iteration3,
    [switch]$AnnouncementsOnly
)

# Stage 6: targeted native reading and search checks for the selected iteration.
# A new process owns every queried HWND and a separate APPDATA. No clipboard use.
# UIA/keyboard checks do not test NVDA speech.
$ErrorActionPreference = 'Stop'
if ((@($Iteration2, $Iteration3, $AnnouncementsOnly) | Where-Object { $_ }).Count -gt 1) { throw 'Select one iteration or the targeted announcement check' }
if ($PSVersionTable.PSEdition -eq 'Core') {
    $childArguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath, '-Exe', $Exe, '-Artifacts', $Artifacts)
    if ($Iteration2) { $childArguments += '-Iteration2' }
    if ($Iteration3) { $childArguments += '-Iteration3' }
    if ($AnnouncementsOnly) { $childArguments += '-AnnouncementsOnly' }
    & "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" @childArguments
    exit $LASTEXITCODE
}
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName UIAutomationClientsideProviders
Add-Type -AssemblyName System.Drawing
$references = @([Windows.Automation.AutomationElement].Assembly.Location,
    [Windows.Automation.AutomationPattern].Assembly.Location,
    [UIAutomationClientsideProviders.UIAutomationClientSideProviders].Assembly.Location)
if (-not ('BksStage6' -as [type])) {
    Add-Type -ReferencedAssemblies $references -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class BksStage6 {
    public sealed class Notice {
        public string Text { get; set; }
        public string Kind { get; set; }
        public string Processing { get; set; }
        public string Activity { get; set; }
    }
    private static readonly object noticeLock = new object();
    private static readonly List<Notice> notices = new List<Notice>();
    private static System.Windows.Automation.AutomationElement noticeSource;
    private static System.Windows.Automation.AutomationEventHandler noticeHandler;
    public static void WatchNotifications(uint process, IntPtr window) {
        Own(process, window);
        noticeSource = System.Windows.Automation.AutomationElement.FromHandle(window);
        noticeHandler = (sender, args) => {
            var notification = args as System.Windows.Automation.NotificationEventArgs;
            if (notification == null) return;
            lock (noticeLock) notices.Add(new Notice {
                Text=notification.DisplayString, Kind=notification.NotificationKind.ToString(),
                Processing=notification.NotificationProcessing.ToString(), Activity=notification.ActivityId
            });
        };
        System.Windows.Automation.Automation.AddAutomationEventHandler(
            System.Windows.Automation.AutomationElementIdentifiers.NotificationEvent,
            noticeSource, System.Windows.Automation.TreeScope.Element, noticeHandler);
    }
    public static Notice[] Notifications() { lock (noticeLock) return notices.ToArray(); }
    public static void StopNotifications() {
        if (noticeSource == null || noticeHandler == null) return;
        System.Windows.Automation.Automation.RemoveAutomationEventHandler(
            System.Windows.Automation.AutomationElementIdentifiers.NotificationEvent, noticeSource, noticeHandler);
        noticeHandler = null; noticeSource = null;
    }
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.NoInlining)]
    public static void InitializeUia() {
        var table = UIAutomationClientsideProviders.UIAutomationClientSideProviders.ClientSideProviderDescriptionTable;
        try { System.Windows.Automation.ClientSettings.RegisterClientSideProviders(table); }
        catch (NullReferenceException) {
            // The managed default loader can encounter a PowerShell dynamic
            // frame. Its one-shot flag is cleared; register the standard table.
            System.Windows.Automation.ClientSettings.RegisterClientSideProviders(table);
        }
    }
    private delegate bool EnumProc(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassNameW(IntPtr window, StringBuilder text, int capacity);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern bool BringWindowToTop(IntPtr window);
    [DllImport("user32.dll")] private static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] private static extern bool AttachThreadInput(uint first, uint second, bool attach);
    [DllImport("kernel32.dll")] private static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] private static extern bool PostMessageW(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)] private static extern IntPtr WriteText(IntPtr window, uint message, IntPtr unused, string text);
    [DllImport("user32.dll", EntryPoint="SendMessageW")] private static extern IntPtr NativeMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", EntryPoint="SendMessageW")] private static extern IntPtr GetSelection(IntPtr window, uint message, out uint start, out uint end);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)] private static extern IntPtr GetText(IntPtr window, uint message, IntPtr capacity, StringBuilder text);
    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] private static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] private static extern bool PrintWindow(IntPtr window, IntPtr hdc, uint flags);
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
    public static void Own(uint process, IntPtr window) {
        if (window == IntPtr.Zero || ProcessId(window) != process)
            throw new InvalidOperationException("Window is not owned by the isolated test process");
    }
    public static string ClassName(IntPtr window) {
        var text = new StringBuilder(128); GetClassNameW(window, text, text.Capacity); return text.ToString();
    }
    public static IntPtr ProcessWindow(uint process) { return ProcessClass(process, "BksTermWindow"); }
    public static IntPtr ProcessClass(uint process, string className) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((window, parameter) => {
            if (ProcessId(window) != process || ClassName(window) != className) return true;
            result = window; return false;
        }, IntPtr.Zero);
        return result;
    }
    public static IntPtr Focus(IntPtr window) {
        uint process; uint thread = GetWindowThreadProcessId(window, out process);
        var info = new GUIINFO(); info.Size = (uint)Marshal.SizeOf(typeof(GUIINFO));
        if (!GetGUIThreadInfo(thread, ref info)) throw new InvalidOperationException("GetGUIThreadInfo failed");
        return info.Focus;
    }
    public static bool MenuActive(uint process, IntPtr window) {
        Own(process, window);
        uint ignored; uint thread = GetWindowThreadProcessId(window, out ignored);
        var info = new GUIINFO(); info.Size = (uint)Marshal.SizeOf(typeof(GUIINFO));
        if (!GetGUIThreadInfo(thread, ref info)) throw new InvalidOperationException("GetGUIThreadInfo failed");
        return info.Menu != IntPtr.Zero;
    }
    public static void Activate(uint process, IntPtr window) {
        Own(process, window);
        uint ignored;
        uint foregroundThread = GetWindowThreadProcessId(GetForegroundWindow(), out ignored);
        uint currentThread = GetCurrentThreadId();
        bool attached = foregroundThread != 0 && foregroundThread != currentThread && AttachThreadInput(currentThread, foregroundThread, true);
        try { ShowWindow(window, 9); BringWindowToTop(window); SetForegroundWindow(window); }
        finally { if (attached) AttachThreadInput(currentThread, foregroundThread, false); }
    }
    public static bool ReadOnly(IntPtr window) { return (GetWindowLongPtrW(window, -16).ToInt64() & 0x800) != 0; }
    public static void SetText(uint process, IntPtr window, string text) {
        Own(process, window);
        if (!ClassName(window).Equals("Edit", StringComparison.OrdinalIgnoreCase)) throw new InvalidOperationException("Fixture target is not EDIT");
        WriteText(window, 0x000C, IntPtr.Zero, text);
    }
    public static string NativeText(uint process, IntPtr window) {
        Own(process, window);
        int length = NativeMessage(window, 0x000E, IntPtr.Zero, IntPtr.Zero).ToInt32();
        if (length < 0 || length > 2 * 1024 * 1024) throw new InvalidOperationException("Unexpected own control text size");
        var text = new StringBuilder(length + 1);
        GetText(window, 0x000D, new IntPtr(text.Capacity), text);
        return text.ToString();
    }
    public static uint[] Selection(uint process, IntPtr window) {
        Own(process, window);
        uint start, end; GetSelection(window, 0x00B0, out start, out end);
        return new[] { start, end };
    }
    public static void Select(uint process, IntPtr window, int start, int end) {
        Own(process, window);
        NativeMessage(window, 0x00B1, new IntPtr(start), new IntPtr(end));
    }
    public static int FirstLine(uint process, IntPtr window) {
        Own(process, window);
        return NativeMessage(window, 0x00CE, IntPtr.Zero, IntPtr.Zero).ToInt32();
    }
    public static int LineIndex(uint process, IntPtr window, int line) {
        Own(process, window);
        return NativeMessage(window, 0x00BB, new IntPtr(line), IntPtr.Zero).ToInt32();
    }
    public static void ScrollTo(uint process, IntPtr window, int character) {
        Own(process, window);
        int desired = NativeMessage(window, 0x00C9, new IntPtr(character), IntPtr.Zero).ToInt32();
        int first = FirstLine(process, window);
        NativeMessage(window, 0x00B6, IntPtr.Zero, new IntPtr(desired - first));
    }
    public static RECT WindowRect(uint process, IntPtr window) {
        Own(process, window);
        RECT rect; if (!GetWindowRect(window, out rect)) throw new InvalidOperationException("GetWindowRect failed");
        return rect;
    }
    public static void Resize(uint process, IntPtr window, int width, int height) {
        Own(process, window);
        if (!SetWindowPos(window, IntPtr.Zero, 0, 0, width, height, 0x0016)) throw new InvalidOperationException("SetWindowPos failed");
    }
    public static bool Print(uint process, IntPtr window, IntPtr hdc) { Own(process, window); return PrintWindow(window, hdc, 2); }
    public static void Post(uint process, IntPtr window, uint message, int value) {
        Own(process, window);
        if (!PostMessageW(window, message, new IntPtr(value), IntPtr.Zero)) throw new InvalidOperationException("PostMessage failed");
    }
    private static INPUT Key(ushort key, bool up) {
        return new INPUT { Type=1, Data=new INPUTUNION { Keyboard=new KEYBDINPUT { Key=key, Flags=up ? 2u : 0u } } };
    }
    private static void Send(uint process, INPUT[] input) {
        if (ProcessId(GetForegroundWindow()) != process) throw new InvalidOperationException("Foreground lost; keyboard input refused");
        if (SendInput((uint)input.Length, input, Marshal.SizeOf(typeof(INPUT))) != input.Length)
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
    }
    public static void ControlKey(uint process, ushort key) { Chord(process, key, false); }
    public static void ControlShiftKey(uint process, ushort key) { Chord(process, key, true); }
    public static void PlainKey(uint process, ushort key) { Send(process, new[] { Key(key, false), Key(key, true) }); }
    public static void ShiftKey(uint process, ushort key) {
        try {
            Send(process, new[] { Key(0x10, false) });
            System.Threading.Thread.Sleep(40);
            Send(process, new[] { Key(key, false), Key(key, true) });
            System.Threading.Thread.Sleep(40);
        } finally {
            var release = new[] { Key(0x10, true) };
            SendInput(1, release, Marshal.SizeOf(typeof(INPUT)));
        }
    }
    private static void Chord(uint process, ushort key, bool shift) {
        try {
            Send(process, shift ? new[] { Key(0x11, false), Key(0x10, false) } : new[] { Key(0x11, false) });
            System.Threading.Thread.Sleep(40);
            Send(process, new[] { Key(key, false), Key(key, true) });
            System.Threading.Thread.Sleep(40);
        } finally {
            // Release only the modifier injected by this helper, even on abort.
            var release = shift ? new[] { Key(0x10, true), Key(0x11, true) } : new[] { Key(0x11, true) };
            SendInput((uint)release.Length, release, Marshal.SizeOf(typeof(INPUT)));
        }
    }
}
'@
}
[BksStage6]::InitializeUia()

$checks = [Collections.Generic.List[string]]::new()
$observations = [Collections.Generic.List[object]]::new()
$iteration = if ($Iteration3 -or $AnnouncementsOnly) { 3 } elseif ($Iteration2) { 2 } else { 1 }
$runPrefix = if ($AnnouncementsOnly) { 'announcements-' } else { "iteration$iteration-" }
$runName = $runPrefix + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Artifacts $runName)
$runDirectory = (Resolve-Path -LiteralPath (Join-Path $Artifacts $runName)).Path
$appData = Join-Path $runDirectory 'appdata'
$null = New-Item -ItemType Directory -Path $appData
$process = $null
$main = [IntPtr]::Zero
$failure = $null
$keyboardUnavailable = $false
$keyboardAttempted = $false

function Check([string]$Name, [bool]$Condition) {
    if (-not $Condition) { throw "FAILED: $Name" }
    $checks.Add("passed: $Name")
}
function Wait-For([scriptblock]$Condition, [string]$Name, [int]$Timeout = 10000) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.ElapsedMilliseconds -lt $Timeout) {
        if (& $Condition) { return }
        if ($process -and $process.HasExited) { throw "Test process exited: $Name" }
        Start-Sleep -Milliseconds 40
    }
    throw "Timeout: $Name"
}
function Element([IntPtr]$Window) {
    [BksStage6]::Own($process.Id, $Window)
    [Windows.Automation.AutomationElement]::FromHandle($Window)
}
function Text-Pattern([IntPtr]$Window) {
    $element = Element $Window
    $pattern = $null
    if (-not $element.TryGetCurrentPattern([Windows.Automation.TextPattern]::Pattern, [ref]$pattern)) {
        throw 'Native reader does not expose TextPattern'
    }
    return $pattern
}
function Read-Text([IntPtr]$Window) { (Text-Pattern $Window).DocumentRange.GetText(-1) }
function Check-Reader([IntPtr]$Window, [string]$Name) {
    $element = Element $Window
    $pattern = Text-Pattern $Window
    $observations.Add([ordered]@{ Name=$element.Current.Name; Class=$element.Current.ClassName; ControlType=$element.Current.ControlType.ProgrammaticName; TextPattern=$true })
    Check "$Name is a native read-only EDIT" ([BksStage6]::ClassName($Window) -ieq 'Edit' -and [BksStage6]::ReadOnly($Window))
    Check "$Name exposes its accessible name and text role" ($element.Current.Name.Contains($Name) -and $element.Current.ControlType -in @([Windows.Automation.ControlType]::Document, [Windows.Automation.ControlType]::Edit))
    Check "$Name is focusable with UIA selection support" ($element.Current.IsKeyboardFocusable -and $pattern.GetSelection().Count -gt 0)
    $attribute = $pattern.DocumentRange.GetAttributeValue([Windows.Automation.TextPattern]::IsReadOnlyAttribute)
    if ($attribute -is [bool]) { Check "$Name TextPattern is read-only" $attribute }
    else { $checks.Add("skipped: $Name IsReadOnlyAttribute unsupported; native ES_READONLY checked") }
}
function Require-OwnForeground {
    if ([BksStage6]::ProcessId([BksStage6]::GetForegroundWindow()) -ne $process.Id) {
        throw 'Foreground lost; reading focus check unavailable'
    }
}
function Select-Source([int]$Command, [IntPtr]$Visible, [IntPtr]$Hidden, [switch]$DataOnly) {
    if (-not $DataOnly) { Require-OwnForeground }
    [BksStage6]::Post($process.Id, $main, 0x111, $Command)
    Wait-For {
        if (-not $DataOnly) { Require-OwnForeground }
        [BksStage6]::IsWindowVisible($Visible) -and -not [BksStage6]::IsWindowVisible($Hidden) -and ($DataOnly -or [BksStage6]::Focus($main) -eq $Visible)
    } 'reading source visibility and focus'
}
function Begin-Command([string]$Command, [string]$Marker) {
    $script:lastFixtureMarker = $Marker
    [BksStage6]::SetText($process.Id, $inputBox, $Command)
    [BksStage6]::Post($process.Id, $execute, 0x00F5, 0) # BM_CLICK: explicit fixture command.
    Wait-For { [string]::IsNullOrWhiteSpace((Read-Text $inputBox)) } 'fixture submit accepted'
    Wait-For { (Read-Text $journal).Contains($Marker) } 'fixture output in journal' 15000
}
function Wait-Prompt {
    Wait-For {
        $text = Read-Text $journal
        $output = $text.LastIndexOf($script:lastFixtureMarker, [StringComparison]::Ordinal)
        $prompt = $text.LastIndexOf('PS ', [StringComparison]::Ordinal)
        $output -ge 0 -and $prompt -gt $output -and $text.IndexOf('> ', $prompt, [StringComparison]::Ordinal) -ge 0
    } 'next PowerShell prompt after fixture output'
}
function Submit([string]$Command, [string]$Marker) { Begin-Command $Command $Marker; Wait-Prompt }
function Reader-State([IntPtr]$Window, [string]$Label) {
    $text = [BksStage6]::NativeText($process.Id, $Window)
    $span = [BksStage6]::Selection($process.Id, $Window)
    $line = [BksStage6]::FirstLine($process.Id, $Window)
    $top = [Math]::Max(0, [BksStage6]::LineIndex($process.Id, $Window, $line))
    $next = [BksStage6]::LineIndex($process.Id, $Window, $line + 1)
    $logicalStart = if ($top -eq 0) { 0 } else { $text.LastIndexOf("`n", [Math]::Min($top - 1, $text.Length - 1)) + 1 }
    $logicalEnd = $text.IndexOf("`r`n", $logicalStart)
    if ($logicalEnd -lt 0) { $logicalEnd = $text.Length }
    $selected = $text.Substring([int]$span[0], [int]($span[1] - $span[0]))
    $logicalText = $text.Substring($logicalStart, $logicalEnd - $logicalStart)
    $observations.Add([ordered]@{ Label=$Label; Selection=@($span); Selected=$selected; FirstVisualLine=$line; TopUtf16=$top; NextVisualUtf16=$next; LogicalLineStart=$logicalStart; LogicalLinePrefix=$logicalText.Substring(0, [Math]::Min(48, $logicalText.Length)) })
    [pscustomobject]@{ Text=$text; Start=$span[0]; End=$span[1]; Selected=$selected; Top=$top; Next=$next; LogicalStart=$logicalStart; LogicalText=$logicalText }
}
function Mark-Reader([IntPtr]$Window, [string]$Marker, [int]$InsideLine = 0) {
    $text = [BksStage6]::NativeText($process.Id, $Window)
    $index = $text.IndexOf($Marker, [StringComparison]::Ordinal)
    Check "bookmark fixture $Marker exists" ($index -ge 0)
    [BksStage6]::Select($process.Id, $Window, $index, $index + $Marker.Length)
    [BksStage6]::ScrollTo($process.Id, $Window, $index + $InsideLine)
    Wait-For { [BksStage6]::Selection($process.Id, $Window)[0] -eq $index } 'native bookmark selection'
    Reader-State $Window $Marker
}
function Check-Bookmark($Expected, [IntPtr]$Window, [string]$Label) {
    $actual = Reader-State $Window $Label
    Check "$Label preserves selected text" ($actual.Selected -eq $Expected.Selected)
    Check "$Label preserves native UTF-16 selection" ($actual.Start -eq $Expected.Start -and $actual.End -eq $Expected.End)
    $anchorStart = $actual.Text.IndexOf($Expected.LogicalText, [StringComparison]::Ordinal)
    Check "$Label keeps the viewport text anchor" ($anchorStart -ge 0)
    $anchor = $anchorStart + $Expected.Top - $Expected.LogicalStart
    Check "$Label keeps the same anchor in the top visual row" ($actual.Top -le $anchor -and ($actual.Next -lt 0 -or $anchor -lt $actual.Next))
}
function Keyboard-Case([string]$Name, [scriptblock]$Action) {
    if ($script:keyboardUnavailable) { $checks.Add("skipped: $Name after external foreground loss"); return $false }
    try {
        if (-not $script:keyboardAttempted) {
            $script:keyboardAttempted = $true
            [BksStage6]::Activate($process.Id, $main)
            Wait-For { [BksStage6]::ProcessId([BksStage6]::GetForegroundWindow()) -eq $process.Id } 'own foreground activation' 1500
        }
        Require-OwnForeground
        & $Action
        $checks.Add("passed: $Name")
        return $true
    } catch {
        if ($_.Exception.Message.Contains('Foreground lost;') -or [BksStage6]::ProcessId([BksStage6]::GetForegroundWindow()) -ne $process.Id) {
            $script:keyboardUnavailable = $true
            $checks.Add("error: $Name unavailable because own process lost foreground")
            $checks.Add("skipped: remaining keys in $Name; independent data checks continue")
            return $false
        }
        throw
    }
}
function New-ProfileTab {
    [BksStage6]::ControlShiftKey($process.Id, 0x54)
    $resolved = @{ Item=$null }
    Wait-For {
        Require-OwnForeground
        $popup = [BksStage6]::ProcessClass($process.Id, '#32768')
        if ($popup -eq [IntPtr]::Zero -or -not [BksStage6]::IsWindowVisible($popup)) { return $false }
        $condition = [Windows.Automation.AndCondition]::new(
            [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::MenuItem),
            [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, 'Windows PowerShell'))
        $resolved.Item = (Element $popup).FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
        $null -ne $resolved.Item
    } 'own profile picker and explicit PowerShell item'
    Require-OwnForeground
    $invoke = $null
    Check 'profile choice exposes native UIA InvokePattern' ($resolved.Item.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern, [ref]$invoke))
    $invoke.Invoke()
    Wait-For { Require-OwnForeground; [BksStage6]::GetDlgItem($main, 2002) -ne [IntPtr]::Zero } 'explicit profile creates second tab'
}
function Invoke-Iteration2 {
    Submit "[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); 0..69|%{`$s=('BKS62_ROW_{0:D3}_Ж' -f `$_); if(`$_ -eq 20){`$s+=('x'*240)+'_TAIL'}; [Console]::WriteLine(`$s)}" 'BKS62_ROW_069_Ж'
    Select-Source 60005 $vt $journal -DataOnly
    Check-Reader $vt 'Экран терминала'
    Wait-For { (Read-Text $vt).Contains('BKS62_ROW_020_Ж') } 'iteration 2 baseline document'

    Begin-Command "[Console]::WriteLine('BKS62_'+'APPEND_ARM'); Start-Sleep -Seconds 3; [Console]::WriteLine('BKS62_'+'APPEND_DONE')" 'BKS62_APPEND_ARM'
    Wait-For { (Read-Text $vt).Contains('BKS62_APPEND_ARM') } 'append producer armed in VT document'
    $mainBookmark = Mark-Reader $vt 'BKS62_ROW_020_Ж' 150
    $draft2 = 'stage6_iteration2_draft_Ж中'
    [BksStage6]::SetText($process.Id, $inputBox, $draft2)
    Wait-For { (Read-Text $vt).Contains('BKS62_APPEND_DONE') } 'append while old VT selection remains' 10000
    Wait-Prompt
    Check 'VT append keeps the unselected journal hidden' ([BksStage6]::IsWindowVisible($vt) -and -not [BksStage6]::IsWindowVisible($journal))
    Check-Bookmark $mainBookmark $vt 'active append'
    Check 'append leaves the independent command draft intact' ((Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft2)

    Select-Source 60006 $journal $vt -DataOnly
    $journalBookmark = Mark-Reader $journal 'BKS62_ROW_040_Ж'
    Select-Source 60005 $vt $journal -DataOnly
    Check-Bookmark $mainBookmark $vt 'return from journal source'
    Select-Source 60006 $journal $vt -DataOnly
    Check-Bookmark $journalBookmark $journal 'return to journal source'
    Begin-Command "[Console]::WriteLine('BKS62_'+'SOURCE_ARM'); Start-Sleep -Seconds 3; [Console]::WriteLine('BKS62_'+'SOURCE_DONE')" 'BKS62_SOURCE_ARM'
    [BksStage6]::SetText($process.Id, $inputBox, $draft2)
    Wait-For { (Read-Text $journal).Contains('BKS62_SOURCE_DONE') } 'output while VT source is inactive' 10000
    Wait-Prompt
    Check 'journal append keeps the unselected VT reader hidden' ([BksStage6]::IsWindowVisible($journal) -and -not [BksStage6]::IsWindowVisible($vt))
    Check-Bookmark $journalBookmark $journal 'journal append'
    Select-Source 60005 $vt $journal -DataOnly
    Wait-For { (Read-Text $vt).Contains('BKS62_SOURCE_DONE') } 'inactive VT document catches up'
    Check-Bookmark $mainBookmark $vt 'inactive source append'
    Check 'source changes preserve the draft' ((Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft2)

    $originalRect = [BksStage6]::WindowRect($process.Id, $main)
    $width = $originalRect.Right - $originalRect.Left
    $height = $originalRect.Bottom - $originalRect.Top
    try {
        [BksStage6]::Resize($process.Id, $main, [Math]::Max(640, $width - 320), $height)
        Wait-For { ([BksStage6]::WindowRect($process.Id, $main).Right - [BksStage6]::WindowRect($process.Id, $main).Left) -lt $width } 'narrower native viewport'
        Wait-For { (Read-Text $vt).Contains('BKS62_ROW_020_Ж') } 'VT document after resize'
        Check-Bookmark $mainBookmark $vt 'resize with native Word Wrap'
        $narrowBookmark = Reader-State $vt 'actual narrow viewport before expansion'
    } finally { [BksStage6]::Resize($process.Id, $main, $width, $height) }
    Wait-For { ([BksStage6]::WindowRect($process.Id, $main).Right - [BksStage6]::WindowRect($process.Id, $main).Left) -eq $width } 'original viewport restored'
    # Each resize preserves the actual prior visual-row anchor. Native Word
    # Wrap rounds to row starts, so resizing twice need not be reversible.
    Check-Bookmark $narrowBookmark $vt 'restored viewport'
    $mainBookmark = Reader-State $vt 'main bookmark after viewport restoration'

    Submit "[Console]::Write(([char]27)+'[?1049h'); 0..15|%{[Console]::WriteLine(('BKS62_ALT_{0:D3}_Я' -f `$_))}" 'BKS62_ALT_015_Я'
    Wait-For { (Read-Text $vt).Contains('BKS62_ALT_010_Я') -and -not (Read-Text $vt).Contains('BKS62_ROW_020_Ж') } 'alternate document selected'
    Check 'alternate update keeps the journal hidden' (-not [BksStage6]::IsWindowVisible($journal))
    $altBookmark = Mark-Reader $vt 'BKS62_ALT_010_Я'
    Check 'alternate selection has its own document range' ($altBookmark.Selected -eq 'BKS62_ALT_010_Я')
    Submit "[Console]::Write(([char]27)+'[?1049l'); [Console]::WriteLine('BKS62_'+'MAIN_RETURN')" 'BKS62_MAIN_RETURN'
    Wait-For { (Read-Text $vt).Contains('BKS62_ROW_020_Ж') -and -not (Read-Text $vt).Contains('BKS62_ALT_010_Я') } 'main document restored'
    Check-Bookmark $mainBookmark $vt 'main bookmark after alternate screen'

    Begin-Command "[Console]::WriteLine('BKS62_'+'CLEAR_ARM'); [Console]::Write(([char]27)+'[2J'+([char]27)+'[H'+([char]27)+'[3J'); Start-Sleep -Seconds 3; 0..79|%{[Console]::WriteLine(('BKS62_CLEAR_{0:D3}_Ж' -f `$_))}" 'BKS62_CLEAR_ARM'
    Wait-For { [string]::IsNullOrWhiteSpace((Read-Text $vt)) } 'temporarily empty VT document'
    $empty = Reader-State $vt 'empty document fallback'
    Check 'empty document safely clamps selection and viewport' ($empty.Start -eq 0 -and $empty.End -eq 0 -and $empty.Top -eq 0)
    Wait-For { (Read-Text $vt).Contains('BKS62_CLEAR_079_Ж') } 'output after empty document' 10000
    Wait-Prompt
    Check 'clear and later output keep the journal hidden' (-not [BksStage6]::IsWindowVisible($journal))
    $afterClear = Reader-State $vt 'empty fallback after later output'
    Check 'clear fallback stays at the start instead of following the tail' ($afterClear.Start -eq 0 -and $afterClear.End -eq 0 -and $afterClear.Top -eq 0 -and $afterClear.Text.Length -gt 1000)
    [BksStage6]::SetText($process.Id, $inputBox, $draft2)

    $null = Keyboard-Case 'Ctrl+Shift+Space moves between command input and selected native reader' {
        Select-Source 60005 $vt $journal -DataOnly
        (Element $inputBox).SetFocus()
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $inputBox } 'command focus before reading toggle'
        [BksStage6]::ControlShiftKey($process.Id, 0x20)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $vt -and [BksStage6]::IsWindowVisible($inputBox) } 'input to reader without hiding input'
        [BksStage6]::ControlShiftKey($process.Id, 0x20)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $inputBox -and [BksStage6]::IsWindowVisible($vt) } 'reader to input without hiding reader'
        Check 'command reading toggle preserves its draft' ((Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft2)
    }
    $null = Keyboard-Case 'direct reading toggle releases capture and returns GUI Tab navigation' {
        [BksStage6]::Post($process.Id, $main, 0x111, 1009)
        Wait-For { [BksStage6]::IsWindowVisible($graph) } 'direct surface'
        [BksStage6]::Post($process.Id, $main, 0x111, 60004)
        [BksStage6]::ControlShiftKey($process.Id, 0x20)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $vt -and -not [BksStage6]::IsWindowVisible($inputBox) } 'direct surface to selected reader'
        [BksStage6]::ControlShiftKey($process.Id, 0x20)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $graph } 'selected reader to direct surface'
        [BksStage6]::PlainKey($process.Id, 9)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -ne $graph } 'uncaptured Tab leaves direct surface'
    }
    # Restore command mode through own HWND even if a keyboard case lost focus.
    if (-not [BksStage6]::IsWindowVisible($inputBox)) {
        [BksStage6]::Post($process.Id, $main, 0x111, 1009)
        Wait-For { [BksStage6]::IsWindowVisible($inputBox) } 'command mode restored for data checks'
    }
    Select-Source 60005 $vt $journal -DataOnly

    Begin-Command "[Console]::WriteLine('BKS62_'+'TAB_ARM'); Start-Sleep -Seconds 3; [Console]::WriteLine('BKS62_'+'TAB_DONE')" 'BKS62_TAB_ARM'
    Wait-For { (Read-Text $vt).Contains('BKS62_TAB_ARM') } 'inactive-tab producer armed'
    $inactiveBookmark = Mark-Reader $vt 'BKS62_CLEAR_020_Ж'
    [BksStage6]::SetText($process.Id, $inputBox, $draft2)
    $created = Keyboard-Case 'create second tab for independent reading state' {
        New-ProfileTab
    }
    if ($created) {
        Wait-For { (Read-Text $journal).Contains('PS ') -and -not (Read-Text $journal).Contains('BKS62_CLEAR_020_Ж') } 'second tab journal' 20000
        Submit "[Console]::WriteLine('BKS62_'+'TAB_TWO_Я')" 'BKS62_TAB_TWO_Я'
        Select-Source 60005 $vt $journal -DataOnly
        Wait-For { (Read-Text $vt).Contains('BKS62_TAB_TWO_Я') } 'second tab document'
        $secondBookmark = Mark-Reader $vt 'BKS62_TAB_TWO_Я'
        [BksStage6]::SetText($process.Id, $inputBox, 'stage6_tab2_draft_Я')
        [BksStage6]::Post($process.Id, $main, 0x111, 2001)
        Wait-For { (Read-Text $journal).Contains('BKS62_TAB_DONE') } 'first tab output while inactive' 10000
        Wait-For { (Read-Text $vt).Contains('BKS62_TAB_DONE') } 'first tab document catches up'
        Check-Bookmark $inactiveBookmark $vt 'inactive tab append'
        Check 'first tab retains its draft' ((Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft2)
        [BksStage6]::Post($process.Id, $main, 0x111, 2002)
        Wait-For { (Read-Text $vt).Contains('BKS62_TAB_TWO_Я') } 'second tab restored'
        Check-Bookmark $secondBookmark $vt 'independent second tab'
        Check 'second tab retains its draft' ((Read-Text $inputBox).TrimEnd("`r", "`n") -eq 'stage6_tab2_draft_Я')
    } else {
        $checks.Add('skipped: inactive-tab GUI assertions require successful own-process tab shortcut; other data cases completed')
        Wait-For { (Read-Text $journal).Contains('BKS62_TAB_DONE') } 'remaining producer finished' 10000
    }
}
function Save-OwnScreenshot([string]$Name) {
    $rect = [BksStage6]::WindowRect($process.Id, $main)
    $bitmap = [Drawing.Bitmap]::new($rect.Right - $rect.Left, $rect.Bottom - $rect.Top)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $hdc = $graphics.GetHdc()
        try { $captured = [BksStage6]::Print($process.Id, $main, $hdc) }
        finally { $graphics.ReleaseHdc($hdc) }
        if ($captured) {
            $bitmap.Save((Join-Path $runDirectory $Name), [Drawing.Imaging.ImageFormat]::Png)
            $checks.Add("passed: own HWND PrintWindow artifact $Name saved; visual review is separate")
        } else { $checks.Add("skipped: own HWND PrintWindow unavailable for $Name") }
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}
function Search-Status { [BksStage6]::NativeText($process.Id, $searchStatus) }
function Stable-Journal {
    $idle = @{ Text=[BksStage6]::NativeText($process.Id, $journal); Timer=[Diagnostics.Stopwatch]::StartNew() }
    Wait-For {
        $current = [BksStage6]::NativeText($process.Id, $journal)
        if ($current -ne $idle.Text) { $idle.Text=$current; $idle.Timer.Restart() }
        $idle.Timer.ElapsedMilliseconds -ge 200
    } 'journal idle after native resize' 5000
    return $idle.Text
}
function Search-Query([string]$Text, [string]$Expected) {
    [BksStage6]::SetText($process.Id, $query, $Text)
    Wait-For { (Search-Status) -eq $Expected } "search status: $Expected"
    Check "search reports $Expected" ((Element $searchStatus).Current.Name -eq $Expected)
}
function Search-Click([IntPtr]$Button, [string]$Expected) {
    [BksStage6]::Post($process.Id, $Button, 0x00F5, 0)
    Wait-For { (Search-Status) -eq $Expected } "search navigation: $Expected"
    Check "search navigation reports $Expected" ((Element $searchStatus).Current.Name -eq $Expected)
}
function Check-SameSelection([uint32[]]$Expected, [string]$Name) {
    $actual = [BksStage6]::Selection($process.Id, $vt)
    Check $Name ($actual[0] -eq $Expected[0] -and $actual[1] -eq $Expected[1])
}
function Expect-Announcement([string]$Expected) {
    Wait-For {
        Require-OwnForeground
        $notices = [BksStage6]::Notifications()
        [BksStage6]::NativeText($process.Id, $announcement) -eq $Expected -and (Element $announcement).Current.Name -eq $Expected -and $notices.Length -gt 0 -and $notices[$notices.Length - 1].Text -eq $Expected
    } "visible native and UIA announcement: $Expected"
    Check "announcement exposes $Expected through native text, UIA Name and Notification event" $true
}
function Invoke-Announcements {
    $announcement = [BksStage6]::GetDlgItem($main, 1026)
    $element = Element $announcement
    $observations.Add([ordered]@{ Label='announcement provider'; Class=$element.Current.ClassName; AutomationId=$element.Current.AutomationId; ControlType=$element.Current.ControlType.ProgrammaticName; Name=$element.Current.Name; Focusable=$element.Current.IsKeyboardFocusable; Focused=$element.Current.HasKeyboardFocus; Patterns=@($element.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName }) })
    Check 'announcement is a visible native STATIC' ([BksStage6]::ClassName($announcement) -ieq 'Static' -and [BksStage6]::IsWindowVisible($announcement))
    Check 'announcement exposes the dedicated UIA Text provider' ($element.Current.AutomationId -eq 'BksTermAnnouncements' -and $element.Current.ControlType -eq [Windows.Automation.ControlType]::Text)
    Check 'announcement is not focusable and has no action or reading patterns' (-not $element.Current.IsKeyboardFocusable -and -not $element.Current.HasKeyboardFocus -and $element.GetSupportedPatterns().Count -eq 0)
    [BksStage6]::WatchNotifications($process.Id, $announcement)

    $target = 'BKS_ANN_Ж' + [char]::ConvertFromUtf32(0x1f600) + [char]0x754c
    $needle = $target.ToLowerInvariant()
    Submit "[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); `$t='BKS_ANN_'+[char]0x416+[char]::ConvertFromUtf32(0x1f600)+[char]0x754c; 1..3|%{[Console]::WriteLine(`$t)}; [Console]::WriteLine('BKS_ANN_'+'DATA_READY')" 'BKS_ANN_DATA_READY'
    Select-Source 60005 $vt $journal -DataOnly
    Wait-For { (Read-Text $vt).Contains($target) } 'announcement Unicode fixture in native VT document'
    [BksStage6]::Post($process.Id, $main, 0x111, 60007)
    $query = [BksStage6]::GetDlgItem($main, 1020)
    $searchStatus = [BksStage6]::GetDlgItem($main, 1024)
    $closeSearch = [BksStage6]::GetDlgItem($main, 1023)
    Wait-For { [BksStage6]::IsWindowVisible($query) } 'announcement search bar'
    $nativeBefore = [BksStage6]::Selection($process.Id, $vt)
    Search-Query $needle '1 из 3'
    Check-SameSelection $nativeBefore 'Unicode query counts results without changing native reader selection'
    $draft = 'announcement_draft_Ж中'
    [BksStage6]::SetText($process.Id, $inputBox, $draft)

    $null = Keyboard-Case 'submit acknowledgement and quiet search keyboard routes' {
        [BksStage6]::Post($process.Id, $closeSearch, 0x00F5, 0)
        Wait-For { -not [BksStage6]::IsWindowVisible($query) } 'search closed for command Enter'
        [BksStage6]::SetText($process.Id, $inputBox, "'BKS_ANN_'+'ENTER_READY'")
        (Element $inputBox).SetFocus()
        [BksStage6]::PlainKey($process.Id, 13)
        Wait-For { [string]::IsNullOrWhiteSpace((Read-Text $inputBox)) -and (Read-Text $journal).Contains('BKS_ANN_ENTER_READY') } 'real Enter submits the command'
        $script:lastFixtureMarker = 'BKS_ANN_ENTER_READY'
        Wait-Prompt
        Expect-Announcement 'Команда отправлена'
        Check 'submit acknowledgement leaves focus in the native command editor' ([BksStage6]::Focus($main) -eq $inputBox)
        [BksStage6]::SetText($process.Id, $inputBox, $draft)
        [BksStage6]::ControlKey($process.Id, 0x46)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $query } 'Ctrl+F focuses the query'
        $nativeBefore = [BksStage6]::Selection($process.Id, $vt)
        Search-Query "Write-Output 'BKS_ANN_QUERY_MUST_NOT_EXECUTE'" 'Ничего не найдено'
        Expect-Announcement 'Ничего не найдено'
        $eventsBeforeQuery = [BksStage6]::Notifications().Length
        $previousAnnouncement = (Element $announcement).Current.Name
        $debounce = [Diagnostics.Stopwatch]::StartNew()
        [BksStage6]::SetText($process.Id, $query, 'BKS_ANN_PARTIAL_QUERY')
        [BksStage6]::SetText($process.Id, $query, $needle.Substring(0, $needle.Length - 1))
        [BksStage6]::SetText($process.Id, $query, $needle)
        $earlyAnnouncement = (Element $announcement).Current.Name
        if ($debounce.ElapsedMilliseconds -lt 300) {
            Check 'query feedback waits for debounce before replacing the previous announcement' ($earlyAnnouncement -eq $previousAnnouncement)
        } else { $checks.Add('skipped: UIA roundtrip exceeded the early debounce observation window') }
        Wait-For { (Search-Status) -eq '1 из 3' } 'latest query count before announcement'
        Expect-Announcement '1 из 3'
        $settled = [Diagnostics.Stopwatch]::StartNew()
        while ($settled.ElapsedMilliseconds -lt 450) { Require-OwnForeground; Start-Sleep -Milliseconds 50 }
        Check 'rapid query edits raise one debounced Notification event' ([BksStage6]::Notifications().Length -eq $eventsBeforeQuery + 1)
        Check-SameSelection $nativeBefore 'debounced search count leaves the native selection unchanged'
        Check 'debounced feedback keeps focus in the query and preserves the command draft' ([BksStage6]::Focus($main) -eq $query -and (Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft)
        Search-Query 'BKS_ANN_NO_RESULT_Я' 'Ничего не найдено'
        Expect-Announcement 'Ничего не найдено'
        Search-Query '' 'Введите текст'
        Expect-Announcement 'Введите текст'
        Search-Query $needle '1 из 3'
        Expect-Announcement '1 из 3'
        [BksStage6]::PlainKey($process.Id, 0x72)
        Wait-For { Require-OwnForeground; (Search-Status) -eq '2 из 3' } 'query F3 advances result'
        Expect-Announcement '2 из 3'
        Check-SameSelection $nativeBefore 'F3 from query announces the next result without native selection changes'
        foreach ($step in 1..4) { [BksStage6]::PlainKey($process.Id, 9) }
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $vt } 'Tab passes the three search buttons and reaches the reader'
        $tabText = [BksStage6]::NativeText($process.Id, $vt)
        $tabFirst = $tabText.IndexOf($target, [StringComparison]::Ordinal)
        $tabSecond = $tabText.IndexOf($target, $tabFirst + $target.Length, [StringComparison]::Ordinal)
        Check 'Tab into the reader selects the current Unicode result instead of its old bookmark' ([BksStage6]::Selection($process.Id, $vt)[0] -eq $tabSecond -and (Text-Pattern $vt).GetSelection()[0].GetText(-1) -eq $target)
        foreach ($step in 1..4) { [BksStage6]::ShiftKey($process.Id, 9) }
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $query } 'Shift+Tab returns through the search buttons to the query'
        $beforeEnterEvents = [BksStage6]::Notifications().Length
        [BksStage6]::PlainKey($process.Id, 13)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $vt } 'search Enter focuses the reader'
        $text = [BksStage6]::NativeText($process.Id, $vt)
        $first = $text.IndexOf($target, [StringComparison]::Ordinal)
        $second = $text.IndexOf($target, $first + $target.Length, [StringComparison]::Ordinal)
        $third = $text.IndexOf($target, $second + $target.Length, [StringComparison]::Ordinal)
        Check 'explicit Enter selects the second original Unicode range' ([BksStage6]::Selection($process.Id, $vt)[0] -eq $second -and (Text-Pattern $vt).GetSelection()[0].GetText(-1) -eq $target)
        Expect-Announcement '2 из 3'
        Wait-For { Require-OwnForeground; [BksStage6]::Notifications().Length -gt $beforeEnterEvents } 'Enter raises an explicit current-result notification'
        [BksStage6]::PlainKey($process.Id, 0x72)
        Wait-For { Require-OwnForeground; (Search-Status) -eq '3 из 3' -and [BksStage6]::Selection($process.Id, $vt)[0] -eq $third } 'reader F3 advances native selection'
        Expect-Announcement '3 из 3'
        Check 'F3 from reader selects the original Unicode glyphs' ((Text-Pattern $vt).GetSelection()[0].GetText(-1) -eq $target)
        [BksStage6]::PlainKey($process.Id, 27)
        Wait-For { Require-OwnForeground; -not [BksStage6]::IsWindowVisible($query) -and [BksStage6]::Focus($main) -eq $inputBox } 'search Escape restores the command editor'
        Check 'search routes never execute query text or replace the command draft' ((Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft -and -not (Read-Text $journal).Contains('BKS_ANN_QUERY_MUST_NOT_EXECUTE'))

        # Submit one delayed producer before reopening search. Passive output
        # updates the model count; it must not replace the last explicit status.
        [BksStage6]::SetText($process.Id, $inputBox, "Start-Sleep -Seconds 3; [Console]::WriteLine('BKS_ANN_'+[char]0x416+[char]::ConvertFromUtf32(0x1f600)+[char]0x754c); [Console]::WriteLine('BKS_ANN_'+'PASSIVE_DONE')")
        [BksStage6]::PlainKey($process.Id, 13)
        Wait-For { [string]::IsNullOrWhiteSpace((Read-Text $inputBox)) } 'delayed producer submitted'
        Expect-Announcement 'Команда отправлена'
        [BksStage6]::SetText($process.Id, $inputBox, $draft)
        [BksStage6]::ControlKey($process.Id, 0x46)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $query } 'query focused before passive output'
        Search-Query '' 'Введите текст'
        Search-Query $needle '1 из 3'
        Expect-Announcement '1 из 3'
        $eventsBeforePassive = [BksStage6]::Notifications().Length
        $quietDocument = [BksStage6]::NativeText($process.Id, $vt)
        Wait-For { Require-OwnForeground; (Read-Text $journal).Contains('BKS_ANN_PASSIVE_DONE') -and (Search-Status) -eq '1 из 4' } 'passive output updates backend search count'
        $quiet = [Diagnostics.Stopwatch]::StartNew()
        $unchanged = $true
        while ($quiet.ElapsedMilliseconds -lt 600) {
            Require-OwnForeground
            $unchanged = $unchanged -and (Element $announcement).Current.Name -eq '1 из 3' -and [BksStage6]::NativeText($process.Id, $announcement) -eq '1 из 3'
            Start-Sleep -Milliseconds 100
        }
        Check 'passive output keeps the last explicit announcement' $unchanged
        Check 'passive output raises no extra Notification event' ([BksStage6]::Notifications().Length -eq $eventsBeforePassive)
        Check 'passive output during query leaves the native reader document quiet' ([BksStage6]::NativeText($process.Id, $vt) -eq $quietDocument)
        Check 'passive output preserves query focus and command draft' ([BksStage6]::Focus($main) -eq $query -and (Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft)
        $observations.Add([ordered]@{ Label='passive search output'; BackendStatus=Search-Status; Announcement=(Element $announcement).Current.Name; NativeDocumentLength=$quietDocument.Length })
    }
    $notifications = [BksStage6]::Notifications()
    if ($notifications.Length -gt 0) {
        Check 'notifications use Other, MostRecent and activity bks.feedback' (@($notifications | Where-Object { $_.Kind -ne 'Other' -or $_.Processing -ne 'MostRecent' -or $_.Activity -ne 'bks.feedback' }).Count -eq 0)
    }
}
function Invoke-Iteration3 {
    Submit "'BKS63_'+'BLOCK_READY'" 'BKS63_BLOCK_READY'
    Submit "[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); `$t='BKS63_'+[char]0x416+[char]::ConvertFromUtf32(0x1f600)+[char]0x754c+'e'+[char]0x301; `$e=[char]27; [Console]::Write(`$e+'[31m'+`$t+`$e+'[0m'+[char]13+[char]10); 1..60|%{[Console]::WriteLine('padding '+`$_)}; [Console]::Write(`$e+'[38;2;100;170;250m'+`$t+`$e+'[0m'+[char]13+[char]10); [Console]::WriteLine('gap'); [Console]::Write(`$e+'[32m'+`$t+`$e+'[0m'+[char]13+[char]10); [Console]::WriteLine('BKS63_'+'READY')" 'BKS63_READY'
    $target = 'BKS63_Ж' + [char]::ConvertFromUtf32(0x1f600) + [char]0x754c + 'e' + [char]0x301
    $needle = 'bks63_ж' + [char]::ConvertFromUtf32(0x1f600) + [char]0x754c + 'e' + [char]0x301
    $draft3 = 'stage6_search_draft_Ж中'
    [BksStage6]::SetText($process.Id, $inputBox, $draft3)
    Select-Source 60005 $vt $journal -DataOnly
    [BksStage6]::Post($process.Id, $main, 0x111, 60007)
    $query = [BksStage6]::GetDlgItem($main, 1020)
    $previous = [BksStage6]::GetDlgItem($main, 1021)
    $next = [BksStage6]::GetDlgItem($main, 1022)
    $closeSearch = [BksStage6]::GetDlgItem($main, 1023)
    $searchStatus = [BksStage6]::GetDlgItem($main, 1024)
    Wait-For { [BksStage6]::IsWindowVisible($query) } 'real menu opens native search bar'
    $queryElement = Element $query
    Check 'search query is a named editable native EDIT' ([BksStage6]::ClassName($query) -ieq 'Edit' -and -not [BksStage6]::ReadOnly($query) -and $queryElement.Current.Name.Contains('Поиск'))
    foreach ($button in @($previous, $next, $closeSearch)) { Check 'search action exposes an accessible native button' ((Element $button).Current.ControlType -eq [Windows.Automation.ControlType]::Button -and (Element $button).Current.Name.Length -gt 0) }
    $journalBefore = [BksStage6]::NativeText($process.Id, $journal)
    $selectionBeforeQuery = [BksStage6]::Selection($process.Id, $vt)
    Search-Query $needle '1 из 3'
    $text = [BksStage6]::NativeText($process.Id, $vt)
    $first = $text.IndexOf($target, [StringComparison]::Ordinal)
    $second = $text.IndexOf($target, $first + $target.Length, [StringComparison]::Ordinal)
    $third = $text.IndexOf($target, $second + $target.Length, [StringComparison]::Ordinal)
    Check 'Unicode search counts all three original history ranges' ($first -ge 0 -and $second -gt $first -and $third -gt $second)
    Check-SameSelection $selectionBeforeQuery 'query edits leave the native document selection quiet until Enter'
    Search-Click $previous '3 из 3 — с конца'
    Check-SameSelection $selectionBeforeQuery 'previous action in the search bar leaves native document selection quiet'
    Search-Click $next '1 из 3 — с начала'
    Search-Click $next '2 из 3'
    Check-SameSelection $selectionBeforeQuery 'next action in the search bar leaves native document selection quiet'
    Search-Query "Write-Output 'BKS63_QUERY_MUST_NOT_EXECUTE'" 'Ничего не найдено'
    Search-Query '' 'Введите текст'
    Search-Query $needle '1 из 3'
    Check 'query edits never execute PTY input or replace the command draft' ([BksStage6]::NativeText($process.Id, $journal) -eq $journalBefore -and (Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft3)
    Write-Output 'Stage3 core data: Unicode/history/count/wrap/no-results/no-PTY passed'
    Save-OwnScreenshot 'search-wide.png'
    $rect = [BksStage6]::WindowRect($process.Id, $main)
    try { [BksStage6]::Resize($process.Id, $main, 480, $rect.Bottom - $rect.Top); Save-OwnScreenshot 'search-narrow.png' }
    finally { [BksStage6]::Resize($process.Id, $main, $rect.Right - $rect.Left, $rect.Bottom - $rect.Top) }

    $null = Keyboard-Case 'Ctrl+F/F3/Shift+F3/Enter/Esc and visible/hidden search Tab' {
        [BksStage6]::Post($process.Id, $closeSearch, 0x00F5, 0)
        Wait-For { -not [BksStage6]::IsWindowVisible($query) } 'bar closed before keyboard open'
        (Element $inputBox).SetFocus()
        [BksStage6]::ControlKey($process.Id, 0x46)
        Wait-For { Require-OwnForeground; [BksStage6]::IsWindowVisible($query) -and [BksStage6]::Focus($main) -eq $query } 'Ctrl+F query focus'
        Search-Query '' 'Введите текст'
        Search-Query $needle '1 из 3'
        $querySelection = [BksStage6]::Selection($process.Id, $vt)
        [BksStage6]::PlainKey($process.Id, 9)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $previous } 'visible bar Tab reaches previous action'
        [BksStage6]::ShiftKey($process.Id, 9)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $query } 'visible bar Shift+Tab returns to query'
        [BksStage6]::PlainKey($process.Id, 0x72)
        Wait-For { (Search-Status) -eq '2 из 3' } 'F3 next result'
        [BksStage6]::ShiftKey($process.Id, 0x72)
        Wait-For { (Search-Status) -eq '1 из 3' } 'Shift+F3 previous result'
        Check-SameSelection $querySelection 'F3 and Shift+F3 in query update the result without moving native selection'
        $beforeEnter = Stable-Journal
        [BksStage6]::PlainKey($process.Id, 13)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $vt } 'Enter focuses selected VT result'
        $selectedActual = (Text-Pattern $vt).GetSelection()[0].GetText(-1)
        $afterEnter = [BksStage6]::NativeText($process.Id, $journal)
        $observations.Add([ordered]@{ Label='Enter search result'; Expected=$target; ActualSelected=$selectedActual; NativeRange=[BksStage6]::Selection($process.Id,$vt); JournalBeforeLength=$beforeEnter.Length; JournalAfterLength=$afterEnter.Length; OldPreResizeJournalLength=$journalBefore.Length })
        Check 'Enter exposes the original Unicode selected glyphs' ($selectedActual -eq $target)
        Check 'Enter preserves the command draft without executing query text' ((Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft3 -and -not $afterEnter.Contains('BKS63_QUERY_MUST_NOT_EXECUTE'))
        [BksStage6]::PlainKey($process.Id, 0x72)
        Wait-For { Require-OwnForeground; (Search-Status) -eq '2 из 3' } 'reader F3 selects next result'
        $latestText = [BksStage6]::NativeText($process.Id, $vt)
        $latestFirst = $latestText.IndexOf($target, [StringComparison]::Ordinal)
        $latestSecond = $latestText.IndexOf($target, $latestFirst + $target.Length, [StringComparison]::Ordinal)
        Check 'F3 from reader exposes the second original UTF-16 range' ([BksStage6]::Selection($process.Id, $vt)[0] -eq $latestSecond -and (Text-Pattern $vt).GetSelection()[0].GetText(-1) -eq $target)
        [BksStage6]::PlainKey($process.Id, 27)
        Wait-For { Require-OwnForeground; -not [BksStage6]::IsWindowVisible($query) -and [BksStage6]::Focus($main) -eq $inputBox } 'Esc restores input origin'
        [BksStage6]::PlainKey($process.Id, 9)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $execute } 'hidden bar Tab reaches execute'
        [BksStage6]::ShiftKey($process.Id, 9)
        Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $inputBox } 'hidden bar Shift+Tab returns to input'
        foreach ($key in @(0x26, 0x28)) {
            [BksStage6]::ControlKey($process.Id, 0x46)
            Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $query } 'search reopened for block regression'
            [BksStage6]::PlainKey($process.Id, 13)
            Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $vt } 'search result focused before block navigation'
            [BksStage6]::ControlKey($process.Id, [uint16]$key)
            Wait-For { Require-OwnForeground; [BksStage6]::Focus($main) -eq $journal -and -not [BksStage6]::IsWindowVisible($query) } 'search result returns to journal blocks'
            Check "Ctrl+$key closes search and returns a journal block caret" ((Text-Pattern $journal).GetSelection()[0].GetText(-1) -eq '')
        }
        Check 'search keyboard routes preserve the draft' ((Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft3)
    }

    [BksStage6]::Post($process.Id, $main, 0x111, 60007)
    Wait-For { [BksStage6]::IsWindowVisible($query) } 'search for tab state'
    Search-Query '' 'Введите текст'
    Search-Query $needle '1 из 3'
    Search-Click $next '2 из 3'
    $created = Keyboard-Case 'create a second tab for independent search' {
        New-ProfileTab
    }
    if ($created) {
        Wait-For { (Read-Text $journal).Contains('PS ') -and -not [BksStage6]::IsWindowVisible($query) } 'new tab has independent closed search' 20000
        Submit "'BKS63_'+'TAB_TWO_Я'" 'BKS63_TAB_TWO_Я'
        [BksStage6]::SetText($process.Id, $inputBox, 'search_tab2_draft_Я')
        [BksStage6]::Post($process.Id, $main, 0x111, 60007)
        Wait-For { [BksStage6]::IsWindowVisible($query) } 'second tab search opened'
        Search-Query 'BKS63_TAB_TWO_Я' '1 из 1'
        [BksStage6]::Post($process.Id, $main, 0x111, 2001)
        Wait-For { [BksStage6]::NativeText($process.Id, $query) -eq $needle -and (Search-Status) -eq '2 из 3' } 'first tab query and current result restored'
        Check 'first tab preserves its query, current search index and draft' ([BksStage6]::NativeText($process.Id, $query) -eq $needle -and (Search-Status) -eq '2 из 3' -and (Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft3)
        [BksStage6]::Post($process.Id, $main, 0x111, 2002)
        Wait-For { (Search-Status) -eq '1 из 1' -and [BksStage6]::NativeText($process.Id, $query) -eq 'BKS63_TAB_TWO_Я' } 'second tab search restored'
        Check 'second tab preserves its draft' ((Read-Text $inputBox).TrimEnd("`r", "`n") -eq 'search_tab2_draft_Я')
        [BksStage6]::Post($process.Id, $main, 0x111, 2001)
        Wait-For { (Search-Status) -eq '2 из 3' } 'return to first search tab'
    } else { $checks.Add('skipped: per-tab search GUI needs successful own-process tab shortcut') }

    # Native graph artifacts demonstrate the overlay and its removal; color
    # restoration is asserted by terminal_view unit tests, not pixel guessing.
    [BksStage6]::Post($process.Id, $main, 0x111, 1009)
    Wait-For { [BksStage6]::IsWindowVisible($graph) } 'graph search surface'
    [BksStage6]::Post($process.Id, $main, 0x111, 60007)
    Wait-For { [BksStage6]::IsWindowVisible($query) } 'graph search bar'
    Save-OwnScreenshot 'search-graph.png'
    [BksStage6]::Post($process.Id, $closeSearch, 0x00F5, 0)
    Wait-For { -not [BksStage6]::IsWindowVisible($query) } 'graph search closed'
    Save-OwnScreenshot 'search-graph-closed.png'
    [BksStage6]::Post($process.Id, $main, 0x111, 1009)
    Wait-For { [BksStage6]::IsWindowVisible($inputBox) } 'command mode for clear refresh'
    Submit "[Console]::Write(([char]27)+'[2J'+([char]27)+'[H'+([char]27)+'[3J'); [Console]::WriteLine('BKS63_'+'CLEAR_READY')" 'BKS63_CLEAR_READY'
    [BksStage6]::Post($process.Id, $main, 0x111, 60007)
    Wait-For { [BksStage6]::IsWindowVisible($query) -and (Search-Status) -eq 'Ничего не найдено' } 'clear removes VT search matches'
    Check 'search refresh follows VT clear while legacy journal retains text' ((Read-Text $journal).Contains($target) -and -not (Read-Text $vt).Contains($target))
}

try {
    $Exe = (Resolve-Path -LiteralPath $Exe).Path
    $start = [Diagnostics.ProcessStartInfo]::new($Exe)
    $start.UseShellExecute = $false
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.EnvironmentVariables['APPDATA'] = $appData
    $process = [Diagnostics.Process]::Start($start)
    Wait-For { [BksStage6]::ProcessWindow($process.Id) -ne [IntPtr]::Zero } 'main window'
    $main = [BksStage6]::ProcessWindow($process.Id)
    Wait-For {
        [BksStage6]::GetDlgItem($main, 1018) -ne [IntPtr]::Zero -and
        [BksStage6]::GetDlgItem($main, 1002) -ne [IntPtr]::Zero -and
        [BksStage6]::GetDlgItem($main, 1003) -ne [IntPtr]::Zero
    } 'native reader, command and execute controls initialized'
    $journal = [BksStage6]::GetDlgItem($main, 1001)
    $inputBox = [BksStage6]::GetDlgItem($main, 1002)
    $execute = [BksStage6]::GetDlgItem($main, 1003)
    $vt = [BksStage6]::GetDlgItem($main, 1018)
    $graph = [BksStage6]::GetDlgItem($main, 1017)
    if (-not ($Iteration2 -or $Iteration3 -or $AnnouncementsOnly)) { [BksStage6]::Activate($process.Id, $main) }
    Wait-For { (Read-Text $journal).Contains('PS ') } 'initial PowerShell prompt' 20000
    Check 'startup keeps native journal and command editor visible' ([BksStage6]::IsWindowVisible($journal) -and [BksStage6]::IsWindowVisible($inputBox) -and -not [BksStage6]::IsWindowVisible($vt))
    Check-Reader $journal 'Журнал терминала'
    if ($AnnouncementsOnly) {
        Invoke-Announcements
    } elseif ($Iteration3) {
        Invoke-Iteration3
    } elseif ($Iteration2) {
        Invoke-Iteration2
    } else {
    $historyMarker = 'BKS6_HISTORY_Ж'
    $longLine = 'BKS6_' + ('x' * 240) + '_WRAP'
    Submit "[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); [Console]::WriteLine('BKS6_'+'BASE_Ж'); [Console]::WriteLine('BKS6_'+('x'*240)+'_WRAP'); [Console]::WriteLine('BKS6_'+'HISTORY_Ж'); 1..80|%{[Console]::WriteLine('row '+`$_)}" 'row 80'
    $draft = 'stage6_draft_Ж中'
    [BksStage6]::SetText($process.Id, $inputBox, $draft)
    Select-Source 60005 $vt $journal
    Check-Reader $vt 'Экран терминала'
    Wait-For { (Read-Text $vt).Contains($historyMarker) -and (Read-Text $vt).Contains($longLine) } 'VT history and joined soft wrap'
    Check 'VT TextPattern exposes scrollback and joins a soft-wrapped output line' $true
    Check 'switching to VT reading preserves the command draft' ((Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft)
    Select-Source 60006 $journal $vt
    Check 'switching back to journal preserves its text and the command draft' ((Read-Text $journal).Contains($historyMarker) -and (Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft)

    foreach ($key in @(0x26, 0x28)) {
        Select-Source 60005 $vt $journal
        [BksStage6]::ControlKey($process.Id, [uint16]$key)
        Wait-For { Require-OwnForeground; [BksStage6]::IsWindowVisible($journal) -and -not [BksStage6]::IsWindowVisible($vt) -and [BksStage6]::Focus($main) -eq $journal } 'block navigation returns to journal'
        Check "Ctrl+$key from VT reading targets journal blocks" ((Text-Pattern $journal).GetSelection()[0].GetText(-1) -eq '')
        Check "Ctrl+$key preserves the command draft" ((Read-Text $inputBox).TrimEnd("`r", "`n") -eq $draft)
    }

    Submit "[Console]::Write(('BKS6_'+'OLD_Ж')+[char]13+('BKS6_'+'NEW_Ж')+[char]13+[char]10)" 'BKS6_NEW_Ж'
    Select-Source 60005 $vt $journal
    Wait-For { (Read-Text $vt).Contains('BKS6_NEW_Ж') -and -not (Read-Text $vt).Contains('BKS6_OLD_Ж') } 'VT carriage-return overwrite'
    Check 'VT document reflects overwritten cells' $true
    Submit "[Console]::Write(([char]27)+'[2J'+([char]27)+'[H'+([char]27)+'[3J'+('BKS6_'+'CLEAR_Ж')+[char]13+[char]10)" 'BKS6_CLEAR_Ж'
    Wait-For { (Read-Text $vt).Contains('BKS6_CLEAR_Ж') -and -not (Read-Text $vt).Contains($historyMarker) } 'VT clear screen and scrollback'
    Check 'VT clear changes the document while the old journal retains history' ((Read-Text $journal).Contains($historyMarker))
    Submit "[Console]::Write(([char]27)+'[?1049h'+('BKS6_'+'ALT_Ж')+[char]13+[char]10)" 'BKS6_ALT_Ж'
    Wait-For { (Read-Text $vt).Contains('BKS6_ALT_Ж') -and -not (Read-Text $vt).Contains('BKS6_CLEAR_Ж') } 'alternate-screen document'
    Check 'VT document follows alternate screen without main-screen history' $true
    Submit "[Console]::Write(([char]27)+'[?1049l'+('BKS6_'+'RETURN_Ж')+[char]13+[char]10)" 'BKS6_RETURN_Ж'
    Wait-For { (Read-Text $vt).Contains('BKS6_CLEAR_Ж') -and -not (Read-Text $vt).Contains('BKS6_ALT_Ж') } 'main screen restored'
    Check 'VT document returns to the main screen' $true
    }
} catch {
    $failure = $_
    if ($process -and $process.HasExited) {
        $observations.Add([ordered]@{ Label='isolated process exited before failure cleanup'; ExitCode=('0x{0:X8}' -f $process.ExitCode) })
    }
    if ($_.Exception.Message.Contains('Foreground lost;')) {
        $checks.Add('error: test process lost foreground; focus/keyboard check unavailable')
        $checks.Add("skipped: remaining iteration $iteration UI checks after external focus loss")
    } else { $checks.Add('failed: ' + $_.Exception.Message) }
    if ($process -and -not $process.HasExited -and $main -ne [IntPtr]::Zero) {
        try {
            $observations.Add([ordered]@{ Observation='failure'; Focus=[BksStage6]::Focus($main).ToInt64(); ForegroundProcess=[BksStage6]::ProcessId([BksStage6]::GetForegroundWindow()); JournalVisible=[BksStage6]::IsWindowVisible($journal); VTVisible=[BksStage6]::IsWindowVisible($vt); Input=Read-Text $inputBox; Journal=Read-Text $journal; VT=Read-Text $vt })
        } catch { $observations.Add([ordered]@{ Observation='failure snapshot unavailable'; Error=$_.Exception.Message }) }
    }
} finally {
    if ($AnnouncementsOnly) {
        $observations.Add([ordered]@{ Label='own announcement notifications'; Events=[BksStage6]::Notifications() })
    }
    try { [BksStage6]::StopNotifications() }
    catch { $checks.Add('error: notification listener cleanup: ' + $_.Exception.Message) }
    if ($process -and -not $process.HasExited) {
        $ownedMain = [BksStage6]::ProcessWindow($process.Id)
        if ($ownedMain -ne [IntPtr]::Zero) { [BksStage6]::Post($process.Id, $ownedMain, 0x10, 0) }
        if (-not $process.WaitForExit(15000)) {
            $process.Kill()
            $checks.Add('failed: isolated test process required forced cleanup')
            if (-not $failure) { $failure = 'Test process close timeout' }
        } else { $checks.Add('passed: isolated test process closes cleanly') }
    }
    [ordered]@{ Scope="Stage 6 iteration $iteration UIA/native state and guarded keys; NVDA speech not tested; clipboard untouched"; Mode=$(if ($AnnouncementsOnly) { 'AnnouncementsOnly' } else { "Iteration$iteration" }); Executable=$Exe; ProcessId=$(if ($process) { $process.Id } else { $null }); AppData=$appData; Checks=$checks.ToArray(); Observations=$observations.ToArray() } |
        ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $runDirectory 'results.json') -Encoding utf8
    $checks | Set-Content -LiteralPath (Join-Path $runDirectory 'results.txt') -Encoding utf8
}
$passed = @($checks | Where-Object { $_.StartsWith('passed:') }).Count
$failed = @($checks | Where-Object { $_.StartsWith('failed:') }).Count
$skipped = @($checks | Where-Object { $_.StartsWith('skipped:') }).Count
$errors = @($checks | Where-Object { $_.StartsWith('error:') }).Count
Write-Output "Stage 6 iteration ${iteration}: passed=$passed failed=$failed skipped=$skipped errors=$errors"
Write-Output "Artifacts: $runDirectory"
if ($failure -or $failed -or $errors) { throw $(if ($failure) { $failure } else { 'Stage 6 checks did not pass' }) }
