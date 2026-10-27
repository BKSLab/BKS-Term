[CmdletBinding()]
param(
    [string]$Exe,
    [string]$Artifacts,
    [ValidateSet('Settings','Sessions','Runtime','Agent','All')][string]$Mode = 'All',
    [switch]$PrepareOnly
)

# Owned PIDs/HWNDs only; private APPDATA/PATH/cwd; no global keyboard/clipboard,
# registry/product integration, actual Codex, credentials, model or network calls.
$ErrorActionPreference = 'Stop'
if (-not $Exe) { $Exe = Join-Path $PSScriptRoot '../target/release/bks-app.exe' }
if (-not $Artifacts) { $Artifacts = Join-Path $PSScriptRoot '../target/stage1011-check' }
if ($PSVersionTable.PSEdition -eq 'Core' -or [Threading.Thread]::CurrentThread.GetApartmentState() -ne 'MTA') {
    $arguments = @('-NoProfile','-Mta','-ExecutionPolicy','Bypass','-File',$PSCommandPath,'-Exe',$Exe,'-Artifacts',$Artifacts,'-Mode',$Mode)
    if ($PrepareOnly) { $arguments += '-PrepareOnly' }
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
$failure = $null

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, UIAutomationClientsideProviders
$references = @([Windows.Automation.AutomationElement].Assembly.Location,
    [Windows.Automation.AutomationPattern].Assembly.Location,
    [UIAutomationClientsideProviders.UIAutomationClientSideProviders].Assembly.Location)
# Reuse the own-PID helper as source; never evaluate an older QA scenario.
$shared = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'check-stage7.ps1'), [Text.Encoding]::UTF8)
$nativeSource = [regex]::Match($shared, "(?s)using System;.*?(?=\r?\n'@)").Value
if (-not $nativeSource.Contains('public static class BksStage7')) { throw 'Owned HWND helper unavailable.' }
$extension = @'
public static class BksStage1011 {
    [DllImport("user32.dll")] private static extern bool PostMessageW(IntPtr window,uint message,IntPtr wparam,IntPtr lparam);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window,out uint process);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr window,uint flags);
    public static uint ProcessId(IntPtr window) { uint process; GetWindowThreadProcessId(window,out process); return process; }
    [StructLayout(LayoutKind.Sequential)] private struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] private struct Gui {
        public uint Size,Flags; public IntPtr Active,Focus,Capture,Menu,MoveSize,Caret; public Rect CaretRect;
    }
    [DllImport("user32.dll")] private static extern bool GetGUIThreadInfo(uint thread,ref Gui info);
    [DllImport("user32.dll",EntryPoint="SendMessageTimeoutW",SetLastError=true)]
    private static extern IntPtr Timeout(IntPtr window,uint message,IntPtr wparam,IntPtr lparam,uint flags,uint timeout,out UIntPtr result);
    public static IntPtr Focus(uint process,IntPtr window) {
        BksStage7.Own(process,window); uint actual; uint thread=GetWindowThreadProcessId(window,out actual);
        var info=new Gui(); info.Size=(uint)Marshal.SizeOf(typeof(Gui));
        if(!GetGUIThreadInfo(thread,ref info))throw new InvalidOperationException("Owned focus query failed");
        return info.Focus;
    }
    public static void Post(uint process,IntPtr window,uint message,int value) {
        BksStage7.Own(process,window);
        if(!PostMessageW(window,message,new IntPtr(value),IntPtr.Zero))throw new InvalidOperationException("Owned post failed");
    }
    public static void Key(uint process,IntPtr window,int key) {
        BksStage7.Own(process,window);
        if(Focus(process,window)!=window)throw new InvalidOperationException("Owned key target lacks focus");
        Post(process,window,0x100,key); Post(process,window,0x101,key);
    }
    public static int Selected(uint process,IntPtr list) {
        BksStage7.Own(process,list); UIntPtr result;
        if(Timeout(list,0x188,IntPtr.Zero,IntPtr.Zero,2,1500,out result)==IntPtr.Zero)throw new TimeoutException("Owned list read timed out");
        return unchecked((int)result.ToUInt64());
    }
    [StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)] private struct Entry {
        public uint Size,Usage,Pid; public IntPtr Heap; public uint Module,Threads,Parent; public int Priority; public uint Flags;
        [MarshalAs(UnmanagedType.ByValTStr,SizeConst=260)] public string File;
    }
    [DllImport("kernel32.dll",SetLastError=true)] private static extern IntPtr CreateToolhelp32Snapshot(uint flags,uint process);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] private static extern bool Process32FirstW(IntPtr snapshot,ref Entry entry);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] private static extern bool Process32NextW(IntPtr snapshot,ref Entry entry);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
    public static int[] Children(uint parent) {
        var result=new List<int>(); IntPtr snapshot=CreateToolhelp32Snapshot(2,0);
        if(snapshot==new IntPtr(-1))throw new InvalidOperationException("Child process snapshot failed");
        try { var entry=new Entry(); entry.Size=(uint)Marshal.SizeOf(typeof(Entry));
            if(Process32FirstW(snapshot,ref entry))do {if(entry.Parent==parent)result.Add((int)entry.Pid);}while(Process32NextW(snapshot,ref entry));
        } finally {CloseHandle(snapshot);} return result.ToArray();
    }
}
'@

