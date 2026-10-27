param(
    [string]$Exe = (Join-Path $PSScriptRoot '../target/release/bks-app.exe'),
    [string]$Artifacts = (Join-Path $PSScriptRoot '../target/stage5-check')
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class BksStage5 {
    private delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc proc, IntPtr p);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassNameW(IntPtr h, StringBuilder s, int size);
    public static IntPtr ProcessWindow(uint process) {
        return ProcessClass(process, "BksTermWindow");
    }
    public static IntPtr ProcessDialog(uint process) {
        return ProcessClass(process, "#32770");
    }
    private static IntPtr ProcessClass(uint process, string className) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((h, p) => {
            uint id; GetWindowThreadProcessId(h, out id);
            var name = new StringBuilder(128); GetClassNameW(h, name, name.Capacity);
            if (id == process && name.ToString() == className) { result = h; return false; }
            return true;
        }, IntPtr.Zero);
        return result;
    }
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct GUIINFO {
        public uint Size, Flags; public IntPtr Active, Focus, Capture, Menu, MoveSize, Caret; public RECT CaretRect;
    }
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint id, ref GUIINFO info);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll", EntryPoint="SendMessageW")] public static extern IntPtr Send(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)] public static extern IntPtr Text(IntPtr h, uint m, IntPtr w, string l);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)] public static extern IntPtr Read(IntPtr h, uint m, IntPtr w, StringBuilder l);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string c, string t);
    [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
}
'@

$checks = [Collections.Generic.List[string]]::new()
function Check([string]$Name, [bool]$Condition) {
    if (-not $Condition) { throw "FAILED: $Name" }
    $checks.Add("passed: $Name")
}
function Wait-For([scriptblock]$Condition, [string]$Name, [int]$Timeout = 10000) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.ElapsedMilliseconds -lt $Timeout) {
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 50
    }
    throw "Timeout: $Name"
}
function Read-Text([IntPtr]$Window) {
    $buffer = [Text.StringBuilder]::new(2 * 1024 * 1024)
    [void][BksStage5]::Read($Window, 13, [IntPtr]$buffer.Capacity, $buffer)
    $buffer.ToString()
}
function Key([IntPtr]$Window, [int]$Code) {
    [void][BksStage5]::PostMessageW($Window, 0x100, [IntPtr]$Code, [IntPtr]::Zero)
}
function Type-Command([IntPtr]$Window, [string]$Command) {
    foreach ($unit in $Command.ToCharArray()) {
        [void][BksStage5]::PostMessageW($Window, 0x102, [IntPtr][int]$unit, [IntPtr]::Zero)
    }
    Key $Window 13
}
function Chord([IntPtr]$Window, [byte]$Code, [bool]$Shift = $true) {
    [void][BksStage5]::SetForegroundWindow($Window)
    [BksStage5]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero)
    if ($Shift) { [BksStage5]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero) }
    [BksStage5]::keybd_event($Code, 0, 0, [UIntPtr]::Zero)
    [BksStage5]::keybd_event($Code, 0, 2, [UIntPtr]::Zero)
    if ($Shift) { [BksStage5]::keybd_event(0x10, 0, 2, [UIntPtr]::Zero) }
    [BksStage5]::keybd_event(0x11, 0, 2, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 200
}
function Capture([IntPtr]$Window, [string]$Path) {
    $rect = [BksStage5+RECT]::new()
    [void][BksStage5]::GetWindowRect($Window, [ref]$rect)
    $bitmap = [Drawing.Bitmap]::new($rect.Right - $rect.Left, $rect.Bottom - $rect.Top)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
        $bitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}

$null = New-Item -ItemType Directory -Force -Path $Artifacts
$Exe = (Resolve-Path -LiteralPath $Exe).Path
$startInfo = [Diagnostics.ProcessStartInfo]::new($Exe)
$startInfo.UseShellExecute = $false
$startInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
$startInfo.Environment['APPDATA'] = (Join-Path (Resolve-Path -LiteralPath $Artifacts).Path 'appdata')
$process = [Diagnostics.Process]::Start($startInfo)
try {
    Wait-For { [BksStage5]::ProcessWindow($process.Id) -ne [IntPtr]::Zero } 'main window'
    $window = [BksStage5]::ProcessWindow($process.Id)
    Wait-For { [BksStage5]::GetDlgItem($window, 1017) -ne [IntPtr]::Zero } 'terminal controls'
    $surface = [BksStage5]::GetDlgItem($window, 1017)
    $journal = [BksStage5]::GetDlgItem($window, 1001)
    $input = [BksStage5]::GetDlgItem($window, 1002)
    # Controls can be created before CONTROLS is published; wait for the loop.
    Wait-For { (Read-Text $journal).Contains('PS ') } 'PowerShell prompt' 20000
    [void][BksStage5]::ShowWindow($window, 5)
    [void][BksStage5]::SetForegroundWindow($window)
    Check 'native command editor visible by default' ([BksStage5]::IsWindowVisible($input))
    Check 'native reading journal visible by default' ([BksStage5]::IsWindowVisible($journal))
    Check 'VT surface hidden by default' (-not [BksStage5]::IsWindowVisible($surface))
    [void][BksStage5]::Send($window, 0x111, [IntPtr]1009, [IntPtr]::Zero)
    Check 'VT is an explicit mode' ([BksStage5]::IsWindowVisible($surface))
    Check 'explicit VT mode hides command editor' (-not [BksStage5]::IsWindowVisible($input))
    Type-Command $surface "[Console]::WriteLine('BKS_VT_'+'READY_Ж')"
    Wait-For { (Read-Text $journal).Contains('BKS_VT_READY_Ж') } 'direct Unicode command'
    Check 'direct Enter and Unicode reach PTY' $true
    Key $surface 27
    Start-Sleep -Milliseconds 100
    Check 'Escape keeps the main window alive' ([BksStage5]::IsWindow($window))

    # Native reactivation must restore focus to the visible terminal.
    [void][BksStage5]::ShowWindow($window, 6)
    [void][BksStage5]::ShowWindow($window, 9)
    [void][BksStage5]::Send($window, 7, [IntPtr]::Zero, [IntPtr]::Zero)
    $pidValue = [uint32]0
    $thread = [BksStage5]::GetWindowThreadProcessId($window, [ref]$pidValue)
    $info = [BksStage5+GUIINFO]::new()
    $info.Size = [Runtime.InteropServices.Marshal]::SizeOf($info)
    [void][BksStage5]::GetGUIThreadInfo($thread, [ref]$info)
    Check 'reactivation restores visible VT focus' ($info.Focus -eq $surface)

    Key $surface 112
    Wait-For { [BksStage5]::FindWindowW('BksTermHelpWindow', 'Справка — BKS Term') -ne [IntPtr]::Zero } 'F1 help'
    $help = [BksStage5]::FindWindowW('BksTermHelpWindow', 'Справка — BKS Term')
    $helpText = [BksStage5]::GetDlgItem($help, 1)
    Check 'help contains keyboard and mode instructions' ((Read-Text $helpText).Contains('Ctrl+Shift+Space'))
    Key $helpText 27
    Wait-For { -not [BksStage5]::IsWindow($help) } 'Escape closes help'
    [void][BksStage5]::GetGUIThreadInfo($thread, [ref]$info)
    Check 'help returns previous terminal focus' ($info.Focus -eq $surface)

    Chord $window 0x20
    Check 'reading shortcut reveals native journal' ([BksStage5]::IsWindowVisible($journal))
    Check 'reading shortcut hides VT' (-not [BksStage5]::IsWindowVisible($surface))
    Chord $window 0x20
    Check 'reading shortcut returns to VT' ([BksStage5]::IsWindowVisible($surface))

    [void][BksStage5]::Send($window, 0x111, [IntPtr]1009, [IntPtr]::Zero)
    Check 'command fallback reveals editor' ([BksStage5]::IsWindowVisible($input))
    [void][BksStage5]::Text($input, 12, [IntPtr]::Zero, "[Console]::WriteLine('BKS_COMMAND_'+'OK')")
    Key $input 13
    Wait-For { (Read-Text $journal).Contains('BKS_COMMAND_OK') } 'command fallback execution'
    Check 'command fallback remains functional' $true
    [void][BksStage5]::Send($window, 0x111, [IntPtr]1009, [IntPtr]::Zero)

    Chord $window 0x54
    Wait-For { [void][BksStage5]::GetGUIThreadInfo($thread, [ref]$info); $info.Menu -ne [IntPtr]::Zero } 'new-tab profile picker'
    Check 'new tab waits for explicit profile selection' ([BksStage5]::GetDlgItem($window, 2002) -eq [IntPtr]::Zero)
    Key $window 0x24
    Key $window 13
    Wait-For { [BksStage5]::GetDlgItem($window, 2002) -ne [IntPtr]::Zero } 'second tab'
    Wait-For { (Read-Text $journal).Contains('PS ') -and -not (Read-Text $journal).Contains('BKS_VT_READY_Ж') } 'second tab prompt' 20000
    Check 'new tab starts with native controls' ([BksStage5]::IsWindowVisible($input) -and [BksStage5]::IsWindowVisible($journal))
    [void][BksStage5]::Send($window, 0x111, [IntPtr]1009, [IntPtr]::Zero)
    Type-Command $surface "[Console]::WriteLine('BKS_SECOND_'+'TAB')"
    Wait-For { (Read-Text $journal).Contains('BKS_SECOND_TAB') } 'second tab output'
    Check 'second tab has independent output' (-not (Read-Text $journal).Contains('BKS_VT_READY_Ж'))
    Chord $window 0x09 $false
    Wait-For { (Read-Text $journal).Contains('BKS_VT_READY_Ж') } 'Ctrl+Tab returns to first'
    Check 'first tab preserves output' (-not (Read-Text $journal).Contains('BKS_SECOND_TAB'))

    Type-Command $surface "[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); `$e=[char]27; [Console]::Write(`"`$e[31mRED `$e[38;5;214m256 `$e[38;2;85;170;255mTRUE `$e[0m Ж `$([char]0x4e2d) e`$([char]0x301)`")"
    Wait-For { (Read-Text $journal).Contains("中 e$([char]0x301)") } 'wide and combining Unicode output'
    Check 'UTF-8 wide and combining output reaches journal' $true
    Start-Sleep -Milliseconds 200
    Capture $window (Join-Path $Artifacts 'terminal.png')
    Check 'terminal screenshot captured' (Test-Path -LiteralPath (Join-Path $Artifacts 'terminal.png'))

    [void][BksStage5]::Send($window, 0x111, [IntPtr]60003, [IntPtr]::Zero)
    [void][BksStage5]::PostMessageW($window, 0x111, [IntPtr]3001, [IntPtr]::Zero)
    Wait-For { [BksStage5]::ProcessDialog($process.Id) -ne [IntPtr]::Zero } 'close confirmation'
    [void][BksStage5]::Send([BksStage5]::ProcessDialog($process.Id), 0x111, [IntPtr]7, [IntPtr]::Zero)
    Check 'cancel close preserves active tab and reading mode' (
        [BksStage5]::GetDlgItem($window, 2001) -ne [IntPtr]::Zero -and [BksStage5]::IsWindowVisible($journal))
    [void][BksStage5]::PostMessageW($window, 0x111, [IntPtr]3001, [IntPtr]::Zero)
    Wait-For { [BksStage5]::ProcessDialog($process.Id) -ne [IntPtr]::Zero } 'second close confirmation'
    [void][BksStage5]::Send([BksStage5]::ProcessDialog($process.Id), 0x111, [IntPtr]6, [IntPtr]::Zero)
    Wait-For { [BksStage5]::GetDlgItem($window, 2001) -eq [IntPtr]::Zero } 'confirmed tab close'
    Check 'confirmed close selects remaining tab' ((Read-Text $journal).Contains('BKS_SECOND_TAB'))
    Check 'closing reading tab restores remaining VT mode' ([BksStage5]::IsWindowVisible($surface))
    [void][BksStage5]::GetGUIThreadInfo($thread, [ref]$info)
    Check 'closing tab restores visible input focus' ($info.Focus -eq $surface)
} catch {
    if ($window -and [BksStage5]::IsWindow($window)) {
        Capture $window (Join-Path $Artifacts 'failure.png')
    }
    throw
} finally {
    if (-not $process.HasExited) {
        [void][BksStage5]::PostMessageW([BksStage5]::ProcessWindow($process.Id), 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
        if (-not $process.WaitForExit(15000)) {
            $process.Kill()
            throw 'FAILED: main window did not close within 15 seconds'
        }
    }
    $checks | Set-Content -LiteralPath (Join-Path $Artifacts 'results.txt') -Encoding utf8
}
Check 'main window closes cleanly' $true
$checks | Set-Content -LiteralPath (Join-Path $Artifacts 'results.txt') -Encoding utf8
$checks