function Check([string]$Name, [bool]$Condition) {
    $status = if ($Condition) { 'passed' } else { 'failed' }
    $checks.Add([ordered]@{Status=$status;Name=$Name;Case=$(if ($script:Case) {$script:Case.Name} else {'prepare'})})
    if (-not $Condition) { throw "FAILED: $Name" }
}
function Wait-For([scriptblock]$Condition, [string]$Name, [int]$Timeout=12000) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.ElapsedMilliseconds -lt $Timeout) {
        if (& $Condition) { return }
        if ($script:Case -and $script:Case.Process.HasExited) { throw "Owned app exited: $Name" }
        Start-Sleep -Milliseconds 40
    }
    throw "Timeout: $Name"
}
function Element([IntPtr]$Window) {
    [BksStage7]::Own($script:Case.Process.Id, $Window)
    return [Windows.Automation.AutomationElement]::FromHandle($Window)
}
function Focus-Own([IntPtr]$Window) {
    if ([BksStage7]::ClassName($Window) -ne 'BksTerminalSurface' -and
        [BksStage1011]::Focus($script:Case.Process.Id,$Window) -ne $Window) {
        (Element $Window).SetFocus()
    }
    Wait-For { [BksStage1011]::Focus($script:Case.Process.Id, $Window) -eq $Window } 'owned native focus'
}
function Invoke-Own([IntPtr]$Window) {
    $pattern = $null
    if (-not (Element $Window).TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern,[ref]$pattern)) { throw 'Owned action lacks InvokePattern.' }
    $pattern.Invoke()
}
function Own-Text([IntPtr]$Window) {
    return [BksStage7]::Text($script:Case.Process.Id, $Window)
}
function Field([IntPtr]$Parent, [int]$Id) {
    [BksStage7]::Own($script:Case.Process.Id,$Parent)
    $window = [BksStage7]::GetDlgItem($Parent,$Id)
    [BksStage7]::Own($script:Case.Process.Id,$window)
    return $window
}
function Open-Dialog([int]$Command, [string]$Class, [int]$LastField) {
    [BksStage1011]::Post($script:Case.Process.Id,$script:Case.Main,0x111,$Command)
    Wait-For { $dialog=[BksStage7]::VisibleClass($script:Case.Process.Id,$Class); $dialog -ne [IntPtr]::Zero -and [BksStage7]::GetDlgItem($dialog,$LastField) -ne [IntPtr]::Zero } "owned $Class initialized"
    return [BksStage7]::VisibleClass($script:Case.Process.Id,$Class)
}
function Shared-Text([string]$Path) {
    $stream = [IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
    $reader = New-Object IO.StreamReader($stream,[Text.Encoding]::UTF8)
    try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
}
function Fixture-Markers([string]$Kind) {
    return @(Get-ChildItem -LiteralPath $script:Case.Task -Filter ($Kind+'-*.json') -File | ForEach-Object { Read-BksJson $_.FullName })
}
function Register-Fixtures {
    foreach ($kind in @('session','server')) {
        foreach ($marker in Fixture-Markers $kind) {
            if (@($script:Case.Mocks | Where-Object { $_.Id -eq $marker.Pid }).Count) { continue }
            try {
                $mock = [Diagnostics.Process]::GetProcessById($marker.Pid)
                $path = $mock.MainModule.FileName
                if ($path -notin @($script:Case.Mock,$script:Case.LocalMock,$script:Case.Codex) -or $marker.Executable -ine $path) { throw 'Fixture PID points outside its private executable directory.' }
                $script:Case.Mocks.Add($mock)
            } catch [ArgumentException] { }
        }
    }
}
function Register-Apps {
    foreach ($childId in [BksStage1011]::Children($script:Case.Process.Id)) {
        if (@($script:Case.Apps | Where-Object { $_.Id -eq $childId }).Count) { continue }
        try {
            $child=[Diagnostics.Process]::GetProcessById($childId)
            if ($child.MainModule.FileName -ieq $Exe) { $script:Case.Apps.Add($child) }
        } catch [ArgumentException] { }
        catch [ComponentModel.Win32Exception] { }
        catch [InvalidOperationException] { }
    }
}
function Foreground-State {
    $foreground=[BksStage1011]::GetForegroundWindow()
    $root=[BksStage1011]::GetAncestor($foreground,2)
    return @{Hwnd=$foreground.ToInt64();Root=$root.ToInt64();Pid=[BksStage1011]::ProcessId($foreground);Class=[BksStage7]::ClassName($foreground);RootClass=[BksStage7]::ClassName($root);ExpectedMain=$script:Case.Main.ToInt64();ExpectedPid=$script:Case.Process.Id}
}
function Window-ActionState([string]$Phase) {
    $children=@(foreach ($childId in [BksStage1011]::Children($script:Case.Process.Id)) {
        try {
            $child=[Diagnostics.Process]::GetProcessById($childId)
            $path=$child.MainModule.FileName
            @{Pid=$childId;Path=$path;MatchesApplication=($path -ieq $Exe);MainHwnd=([BksStage7]::MainWindow($childId)).ToInt64()}
        } catch { @{Pid=$childId;ReadError=$_.Exception.Message} }
    })
    $apps=@(foreach ($app in $script:Case.Apps) {
        $exited=$app.HasExited
        @{Pid=$app.Id;Exited=$exited;ExitCode=$(if ($exited) {$app.ExitCode} else {$null})}
    })
    return @{Case=$script:Case.Name;Phase=$Phase;Foreground=(Foreground-State);PopupHwnd=([BksStage7]::VisibleClass($script:Case.Process.Id,'#32768')).ToInt64();KnownApplications=$apps;DirectChildren=$children;SessionMarkers=@(Fixture-Markers 'session')}
}
function Require-ChooserForeground {
    $state=Foreground-State
    $popup=[BksStage7]::VisibleClass($script:Case.Process.Id,'#32768')
    if ($state.Pid -ne $script:Case.Process.Id -or
        ($state.Root -ne $script:Case.Main.ToInt64() -and ($popup -eq [IntPtr]::Zero -or $state.Root -ne $popup.ToInt64()))) {
        throw ('Environment foreground interference before owned profile choice: pid='+$state.Pid+' class='+$state.Class+' root='+$state.Root+' expectedMain='+$state.ExpectedMain)
    }
}
function Start-Case([string]$Name, [string]$LastTab='keep_window', [ValidateSet('stage1011-local','cli-codex')][string]$LaunchProfile='stage1011-local') {
    $directory = Join-Path $runDirectory $Name
    $appData = Join-Path $directory 'appdata'
    $task = Join-Path $appData 'BKSStage1011'
    $bin = Join-Path $directory 'bin'
    $configuration = Join-Path $appData 'BKS Term'
    $cwd = Join-Path $directory ('working folder ' + [char]0x0416)
    foreach ($path in @($task,$bin,$configuration,$cwd)) { $null=[IO.Directory]::CreateDirectory($path) }
    $mock = Join-Path $bin 'session-fixture.exe'
    $codex = Join-Path $bin 'codex.exe'
    Copy-Item -LiteralPath $templateExe -Destination $mock
    Copy-Item -LiteralPath $templateExe -Destination $codex
    Write-BksJson (Join-Path $configuration 'profiles.json') @{
        schema_version=1; default_profile_id='stage1011-local'; profiles=@(@{
            id='stage1011-local';name='Stage1011 local fixture';kind='local';executable=$mock;args=@('--session');working_directory=$null
        })
    }
    Write-BksJson (Join-Path $configuration 'settings.json') @{
        speech_mode='important';last_tab=$LastTab;remember_workspace=$false;shortcuts=@{}
    }
    $profilesHash=Get-BksHash (Join-Path $configuration 'profiles.json')
    $expectedExe=if ($LaunchProfile -eq 'cli-codex') { $codex } else { $mock }
    $start = New-Object Diagnostics.ProcessStartInfo($Exe)
    $start.UseShellExecute=$false
    $start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.WorkingDirectory=$directory
    $start.Arguments='--profile ' + $LaunchProfile + ' --cwd "' + $cwd + '"'
    $start.EnvironmentVariables['APPDATA']=$appData
    $start.EnvironmentVariables['LOCALAPPDATA']=(Join-Path $directory 'localappdata')
    $start.EnvironmentVariables['USERPROFILE']=(Join-Path $directory 'private-user')
    $start.EnvironmentVariables['CODEX_HOME']=(Join-Path $directory 'private-codex-home')
    $start.EnvironmentVariables['PATH']=$bin + ';' + $env:SystemRoot + '\System32;' + $env:SystemRoot + '\System32\WindowsPowerShell\v1.0'
    $process=[Diagnostics.Process]::Start($start)
    $script:Case=@{Name=$Name;Process=$process;Main=[IntPtr]::Zero;Directory=$directory;AppData=$appData;Task=$task;Cwd=$cwd;Mock=$expectedExe;LocalMock=$mock;Codex=$codex;Configuration=$configuration;ProfilesHash=$profilesHash;LaunchProfile=$LaunchProfile;Mocks=(New-Object 'System.Collections.Generic.List[object]');Apps=(New-Object 'System.Collections.Generic.List[object]')}
    $script:Case.Apps.Add($process)
    Wait-For { [BksStage7]::MainWindow($process.Id) -ne [IntPtr]::Zero } 'owned main window'
    $script:Case.Main=[BksStage7]::MainWindow($process.Id)
    Wait-For { [BksStage7]::GetDlgItem($script:Case.Main,1002) -ne [IntPtr]::Zero -and @(Fixture-Markers 'session').Count -eq 1 } 'owned mock session startup'
    Register-Fixtures
    $marker=@(Fixture-Markers 'session')[0]
    $observations.Add(@{Case=$Name;ApplicationPid=$process.Id;AppData=$appData;ApplicationCwd=$directory;WorkingDirectory=$cwd;LaunchProfile=$LaunchProfile;Fixture=$marker;SavedProfiles=(Read-BksJson (Join-Path $configuration 'profiles.json'))})
    Check 'CLI --profile/--cwd selects only the requested mock and preserves Unicode/space cwd' ($marker.Cwd -ceq $cwd -and $marker.Executable -ieq $expectedExe)
}
function Exit-Sessions {
    Register-Fixtures
    $sessions=@(Fixture-Markers 'session')
    foreach ($marker in $sessions) { [IO.File]::WriteAllText((Join-Path $script:Case.Task ('exit-'+$marker.Pid)),'exit') }
    foreach ($mock in $script:Case.Mocks | Where-Object { -not $_.HasExited -and $_.Id -in @($sessions | ForEach-Object { $_.Pid }) }) { $null=$mock.WaitForExit(5000) }
}
function Stop-Case {
    if (-not $script:Case) { return }
    Register-Apps
    Register-Fixtures
    Exit-Sessions
    foreach ($app in @($script:Case.Apps.ToArray()) | Sort-Object Id -Descending) {
        if ($app.HasExited) { continue }
        if ($app.MainModule.FileName -ine $Exe) { throw 'Refusing cleanup of an unknown application executable.' }
        $window=[BksStage7]::MainWindow($app.Id)
        # Same-EXE adapter workers have no main HWND and belong to their host
        # job. Close/wait GUI hosts first; keep every worker PID for phase two.
        if ($window -eq [IntPtr]::Zero) { continue }
        foreach ($class in @('BksTermAppServerClient','BksTermPreferences','BksTermSshProfileEditor','BksTermHelpWindow')) {
            $dialog=[BksStage7]::VisibleClass($app.Id,$class)
            if ($dialog -ne [IntPtr]::Zero) { [BksStage1011]::Post($app.Id,$dialog,0x10,0) }
        }
        if ($window -ne [IntPtr]::Zero) {
            [BksStage1011]::Post($app.Id,$window,0x1f,0)
            [BksStage1011]::Post($app.Id,$window,0x10,0)
        }
        if (-not $app.WaitForExit(10000)) { $app.Kill();$checks.Add(@{Status='failed';Name='Owned application required forced cleanup';Case=$script:Case.Name}) }
    }
    foreach ($app in @($script:Case.Apps.ToArray()) | Sort-Object Id -Descending) {
        if ($app.HasExited) { continue }
        if ($app.MainModule.FileName -ine $Exe) { throw 'Refusing cleanup of an unknown application executable.' }
        # A tracked GUI child can finish startup after the first snapshot.
        # Give it its normal close path; windowless peers get a bounded wait
        # only after all known GUI hosts have already closed.
        $window=[BksStage7]::MainWindow($app.Id)
        if ($window -ne [IntPtr]::Zero) {
            [BksStage1011]::Post($app.Id,$window,0x1f,0)
            [BksStage1011]::Post($app.Id,$window,0x10,0)
        }
        if (-not $app.WaitForExit(10000)) { $app.Kill();$checks.Add(@{Status='failed';Name='Owned application required forced cleanup';Case=$script:Case.Name}) }
    }
    # A deferred CLI startup may have produced a marker during app cleanup.
    # Register and retire that known peer as well, even on an earlier failure.
    Exit-Sessions
    foreach ($mock in $script:Case.Mocks) {
        if ($mock.WaitForExit(4000)) { continue }
        if ($mock.MainModule.FileName -notin @($script:Case.Mock,$script:Case.LocalMock,$script:Case.Codex)) { throw 'Refusing cleanup of an unknown executable.' }
        $mock.Kill();$checks.Add(@{Status='failed';Name='Owned fixture required forced cleanup';Case=$script:Case.Name})
    }
    Check 'owned application and known fixtures closed without forced cleanup' (@($script:Case.Apps | Where-Object { -not $_.HasExited }).Count -eq 0 -and @($script:Case.Mocks | Where-Object { -not $_.HasExited }).Count -eq 0)
    $script:Case=$null
}

function Check-Settings {
    Start-Case 'settings'
    $settingsPath=Join-Path $script:Case.Configuration 'settings.json'
    $before=Get-BksHash $settingsPath
    $dialog=Open-Dialog 60018 'BksTermPreferences' 4608
    foreach ($id in 4600..4604) {
        $field=Field $dialog $id
        Check "Preferences field $id has a native role and accessible name" ([BksStage7]::ClassName($field) -in @('Edit','ComboBox') -and (Element $field).Current.Name.Length -gt 0)
    }
    $hint=Field $dialog 4609
    Check 'Preferences hint is named, selectable and read-only' ([BksStage7]::ReadOnly($script:Case.Process.Id,$hint) -and (Element $hint).Current.Name.Length -gt 0)
    [BksStage7]::ComboChoice($script:Case.Process.Id,$dialog,(Field $dialog 4601),4601,1)
    Invoke-Own (Field $dialog 4608)
    Wait-For { [BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermPreferences') -eq [IntPtr]::Zero } 'Preferences cancelled'
    Check 'cancel leaves the settings file unchanged' ((Get-BksHash $settingsPath) -eq $before)
    $dialog=Open-Dialog 60018 'BksTermPreferences' 4608
    [BksStage7]::ComboChoice($script:Case.Process.Id,$dialog,(Field $dialog 4603),4603,16)
    Wait-For { (Own-Text (Field $dialog 4604)) -eq 'F1' } 'Help action binding loaded'
    [BksStage7]::SetText($script:Case.Process.Id,(Field $dialog 4604),'F8')
    # Post the click because validation opens a modal MessageBox on this thread.
    [BksStage7]::Post($script:Case.Process.Id,(Field $dialog 4605),0x00f5)
    $validationAction=@{Pattern=$null}
    Wait-For {
        $validation=[BksStage7]::VisibleClass($script:Case.Process.Id,'#32770')
        if ($validation -eq [IntPtr]::Zero) { return $false }
        $condition=New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Button)
        $button=(Element $validation).FindFirst([Windows.Automation.TreeScope]::Descendants,$condition)
        $pattern=$null
        if ($button -and $button.Current.ProcessId -eq $script:Case.Process.Id -and $button.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern,[ref]$pattern)) {
            $validationAction.Pattern=$pattern
            return $true
        }
        return $false
    } 'mandatory F1 validation with initialized native action'
    Check 'removing mandatory F1 is rejected before persistence' ((Get-BksHash $settingsPath) -eq $before)
    $validationAction.Pattern.Invoke()
    [BksStage7]::SetText($script:Case.Process.Id,(Field $dialog 4604),'F1; F8')
    Invoke-Own (Field $dialog 4605)
    [BksStage7]::ComboChoice($script:Case.Process.Id,$dialog,(Field $dialog 4600),4600,0)
    Invoke-Own (Field $dialog 4607)
    Wait-For { [BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermPreferences') -eq [IntPtr]::Zero -and (Read-BksJson $settingsPath).speech_mode -eq 'manual' } 'Preferences saved'
    $saved=Read-BksJson $settingsPath
    Check 'save persists the alias while preserving mandatory F1' (@($saved.shortcuts.help).Count -eq 2 -and $saved.shortcuts.help -contains 'F1' -and $saved.shortcuts.help -contains 'F8')
    foreach ($key in @(0x77,0x70)) {
        $focusTarget=Field $script:Case.Main 1002
        if ($key -eq 0x70) {
            [BksStage1011]::Post($script:Case.Process.Id,$script:Case.Main,0x111,1009)
            [BksStage1011]::Post($script:Case.Process.Id,$script:Case.Main,0x111,60004)
            $focusTarget=Field $script:Case.Main 1011
            Wait-For { [BksStage7]::IsWindowVisible($focusTarget) -and [BksStage7]::IsWindowVisible((Field $script:Case.Main 1017)) } 'native direct input and graphical terminal become visible'
        }
        Focus-Own $focusTarget
        [BksStage1011]::Key($script:Case.Process.Id,$focusTarget,$key)
        Wait-For { [BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermHelpWindow') -ne [IntPtr]::Zero } 'Help key opens owned help'
        $help=[BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermHelpWindow')
        Check "Help key $key opens text with the actual configured alias" ((Own-Text (Field $help 1)).Contains('F1; F8'))
        Invoke-Own (Field $help 2)
        Wait-For { [BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermHelpWindow') -eq [IntPtr]::Zero } 'owned Help closes'
        if ($key -eq 0x70) {
            Focus-Own $focusTarget
            [BksStage1011]::Key($script:Case.Process.Id,$focusTarget,9)
            Wait-For { [BksStage1011]::Focus($script:Case.Process.Id,$script:Case.Main) -ne $focusTarget } 'F1 releases terminal capture'
            Check 'mandatory F1 returns Tab navigation after terminal capture' $true
        }
    }
    $ssh=Open-Dialog 60009 'BksTermSshProfileEditor' 4107
    $sshHint=Field $ssh 4206
    Check 'SSH hint is a named native read-only edit control' ([BksStage7]::ClassName($sshHint) -ieq 'Edit' -and [BksStage7]::ReadOnly($script:Case.Process.Id,$sshHint) -and (Element $sshHint).Current.Name.Length -gt 0)
    Focus-Own (Field $ssh 4105)
    [BksStage1011]::Key($script:Case.Process.Id,(Field $ssh 4105),9)
    Wait-For { [BksStage1011]::Focus($script:Case.Process.Id,$ssh) -eq $sshHint } 'SSH hint is in the Tab order'
    Check 'Tab reaches the SSH hint without saving or connecting' (@(Fixture-Markers 'session').Count -eq 1)
    Invoke-Own (Field $ssh 4107)
    Stop-Case
}

function Check-Sessions {
    Start-Case 'sessions'
    $child=New-OwnedWindow 'Stage1011 local fixture'
    Wait-For { @(Fixture-Markers 'session').Count -eq 2 } 'new-window mock session'
    Register-Fixtures
    Check 'new window uses a separate owned PID and inherits the selected cwd' ($child.Id -ne $script:Case.Process.Id -and @((Fixture-Markers 'session') | Where-Object { $_.Cwd -cne $script:Case.Cwd }).Count -eq 0)
    Exit-Sessions
    Wait-For { (Own-Text (Field $script:Case.Main 1005)).Contains('0') } 'primary mock session completed'
    [BksStage1011]::Post($script:Case.Process.Id,$script:Case.Main,0x111,3001)
    Wait-For { [BksStage7]::GetDlgItem($script:Case.Main,3001) -eq [IntPtr]::Zero } 'last tab closed'
    Check 'keep-window policy leaves the empty window responsive' (-not $script:Case.Process.HasExited -and [BksStage7]::Responsive($script:Case.Process.Id,$script:Case.Main,1500,2))
    Stop-Case
    Start-Case 'last-tab-close' 'close_window'
    Exit-Sessions
    Wait-For { (Own-Text (Field $script:Case.Main 1005)).Contains('0') } 'last-tab-close mock completed'
    [BksStage1011]::Post($script:Case.Process.Id,$script:Case.Main,0x111,3001)
    $closed=$script:Case.Process.WaitForExit(10000)
    Check 'close-window policy closes the application after its last exited tab' $closed
    Stop-Case
    Check-RuntimeCliSessions
}

function New-OwnedWindow([string]$ProfileName) {
    $knownApps=@($script:Case.Apps | ForEach-Object { $_.Id })
    $observations.Add((Window-ActionState 'before new-window menu'))
    Require-ChooserForeground
    [BksStage1011]::Post($script:Case.Process.Id,$script:Case.Main,0x111,60016)
    $choice=@{Pattern=$null;Name='';Menu=[IntPtr]::Zero}
    try {
    Wait-For {
        $menu=[BksStage7]::VisibleClass($script:Case.Process.Id,'#32768')
        if ($menu -eq [IntPtr]::Zero) { return $false }
        $item=(Element $menu).FindFirst([Windows.Automation.TreeScope]::Descendants,(New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty,$ProfileName)))
        $pattern=$null
        if ($item -and $item.Current.ProcessId -eq $script:Case.Process.Id -and $item.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern,[ref]$pattern)) {
            $choice.Pattern=$pattern; $choice.Name=$item.Current.Name; $choice.Menu=$menu
            return $true
        }
        return $false
    } ('new-window profile chooser offers '+$ProfileName)
    Check ('new window first displays the explicit '+$ProfileName+' choice') ($choice.Name -ceq $ProfileName)
    $observations.Add((Window-ActionState 'before profile Invoke'))
    Require-ChooserForeground
    if ([BksStage7]::VisibleClass($script:Case.Process.Id,'#32768') -ne $choice.Menu) { throw 'Environment: the owned profile popup disappeared before Invoke.' }
    $choice.Pattern.Invoke()
    $observations.Add((Window-ActionState 'after profile Invoke'))
    $childFound=@{Process=$null}
    Wait-For {
        Register-Apps
        foreach ($app in $script:Case.Apps) {
            if ($app.Id -notin $knownApps -and -not $app.HasExited -and [BksStage7]::MainWindow($app.Id) -ne [IntPtr]::Zero) {
                $childFound.Process=$app
                return $true
            }
        }
        return $false
    } 'new owned application PID'
    } finally {
        try { $observations.Add((Window-ActionState 'new-window completion or failure')) }
        catch { $observations.Add(@{Case=$script:Case.Name;Phase='new-window diagnostic unavailable';Reason=$_.Exception.Message}) }
    }
    $observations.Add(@{Case=$script:Case.Name;ProfileChoice=$choice.Name;ParentApplicationPid=$script:Case.Process.Id;NewApplicationPid=$childFound.Process.Id;RequestedCwd=$script:Case.Cwd})
    return $childFound.Process
}

function Check-RuntimeProfilesNotSaved([string]$Phase) {
    $path=Join-Path $script:Case.Configuration 'profiles.json'
    $saved=Read-BksJson $path
    $ids=@($saved.profiles | ForEach-Object { $_.id })
    $observations.Add(@{Case=$script:Case.Name;Phase=$Phase;SavedProfiles=$saved;InitialProfilesSha256=$script:Case.ProfilesHash;CurrentProfilesSha256=(Get-BksHash $path)})
    Check ($Phase+': runtime CLI remains absent from the saved local-only profiles') ($ids.Count -eq 1 -and $ids[0] -ceq 'stage1011-local' -and $saved.default_profile_id -ceq 'stage1011-local')
}

function Check-RuntimeCliSessions {
    Start-Case 'runtime-cli-startup' -LaunchProfile 'cli-codex'
    Check-RuntimeProfilesNotSaved 'after cold CLI startup'
    # Discovery uses bounded --version probes; the actual zero-argument CLI
    # session starts only after the asynchronous catalog resolves its ID.
    Start-Sleep -Milliseconds 500
    $startup=@(Fixture-Markers 'session')
    $observations.Add(@{Case=$script:Case.Name;Phase='settled cold runtime startup';Sessions=$startup;ApplicationPid=$script:Case.Process.Id;RequestedCwd=$script:Case.Cwd})
    Check 'cold runtime CLI startup creates exactly one zero-argument private CLI peer' ($startup.Count -eq 1 -and $startup[0].Executable -ieq $script:Case.Codex -and $startup[0].Cwd -ceq $script:Case.Cwd -and @($startup[0].Arguments).Count -eq 0 -and $startup[0].Pid -in [BksStage1011]::Children($script:Case.Process.Id))
    $startupPid=$startup[0].Pid
    $child=New-OwnedWindow 'Codex CLI'
    Wait-For { @(Fixture-Markers 'session').Count -eq 2 } 'new window resolves its runtime-only CLI profile after cold discovery'
    Register-Fixtures
    Start-Sleep -Milliseconds 500
    $sessions=@(Fixture-Markers 'session')
    $second=@($sessions | Where-Object { $_.Pid -ne $startupPid })
    $observations.Add(@{Case=$script:Case.Name;Startup=$startup;AfterNewWindow=$sessions;OwnedApplicationPids=@($script:Case.Apps | ForEach-Object { $_.Id });RequestedCwd=$script:Case.Cwd;ProcessStartCwd=$script:Case.Directory})
    Check 'runtime new-window choice creates exactly one additional private CLI peer in the requested cwd' ($sessions.Count -eq 2 -and $second.Count -eq 1 -and $second[0].Executable -ieq $script:Case.Codex -and @($second[0].Arguments).Count -eq 0 -and @($sessions | Where-Object { $_.Cwd -cne $script:Case.Cwd -or $_.Executable -ine $script:Case.Codex }).Count -eq 0)
    $guiApps=@($script:Case.Apps | Where-Object { -not $_.HasExited -and [BksStage7]::MainWindow($_.Id) -ne [IntPtr]::Zero })
    Check 'runtime new window has a distinct owned app PID and owns the second CLI session' ($guiApps.Count -eq 2 -and $child.Id -ne $script:Case.Process.Id -and $second[0].Pid -in [BksStage1011]::Children($child.Id))
    Check-RuntimeProfilesNotSaved 'after runtime new-window choice'
    Stop-Case
}

function Check-Agent {
    Start-Case 'agent'
    # Discovery is limited to the private mock PATH/npm location.
    $panel=Open-Dialog 60015 'BksTermAppServerClient' 4509
    for ($retry=0; $retry -lt 5 -and -not [BksStage7]::IsWindowEnabled((Field $panel 4502)); $retry++) {
        Invoke-Own (Field $panel 4509)
        Start-Sleep -Milliseconds 500
        $panel=Open-Dialog 60015 'BksTermAppServerClient' 4509
    }
    Check 'opening the native AI panel creates no App Server process' (@(Fixture-Markers 'server').Count -eq 0)
    Check 'AI panel exposes named native read-only transcript and explicit Start' ([BksStage7]::ReadOnly($script:Case.Process.Id,(Field $panel 4500)) -and (Element (Field $panel 4500)).Current.Name.Length -gt 0 -and [BksStage7]::IsWindowEnabled((Field $panel 4502)))
    Invoke-Own (Field $panel 4502)
    Wait-For { @(Fixture-Markers 'server').Count -eq 1 -and [BksStage7]::IsWindowEnabled((Field $panel 4503)) } 'explicit Start handshakes with the mock 0.164.0 peer'
    Register-Fixtures
    $server=@(Fixture-Markers 'server')[0]
    Check 'explicit App Server uses only the private mock in the selected cwd' ($server.Cwd -ceq $script:Case.Cwd -and $server.Executable -ieq $script:Case.Codex)
    [BksStage7]::SetText($script:Case.Process.Id,(Field $panel 4501),'stage1011 fixture prompt')
    Invoke-Own (Field $panel 4503)
    Wait-For { (Own-Text (Field $panel 4500)).Contains('STAGE1011_RESPONSE') -and [BksStage7]::IsWindowEnabled((Field $panel 4506)) } 'streamed mock reply and explicit approval controls'
    Check 'streamed final text is not duplicated' (([regex]::Matches((Own-Text (Field $panel 4500)),'STAGE1011_RESPONSE')).Count -eq 1)
    Check 'an approval has no default selection and Apply remains disabled' ([BksStage1011]::Selected($script:Case.Process.Id,(Field $panel 4506)) -eq -1 -and -not [BksStage7]::IsWindowEnabled((Field $panel 4507)))
    Start-Sleep -Milliseconds 500
    $wire=Shared-Text (Join-Path $script:Case.Task ('server-'+$server.Pid+'.ndjson'))
    Check 'no approval decision is sent automatically to the mock peer' (-not $wire.Contains('qa-approval'))
    Invoke-Own (Field $panel 4509)
    Wait-For { [BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermAppServerClient') -eq [IntPtr]::Zero } 'native AI panel closed'
    $serverProcess=$script:Case.Mocks | Where-Object { $_.Id -eq $server.Pid } | Select-Object -First 1
    Check 'closing the panel stops only its known App Server process' ($serverProcess.WaitForExit(5000) -and -not $script:Case.Process.HasExited)
    Stop-Case
}

try {
    Add-Type -ReferencedAssemblies $references -TypeDefinition ($nativeSource+"`r`n"+$extension)
    [BksStage7]::InitializeUia()
    $templateExe=Join-Path $runDirectory 'fixture-template.exe'
    $csc=Join-Path $env:SystemRoot 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
    & $csc /nologo /target:exe /platform:x64 /warnaserror+ /reference:System.Web.Extensions.dll ('/out:'+$templateExe) (Join-Path $PSScriptRoot 'fixtures/stage1011.cs')
    if ($LASTEXITCODE -ne 0) { throw 'Hermetic fixture compilation failed.' }
    Check 'native ownership/UIA helpers and the hermetic peer compile' (Test-Path -LiteralPath $templateExe -PathType Leaf)
    & $templateExe --self-test
    Check 'zero-argument CLI and explicit session use only the harmless fixture route' ($LASTEXITCODE -eq 0)
    $probeTask=Join-Path $runDirectory 'marker-contract'
    $null=[IO.Directory]::CreateDirectory($probeTask)
    $script:Case=@{Name='prepare';Task=$probeTask}
    try {
        Check 'an empty fixture marker query has zero records' (@(Fixture-Markers 'session').Count -eq 0)
        Write-BksJson (Join-Path $probeTask 'session-1.json') @{Pid=1;Cwd='fixture-only';Executable='never-launched';Arguments=@()}
        $probeRecords=@(Fixture-Markers 'session')
        Check 'a singleton marker remains one record with its PID' ($probeRecords.Count -eq 1 -and $probeRecords[0].Pid -eq 1)
        Check 'a zero-argument runtime marker preserves its empty JSON argument array' (@($probeRecords[0].Arguments).Count -eq 0)
    } finally { $script:Case=$null }
    if ($PrepareOnly) { $checks.Add(@{Status='skipped';Name='PrepareOnly: no application, peer or GUI was launched';Case='prepare'}) }
    else {
        $Exe=Get-BksFullPath $Exe
        Assert-BksNoReparse $Exe
        if ($Mode -in @('Settings','All')) { Check-Settings }
        if ($Mode -in @('Sessions','All')) { Check-Sessions }
        if ($Mode -eq 'Runtime') { Check-RuntimeCliSessions }
        if ($Mode -in @('Agent','All')) { Check-Agent }
    }
} catch {
    $failure=$_.Exception.Message
    if (-not $failure.StartsWith('FAILED:')) { $checks.Add(@{Status='error';Name=$failure;Case=$(if ($script:Case) {$script:Case.Name} else {'prepare'})}) }
    $observations.Add(@{Failure=$failure;Stack=$_.ScriptStackTrace})
    if ($script:Case -and $script:Case.Process -and -not $script:Case.Process.HasExited -and $script:Case.Main -ne [IntPtr]::Zero) {
        try {
            $observations.Add((Window-ActionState 'failure before cleanup'))
            $fields=@{}
            foreach ($id in @(1001,1002,1005,1011,1018,1026)) {
                $field=[BksStage7]::GetDlgItem($script:Case.Main,$id)
                if ($field -ne [IntPtr]::Zero) {
                    $value=Own-Text $field
                    $fields["$id"]=$value.Substring([Math]::Max(0,$value.Length-2000))
                }
            }
            $observations.Add(@{Case=$script:Case.Name;Phase='owned native fields before cleanup';Fields=$fields})
        } catch { $observations.Add(@{Case=$script:Case.Name;DiagnosticError=$_.Exception.Message}) }
    }
} finally {
    try { Stop-Case } catch { $checks.Add(@{Status='error';Name=('Owned cleanup: '+$_.Exception.Message);Case='cleanup'}) }
    $counts=@{passed=@($checks | Where-Object {$_.Status -eq 'passed'}).Count;failed=@($checks | Where-Object {$_.Status -eq 'failed'}).Count;skipped=@($checks | Where-Object {$_.Status -eq 'skipped'}).Count;error=@($checks | Where-Object {$_.Status -eq 'error'}).Count}
    Write-BksJson (Join-Path $runDirectory 'summary.json') @{
        Scope='Owned PIDs/HWNDs/private APPDATA/cwd; no real Codex/auth/registry/global input; UIA is not human NVDA';Mode=$Mode;PreparedOnly=[bool]$PrepareOnly;Executable=$Exe;Counts=$counts;Checks=$checks.ToArray();Observations=$observations.ToArray()
    }
    [IO.File]::WriteAllLines((Join-Path $runDirectory 'results.txt'),[string[]]@($checks | ForEach-Object { $_.Status+': '+$_.Name }), (New-Object Text.UTF8Encoding $false))
}
Write-Output "Stage1011 ${Mode}: passed=$($counts.passed) failed=$($counts.failed) skipped=$($counts.skipped) error=$($counts.error)"
Write-Output "Artifacts: $runDirectory"
if ($failure -or $counts.failed -or $counts.error) { throw $(if ($failure) {$failure} else {'Stage1011 checks or cleanup failed.'}) }
