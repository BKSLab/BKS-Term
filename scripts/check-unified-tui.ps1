[CmdletBinding()]
param([string]$Exe,[string]$Artifacts,[switch]$PrepareOnly,[switch]$Minimal,[switch]$ReadNavigation,[switch]$ShellRegression,[switch]$CliHandoff,[switch]$PaintIsolation,[switch]$TuiSelection,[switch]$SingleInput,[switch]$TuiTurns,[string]$PrivateDesktopName,[string]$PrivateDesktopResult)

# preview.12 acceptance: -TuiTurns (controlled phases, fixed cancellation
# footer, bullet answers and answer archive).
# preview.10 acceptance: -SingleInput (shared source-owned field/menu and shell
# output/clear) or -ShellRegression. Other TUI modes retain the historical
# ADR0012 canvas/companion contract and are fixtures for preview.8/9 binaries.

$ErrorActionPreference='Stop'
if($SingleInput -and $TuiTurns) {throw 'SingleInput and TuiTurns are separate acceptance scenarios'}
$privateScenario=$SingleInput -or $TuiTurns
# Resolve defaults in the body: Windows PowerShell may not populate PSScriptRoot
# while parameter defaults are being evaluated.
if(-not $Exe) { $Exe=Join-Path $PSScriptRoot '../target/release/bks-app.exe' }
if(-not $Artifacts) { $Artifacts=Join-Path $PSScriptRoot $(if($TuiTurns){'../target/tui-turns'}elseif($SingleInput){'../target/single-input'}elseif($PaintIsolation){'../target/paint-isolation'}elseif($TuiSelection){'../target/tui-selection'}else{'../target/unified-tui'}) }
if($PSVersionTable.PSEdition -eq 'Core' -or [Threading.Thread]::CurrentThread.GetApartmentState() -ne 'MTA') {
    $arguments=@('-NoProfile','-Mta','-ExecutionPolicy','Bypass','-File',$PSCommandPath,'-Exe',$Exe,'-Artifacts',$Artifacts)
    if($PrepareOnly) { $arguments+='-PrepareOnly' }
    if($Minimal) { $arguments+='-Minimal' }
    if($ReadNavigation) { $arguments+='-ReadNavigation' }
    if($ShellRegression) { $arguments+='-ShellRegression' }
    if($CliHandoff) { $arguments+='-CliHandoff' }
    if($PaintIsolation) { $arguments+='-PaintIsolation' }
    if($TuiSelection) { $arguments+='-TuiSelection' }
    if($SingleInput) { $arguments+='-SingleInput' }
    if($TuiTurns) { $arguments+='-TuiTurns' }
    if($PrivateDesktopName) { $arguments+=@('-PrivateDesktopName',$PrivateDesktopName,'-PrivateDesktopResult',$PrivateDesktopResult) }
    & "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" @arguments
    exit $LASTEXITCODE
}
. (Join-Path $PSScriptRoot '../packaging/common.ps1')
$artifactRoot=Get-BksFullPath $Artifacts
Assert-BksNoReparse $artifactRoot
# SingleInput/TuiTurns run in a fresh process whose startup desktop is private. Moving
# only this PowerShell thread would leave .NET/UIA worker threads on Default.
# No desktop is ever switched, activated, or attached to an input queue.
$privateDesktopSource=@'
using System;
using System.Collections;
using System.ComponentModel;
using System.Diagnostics;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
public sealed class BksQaDesktop:IDisposable {
    [StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)] struct Startup {
        public int Size;public string Reserved,Desktop,Title;
        public uint X,Y,Width,Height,CharsX,CharsY,Fill,Flags;
        public ushort Show,ReservedSize;public IntPtr ReservedBytes,Input,Output,Error;
    }
    [StructLayout(LayoutKind.Sequential)] struct ProcessInfo {public IntPtr Process,Thread;public uint Pid,Tid;}
    [StructLayout(LayoutKind.Sequential)] struct BasicLimit {
        public long ProcessTime,JobTime;public uint Flags;
        public UIntPtr Minimum,Maximum;public uint Active;public UIntPtr Affinity;public uint Priority,Scheduling;
    }
    [StructLayout(LayoutKind.Sequential)] struct IoCounters {public ulong ReadOps,WriteOps,OtherOps,ReadBytes,WriteBytes,OtherBytes;}
    [StructLayout(LayoutKind.Sequential)] struct ExtendedLimit {
        public BasicLimit Basic;public IoCounters Io;public UIntPtr ProcessMemory,JobMemory,PeakProcess,PeakJob;
    }
    [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr CreateDesktopW(string name,IntPtr device,IntPtr mode,uint flags,uint access,IntPtr security);
    [DllImport("user32.dll",SetLastError=true)] static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll",SetLastError=true)] static extern bool SetThreadDesktop(IntPtr desktop);
    [DllImport("user32.dll")] static extern IntPtr GetThreadDesktop(uint thread);
    [DllImport("user32.dll")] static extern IntPtr GetProcessWindowStation();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window,out uint process);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool GetUserObjectInformationW(IntPtr handle,int kind,StringBuilder text,int bytes,out int needed);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool CreateProcessW(string application,StringBuilder command,IntPtr processSecurity,IntPtr threadSecurity,bool inherit,uint flags,IntPtr environment,string cwd,ref Startup startup,out ProcessInfo process);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr CreateJobObjectW(IntPtr security,string name);
    [DllImport("kernel32.dll",SetLastError=true)] static extern bool SetInformationJobObject(IntPtr job,int kind,ref ExtendedLimit limit,uint length);
    [DllImport("kernel32.dll",SetLastError=true)] static extern bool AssignProcessToJobObject(IntPtr job,IntPtr process);
    [DllImport("kernel32.dll",SetLastError=true)] static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll",SetLastError=true)] static extern bool TerminateProcess(IntPtr process,uint code);
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr handle,uint milliseconds);
    [DllImport("kernel32.dll",SetLastError=true)] static extern bool CloseHandle(IntPtr handle);
    [DllImport("ole32.dll")] static extern int CoInitializeEx(IntPtr reserved,uint flags);
    [DllImport("ole32.dll")] static extern void CoUninitialize();
    readonly IntPtr original;IntPtr owned,job;Process child;bool disposed;
    static IntPtr callerDesktop;static uint callerThread;static string expected;
    static Thread worker;static ManualResetEvent ready,stop;
    static Exception workerError;
    public string Name {get;private set;}
    public string OriginalDesktop {get;private set;}
    public string RestoredDesktop {get;private set;}
    public bool DesktopClosed {get;private set;}
    public bool JobClosed {get;private set;}
    public string Path {get{return "WinSta0\\"+Name;}}
    public static string WorkerDesktop {get;private set;}
    static Exception NativeError(string operation) {return new Win32Exception(Marshal.GetLastWin32Error(),operation);}
    static string ObjectName(IntPtr handle) {
        var text=new StringBuilder(512);int needed;
        if(handle==IntPtr.Zero || !GetUserObjectInformationW(handle,2,text,text.Capacity*2,out needed))throw NativeError("Private desktop name query failed");
        return text.ToString();
    }
    public static string CurrentDesktop {get{return ObjectName(GetThreadDesktop(GetCurrentThreadId()));}}
    public static string WindowDesktop(uint process,IntPtr window) {
        uint actual;uint thread=GetWindowThreadProcessId(window,out actual);
        if(window==IntPtr.Zero || thread==0 || actual!=process)throw new InvalidOperationException("Private desktop window PID identity changed");
        return ObjectName(GetThreadDesktop(thread));
    }
    static void ValidateName(string name) {
        if(name==null || !System.Text.RegularExpressions.Regex.IsMatch(name,@"\ABKS-Term-QA-[0-9a-f]{32}\z"))throw new InvalidOperationException("Invalid private desktop identity");
        if(!ObjectName(GetProcessWindowStation()).Equals("WinSta0",StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException("Private fixture requires its existing WinSta0 window station");
    }
    public BksQaDesktop(string name) {
        ValidateName(name);Name=name;original=GetThreadDesktop(GetCurrentThreadId());OriginalDesktop=ObjectName(original);
        // Omit DESKTOP_SWITCHDESKTOP, HOOKCONTROL and cross-queue permissions.
        owned=CreateDesktopW(name,IntPtr.Zero,IntPtr.Zero,0,0x200c7,IntPtr.Zero);
        if(owned==IntPtr.Zero)throw NativeError("CreateDesktopW failed");
        try {
            if(!SetThreadDesktop(original))throw NativeError("Restore bootstrap thread desktop failed");
            job=CreateJobObjectW(IntPtr.Zero,null);if(job==IntPtr.Zero)throw NativeError("Private bootstrap job creation failed");
            var limit=new ExtendedLimit();limit.Basic.Flags=0x2000;
            if(!SetInformationJobObject(job,9,ref limit,(uint)Marshal.SizeOf(typeof(ExtendedLimit))))throw NativeError("Private bootstrap job limits failed");
        } catch {Dispose();throw;}
    }
    public static void Connect(string name) {
        ValidateName(name);
        // Refuse an already-running process on Default: internal workers must
        // inherit the private startup desktop, even before InitializeUia.
        if(!CurrentDesktop.Equals(name,StringComparison.Ordinal))throw new InvalidOperationException("PowerShell did not start on the private desktop");
        expected=name;callerThread=GetCurrentThreadId();callerDesktop=GetThreadDesktop(callerThread);
        if(!SetThreadDesktop(callerDesktop))throw NativeError("Attach private PowerShell calling thread failed");
    }
    static string Quote(string value) {
        var text=new StringBuilder("\"");int slashes=0;
        foreach(char ch in value) {
            if(ch=='\\'){slashes++;continue;}
            if(ch=='\"'){text.Append('\\',slashes*2+1).Append(ch);slashes=0;continue;}
            text.Append('\\',slashes).Append(ch);slashes=0;
        }
        return text.Append('\\',slashes*2).Append('"').ToString();
    }
    public Process StartBootstrap(string executable,string[] arguments,string cwd) {
        var command=new StringBuilder(Quote(executable));foreach(string argument in arguments)command.Append(' ').Append(Quote(argument));
        var startup=new Startup();startup.Size=Marshal.SizeOf(typeof(Startup));startup.Desktop=Path;
        ProcessInfo info;
        if(!CreateProcessW(executable,command,IntPtr.Zero,IntPtr.Zero,false,0x08000004,IntPtr.Zero,cwd,ref startup,out info))throw NativeError("Private PowerShell creation failed");
        try {
            if(!AssignProcessToJobObject(job,info.Process))throw NativeError("Private PowerShell job assignment failed");
            child=Process.GetProcessById((int)info.Pid);var pinned=child.Handle;
            if(ResumeThread(info.Thread)==uint.MaxValue)throw NativeError("Private PowerShell resume failed");
            return child;
        } catch {TerminateProcess(info.Process,1);WaitForSingleObject(info.Process,5000);throw;}
        finally {CloseHandle(info.Thread);CloseHandle(info.Process);}
    }
    public static Process StartApplication(ProcessStartInfo start) {
        if(expected==null || !CurrentDesktop.Equals(expected,StringComparison.Ordinal))throw new InvalidOperationException("Private application launch requires attached desktop");
        if(start.UseShellExecute || String.IsNullOrEmpty(start.WorkingDirectory))throw new InvalidOperationException("Explicit private environment and cwd required");
        var keys=new string[start.EnvironmentVariables.Count];start.EnvironmentVariables.Keys.CopyTo(keys,0);Array.Sort(keys,StringComparer.OrdinalIgnoreCase);
        var block=new StringBuilder();foreach(string key in keys)block.Append(key).Append('=').Append(start.EnvironmentVariables[key]).Append('\0');block.Append('\0');
        char[] chars=block.ToString().ToCharArray();IntPtr environment=Marshal.AllocHGlobal(chars.Length*2);
        try {
            Marshal.Copy(chars,0,environment,chars.Length);
            var startup=new Startup();startup.Size=Marshal.SizeOf(typeof(Startup));startup.Desktop="WinSta0\\"+expected;
            var command=new StringBuilder(Quote(start.FileName)+" "+start.Arguments);ProcessInfo info;
            if(!CreateProcessW(start.FileName,command,IntPtr.Zero,IntPtr.Zero,false,0x08000400,environment,start.WorkingDirectory,ref startup,out info))throw NativeError("Private BKS application creation failed");
            try {var process=Process.GetProcessById((int)info.Pid);var pinned=process.Handle;return process;}
            catch {TerminateProcess(info.Process,1);WaitForSingleObject(info.Process,5000);throw;}
            finally {CloseHandle(info.Thread);CloseHandle(info.Process);}
        } finally {Marshal.FreeHGlobal(environment);}
    }
    public static void InitializeUiaOnWorker(Type helper) {
        if(expected==null || GetCurrentThreadId()!=callerThread || !CurrentDesktop.Equals(expected,StringComparison.Ordinal))throw new InvalidOperationException("UIA caller is not on the private desktop");
        MethodInfo initialize=helper.GetMethod("InitializeUia",BindingFlags.Public|BindingFlags.Static);
        if(initialize==null)throw new InvalidOperationException("UIA initializer unavailable");
        ready=new ManualResetEvent(false);stop=new ManualResetEvent(false);
        worker=new Thread(delegate() {
            bool com=false;
            try {
                IntPtr desktop=GetThreadDesktop(GetCurrentThreadId());
                if(!ObjectName(desktop).Equals(expected,StringComparison.Ordinal) || !SetThreadDesktop(desktop))throw new InvalidOperationException("UIA worker did not inherit private desktop");
                WorkerDesktop=CurrentDesktop;
                int status=CoInitializeEx(IntPtr.Zero,0);if(status<0)Marshal.ThrowExceptionForHR(status);com=true;
                initialize.Invoke(null,null);ready.Set();stop.WaitOne();
            } catch(Exception error) {workerError=error;ready.Set();}
            finally {if(com)CoUninitialize();}
        });
        worker.IsBackground=true;worker.Name="BKS private desktop UIA";worker.Start();
        if(!ready.WaitOne(15000))throw new InvalidOperationException("Private UIA worker initialization timed out");
        if(workerError!=null)throw new InvalidOperationException("Private UIA worker initialization failed",workerError);
    }
    public static void Disconnect() {
        if(stop!=null)stop.Set();if(worker!=null && !worker.Join(5000))throw new InvalidOperationException("Private UIA worker did not stop");
        if(callerDesktop!=IntPtr.Zero && (GetCurrentThreadId()!=callerThread || !SetThreadDesktop(callerDesktop)))throw new InvalidOperationException("Private PowerShell desktop restore failed");
        if(ready!=null){ready.Close();ready=null;}if(stop!=null){stop.Close();stop=null;}
        // GetThreadDesktop handles are borrowed (including the process startup
        // desktop). Only the outer creator closes its CreateDesktopW handle.
    }
    public void Dispose() {
        if(disposed)return;disposed=true;Exception failure=null;
        if(job!=IntPtr.Zero){JobClosed=CloseHandle(job);if(!JobClosed)failure=NativeError("Private job close failed");job=IntPtr.Zero;}
        if(child!=null){try {if(!child.WaitForExit(10000))failure=new InvalidOperationException("Private child remained after job cleanup");}finally {child.Dispose();child=null;}}
        if(original!=IntPtr.Zero) {if(!SetThreadDesktop(original))failure=NativeError("Bootstrap desktop restore failed");else RestoredDesktop=CurrentDesktop;}
        if(owned!=IntPtr.Zero){DesktopClosed=CloseDesktop(owned);if(!DesktopClosed)failure=NativeError("Private desktop close failed");owned=IntPtr.Zero;}
        if(failure!=null)throw failure;
    }
}
'@
if($PrivateDesktopName -and -not $privateScenario) { throw 'Private desktop parameters belong only to isolated acceptance scenarios' }
if($privateScenario) {
    Add-Type -TypeDefinition $privateDesktopSource
    if(-not $PrivateDesktopName) {
        $null=[IO.Directory]::CreateDirectory($artifactRoot)
        $desktopName='BKS-Term-QA-'+[Guid]::NewGuid().ToString('N')
        $receipt=Join-Path $artifactRoot ('bootstrap-'+[Guid]::NewGuid().ToString('N')+'.json')
        $desktop=$null;$bootstrapFailure=$null;$bootstrapExit=1
        try {
            $desktop=New-Object BksQaDesktop($desktopName)
            $scenarioFlag=if($TuiTurns){'-TuiTurns'}else{'-SingleInput'}
            $arguments=@('-NoProfile','-Mta','-ExecutionPolicy','Bypass','-File',$PSCommandPath,'-Exe',$Exe,'-Artifacts',$artifactRoot,$scenarioFlag,'-PrivateDesktopName',$desktopName,'-PrivateDesktopResult',$receipt)
            foreach($option in @('PrepareOnly','Minimal','ReadNavigation','ShellRegression','CliHandoff','PaintIsolation','TuiSelection')) {if((Get-Variable -Name $option -ValueOnly)){$arguments+='-'+$option}}
            $bootstrap=$desktop.StartBootstrap((Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'),[string[]]$arguments,(Get-Location).Path)
            $timer=[Diagnostics.Stopwatch]::StartNew()
            while(-not $bootstrap.WaitForExit(1000)) {if($timer.ElapsedMilliseconds -ge 300000){throw 'Private desktop harness timeout; only its own job will be closed'}}
            $bootstrapExit=$bootstrap.ExitCode
        } catch {$bootstrapFailure=$_.Exception.Message}
        finally {if($desktop){try {$desktop.Dispose()} catch {$bootstrapFailure=$_.Exception.Message}}}
        if([IO.File]::Exists($receipt)) {
            $result=Read-BksJson $receipt
            if($result.RunDirectory -and $desktop) {Write-BksJson (Join-Path $result.RunDirectory 'private-desktop-bootstrap.json') @{Desktop=$desktopName;OriginalThreadDesktop=$desktop.OriginalDesktop;RestoredThreadDesktop=$desktop.RestoredDesktop;DesktopClosed=$desktop.DesktopClosed;JobClosed=$desktop.JobClosed;ChildExitCode=$bootstrapExit;Failure=$bootstrapFailure}}
            if($result.Counts) {Write-Output "Unified TUI: passed=$($result.Counts.passed) failed=$($result.Counts.failed) skipped=$($result.Counts.skipped) error=$($result.Counts.error)"}
            if($result.RunDirectory) {Write-Output ('Artifacts: '+$result.RunDirectory)}
            if($result.Failure) {$bootstrapFailure=$result.Failure}
        } elseif(-not $bootstrapFailure) {$bootstrapFailure='Private desktop child produced no receipt; initialization did not complete'}
        if($bootstrapFailure){Write-Output ('Private desktop harness failed: '+$bootstrapFailure);exit 1}
        exit $bootstrapExit
    }
    # Install the fail-closed receipt before loading UIAutomation assemblies.
    trap {
        if($PrivateDesktopResult -and -not [IO.File]::Exists($PrivateDesktopResult)) {Write-BksJson $PrivateDesktopResult @{Failure=$_.Exception.Message;PrivateDesktop=$PrivateDesktopName}}
        exit 1
    }
    [BksQaDesktop]::Connect($PrivateDesktopName)
}
$runDirectory=Join-Path $artifactRoot ('run-'+(Get-Date -Format 'yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N').Substring(0,8))
$null=[IO.Directory]::CreateDirectory($runDirectory)
$checks=New-Object 'System.Collections.Generic.List[object]'
$observations=New-Object 'System.Collections.Generic.List[object]'
$script:Case=$null
$script:ExpectedInput=''
$failure=$null

Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,UIAutomationClientsideProviders,System.Drawing
$references=@([Windows.Automation.AutomationElement].Assembly.Location,[Windows.Automation.AutomationPattern].Assembly.Location,[UIAutomationClientsideProviders.UIAutomationClientSideProviders].Assembly.Location,[Drawing.Bitmap].Assembly.Location)
# Load helper definitions only, never another script's QA scenario.
$stage7=[IO.File]::ReadAllText((Join-Path $PSScriptRoot 'check-stage7.ps1'),[Text.Encoding]::UTF8)
$nativeSource=[regex]::Match($stage7,"(?s)using System;.*?(?=\r?\n'@)").Value
$directScript=[IO.File]::ReadAllText((Join-Path $PSScriptRoot 'check-direct-accessibility.ps1'),[Text.Encoding]::UTF8)
$directSource=[regex]::Match($directScript,"(?s)\`$extension = @'\r?\n(.*?)\r?\n'@").Groups[1].Value
if(-not $nativeSource.Contains('public static class BksStage7') -or -not $directSource.Contains('public static class BksDirect')) { throw 'Owned helper definitions unavailable' }
$parseErrors=$null
$tokens=$null
$helperAst=[Management.Automation.Language.Parser]::ParseInput($directScript,[ref]$tokens,[ref]$parseErrors)
if($parseErrors.Count) { throw 'Direct helper script does not parse' }
$allowed=@('Check','Wait-For','Field','Element','Own-Text','Normalize-Text','Same-UiaText','Rect-Key','Uia-Text','Focus-Diagnostics','Menu','Shared-Text','Raw-Input','Assert-ExpectedInput','Confirm-Input','Uia-Selected','Uia-Rows','Invoke-Own','Request-Scene','Check-Key','Stop-Case')
foreach($definition in $helperAst.FindAll({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst]},$true)) {
    if($definition.Name -in $allowed) { . ([scriptblock]::Create($definition.Extent.Text)) }
}
$extraSource=@'
public static class BksUnified {
    public const string LivePrefix="\u0423\u043f\u0440\u0430\u0432\u043b\u0435\u043d\u0438\u0435 TUI.";
    public const string Pending="\u043e\u0436\u0438\u0434\u0430\u043d\u0438\u0435";
    public const string SnapshotButton="\u0427\u0438\u0442\u0430\u0442\u044c \u0441\u043d\u0438\u043c\u043e\u043a";
    public const string ControlButton="\u0423\u043f\u0440\u0430\u0432\u043b\u044f\u0442\u044c \u043a\u043b\u0430\u0432\u0438\u0448\u0430\u043c\u0438 TUI";
    public const string KeyRejected="\u041a\u043b\u0430\u0432\u0438\u0448\u0430 \u043d\u0435 \u043e\u0442\u043f\u0440\u0430\u0432\u043b\u0435\u043d\u0430";
    public const string SingleFieldName="\u0422\u0435\u0440\u043c\u0438\u043d\u0430\u043b \u2014 \u043f\u0440\u044f\u043c\u043e\u0439 \u0432\u0432\u043e\u0434";
    public const string TurnThinking="\u0420\u0430\u0437\u043c\u044b\u0448\u043b\u0435\u043d\u0438\u0435";
    public const string TurnSearching="\u041f\u043e\u0438\u0441\u043a";
    public const string TurnRunning="\u0412\u044b\u043f\u043e\u043b\u043d\u0435\u043d\u0438\u0435";
    public const string TurnReady="\u041e\u0442\u0432\u0435\u0442 \u0433\u043e\u0442\u043e\u0432";
    public const string RequestCollapsed="\u0412\u0430\u0448 \u0437\u0430\u043f\u0440\u043e\u0441, \u0441\u0432\u0451\u0440\u043d\u0443\u0442\u043e: ";
    public const string RequestExpanded="\u0412\u0430\u0448 \u0437\u0430\u043f\u0440\u043e\u0441, \u0440\u0430\u0437\u0432\u0451\u0440\u043d\u0443\u0442\u043e: ";
    public const string TurnNeedsInput="\u0422\u0440\u0435\u0431\u0443\u0435\u0442\u0441\u044f \u0434\u0435\u0439\u0441\u0442\u0432\u0438\u0435";
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window,out uint process);
    [DllImport("kernel32.dll")] private static extern uint GetCurrentThreadId();
    [DllImport("user32.dll",SetLastError=true)] private static extern bool AttachThreadInput(uint first,uint second,bool attach);
    [DllImport("user32.dll",SetLastError=true)] private static extern bool GetKeyboardState(byte[] state);
    [DllImport("user32.dll",SetLastError=true)] private static extern bool SetKeyboardState(byte[] state);
    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll",SetLastError=true)] private static extern bool PostMessageW(IntPtr window,uint message,IntPtr wparam,IntPtr lparam);
    [DllImport("user32.dll")] private static extern bool PrintWindow(IntPtr window,IntPtr dc,uint flags);
    [DllImport("user32.dll")] private static extern bool IsWindowUnicode(IntPtr window);
    [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll",SetLastError=true)] private static extern bool GetGUIThreadInfo(uint thread,ref Gui info);
    [DllImport("user32.dll",SetLastError=true)] private static extern bool GetScrollInfo(IntPtr window,int bar,ref ScrollInfo info);
    [DllImport("user32.dll",SetLastError=true)] private static extern bool SetWindowPos(IntPtr window,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll",EntryPoint="GetWindowLongPtrW")] private static extern IntPtr WindowStyle(IntPtr window,int index);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern IntPtr SendMessageTimeoutW(IntPtr window,uint message,IntPtr wparam,IntPtr lparam,uint flags,uint timeout,out IntPtr result);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageTimeoutW",SetLastError=true)] private static extern IntPtr SendTextTimeout(IntPtr window,uint message,IntPtr wparam,StringBuilder text,uint flags,uint timeout,out IntPtr result);
    [DllImport("user32.dll")] private static extern bool RedrawWindow(IntPtr window,IntPtr rect,IntPtr region,uint flags);
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr window,uint command);
    [StructLayout(LayoutKind.Sequential)] private struct Gui {
        public uint Size,Flags; public IntPtr Active,Focus,Capture,Menu,MoveSize,Caret; public BksDirect.Rect CaretRect;
    }
    [StructLayout(LayoutKind.Sequential)] private struct ScrollInfo {
        public uint Size,Mask; public int Minimum,Maximum; public uint Page; public int Position,TrackPosition;
    }
    public sealed class ViewportState {
        public int FirstLine,Minimum,Maximum,Position; public uint Page; public long Bottom; public bool AtBottom;
    }
    public static ViewportState Viewport(uint process,IntPtr window) {
        BksStage7.Own(process,window);
        var info=new ScrollInfo();info.Size=(uint)Marshal.SizeOf(typeof(ScrollInfo));info.Mask=0x17;
        if(!GetScrollInfo(window,1,ref info))throw new InvalidOperationException("Owned vertical scroll query failed");
        long bottom=Math.Max(info.Minimum,(long)info.Maximum-Math.Max(0L,(long)info.Page-1));
        return new ViewportState {FirstLine=BksStage7.FirstLine(process,window),Minimum=info.Minimum,Maximum=info.Maximum,Position=info.Position,Page=info.Page,Bottom=bottom,AtBottom=(long)info.Position>=bottom-1};
    }
    public sealed class CaretState {
        public long Window,Focus; public uint Flags; public bool Visible; public BksDirect.Rect Rect;
    }
    public static CaretState NativeCaret(uint process,IntPtr window) {
        BksStage7.Own(process,window);
        uint actual;uint thread=GetWindowThreadProcessId(window,out actual);
        if(actual!=process)throw new InvalidOperationException("Owned caret thread changed");
        var info=new Gui();info.Size=(uint)Marshal.SizeOf(typeof(Gui));
        if(!GetGUIThreadInfo(thread,ref info))throw new InvalidOperationException("Owned caret query failed");
        return new CaretState {Window=info.Caret.ToInt64(),Focus=info.Focus.ToInt64(),Flags=info.Flags,Visible=(info.Flags&1)!=0,Rect=info.CaretRect};
    }
    public static void Resize(uint process,IntPtr window,int width,int height) {
        BksStage7.Own(process,window);
        if(BksDirect.GetAncestor(window,2)!=window || BksDirect.GetAncestor(BksDirect.GetForegroundWindow(),2)!=window)
            throw new InvalidOperationException("Own foreground root required for resize");
        if(!SetWindowPos(window,IntPtr.Zero,0,0,width,height,0x16))throw new InvalidOperationException("Own resize failed");
    }
    public static bool UnicodeWindow(uint process,IntPtr window) { BksStage7.Own(process,window);return IsWindowUnicode(window); }
    public static bool Activate(uint process,IntPtr root) {BksStage7.Own(process,root);return SetForegroundWindow(root);}
    public static bool OverlayProtected(uint process,IntPtr provider,IntPtr canvas) {
        BksStage7.Own(process,provider);BksStage7.Own(process,canvas);
        if(BksDirect.GetParent(provider)!=BksDirect.GetParent(canvas) ||
           (WindowStyle(provider,-16).ToInt64()&0x04000000)==0 ||
           (WindowStyle(canvas,-16).ToInt64()&0x04000000)==0 ||
           (WindowStyle(canvas,-16).ToInt64()&0x10000)==0 ||
           (WindowStyle(provider,-16).ToInt64()&0x10000)!=0 ||
           BksStage7.IsTextControl(process,canvas))return false;
        var sibling=provider;
        for(int index=0;index<256;index++) {
            sibling=GetWindow(sibling,3);
            if(sibling==IntPtr.Zero)return false;
            BksStage7.Own(process,sibling);
            if(sibling==canvas)return true;
        }
        return false;
    }
    static void NativeMessage(uint process,IntPtr window,uint message,int first,int second) {
        BksStage7.Own(process,window);IntPtr result;
        if(SendMessageTimeoutW(window,message,new IntPtr(first),new IntPtr(second),2,1000,out result)==IntPtr.Zero)
            throw new InvalidOperationException("Owned bounded native provider message failed");
    }
    public static int ListSelection(uint process,IntPtr window) {
        BksStage7.Own(process,window);IntPtr result;
        if(!BksStage7.ClassName(window).Equals("ListBox",StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException("Owned LISTBOX required");
        if(SendMessageTimeoutW(window,0x188,IntPtr.Zero,IntPtr.Zero,2,1000,out result)==IntPtr.Zero)throw new InvalidOperationException("Owned list selection query failed");
        return result.ToInt32();
    }
    public static string[] ListItems(uint process,IntPtr window) {
        BksStage7.Own(process,window);IntPtr result;
        if(!BksStage7.ClassName(window).Equals("ListBox",StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException("Owned LISTBOX required");
        if(SendMessageTimeoutW(window,0x18b,IntPtr.Zero,IntPtr.Zero,2,1000,out result)==IntPtr.Zero)throw new InvalidOperationException("Owned list count query failed");
        int count=result.ToInt32();if(count<0 || count>64)throw new InvalidOperationException("Owned list count exceeds fixture bounds");
        var items=new string[count];
        for(int index=0;index<count;index++) {
            if(SendMessageTimeoutW(window,0x18a,new IntPtr(index),IntPtr.Zero,2,1000,out result)==IntPtr.Zero || result.ToInt64()<0 || result.ToInt64()>512)throw new InvalidOperationException("Owned list label exceeds fixture bounds");
            var text=new StringBuilder(514);
            if(SendTextTimeout(window,0x189,new IntPtr(index),text,2,1000,out result)==IntPtr.Zero || result.ToInt64()<0)throw new InvalidOperationException("Owned list label query failed");
            items[index]=text.ToString();
        }
        return items;
    }
    public sealed class RangeAttributes {public string Text;public int Foreground,Weight;}
    public static RangeAttributes TextAttributes(uint process,IntPtr window,string needle) {
        BksStage7.Own(process,window);
        var element=System.Windows.Automation.AutomationElement.FromHandle(window);object candidate;
        if(!element.TryGetCurrentPattern(System.Windows.Automation.TextPattern.Pattern,out candidate))throw new InvalidOperationException("Owned styled output has no TextPattern");
        var range=((System.Windows.Automation.TextPattern)candidate).DocumentRange.FindText(needle,false,false);
        if(range==null)throw new InvalidOperationException("Owned styled answer range is absent");
        object foreground=range.GetAttributeValue(System.Windows.Automation.TextPattern.ForegroundColorAttribute);
        object weight=range.GetAttributeValue(System.Windows.Automation.TextPattern.FontWeightAttribute);
        if(!(foreground is int) || !(weight is int))throw new InvalidOperationException("Owned answer style is mixed or unavailable");
        return new RangeAttributes {Text=range.GetText(-1),Foreground=(int)foreground,Weight=(int)weight};
    }
    public static bool PatchProvider(uint process,IntPtr provider) {
        BksStage7.Own(process,provider);
        if(!BksStage7.ReadOnly(process,provider))throw new InvalidOperationException("Owned read-only provider required");
        NativeMessage(process,provider,0xb1,0,-1);
        var text=new StringBuilder("NATIVE_ONLY_PAINT_PROBE\r\n");
        for(int index=0;index<160;index++)text.Append("Native provider scroll line ").Append(index).Append("\r\n");
        IntPtr result;
        if(SendTextTimeout(provider,0xc2,IntPtr.Zero,text,2,1000,out result)==IntPtr.Zero)
            throw new InvalidOperationException("Owned native provider replacement failed");
        NativeMessage(process,provider,0xb1,int.MaxValue,int.MaxValue);
        NativeMessage(process,provider,0xb7,0,0);
        if(!RedrawWindow(provider,IntPtr.Zero,IntPtr.Zero,0x105))throw new InvalidOperationException("Owned native provider paint request failed");
        return BksStage7.Text(process,provider).Contains("NATIVE_ONLY_PAINT_PROBE");
    }
    public sealed class PixelComparison {public int Compared,Changed,DistinctBefore;}
    public static PixelComparison CompareCanvas(uint process,IntPtr canvas,string beforePath,string afterPath) {
        BksStage7.Own(process,canvas);var root=BksDirect.GetAncestor(canvas,2);BksStage7.Own(process,root);
        var whole=BksDirect.Bounds(process,root);var area=BksDirect.Bounds(process,canvas);
        int x=area.Left-whole.Left,y=area.Top-whole.Top,width=area.Right-area.Left,height=area.Bottom-area.Top;
        using(var before=new System.Drawing.Bitmap(beforePath))using(var after=new System.Drawing.Bitmap(afterPath)) {
            if(width<1 || height<1 || x<0 || y<0 || before.Size!=after.Size || x+width>before.Width || y+height>before.Height)
                throw new InvalidOperationException("Owned canvas image bounds changed");
            var colors=new System.Collections.Generic.Dictionary<int,bool>();int changed=0;
            for(int row=y;row<y+height;row++)for(int column=x;column<x+width;column++) {
                int first=before.GetPixel(column,row).ToArgb();colors[first]=true;
                if(first!=after.GetPixel(column,row).ToArgb())changed++;
            }
            return new PixelComparison {Compared=width*height,Changed=changed,DistinctBefore=colors.Count};
        }
    }
    public static void EmojiPair(uint process,IntPtr window) {
        BksStage7.Own(process,window);
        if(!IsWindowUnicode(window) || BksDirect.Focus(process,window)!=window)throw new InvalidOperationException("Own Unicode pair target is not a focused Unicode window");
        if(!PostMessageW(window,0x102,new IntPtr(0xd83d),new IntPtr(1)) ||
           !PostMessageW(window,0x102,new IntPtr(0xde00),new IntPtr(1)))throw new InvalidOperationException("Own numeric Unicode pair post failed");
    }
    public sealed class ChordLease:IDisposable {
        private uint ownThread,targetThread;
        private bool attached,disposed;
        private byte[] saved;
        internal ChordLease(uint process,IntPtr window,int key,bool control,bool shift,IntPtr root) {
            BksStage7.Own(process,window); BksStage7.Own(process,root);
            if(BksDirect.GetAncestor(BksDirect.GetForegroundWindow(),2)!=root || BksDirect.Focus(process,window)!=window)
                throw new InvalidOperationException("Environment foreground/focus lost; owned chord refused");
            foreach(int modifier in new[]{0x10,0x11,0x12,0x5b,0x5c})
                if((GetAsyncKeyState(modifier)&0x8000)!=0)throw new InvalidOperationException("Physical modifier held; owned chord refused");
            uint actual; targetThread=GetWindowThreadProcessId(window,out actual); ownThread=GetCurrentThreadId();
            if(actual!=process)throw new InvalidOperationException("Owned input thread changed");
            try {
                attached=ownThread!=targetThread;
                if(attached && !AttachThreadInput(ownThread,targetThread,true))throw new InvalidOperationException("Owned input queues could not attach");
                saved=new byte[256];
                if(!GetKeyboardState(saved))throw new InvalidOperationException("Own queue keyboard-state read failed");
                var state=(byte[])saved.Clone();
                state[0x11]=state[0xa2]=(byte)(control?0x80:0); state[0x10]=state[0xa0]=(byte)(shift?0x80:0); state[0x12]=0;
                if(!SetKeyboardState(state))throw new InvalidOperationException("Own queue keyboard-state update failed");
                if(!PostMessageW(window,0x100,new IntPtr(key),new IntPtr(1)) ||
                   !PostMessageW(window,0x101,new IntPtr(key),new IntPtr(unchecked((int)0xc0000001))))
                    throw new InvalidOperationException("Own queue chord post failed");
            } catch { Dispose(); throw; }
        }
        public void Dispose() {
            if(disposed)return; disposed=true;
            try { if(saved!=null)SetKeyboardState(saved); }
            finally { if(attached)AttachThreadInput(ownThread,targetThread,false); }
        }
    }
    public static IDisposable BeginChord(uint process,IntPtr window,int key,bool control,bool shift,IntPtr root) {
        return new ChordLease(process,window,key,control,shift,root);
    }
    public static void Repeat(uint process,IntPtr window,int key) {
        BksStage7.Own(process,window);
        if(BksDirect.Focus(process,window)!=window)throw new InvalidOperationException("Own repeat target lacks focus");
        if(!PostMessageW(window,0x100,new IntPtr(key),new IntPtr(0x40000001)) ||
           !PostMessageW(window,0x101,new IntPtr(key),new IntPtr(unchecked((int)0xc0000001))))throw new InvalidOperationException("Own repeat post failed");
    }
    public static bool Screenshot(uint process,IntPtr window,string path) {
        BksStage7.Own(process,window);
        window=BksDirect.GetAncestor(window,2);BksStage7.Own(process,window);
        var rect=BksDirect.Bounds(process,window);
        int width=rect.Right-rect.Left,height=rect.Bottom-rect.Top;
        if(width<1 || height<1 || width>4096 || height>4096)throw new InvalidOperationException("Own screenshot bounds invalid");
        using(var image=new System.Drawing.Bitmap(width,height)) {
            using(var graphics=System.Drawing.Graphics.FromImage(image)) {
                IntPtr dc=graphics.GetHdc();
                try { if(!PrintWindow(window,dc,2))return false; }
                finally { graphics.ReleaseHdc(dc); }
            }
            image.Save(path,System.Drawing.Imaging.ImageFormat.Png); return true;
        }
    }
}
'@
$fixtureSource=@'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;
internal static class UnifiedFixture {
    sealed class CapturingWriter:TextWriter {
        readonly TextWriter terminal,capture;
        public CapturingWriter(TextWriter terminal,string path) {this.terminal=terminal;capture=new StreamWriter(path,false,new UTF8Encoding(false)){AutoFlush=true};}
        public override Encoding Encoding {get{return terminal.Encoding;}}
        public override void Write(string value) {capture.Write(value);terminal.Write(value);}
        public override void Write(char value) {capture.Write(value);terminal.Write(value);}
        public override void Flush() {capture.Flush();terminal.Flush();}
    }
    [DllImport("kernel32.dll")] static extern IntPtr GetStdHandle(int kind);
    [DllImport("kernel32.dll")] static extern bool GetConsoleMode(IntPtr handle,out uint mode);
    [DllImport("kernel32.dll")] static extern bool SetConsoleMode(IntPtr handle,uint mode);
    [DllImport("kernel32.dll")] static extern bool SetConsoleCP(uint codePage);
    [DllImport("kernel32.dll")] static extern bool SetConsoleOutputCP(uint codePage);
    [DllImport("kernel32.dll")] static extern bool GetConsoleScreenBufferInfo(IntPtr output,out ConsoleInfo info);
    [StructLayout(LayoutKind.Sequential)] struct Coordinate {public short X,Y;}
    [StructLayout(LayoutKind.Sequential)] struct ConsoleRect {public short Left,Top,Right,Bottom;}
    [StructLayout(LayoutKind.Sequential)] struct ConsoleInfo {public Coordinate Size,Cursor;public ushort Attributes;public ConsoleRect Window;public Coordinate Maximum;}
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool ReadConsoleW(IntPtr input,[Out] char[] buffer,uint capacity,out uint read,IntPtr reserved);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] static extern IntPtr GetThreadDesktop(uint thread);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool GetUserObjectInformationW(IntPtr handle,int kind,StringBuilder text,int bytes,out int needed);
    static string DesktopName() {
        var text=new StringBuilder(512);int needed;
        if(!GetUserObjectInformationW(GetThreadDesktop(GetCurrentThreadId()),2,text,text.Capacity*2,out needed))throw new InvalidOperationException("Own fixture desktop identity unavailable");
        return text.ToString();
    }
    static readonly object Sync=new object();
    static readonly UTF8Encoding Utf8=new UTF8Encoding(false);
    static readonly JavaScriptSerializer Json=new JavaScriptSerializer();
    static readonly List<byte> Pending=new List<byte>();
    static readonly string[] Labels={"Alpha","Bravo","Charlie"};
    static readonly string[] SingleLabels={"Alpha > 1 \u2192 target","Bravo | literal (2)","Charlie [on|off]"};
    static readonly string SingleSeed="BKS_SINGLE_INPUT \u0416\u754ce\u0301\U0001f600";
    static readonly string RuleSeed="BKS_RULE_SOURCE \u0416\u754ce\u0301\U0001f600";
    static TextWriter output;
    static StreamWriter raw;
    static string root,scene="menu",typed="",last="none";
    static int selected,events,tick,replies,storm,turnNumber,turnRows,bulletNumber,navigationTurn,nativeCaret;
    static bool hold,shellCr,handoff,clearPromptPending,singleInput,tuiTurns,bulletTurns;
    static long escapeAt;
    static int Columns(string text) {
        int width=0;
        for(int i=0;i<text.Length;i++) {
            if(char.IsHighSurrogate(text[i]) && i+1<text.Length && char.IsLowSurrogate(text[i+1])) { width+=2; i++; }
            else if(text[i]=='\u0301'){}
            else width+=text[i]=='\u754c'?2:1;
        }
        return width;
    }
    static void Render(bool reset) {
        if(hold)return;
        if(tuiTurns) {RenderTurns(reset);return;}
        if(scene.StartsWith("native-",StringComparison.Ordinal)) {RenderSingle(reset);return;}
        if(scene=="paint") {
            output.Write("\x1b[?25l"+(reset?"\x1b[2J":"")+"\x1b[H");
            output.Write("BKS_PAINT_READY\x1b[K\r\n\u250c"+new string('\u2500',50)+"\u2510\x1b[K\r\n");
            output.Write("\u2502 \x1b[32mVT canvas only \u0416\u754ce\u0301\U0001f600\x1b[0m                          \u2502\x1b[K\r\n");
            output.Write("\u2502 Stable source; native provider is underneath       \u2502\x1b[K\r\n");
            output.Write("\u2514"+new string('\u2500',50)+"\u2518\x1b[K\r\n");
            output.Write("Spinner: "+new[]{'|','/','-','\\'}[tick%4]+" Tick: "+tick.ToString("D6")+"\x1b[K\x1b[?25l");
        } else if(scene=="selection") {
            output.Write("\x1b[?25l"+(reset?"\x1b[2J":"")+"\x1b[HBKS_UNIFIED_SELECTION_READY\x1b[K\r\n");
            output.Write("Input> \u0416\u754ce\u0301\U0001f600 alpha\x1b[7m \x1b[0mbeta tail\x1b[K\x1b[1;1H\x1b[?25l");
        } else if(scene=="prompt") {
            if(reset)output.Write("\x1b[?25l\x1b[2J\x1b[H\x1b[KBKS_UNIFIED_PROMPT_READY\r\n");
            output.Write("\x1b[2;1H\x1b[2KInput> "+typed+"\x1b[2;"+(8+Columns(typed))+"H\x1b[?25h");
        } else {
            output.Write("\x1b[?25l"+(reset?"\x1b[2J":"")+"\x1b[H");
            output.Write("BKS_UNIFIED_READY \u0416\u754ce\u0301\U0001f600\x1b[K\r\nLiteral test menu; no command execution\x1b[K\r\n");
            for(int row=0;row<Labels.Length;row++)output.Write((row==selected?"> ":"  ")+Labels[row]+"\x1b[K\r\n");
            output.Write("Event: "+events.ToString("D4")+" "+last+"\x1b[K\r\nTick: "+tick.ToString("D6")+"\x1b[K\r\n");
            output.Write("Input> "+typed+"\x1b[K\x1b[8;"+(8+Columns(typed))+"H\x1b[?25h");
        }
        output.Flush();
    }
    static void RenderSingle(bool reset) {
        if(hold)return;
        if(scene=="native-padded" || scene=="native-ruled") {RenderNativeEditor(reset);return;}
        if(scene.StartsWith("native-rule-",StringComparison.Ordinal)) {RenderRule(reset);return;}
        output.Write("\x1b[0m\x1b[?25l"+(reset?"\x1b[2J":"")+"\x1b[H");
        if(scene=="native-unknown") {
            output.Write("BKS_SINGLE_FALLBACK_UNIQUE\x1b[K\r\n> candidate one\x1b[K\r\n> candidate two\x1b[K\x1b[J\x1b[?25l");
            output.Flush();return;
        }
        var rows=new List<string>();
        rows.Add("BKS_SINGLE_OUTPUT_UNIQUE");
        rows.Add("BKS_FAKE_CLI_READY_"+replies.ToString("D3")+" Tick: "+tick.ToString("D6"));
        rows.Add("");
        if(scene=="native-menu") {
            for(int index=0;index<SingleLabels.Length;index++)rows.Add((index==selected?"  \u276f ":"    ")+SingleLabels[index]);
        }
        rows.Add("\u250c"+new string('\u2500',58)+"\u2510");
        int firstComposerRow=rows.Count+1;
        string value=scene=="native-masked"?"NEVER_EXPOSE_SOURCE":typed;
        string[] lines=value.Split(new[]{"\r\n"},StringSplitOptions.None);
        for(int index=0;index<lines.Length;index++) {
            string payload=scene=="native-masked"?"\x1b[8m"+lines[index]+"\x1b[0m":lines[index];
            rows.Add((index==0?"\u2502 > ":"\u2502 ")+payload+new string(' ',Math.Max(0,(index==0?55:57)-Columns(lines[index])))+"\u2502");
        }
        rows.Add("\u2514"+new string('\u2500',58)+"\u2518");
        for(int index=0;index<rows.Count;index++)output.Write(rows[index]+"\x1b[K"+(index+1<rows.Count?"\r\n":""));
        string[] prefix=value.Substring(0,Math.Min(nativeCaret,value.Length)).Split(new[]{"\r\n"},StringSplitOptions.None);
        output.Write("\x1b[J\x1b["+(firstComposerRow+prefix.Length-1)+";"+((prefix.Length==1?5:3)+Columns(prefix[prefix.Length-1]))+"H\x1b[?25h");
        output.Flush();
    }
    static void RenderNativeEditor(bool reset) {
        ConsoleInfo info;int columns=GetConsoleScreenBufferInfo(GetStdHandle(-11),out info)?info.Size.X:100;
        columns=Math.Max(40,Math.Min(1024,columns));
        bool ruled=scene=="native-ruled";
        string value=typed.Length==0 && !ruled?"Generic unentered hint":typed;
        var rows=new List<string>();
        rows.Add("BKS_NATIVE_EDITOR_"+(ruled?"RULED":"PADDED"));
        rows.Add(ruled?new string('\u2500',columns-2):"");
        rows.Add((ruled?"\u276f\u00a0":"\u203a ")+value);
        rows.Add(ruled?new string('\u2500',columns-2):"");
        string footer="source footer";
        rows.Add(new string(' ',Math.Max(2,columns-1-footer.Length))+footer);
        output.Write("\x1b[0m\x1b[?25l"+(reset?"\x1b[2J":"")+"\x1b[H");
        for(int index=0;index<rows.Count;index++)output.Write(rows[index]+"\x1b[K"+(index+1<rows.Count?"\r\n":""));
        output.Write("\x1b[J\x1b[3;"+(3+Columns(typed.Substring(0,Math.Min(nativeCaret,typed.Length))))+"H\x1b[?25h");
        output.Flush();
    }
    static void RenderRule(bool reset) {
        ConsoleInfo info;int columns=GetConsoleScreenBufferInfo(GetStdHandle(-11),out info)?info.Size.X:113;
        columns=Math.Max(40,Math.Min(1024,columns));
        int prefix=Math.Max(12,columns-Columns(typed)-5);
        var rows=new List<string>();
        rows.Add("BKS_SINGLE_OUTPUT_UNIQUE");
        rows.Add("BKS_RULE_LAYOUT_READY Tick: "+tick.ToString("D6"));
        rows.Add("");
        if(scene=="native-rule-menu")for(int index=0;index<SingleLabels.Length;index++)rows.Add((index==selected?"  \u276f ":"    ")+SingleLabels[index]);
        rows.Add(new string('\u2500',prefix)+"\u276f "+typed);
        int composerRow=rows.Count;
        rows.Add(new string('\u2500',columns-2));
        output.Write("\x1b[0m\x1b[?25l"+(reset?"\x1b[2J":"")+"\x1b[H");
        for(int index=0;index<rows.Count;index++)output.Write(rows[index]+"\x1b[K"+(index+1<rows.Count?"\r\n":""));
        output.Write("\x1b[J\x1b["+composerRow+";"+(prefix+3+Columns(typed.Substring(0,Math.Min(nativeCaret,typed.Length))))+"H\x1b[?25h");
        output.Flush();
    }
    static void RenderTurns(bool reset) {
        if(hold)return;
        output.Write("\x1b[0m\x1b[?25l"+(reset?"\x1b[2J\x1b[3J":"")+"\x1b[H");
        var rows=new List<string>();
        rows.Add("BKS_TURN_FRAME_"+turnNumber.ToString("D3")+" "+scene);
        bool answer=scene=="turn-answer" || scene=="turn-answer-more" || scene=="turn-done" || scene=="turn-unarmed-done";
        bool busy=scene=="turn-thinking" || scene=="turn-searching" || scene=="turn-running" || scene=="turn-answer" || scene=="turn-answer-more";
        if(scene!="turn-initial" && scene!="turn-menu")rows.Add("User: q");
        if(scene=="turn-thinking")rows.Add("Thinking: examining the synthetic request");
        if(scene=="turn-searching")rows.Add("Searching: synthetic documentation");
        if(scene=="turn-running")rows.Add("Running: synthetic local fixture tool");
        if(answer) {
            rows.Add("Assistant:");
            string prefix="BKS_TURN_"+turnNumber.ToString("D3")+"_BODY_";
            string color=turnNumber%2==1?"17;187;119":"221;119;51";
            rows.Add("\x1b[1;38;2;"+color+"m"+prefix+"HEAD\x1b[0m");
            for(int index=0;index<turnRows;index++)rows.Add(prefix+"ROW_"+index.ToString("D3"));
            if(scene=="turn-done" || scene=="turn-unarmed-done")rows.Add(prefix+"TAIL");
        }
        if(busy)rows.Add("Status: busy");
        bool ready=scene=="turn-initial" || scene=="turn-menu" || scene=="turn-done" || scene=="turn-unarmed-done";
        if(scene=="turn-done" || scene=="turn-unarmed-done") {
            rows.Add("Status: done");
            rows.Add("Footer: synthetic service summary tick "+tick.ToString("D6"));
        }
        if(scene=="turn-menu")for(int index=0;index<SingleLabels.Length;index++)rows.Add((index==selected?"  \u276f ":"    ")+SingleLabels[index]);
        int composerRow=0;
        if(ready) {
            rows.Add("\u250c"+new string('\u2500',58)+"\u2510");
            rows.Add("\u2502 > "+typed+new string(' ',Math.Max(0,55-Columns(typed)))+"\u2502");
            composerRow=rows.Count;
            rows.Add("\u2514"+new string('\u2500',58)+"\u2518");
        }
        for(int index=0;index<rows.Count;index++)output.Write(rows[index]+"\x1b[K"+(index+1<rows.Count?"\r\n":""));
        output.Write("\x1b[J");
        if(ready)output.Write("\x1b["+composerRow+";"+(5+Columns(typed))+"H\x1b[?25h");
        output.Flush();
    }
    static void TurnEvent(string kind) {
        if(kind.StartsWith("Text:",StringComparison.Ordinal)) {
            typed+=kind.Substring(5);
            if(typed=="/") {scene="turn-menu";selected=0;}
        } else if(scene=="turn-menu") {
            if(kind=="Down")selected=Math.Min(SingleLabels.Length-1,selected+1);
            else if(kind=="Up")selected=Math.Max(0,selected-1);
            else if(kind=="Enter" || kind=="Escape") {scene="turn-initial";typed="";}
        } else if(kind=="Enter" && typed=="q" && (scene=="turn-initial" || scene=="turn-done")) {
            turnNumber++;turnRows=0;typed="";scene="turn-submitted";
        }
        RenderTurns(false);
    }
    static bool TurnRequest(string request) {
        if(request=="turn-initial") {scene=request;typed="";hold=false;RenderTurns(true);return true;}
        if(request=="turn-passive") {tick++;RenderTurns(false);return true;}
        if(request=="turn-unarmed-done") {scene=request;typed="";turnRows=2;RenderTurns(false);return true;}
        if(turnNumber<1 || (request!="turn-thinking" && request!="turn-searching" && request!="turn-running" && request!="turn-answer" && request!="turn-answer-more" && request!="turn-done"))return false;
        scene=request;typed="";turnRows=request=="turn-answer"?2:request=="turn-answer-more" || request=="turn-done"?12:0;
        RenderTurns(false);return true;
    }
    static void RenderBulletTurns(bool reset,bool clearHistory) {
        if(hold)return;
        ConsoleInfo info;int columns=GetConsoleScreenBufferInfo(GetStdHandle(-11),out info)?info.Size.X:113;
        columns=Math.Max(60,Math.Min(1024,columns));
        bool active=scene!="bullet-initial",done=scene=="bullet-done";
        bool answer=scene=="bullet-answer" || scene=="bullet-answer-more" || done;
        bool composer=!active || done || bulletNumber==2;
        var frame=new StringBuilder("\x1b[0m\x1b[?25l");
        if(reset)frame.Append("\x1b[2J").Append(clearHistory?"\x1b[3J":"");
        frame.Append("\x1b[1;1HBKS_BULLET_SOURCE_HEADER\x1b[K");
        for(int row=2;row<=17;row++)frame.Append("\x1b[").Append(row).Append(";1H\x1b[2K");
        if(active)frame.Append("\x1b[3;1H\u276f q");
        if(scene=="bullet-thinking" || answer)frame.Append("\x1b[4;1H\x1b[2;3m\u25cf BKS_BULLET_REASON_").Append(bulletNumber.ToString("D3")).Append(" inspecting generic visible input with a logical paragraph that wraps across physical columns; Unicode \u0416\u754ce\u0301\U0001f600\x1b[0m");
        if(answer) {
            string prefix="BKS_BULLET_"+bulletNumber.ToString("D3")+"_BODY_";
            string color=bulletNumber==1?"17;187;119":"221;119;51";
            frame.Append("\x1b[7;1H\u25cf \x1b[1;38;2;").Append(color).Append('m').Append(prefix).Append("HEAD\x1b[0m");
            int count=scene=="bullet-answer"?2:6;
            for(int row=0;row<count;row++)frame.Append("\x1b[").Append(8+row).Append(";1H").Append(prefix).Append("ROW_").Append(row.ToString("D3"));
            if(done)frame.Append("\x1b[14;1H\x1b[1;38;2;").Append(color).Append('m').Append(prefix).Append("TAIL\x1b[0m");
        }
        // Fixed physical footer; new source text grows above it, with no role
        // labels or a Status: done signal. The second busy turn keeps input.
        frame.Append("\x1b[18;1H\x1b[2Kmodel: local \u00b7 thinking: high");
        if(active && !done)frame.Append("\x1b[18;").Append(columns-17).Append("Hctrl+c: cancel");
        frame.Append("\x1b[19;1H\x1b[2Kcontext: 2%");
        for(int row=20;row<=22;row++)frame.Append("\x1b[").Append(row).Append(";1H\x1b[2K");
        if(composer) {
            frame.Append("\x1b[20;1H\u250c").Append(new string('\u2500',58)).Append("\u2510\r\n\u2502 > ").Append(typed).Append(new string(' ',Math.Max(0,55-Columns(typed)))).Append("\u2502\r\n\u2514").Append(new string('\u2500',58)).Append("\u2518\x1b[21;").Append(5+Columns(typed)).Append("H\x1b[?25h");
        }
        output.Write(frame.ToString());output.Flush();
    }
    static void BulletEvent(string kind) {
        if(kind.StartsWith("Text:",StringComparison.Ordinal))typed+=kind.Substring(5);
        else if(kind=="Enter" && typed=="q" && (scene=="bullet-initial" || scene=="bullet-done")) {
            bulletNumber++;typed="";scene="bullet-submitted";
        }
        RenderBulletTurns(false,false);
    }
    static bool BulletRequest(string request) {
        if(request=="bullet-initial") {bulletTurns=true;bulletNumber=0;scene=request;typed="";hold=false;RenderBulletTurns(true,true);return true;}
        if(!bulletTurns)return false;
        if(request=="bullet-reset") {scene="bullet-initial";typed="";RenderBulletTurns(true,false);return true;}
        if(request=="bullet-passive") {RenderBulletTurns(false,false);return true;}
        if(bulletNumber<1 || (request!="bullet-thinking" && request!="bullet-answer" && request!="bullet-answer-more" && request!="bullet-done"))return false;
        scene=request;typed="";RenderBulletTurns(false,false);return true;
    }
    static void Event(string kind) {
        last=kind; events++;
        if(tuiTurns && bulletTurns) {BulletEvent(kind);return;}
        if(tuiTurns) {TurnEvent(kind);return;}
        if(scene=="shell") {
            if(kind.StartsWith("Text:",StringComparison.Ordinal)) {typed+=kind.Substring(5);output.Write(kind.Substring(5));}
            else if(kind=="Enter") {
                replies++;
                if(handoff && typed=="kimi") {
                    scene=singleInput?"native-prompt":"fake-cli";typed=singleInput?SingleSeed:"";nativeCaret=typed.Length;
                    output.Write("\x1b]133;C\x07");
                    if(singleInput)RenderSingle(true);
                    else output.Write("\r\nBKS_FAKE_CLI_READY_"+replies.ToString("D3")+"\r\nManaged fake CLI; no executable lookup or command execution\r\nFAKE CLI> ");
                } else if(handoff && typed=="BKS_OUTPUT_PROBE") {
                    typed="";
                    output.Write("\x1b]133;C\x07\r\nBKS_SHELL_OUTPUT_BEGIN\r\n");
                    for(int row=0;row<128;row++)output.Write("BKS_SHELL_OUTPUT_ROW_"+row.ToString("D3")+" \x1b[32m\u0416\u754ce\u0301\U0001f600\x1b[0m\r\n");
                    output.Write("BKS_SHELL_OUTPUT_TAIL_UNIQUE\r\n\x1b]133;D;0\u0007\x1b]133;A\u0007fake> \x1b]133;B\u0007");
                } else if(singleInput && (typed=="ls" || typed=="docker ps")) {
                    typed="";navigationTurn++;
                    for(int blank=0;blank<navigationTurn+2;blank++)output.Write("\r\n");
                    output.Write("    BKS_NAV_OUTPUT_"+navigationTurn.ToString("D3")+" \u0416\U0001f600\r\n");
                    // Directory-like output must scroll the real console;
                    // a tiny frame can serialize only CUP/EL screen rewrites.
                    for(int row=0;row<80;row++)output.Write("BKS_NAV_ROW_"+navigationTurn.ToString("D3")+"_"+row.ToString("D3")+"\r\n");
                    output.Write("BKS_NAV_TAIL_"+navigationTurn.ToString("D3")+"\r\n");
                    output.Write("\x1b]133;A\u0007fake> \x1b]133;B\u0007");
                } else if(handoff && typed=="BKS_CLEAR_PROBE") {
                    typed="";clearPromptPending=true;
                    // Deliberately separate reads: the native journal must be
                    // reset even when the completed clear produces no text.
                    output.Write("\x1b[");output.Flush();Thread.Sleep(80);
                    output.Write("2J\x1b[");output.Flush();Thread.Sleep(80);
                    output.Write("3J\x1b[H");
                } else {typed="";output.Write("\r\nBKS_FAKE_RESULT_"+replies.ToString("D3")+"\r\nfake> ");}
            }
            output.Flush();return;
        }
        if(scene=="fake-cli")return;
        if(scene.StartsWith("native-",StringComparison.Ordinal)) {
            bool ruleScene=scene.StartsWith("native-rule-",StringComparison.Ordinal);
            bool editorScene=scene=="native-padded" || scene=="native-ruled";
            if(kind=="Down")selected=Math.Min(SingleLabels.Length-1,selected+1);
            else if(kind=="Up")selected=Math.Max(0,selected-1);
            else if(kind=="Left")nativeCaret=PreviousNative(typed,nativeCaret);
            else if(kind=="Right")nativeCaret=NextNative(typed,nativeCaret);
            else if(kind=="Home")nativeCaret=0;
            else if(kind=="End")nativeCaret=typed.Length;
            else if(kind=="Backspace" && nativeCaret>0) {
                int start=PreviousNative(typed,nativeCaret);
                typed=typed.Remove(start,nativeCaret-start);nativeCaret=start;
            } else if(kind=="Delete" && nativeCaret<typed.Length)typed=typed.Remove(nativeCaret,NextNative(typed,nativeCaret)-nativeCaret);
            else if(kind.StartsWith("Text:",StringComparison.Ordinal)) {
                string inserted=kind.Substring(5);typed=typed.Insert(nativeCaret,inserted);nativeCaret+=inserted.Length;
                if(scene=="native-prompt" && kind=="Text:/") {scene="native-menu";selected=0;}
                if(scene=="native-rule-prompt" && kind=="Text:/") {scene="native-rule-menu";selected=0;}
            } else if(kind=="Enter" || kind=="Escape") {if(!editorScene)scene=ruleScene?"native-rule-prompt":"native-prompt";if(kind=="Enter") {typed="";nativeCaret=0;}}
            RenderSingle(false);return;
        }
        if(kind=="Up")selected=Math.Max(0,selected-1);
        if(kind=="Down")selected=Math.Min(Labels.Length-1,selected+1);
        if(kind.StartsWith("Text:",StringComparison.Ordinal))typed+=kind.Substring(5);
        if(scene=="prompt" && kind=="Enter" && !hold) {
            replies++;
            output.Write("\r\nBKS_UNIFIED_NEW_OUTPUT_"+replies.ToString("D3")+" \u0416\U0001f600\r\nNEXT> ");
            output.Flush();
        } else Render(false);
    }
    static bool Drain(bool expire) {
        bool changed=false;
        while(Pending.Count>0) {
            if((scene=="shell" || handoff) && Pending[0]==10 && shellCr) {Pending.RemoveAt(0);shellCr=false;changed=true;continue;}
            if(scene=="shell")shellCr=Pending[0]==13;
            int length=1; string kind;
            if(Pending[0]==27) {
                if(Pending.Count>1 && (Pending[1]=='[' || Pending[1]=='O')) {
                    int end=-1;
                    for(int i=2;i<Pending.Count;i++)if((Pending[i]>='A' && Pending[i]<='Z') || (Pending[i]>='a' && Pending[i]<='z') || Pending[i]=='~') {end=i;break;}
                    if(end<0 && !expire)break;
                    if(end>=0) {
                        length=end+1; string sequence=Encoding.ASCII.GetString(Pending.GetRange(0,length).ToArray());
                        char final=(char)Pending[end];
                        kind=final=='A'?"Up":final=='B'?"Down":final=='C'?"Right":final=='D'?"Left":final=='H'?"Home":final=='F'?"End":final=='Z'?"ShiftTab":sequence=="\x1b[3~"?"Delete":sequence;
                        if(sequence.Contains(";5"))kind="Ctrl"+kind;
                    } else kind="Escape";
                } else if(!expire)break; else kind="Escape";
            } else if(Pending[0]==13 || Pending[0]==10)kind="Enter";
            else if(Pending[0]==9)kind="Tab";
            else if(singleInput && Pending[0]==127)kind="Backspace";
            else {
                byte first=Pending[0]; length=first<0x80?1:first<0xe0?2:first<0xf0?3:4;
                if(Pending.Count<length)break;
                kind="Text:"+Utf8.GetString(Pending.GetRange(0,length).ToArray());
            }
            Pending.RemoveRange(0,length); Event(kind); changed=true;
        }
        return changed;
    }
    static void ReadInput() {
        try {
            // ReadFile/ReadConsoleA CP65001 can replace individual surrogate
            // KEY_EVENTs before the fixture sees them. Keep the console UTF16
            // boundary explicit and retain encoder state across split reads.
            var input=GetStdHandle(-10);var encoder=Utf8.GetEncoder();
            char[] characters=new char[256];byte[] bytes=new byte[1024];uint read;
            while(ReadConsoleW(input,characters,(uint)characters.Length,out read,IntPtr.Zero) && read>0)lock(Sync) {
                int count=encoder.GetBytes(characters,0,(int)read,bytes,0,false);
                if(count==0)continue;
                byte[] chunk=new byte[count]; Array.Copy(bytes,chunk,count);
                raw.WriteLine(Json.Serialize(new {Bytes=BitConverter.ToString(chunk).Replace("-",""),Count=count})); raw.Flush();
                Pending.AddRange(chunk);escapeAt=Stopwatch.GetTimestamp();Drain(false);
            }
        } catch(IOException){}
    }
    static int PreviousNative(string value,int at) {
        if(at<=0)return 0;
        return at>1 && ((char.IsLowSurrogate(value[at-1]) && char.IsHighSurrogate(value[at-2])) || (value[at-1]=='\n' && value[at-2]=='\r'))?at-2:at-1;
    }
    static int NextNative(string value,int at) {
        if(at>=value.Length)return value.Length;
        return at+1<value.Length && ((char.IsHighSurrogate(value[at]) && char.IsLowSurrogate(value[at+1])) || (value[at]=='\r' && value[at+1]=='\n'))?at+2:at+1;
    }
    static bool Request(string request) {
        if(tuiTurns && request.StartsWith("bullet-",StringComparison.Ordinal))return BulletRequest(request);
        if(tuiTurns && request.StartsWith("turn-",StringComparison.Ordinal))return TurnRequest(request);
        if(request=="cli-exit" && handoff && (scene=="fake-cli" || (singleInput && scene.StartsWith("native-",StringComparison.Ordinal)))) {
            scene="shell";typed="";shellCr=false;
            output.Write("\r\nBKS_FAKE_CLI_DONE_"+replies.ToString("D3")+"\r\n\x1b]133;A\u0007fake> \x1b]133;B\u0007");output.Flush();
        }
        else if(singleInput && request=="single-shell") {
            scene="shell";typed="";hold=false;shellCr=false;
            output.Write("\x1b[0m\x1b[?25h\r\nBKS_SINGLE_SHELL_READY\r\n\x1b]133;A\u0007fake> \x1b]133;B\u0007");output.Flush();
        }
        else if(singleInput && (request=="native-padded" || request=="native-ruled")) {
            scene=request;typed="";nativeCaret=0;selected=0;hold=false;RenderSingle(true);
        }
        else if(singleInput && (request=="native-prompt" || request=="native-multiline" || request=="native-menu" || request=="native-unknown" || request=="native-masked" || request=="native-rule-prompt" || request=="native-rule-menu" || request=="native-selection")) {
            scene=request;typed=request=="native-selection"?"A\u0416\U0001f600\r\n\u754cZ":request.StartsWith("native-rule-",StringComparison.Ordinal)?RuleSeed:SingleSeed+(request=="native-multiline"?"\r\nBKS_SINGLE_SECOND_ROW":"");nativeCaret=typed.Length;selected=0;hold=false;RenderSingle(true);
        }
        else if(request=="shell-follow" && handoff && scene=="shell") {output.Write("\r\nBKS_SHELL_FOLLOW_TAIL \u0416\U0001f600\r\n");output.Flush();}
        else if(request=="shell-passive-history" && handoff && scene=="shell") {output.Write("BKS_SHELL_PASSIVE_TAIL\r\n");output.Flush();}
        else if(request=="shell-clear-prompt" && handoff && scene=="shell" && clearPromptPending) {
            clearPromptPending=false;
            output.Write("BKS_SHELL_CLEARED_READY\r\n\x1b]133;A\u0007fake-cleared> \x1b]133;B\u0007");output.Flush();
        }
        else if(request=="shell") {scene="shell";typed="";hold=false;shellCr=false;output.Write("\x1b[?25h\r\nBKS_SHELL_READY\r\nfake> ");output.Flush();}
        else if(request=="menu" || request=="prompt" || request=="paint" || request=="selection") {scene=request;typed="";selected=0;hold=false;Render(true);}
        else if(request=="hold")hold=true;
        else if(request=="release") {hold=false;Render(false);}
        else if(request=="passive-hidden" && singleInput) {tick++;Render(false);output.Write("\x1b[?25l");output.Flush();}
        else if(request=="passive") {tick++;Render(false);}
        else if(request=="storm") {storm=100;tick++;Render(false);}
        else return false;
        return true;
    }
    static int SelfTest() {
        output=new StringWriter();
        Pending.AddRange(new byte[]{27,91,66});if(!Drain(false)||selected!=1)return 2;
        Pending.AddRange(new byte[]{27,91,49,59,53,65});if(!Drain(false)||last!="CtrlUp")return 3;
        Pending.Add(27);if(Drain(false)||!Drain(true)||last!="Escape")return 4;
        Request("prompt");Pending.AddRange(Utf8.GetBytes("\u0416\U0001f600"));if(!Drain(false)||Columns(typed)!=3)return 5;
        int before=output.ToString().Length;Request("hold");Pending.Add((byte)'x');Drain(false);if(output.ToString().Length!=before)return 6;
        Request("release");if(!output.ToString().Contains("Input> \u0416\U0001f600x"))return 7;
        Pending.Add(13);Drain(false);if(!output.ToString().Contains("BKS_UNIFIED_NEW_OUTPUT_001"))return 8;
        var encoder=Utf8.GetEncoder();byte[] encoded=new byte[8];
        if(encoder.GetBytes(new[]{(char)0xd83d},0,1,encoded,0,false)!=0)return 9;
        if(encoder.GetBytes(new[]{(char)0xde00},0,1,encoded,0,false)!=4 || BitConverter.ToString(encoded,0,4)!="F0-9F-98-80")return 9;
        Request("shell");Event("Text:literal");Pending.AddRange(new byte[]{13,10});Drain(false);if(replies!=2 || !output.ToString().Contains("literal\r\nBKS_FAKE_RESULT_002"))return 9;
        handoff=true;replies=0;typed="";output=new StringWriter();
        Pending.AddRange(Utf8.GetBytes("kimi\r\n"));Drain(false);
        if(scene!="fake-cli" || replies!=1 || !output.ToString().Contains("BKS_FAKE_CLI_READY_001"))return 12;
        if(!Request("cli-exit") || scene!="shell" || !output.ToString().Contains("\x1b]133;A"+(char)7+"fake> \x1b]133;B"+(char)7))return 13;
        output=new StringWriter();Pending.AddRange(Utf8.GetBytes("BKS_OUTPUT_PROBE\r\n"));Drain(false);
        string shellOutput=output.ToString();
        if(shellOutput.Split(new[]{"BKS_SHELL_OUTPUT_ROW_"},StringSplitOptions.None).Length-1!=128 ||
            !shellOutput.Contains("\x1b[32m\u0416\u754ce\u0301\U0001f600\x1b[0m") || !shellOutput.Contains("BKS_SHELL_OUTPUT_TAIL_UNIQUE"))return 18;
        if(!Request("shell-follow") || !Request("shell-passive-history") || !output.ToString().Contains("BKS_SHELL_PASSIVE_TAIL"))return 19;
        Pending.AddRange(Utf8.GetBytes("BKS_CLEAR_PROBE\r\n"));Drain(false);
        if(!clearPromptPending || !output.ToString().EndsWith("\x1b[2J\x1b[3J\x1b[H",StringComparison.Ordinal) || output.ToString().Contains("BKS_SHELL_CLEARED_READY"))return 20;
        if(!Request("shell-clear-prompt") || clearPromptPending || !output.ToString().Contains("BKS_SHELL_CLEARED_READY\r\n\x1b]133;A"+(char)7+"fake-cleared> \x1b]133;B"+(char)7))return 21;
        Request("paint");if(!output.ToString().Contains("BKS_PAINT_READY") || !output.ToString().EndsWith("\x1b[?25l",StringComparison.Ordinal))return 14;
        string painted=output.ToString();Request("hold");Request("passive");if(output.ToString()!=painted)return 15;
        Request("release");if(output.ToString()==painted || !output.ToString().Contains("Spinner: / Tick:"))return 16;
        Request("selection");if(!output.ToString().Contains("Input> \u0416\u754ce\u0301\U0001f600 alpha\x1b[7m \x1b[0mbeta tail") ||
            !output.ToString().EndsWith("\x1b[1;1H\x1b[?25l",StringComparison.Ordinal))return 17;
        singleInput=true;output=new StringWriter();Request("native-prompt");
        if(!output.ToString().Contains("BKS_SINGLE_OUTPUT_UNIQUE") || !output.ToString().Contains("\u2502 > "+SingleSeed) || !output.ToString().EndsWith("\x1b[?25h",StringComparison.Ordinal))return 22;
        string nativeBefore=output.ToString();Request("hold");Pending.AddRange(Utf8.GetBytes("/"));Drain(false);
        if(scene!="native-menu" || output.ToString()!=nativeBefore)return 23;
        Request("release");if(!output.ToString().Contains("  \u276f "+SingleLabels[0]))return 24;
        nativeBefore=output.ToString();Request("hold");Pending.AddRange(new byte[]{27,91,66});Drain(false);
        if(selected!=1 || output.ToString()!=nativeBefore)return 25;
        Request("release");if(!output.ToString().Contains("  \u276f "+SingleLabels[1]))return 26;
        Pending.Add(13);Drain(false);if(scene!="native-prompt" || typed!="")return 27;
        Request("native-unknown");if(!output.ToString().EndsWith("\x1b[?25l",StringComparison.Ordinal) || !output.ToString().Contains("> candidate two"))return 28;
        Request("native-masked");if(!output.ToString().Contains("\x1b[8mNEVER_EXPOSE_SOURCE\x1b[0m"))return 29;
        Request("native-multiline");if(!output.ToString().Contains("\u2502 BKS_SINGLE_SECOND_ROW") || typed!=SingleSeed+"\r\nBKS_SINGLE_SECOND_ROW")return 32;
        output=new StringWriter();Request("native-rule-prompt");
        if(!output.ToString().Contains("\u2500\u276f "+RuleSeed) || output.ToString().Contains("\u2502"))return 34;
        Pending.Add((byte)'/');Drain(false);if(scene!="native-rule-menu" || !output.ToString().Contains("  \u276f "+SingleLabels[0]))return 35;
        Pending.AddRange(new byte[]{27,91,66});Drain(false);if(selected!=1)return 36;
        Pending.Add(27);Drain(true);if(scene!="native-rule-prompt" || typed!=RuleSeed+"/")return 37;
        Request("native-selection");
        Pending.AddRange(new byte[]{27,91,68,127,127,127,127});Drain(false);
        if(typed!="AZ" || nativeCaret!=1)return 38;
        Event("Text:Q");Pending.AddRange(new byte[]{27,91,51,126});Drain(false);
        if(typed!="AQ" || nativeCaret!=2 || PreviousNative("\U0001f600\r\n",4)!=2 || NextNative("\U0001f600\r\n",0)!=2)return 39;
        if(!Request("single-shell") || scene!="shell" || !output.ToString().Contains("BKS_SINGLE_SHELL_READY"))return 30;
        Pending.AddRange(Utf8.GetBytes("kimi\r\n"));Drain(false);
        if(scene!="native-prompt" || !Request("cli-exit") || scene!="shell")return 31;
        Console.WriteLine("unified fixture parser, confirmed echo, shell output/follow, split clear-only/prompt and single-field source/menu/masked-frame self-test passed");return 0;
    }
    static int TurnsSelfTest() {
        tuiTurns=true;scene="turn-initial";output=new StringWriter();RenderTurns(true);
        if(!output.ToString().Contains("\u2502 > ") || !output.ToString().Contains("\x1b[2J\x1b[3J"))return 40;
        Event("Text:/");if(scene!="turn-menu" || !output.ToString().Contains(SingleLabels[0]))return 41;
        Event("Enter");if(turnNumber!=0 || scene!="turn-initial")return 42;
        for(int request=0;request<2;request++) {
            if(!TurnRequest("turn-initial"))return 43;
            Pending.AddRange(new byte[]{(byte)'q',13});Drain(false);
            if(turnNumber!=request+1 || scene!="turn-submitted" || typed!="")return 44;
            foreach(string phase in new[]{"turn-thinking","turn-searching","turn-running","turn-answer","turn-answer-more"}) {
                output=new StringWriter();if(!TurnRequest(phase))return 45;
                string frame=output.ToString();if(!frame.Contains("Status: busy") || frame.Contains("\u2502 > ") || !frame.EndsWith("\x1b[J",StringComparison.Ordinal))return 46;
                if(phase=="turn-answer-more" && (!frame.Contains("Assistant:") || !frame.Contains("\x1b[1;38;2;") || !frame.Contains("_BODY_ROW_011")))return 47;
            }
            output=new StringWriter();if(!TurnRequest("turn-done"))return 48;
            string done=output.ToString();if(!done.Contains("Status: done") || done.Contains("Status: busy") || !done.Contains("_BODY_TAIL") || !done.Contains("\u2502 > ") || !done.EndsWith("\x1b[?25h",StringComparison.Ordinal))return 49;
        }
        if(!BulletRequest("bullet-initial"))return 50;
        for(int request=1;request<=2;request++) {
            Pending.AddRange(new byte[]{(byte)'q',13});Drain(false);
            if(bulletNumber!=request || scene!="bullet-submitted" || typed!="")return 51;
            foreach(string phase in new[]{"bullet-thinking","bullet-answer","bullet-answer-more"}) {
                output=new StringWriter();if(!BulletRequest(phase))return 52;
                string frame=output.ToString();
                if(!frame.Contains("\x1b[18;1H\x1b[2Kmodel: local \u00b7 thinking: high") || !frame.Contains("ctrl+c: cancel") || !frame.Contains("\x1b[2;3m\u25cf BKS_BULLET_REASON_") || frame.Contains("Assistant:") || frame.Contains("Status:"))return 53;
                if(frame.Contains("\u2502 > ")!=(request==2))return 54;
                if(phase=="bullet-answer-more" && (!frame.Contains("_BODY_ROW_005") || !frame.Contains("\x1b[7;1H\u25cf \x1b[1;38;2;")))return 55;
            }
            output=new StringWriter();if(!BulletRequest("bullet-done"))return 56;
            string done=output.ToString();if(done.Contains("ctrl+c: cancel") || done.Contains("Status:") || !done.Contains("_BODY_TAIL") || !done.Contains("\u2502 > ") || !done.EndsWith("\x1b[?25h",StringComparison.Ordinal))return 57;
            if(request==1) {output=new StringWriter();if(!BulletRequest("bullet-reset") || !output.ToString().Contains("\x1b[2J") || output.ToString().Contains("\x1b[3J"))return 58;}
        }
        Console.WriteLine("controlled generic turns and bullet realism: fixed footer, inserted styled reasoning/answers, cancellation control removal, two q+Enter, persistent second input and ED2 retention self-tests passed");return 0;
    }
    public static int Main(string[] args) {
        if(args.Length==1 && args[0]=="--desktop-probe") {
            string probeRoot=Environment.GetEnvironmentVariable("BKS_QA_PROBE");
            string probeDesktop=DesktopName();
            File.WriteAllText(Path.Combine(probeRoot,"desktop.json"),Json.Serialize(new {Pid=Process.GetCurrentProcess().Id,Executable=Process.GetCurrentProcess().MainModule.FileName,Desktop=probeDesktop,Cwd=Environment.CurrentDirectory,AppData=Environment.GetEnvironmentVariable("APPDATA"),Unicode=Environment.GetEnvironmentVariable("BKS_QA_UNICODE")}),Utf8);
            return probeDesktop==Environment.GetEnvironmentVariable("BKS_QA_DESKTOP")?0:33;
        }
        if(args.Length==1 && args[0]=="--self-test")return SelfTest();
        if(args.Length==1 && args[0]=="--tui-turns-self-test")return TurnsSelfTest();
        if(args.Length!=1 || (args[0]!="--session" && args[0]!="--handoff-session" && args[0]!="--single-input-session" && args[0]!="--tui-turns-session"))return 10;
        singleInput=args[0]=="--single-input-session";
        tuiTurns=args[0]=="--tui-turns-session";
        handoff=args[0]=="--handoff-session" || singleInput;
        root=Path.Combine(Environment.GetEnvironmentVariable("APPDATA"),"BKSUnifiedFixture");Directory.CreateDirectory(root);
        uint im,om;var ih=GetStdHandle(-10);var oh=GetStdHandle(-11);
        if(!GetConsoleMode(ih,out im)||!SetConsoleMode(ih,(im&~7u)|0x200u)||!GetConsoleMode(oh,out om)||!SetConsoleMode(oh,om|4u))return 11;
        SetConsoleCP(65001);SetConsoleOutputCP(65001);
        output=new CapturingWriter(new StreamWriter(Console.OpenStandardOutput(),Utf8){AutoFlush=true},Path.Combine(root,"fixture-output.vt"));
        raw=new StreamWriter(new FileStream(Path.Combine(root,"input.jsonl"),FileMode.Create,FileAccess.Write,FileShare.ReadWrite),Utf8){AutoFlush=true};
        string identity=Path.Combine(root,"identity.json");
        string desktop=singleInput || tuiTurns?DesktopName():null;
        if((singleInput || tuiTurns) && desktop!=Environment.GetEnvironmentVariable("BKS_QA_DESKTOP"))return 33;
        File.WriteAllText(identity+".pending",Json.Serialize(new {Pid=Process.GetCurrentProcess().Id,Executable=Process.GetCurrentProcess().MainModule.FileName,Cwd=Environment.CurrentDirectory,Desktop=desktop}),Utf8);File.Move(identity+".pending",identity);
        if(tuiTurns) {scene="turn-initial";RenderTurns(true);}
        else if(handoff) {
            scene="shell";
            for(int i=0;i<128;i++)output.Write("BKS_HANDOFF_HISTORY_"+i.ToString("D3")+"\r\n");
            output.Write("BKS_FAKE_SHELL_READY\r\n\x1b]133;A\u0007fake> \x1b]133;B\u0007");output.Flush();
        } else {for(int i=0;i<60;i++)output.Write("BKS_UNIFIED_HISTORY_"+i.ToString("D3")+"\r\n");Render(true);}
        new Thread(ReadInput){IsBackground=true}.Start();
        while(!File.Exists(Path.Combine(root,"exit"))) {
            lock(Sync) {
                if(Pending.Count>0 && (Stopwatch.GetTimestamp()-escapeAt)*1000/Stopwatch.Frequency>=80)Drain(true);
                string request=Path.Combine(root,"scene.request");
                if(File.Exists(request)) {string value=File.ReadAllText(request,Utf8).Trim();if(Request(value)){File.Delete(request);File.WriteAllText(Path.Combine(root,"scene.done"),value,Utf8);}}
                if(storm>0){tick++;Render(false);storm--;if(storm==0)File.WriteAllText(Path.Combine(root,"storm.done"),"done",Utf8);}
            }
            Thread.Sleep(20);
        }
        output.Write("\x1b[?25h\r\nBKS_UNIFIED_FIXTURE_EXIT\r\n");output.Flush();return 0;
    }
}
'@

function Focus-Own([IntPtr]$Window,[switch]$ForceUia,[IntPtr]$Root=[IntPtr]::Zero) {
    if($Root -eq [IntPtr]::Zero) { $Root=$script:Case.Main }
    [BksStage7]::Own($script:Case.Process.Id,$Window)
    if([BksDirect]::ForegroundProcess() -ne $script:Case.Process.Id) { throw 'Environment foreground belongs to another process; focus will not be forced' }
    if($ForceUia -or [BksDirect]::Focus($script:Case.Process.Id,$Window) -ne $Window) { (Element $Window).SetFocus() }
    Wait-For { [BksDirect]::GetAncestor([BksDirect]::GetForegroundWindow(),2) -eq $Root -and [BksDirect]::Focus($script:Case.Process.Id,$Window) -eq $Window -and (Element $Window).Current.HasKeyboardFocus } 'owned native/UIA focus'
}
function Chord([IntPtr]$Window,[int]$Key,[scriptblock]$Until,[string]$Name,[bool]$Control=$true,[bool]$Shift=$true,[IntPtr]$Root=[IntPtr]::Zero) {
    if($Root -eq [IntPtr]::Zero) { $Root=$script:Case.Main }
    $lease=[BksUnified]::BeginChord($script:Case.Process.Id,$Window,$Key,$Control,$Shift,$Root)
    try { Wait-For $Until $Name } finally { $lease.Dispose() }
}
function Assert-ExpectedInput([string]$Expected,[string]$Phase) {
    $actual=Raw-Input
    if($actual -cne $Expected) {
        $observations.Add(@{Case='unexpected fixture bytes; attribution requires boundary diagnostics';Phase=$Phase;ExpectedRawInput=$Expected;ActualRawInput=$actual})
        throw ("Unexpected fixture bytes at ${Phase}: expected=$Expected actual=$actual")
    }
}
function Start-Case {
    $directory=Join-Path $runDirectory 'unified'
    $appData=Join-Path $directory 'appdata'
    $configuration=Join-Path $appData 'BKS Term'
    $task=Join-Path $appData 'BKSUnifiedFixture'
    $bin=Join-Path $directory 'bin'
    foreach($path in @($configuration,$task,$bin)) { $null=[IO.Directory]::CreateDirectory($path) }
    $mock=Join-Path $bin 'unified-fixture.exe'
    Copy-Item -LiteralPath $templateExe -Destination $mock
    $fixtureArgument=if($TuiTurns){'--tui-turns-session'}elseif($SingleInput){'--single-input-session'}elseif($CliHandoff){'--handoff-session'}else{'--session'}
    $fixtureProfiles=@(@{id='unified-fixture';name='Unified fixture';kind='local';executable=$mock;args=@($fixtureArgument);working_directory=$directory})
    if($SingleInput) {$fixtureProfiles+=@{id='windows-cmd';name='CMD fixture shell';kind='local';executable=(Join-Path $env:SystemRoot 'System32/cmd.exe');args=@('/D','/Q');working_directory=$directory}}
    Write-BksJson (Join-Path $configuration 'profiles.json') @{schema_version=1;default_profile_id='unified-fixture';profiles=$fixtureProfiles}
    $privateSettings=@{speech_mode='important';last_tab='keep_window';remember_workspace=$false;shortcuts=@{}}
    if($SingleInput) {
        # Private aliases exercise the same AppAction routes as Ctrl+arrows
        # without changing keyboard state or injecting global Ctrl input.
        $privateSettings.shortcuts=@{previous_block=@('Ctrl+Up','Alt+Up','F7');next_block=@('Ctrl+Down','Alt+Down','F8')}
    }
    if($TuiTurns) {$privateSettings.answer_ready_sound=$false;$privateSettings.auto_read_answers=$false}
    Write-BksJson (Join-Path $configuration 'settings.json') $privateSettings
    $start=New-Object Diagnostics.ProcessStartInfo($Exe)
    $start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.WorkingDirectory=$directory;$start.Arguments='--profile unified-fixture'
    $start.EnvironmentVariables.Clear()
    foreach($entry in @{'SystemRoot'=$env:SystemRoot;'WINDIR'=$env:SystemRoot;'COMSPEC'=(Join-Path $env:SystemRoot 'System32\cmd.exe');'APPDATA'=$appData;'LOCALAPPDATA'=(Join-Path $directory 'local');'USERPROFILE'=(Join-Path $directory 'home');'HOME'=(Join-Path $directory 'home');'TEMP'=(Join-Path $directory 'temp');'TMP'=(Join-Path $directory 'temp');'CODEX_HOME'=(Join-Path $directory 'codex');'PATH'=($bin+';'+$env:SystemRoot+'\System32')}.GetEnumerator()) { $start.EnvironmentVariables[$entry.Key]=$entry.Value }
    foreach($name in @('local','home','temp','codex')) { $null=[IO.Directory]::CreateDirectory((Join-Path $directory $name)) }
    if($privateScenario) {
        $start.EnvironmentVariables['BKS_QA_DESKTOP']=$PrivateDesktopName
        $process=[BksQaDesktop]::StartApplication($start)
    } else { $process=[Diagnostics.Process]::Start($start) }
    $script:Case=@{Process=$process;Main=[IntPtr]::Zero;Task=$task;Mock=$mock;Fixture=$null;AppData=$appData}
    Wait-For { [BksStage7]::MainWindow($process.Id) -ne [IntPtr]::Zero } 'own unified main window'
    $script:Case.Main=[BksStage7]::MainWindow($process.Id)
    if($privateScenario) {Check 'owned BKS native window stays on the private startup desktop' ([BksQaDesktop]::WindowDesktop($process.Id,$script:Case.Main) -ceq $PrivateDesktopName)}
    Wait-For { [IO.File]::Exists((Join-Path $task 'identity.json')) } 'own fixture identity'
    $identity=Read-BksJson (Join-Path $task 'identity.json')
    $fixture=[Diagnostics.Process]::GetProcessById($identity.Pid)
    $null=$fixture.Handle
    if($identity.Executable -ine $mock -or $fixture.MainModule.FileName -ine $mock) { $fixture.Dispose();throw 'Unknown fixture identity refused' }
    $script:Case.Fixture=$fixture
    if($privateScenario) {
        Check 'managed ConPTY fixture inherits the private startup desktop' ($identity.Desktop -ceq $PrivateDesktopName -and $identity.Cwd -ieq $directory)
        $observations.Add(@{Case='private desktop runtime';Desktop=$PrivateDesktopName;CallerDesktop=[BksQaDesktop]::CurrentDesktop;UiaWorkerDesktop=[BksQaDesktop]::WorkerDesktop;FixtureDesktop=$identity.Desktop;ForegroundHwnd=[BksDirect]::GetForegroundWindow().ToInt64();ForegroundPid=[BksDirect]::ForegroundProcess();ApplicationPid=$process.Id})
    }
    $observations.Add(@{ApplicationPid=$process.Id;FixturePid=$fixture.Id;PrivateAppData=$appData;ExeSha256=(Get-BksHash $Exe);ProductVersion=(Get-Item -LiteralPath $Exe).VersionInfo.ProductVersion})
}
function Wait-Live([IntPtr]$Status) { Wait-For { (Own-Text $Status).StartsWith([BksUnified]::LivePrefix,[StringComparison]::Ordinal) } 'live TUI frame acknowledged' }
function Live-Key([IntPtr]$List,[IntPtr]$Status,[int]$Key,[string]$Bytes,[string]$Name) {
    Wait-Live $Status
    Check-Key $List $Key $Bytes $Name
    Wait-Live $Status
}
function Save-OwnScreenshot([IntPtr]$Window,[string]$Name) {
    if([BksUnified]::Screenshot($script:Case.Process.Id,$Window,(Join-Path $runDirectory $Name))) {
        $checks.Add(@{Status='passed';Name=('own top-level PrintWindow artifact '+$Name+' saved; visual review is separate')})
    } else {
        $checks.Add(@{Status='skipped';Name=('own top-level PrintWindow unavailable for '+$Name+'; no rendering verdict')})
    }
}
function Assert-OwnReaderKey([IntPtr]$Window,[int]$Key) {
    if([BksDirect]::GetAncestor([BksDirect]::GetForegroundWindow(),2) -ne $script:Case.Main -or [BksDirect]::Focus($script:Case.Process.Id,$Window) -ne $Window) { throw 'Own reader/edit foreground lost; key refused' }
    [BksDirect]::Key($script:Case.Process.Id,$Window,$Key)
}
function Focus-Own([IntPtr]$Window,[switch]$ForceUia,[IntPtr]$Root=[IntPtr]::Zero) {
    if($Root -eq [IntPtr]::Zero) { $Root=$script:Case.Main }
    [BksStage7]::Own($script:Case.Process.Id,$Root)
    [BksStage7]::Own($script:Case.Process.Id,$Window)
    if([BksDirect]::GetAncestor([BksDirect]::GetForegroundWindow(),2) -ne $Root -or [BksDirect]::GetAncestor($Window,2) -ne $Root) { throw 'Own foreground lost before UIA focus; no focus change attempted' }
    if($ForceUia -or [BksDirect]::Focus($script:Case.Process.Id,$Window) -ne $Window -or -not (Element $Window).Current.HasKeyboardFocus) { (Element $Window).SetFocus() }
    Wait-For { [BksDirect]::GetAncestor([BksDirect]::GetForegroundWindow(),2) -eq $Root -and [BksDirect]::Focus($script:Case.Process.Id,$Window) -eq $Window -and (Element $Window).Current.HasKeyboardFocus } 'owned foreground plus native and UIA focus'
}
function Activate-OwnOnce([IntPtr]$Window) {
    $own=$script:Case.Process.Id;$root=$script:Case.Main
    [BksStage7]::Own($own,$root);[BksStage7]::Own($own,$Window)
    if([BksDirect]::GetAncestor($Window,2) -ne $root) { throw 'Initial activation target is not an owned child of the private root' }
    $activated=[BksUnified]::Activate($own,$root)
    $observations.Add(@{Case='single explicit owned activation attempt';ApplicationPid=$own;MainHwnd=$root.ToInt64();TargetHwnd=$Window.ToInt64();SetForegroundAccepted=$activated})
    Wait-For { [BksDirect]::GetAncestor([BksDirect]::GetForegroundWindow(),2) -eq $root } 'one explicit private-root activation; no retry'
    Focus-Own $Window
}
function Check-HiddenRawCaret([IntPtr]$Direct,[string]$Label) {
    $own=$script:Case.Process.Id
    Wait-For { $state=[BksUnified]::NativeCaret($own,$Direct);$state.Window -eq $Direct.ToInt64() -and -not $state.Visible } ('raw native caret hidden '+$Label)
    $state=[BksUnified]::NativeCaret($own,$Direct)
    $selection=[BksStage7]::Selection($own,$Direct)
    $caret=[BksDirect]::UiaCaret($own,$Direct)
    $text=Own-Text $Direct
    Check ('raw native caret remains hidden with an accessible UIA/native caret '+$Label) (-not $state.Visible -and $state.Window -eq $Direct.ToInt64() -and $state.Focus -eq $Direct.ToInt64() -and $selection[0] -eq $selection[1] -and $caret.Collapsed -and (Normalize-Text $caret.Prefix) -ceq (Normalize-Text $text.Substring(0,$selection[0])))
    $observations.Add(@{Case=('raw caret '+$Label);Native=$state;Selection=$selection;UiaCaretNormalized=(Normalize-Text $caret.Prefix).Length})
}
function Check-RawCanvasFocus([IntPtr]$Provider,[IntPtr]$Canvas,[string]$Phase) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main
    [BksStage7]::Own($own,$Provider);[BksStage7]::Own($own,$Canvas)
    # Read only: GetGUIThreadInfo on this private application's UI thread.
    # Do not repair a failed focus contract with UIA.SetFocus or activation.
    Wait-For { ([BksUnified]::NativeCaret($own,$main)).Focus -eq $Canvas.ToInt64() } ('raw canvas focus '+$Phase)
    $state=[BksUnified]::NativeCaret($own,$main)
    Check ('raw input focus belongs to the canvas rather than the TextPattern companion '+$Phase) ($state.Focus -eq $Canvas.ToInt64() -and $state.Focus -ne $Provider.ToInt64() -and [BksStage7]::ClassName($Canvas) -ceq 'BksTerminalSurface')
    $observations.Add(@{Case=('read-only raw focus '+$Phase);ApplicationPid=$own;CanvasHwnd=$Canvas.ToInt64();ProviderHwnd=$Provider.ToInt64();Native=$state;Probe='GetGUIThreadInfo only; no activation, SetFocus or keys';NvdaSpeech='not tested'})
}
function Check-RawTextPatterns([IntPtr]$Provider,[IntPtr]$Canvas,[string]$Phase) {
    $own=$script:Case.Process.Id
    [BksStage7]::Own($own,$Provider);[BksStage7]::Own($own,$Canvas)
    $canvasPattern=$null;$providerPattern=$null
    $canvasHasText=(Element $Canvas).TryGetCurrentPattern([Windows.Automation.TextPattern]::Pattern,[ref]$canvasPattern)
    $providerHasText=(Element $Provider).TryGetCurrentPattern([Windows.Automation.TextPattern]::Pattern,[ref]$providerPattern)
    Check ('raw canvas has no native TextPattern '+$Phase) (-not $canvasHasText)
    Check ('read-only companion retains a complete TextPattern '+$Phase) ($providerHasText -and [BksStage7]::ReadOnly($own,$Provider) -and (Same-UiaText ($providerPattern.DocumentRange.GetText(-1)) (Own-Text $Provider)))
}
function Test-ReadNavigation([IntPtr]$Direct,[IntPtr]$Reader) {
    $before=Raw-Input
    Check-HiddenRawCaret $Direct 'after initial focus'
    Request-Scene 'passive'
    Start-Sleep -Milliseconds 200
    Check-HiddenRawCaret $Direct 'after VT update'
    Focus-Own (Field $script:Case.Main 1007)
    Focus-Own $Direct
    Check-HiddenRawCaret $Direct 'after returning focus'
    $bounds=[BksDirect]::Bounds($script:Case.Process.Id,$script:Case.Main)
    try {
        [BksUnified]::Resize($script:Case.Process.Id,$script:Case.Main,$bounds.Right-$bounds.Left+16,$bounds.Bottom-$bounds.Top+16)
        Start-Sleep -Milliseconds 200
        Check-HiddenRawCaret $Direct 'after resize'
    } finally {
        [BksUnified]::Resize($script:Case.Process.Id,$script:Case.Main,$bounds.Right-$bounds.Left,$bounds.Bottom-$bounds.Top)
    }
    Start-Sleep -Milliseconds 200
    Check-HiddenRawCaret $Direct 'after restoring size'
    Chord $Direct 0x20 { [BksDirect]::Focus($script:Case.Process.Id,$script:Case.Main) -eq $Reader } 'explicit CtrlShiftSpace enters native reader'
    Wait-For { (Own-Text $Reader).Contains('BKS_UNIFIED_READY') } 'native reading document ready'
    $text=Own-Text $Reader
    $element=Element $Reader
    $pattern=$element.GetCurrentPattern([Windows.Automation.TextPattern]::Pattern)
    Check 'explicit reader is focused named read-only native text with TextPattern' ([BksStage7]::IsTextControl($script:Case.Process.Id,$Reader) -and [BksStage7]::ReadOnly($script:Case.Process.Id,$Reader) -and $element.Current.IsKeyboardFocusable -and $element.Current.HasKeyboardFocus -and $element.Current.Name.Length -gt 0 -and (Same-UiaText (Uia-Text $Reader) $text))
    $initial=[BksStage7]::Selection($script:Case.Process.Id,$Reader)
    Assert-OwnReaderKey $Reader 0x26
    Start-Sleep -Milliseconds 100
    $firstUp=[BksStage7]::Selection($script:Case.Process.Id,$Reader)
    Assert-OwnReaderKey $Reader 0x28
    Wait-For { ([BksStage7]::Selection($script:Case.Process.Id,$Reader))[0] -gt $firstUp[0] } 'ordinary reader Down changes the native caret'
    $firstDown=[BksStage7]::Selection($script:Case.Process.Id,$Reader)
    $observations.Add(@{Case='natural reader entry before any programmatic positioning';InitialSelection=$initial;AfterUp=$firstUp;AfterDown=$firstDown;InitialAtDocumentStart=($initial[0] -eq 0);RawInput=(Raw-Input);NvdaSpeech='not tested'})
    Check 'reader Down actually moves EM_GETSEL without changing terminal text' ($firstDown[0] -gt $firstUp[0] -and $firstDown[0] -eq $firstDown[1] -and (Own-Text $Reader) -ceq $text)
    $history=[regex]::Matches($text,'BKS_UNIFIED_HISTORY_[0-9]{3}')
    Check 'reader exposes retained history above the current application screen' ($history.Count -ge 4)
    $middle=$history[[int][Math]::Floor($history.Count/2)]
    $anchor=$middle.Index+4
    [BksStage7]::Select($script:Case.Process.Id,$Reader,$anchor,$anchor)
    [BksStage7]::ScrollTo($script:Case.Process.Id,$Reader,$anchor)
    Assert-OwnReaderKey $Reader 0x26
    Wait-For { ([BksStage7]::Selection($script:Case.Process.Id,$Reader))[0] -lt $anchor } 'reader Up moves to preceding history line'
    $up=[BksStage7]::Selection($script:Case.Process.Id,$Reader)
    $caret=[BksDirect]::UiaCaret($script:Case.Process.Id,$Reader)
    Check 'UIA caret agrees with native selection after reading Up' ($up[0] -eq $up[1] -and $caret.Collapsed -and (Normalize-Text $caret.Prefix) -ceq (Normalize-Text $text.Substring(0,$up[0])))
    Assert-OwnReaderKey $Reader 0x28
    Wait-For { ([BksStage7]::Selection($script:Case.Process.Id,$Reader))[0] -eq $anchor } 'reader Down returns to the same history column'
    Assert-OwnReaderKey $Reader 0x25
    Wait-For { ([BksStage7]::Selection($script:Case.Process.Id,$Reader))[0] -eq $anchor-1 } 'reader Left moves the native caret'
    Assert-OwnReaderKey $Reader 0x27
    Wait-For { ([BksStage7]::Selection($script:Case.Process.Id,$Reader))[0] -eq $anchor } 'reader Right restores the native caret'
    Check 'reader Up Down Left Right are real native caret navigation and send no PTY input' ((Raw-Input) -ceq $before -and (Own-Text $Reader) -ceq $text)
    $range=$pattern.GetSelection()[0].Clone()
    $range.ExpandToEnclosingUnit([Windows.Automation.Text.TextUnit]::Line)
    $currentLine=$range.GetText(-1)
    $previous=$range.Clone()
    $moved=$previous.Move([Windows.Automation.Text.TextUnit]::Line,-1)
    $previousLine=$previous.GetText(-1)
    Check 'UIA review range can expose the preceding historical line without moving the edit caret' ($moved -eq -1 -and $currentLine.Contains($middle.Value) -and $previousLine.Contains('BKS_UNIFIED_HISTORY_') -and $previousLine -cne $currentLine -and ([BksStage7]::Selection($script:Case.Process.Id,$Reader))[0] -eq $anchor)
    Request-Scene 'passive'
    Start-Sleep -Milliseconds 400
    Check 'passive terminal output preserves the reader caret and its historical text anchor' (([BksStage7]::Selection($script:Case.Process.Id,$Reader))[0] -eq $anchor -and (Own-Text $Reader).Substring($middle.Index,$middle.Length) -ceq $middle.Value -and [BksDirect]::Focus($script:Case.Process.Id,$Reader) -eq $Reader)
    $observations.Add(@{Case='explicit reader native and UIA review evidence';HistoryAnchor=$middle.Value;AnchorOffset=$anchor;UpSelection=$up;CurrentLine=$currentLine;PreviousReviewLine=$previousLine;ControlType=$element.Current.ControlType.ProgrammaticName;SupportedPatterns=@($element.GetSupportedPatterns()|ForEach-Object {$_.ProgrammaticName});RawInput=(Raw-Input);NvdaSpeech='requires human NVDA test'})
    Save-OwnScreenshot $Reader 'reader-navigation.png'
    Assert-OwnReaderKey $Reader 27
    Wait-For { [BksDirect]::Focus($script:Case.Process.Id,$script:Case.Main) -eq $Direct } 'reader Escape returns without PTY input'
    Check 'complete explicit reader navigation and Escape sent zero PTY bytes' ((Raw-Input) -ceq $before)
    Chord $Direct 0x26 { [BksDirect]::Focus($script:Case.Process.Id,$script:Case.Main) -eq $Reader } 'raw TUI CtrlUp enters the reader at the preceding logical line' $true $false
    $fromRaw=[BksStage7]::Selection($script:Case.Process.Id,$Reader)
    Check 'raw CtrlUp reads the preceding VT line instead of forwarding a control sequence' ((Own-Text $Reader).Substring($fromRaw[0]).StartsWith('Tick:',[StringComparison]::Ordinal) -and (Raw-Input) -ceq $before)
    Chord $Reader 0x26 { ([BksStage7]::Selection($script:Case.Process.Id,$Reader))[0] -lt $fromRaw[0] } 'reader CtrlUp moves one more logical line' $true $false
    Chord $Reader 0x28 { ([BksStage7]::Selection($script:Case.Process.Id,$Reader))[0] -eq $fromRaw[0] } 'reader CtrlDown returns to the previous logical line' $true $false
    Assert-OwnReaderKey $Reader 27
    Wait-For { [BksDirect]::Focus($script:Case.Process.Id,$script:Case.Main) -eq $Direct } 'Escape after logical line navigation restores direct input'
    Check 'TUI CtrlUp CtrlDown and Escape remain entirely local with zero PTY bytes' ((Raw-Input) -ceq $before)
    $checks.Add(@{Status='skipped';Name='Human NVDA speech/review commands were not invoked; TextPattern/line-range/native-caret evidence only'})
}
function Test-TuiSelection([IntPtr]$Direct,[IntPtr]$Reader) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main;$before=Raw-Input
    $prefix='Input> '+[char]0x0416+[char]0x754c+'e'+[char]0x301+[char]::ConvertFromUtf32(0x1f600)+' alpha'
    Request-Scene 'selection'
    Wait-For { (Own-Text $Direct).Contains('BKS_UNIFIED_SELECTION_READY') -and (Own-Text $Direct).Contains($prefix+' beta tail') } 'private Unicode prompt with one inverse software cursor'
    $text=Own-Text $Direct;$lineStart=$text.LastIndexOf($prefix+' beta tail',[StringComparison]::Ordinal);$cliCaret=$lineStart+$prefix.Length
    Wait-For { $selection=[BksStage7]::Selection($own,$Direct);$selection[0] -eq $cliCaret -and $selection[1] -eq $cliCaret } 'known UTF16 CLI caret follows inverse cell instead of hidden physical cursor'
    $observations.Add(@{Case='selection fixture before activation';CliCaret=$cliCaret;LineStart=$lineStart;Prefix=$prefix;NativeSelection=([BksStage7]::Selection($own,$Direct));RawInput=$before})
    Check 'selection fixture starts with zero PTY input and a Unicode CLI caret inside its line' ($before.Length -eq 0 -and $lineStart -ge 0 -and $cliCaret -lt $text.Length)
    Activate-OwnOnce $Direct
    $checkSelection={param([int]$Start,[int]$End,[string]$Label)
        $selection=[BksStage7]::Selection($own,$Reader);$document=Own-Text $Reader
        $pattern=(Element $Reader).GetCurrentPattern([Windows.Automation.TextPattern]::Pattern);$ranges=$pattern.GetSelection()
        $selected=if($ranges.Length -eq 1){$ranges[0].GetText(-1)}else{''}
        Check ($Label+' exposes exact native selection through read-only TextPattern with zero PTY bytes') ([BksDirect]::Focus($own,$main) -eq $Reader -and [BksStage7]::ReadOnly($own,$Reader) -and $selection[0] -eq $Start -and $selection[1] -eq $End -and $document -ceq $text -and $selected -ceq $text.Substring($Start,$End-$Start) -and (Same-UiaText (Uia-Text $Reader) $document) -and (Raw-Input) -ceq $before)
        $observations.Add(@{Case=$Label;CliCaret=$cliCaret;NativeSelection=$selection;SelectedText=$selected;RawInput=(Raw-Input)})
    }
    foreach($entry in @(@{Key=0x25;Name='ShiftLeft';Start=$cliCaret-1},@{Key=0x24;Name='ShiftHome';Start=$lineStart})) {
        Wait-For { $selection=[BksStage7]::Selection($own,$Direct);$selection[0] -eq $cliCaret -and $selection[1] -eq $cliCaret } 'raw TUI caret restored before local selection'
        $caret=[BksDirect]::UiaCaret($own,$Direct)
        Check ('known native and UIA CLI caret before '+$entry.Name) ($caret.Collapsed -and (Normalize-Text $caret.Prefix) -ceq (Normalize-Text $text.Substring(0,$cliCaret)))
        $observations.Add(@{Case=('before '+$entry.Name);CliCaret=$cliCaret;SourceControl=1011;TargetControl=1018;NativeSelection=([BksStage7]::Selection($own,$Direct));UiaPrefix=$caret.Prefix;RawInput=(Raw-Input)})
        Chord $Direct $entry.Key { $selection=[BksStage7]::Selection($own,$Reader);[BksDirect]::Focus($own,$main) -eq $Reader -and $selection[0] -eq $entry.Start -and $selection[1] -eq $cliCaret } ($entry.Name+' enters native reader directly from CLI caret') $false $true
        & $checkSelection $entry.Start $cliCaret ($entry.Name+' from direct TUI')
        if($entry.Key -eq 0x25) {
            Chord $Reader 0x25 { ([BksStage7]::Selection($own,$Reader))[0] -eq $cliCaret-5 } 'native CtrlShiftLeft extends selection to the word start' $true $true
            & $checkSelection ($cliCaret-5) $cliCaret 'reader CtrlShiftLeft'
            Chord $Reader 0x24 { ([BksStage7]::Selection($own,$Reader))[0] -eq $lineStart } 'native ShiftHome extends selection to the Unicode line start' $false $true
            & $checkSelection $lineStart $cliCaret 'reader ShiftHome'
        }
        Assert-OwnReaderKey $Reader 27
        Wait-For { [BksDirect]::Focus($own,$main) -eq $Direct -and -not [BksStage7]::IsWindowVisible($Reader) } 'selection Escape returns to raw TUI'
        Assert-ExpectedInput $before ('local selection and Escape after '+$entry.Name)
    }
    Assert-OwnReaderKey $Direct 9
    Wait-For { (Raw-Input).Length -ge $before.Length+2 } 'Tab after selection Escape reaches the fixture'
    Start-Sleep -Milliseconds 250
    Confirm-Input $before '09' 'Tab after selection Escape reaches only the private CLI exactly once'
    Check 'selection keeps the same live fixture and responsive terminal' (-not $script:Case.Fixture.HasExited -and [BksStage7]::Responsive($own,$main,1000,2))
    $checks.Add(@{Status='skipped';Name='Human NVDA selection speech and clipboard copying require manual verification; clipboard was not written'})
}
function Test-ShellRegression([IntPtr]$InputBox,[IntPtr]$Journal,[IntPtr]$Direct) {
    $own=$script:Case.Process.Id
    $before=Raw-Input
    Assert-OwnReaderKey $InputBox 9
    Wait-For { [BksDirect]::Focus($own,$script:Case.Main) -ne $InputBox } 'ordinary Tab is GUI focus navigation'
    Check 'ordinary profile Tab is not captured by the PTY' ((Raw-Input) -ceq $before)
    Focus-Own $InputBox
    $quiet=@{Count=[BksDirect]::Notices().Length;Timer=[Diagnostics.Stopwatch]::StartNew()}
    Wait-For { $count=[BksDirect]::Notices().Length;if($count -ne $quiet.Count){$quiet.Count=$count;$quiet.Timer.Restart()};$quiet.Timer.ElapsedMilliseconds -ge 300 } 'ordinary input notification baseline quiet'
    $noticeStart=[BksDirect]::Notices().Length
    $typed='abc'+[string][char]0x0416+'Z'
    foreach($unit in $typed.ToCharArray()) {
        if([BksDirect]::GetAncestor([BksDirect]::GetForegroundWindow(),2) -ne $script:Case.Main) { throw 'Own shell foreground lost before typed character' }
        [BksDirect]::Character($own,$InputBox,$unit)
    }
    Wait-For { (Own-Text $InputBox) -ceq $typed } 'native field accepts actual typed Unicode'
    Assert-OwnReaderKey $InputBox 0x24
    Wait-For { ([BksStage7]::Selection($own,$InputBox))[0] -eq 0 } 'native Home moves to line start'
    foreach($length in 1..3) {
        Chord $InputBox 0x27 { $selection=[BksStage7]::Selection($own,$InputBox);$selection[0] -eq 0 -and $selection[1] -eq $length } 'native ShiftRight extends input selection' $false $true
    }
    $pattern=(Element $InputBox).GetCurrentPattern([Windows.Automation.TextPattern]::Pattern)
    Check 'native input exposes the selected own text through UIA' ($pattern.GetSelection()[0].GetText(-1) -ceq 'abc')
    Assert-OwnReaderKey $InputBox 8
    $remaining=[string][char]0x0416+'Z'
    Wait-For { (Own-Text $InputBox) -ceq $remaining } 'native Backspace deletes selection'
    Assert-OwnReaderKey $InputBox 0x27
    Assert-OwnReaderKey $InputBox 0x25
    Start-Sleep -Milliseconds 150
    Check 'typing selection deletion and ordinary arrows remain in the native field without PTY bytes or stale TUI echo' ((Own-Text $InputBox) -ceq $remaining -and (Raw-Input) -ceq $before -and [BksDirect]::Notices().Length -eq $noticeStart)
    $observations.Add(@{Case='ordinary native editing';Typed=$typed;Remaining=(Own-Text $InputBox);Selection=[BksStage7]::Selection($own,$InputBox);RawInput=(Raw-Input);CustomNotifications=@([BksDirect]::Notices()|Select-Object -Skip $noticeStart);NvdaNativeKeyboardEcho='not tested'})
    Request-Scene 'shell'
    Wait-For { (Own-Text $Journal).Contains('BKS_SHELL_READY') } 'managed fake shell uses ordinary linear output'
    foreach($literal in @('BKS_FAKE_BLOCK_1','BKS_FAKE_BLOCK_2')) {
        [BksStage7]::SetText($own,$InputBox,$literal)
        $bytes=([BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes($literal+"`r`n"))).Replace('-','')
        Check-Key $InputBox 13 $bytes 'explicit harmless fake submission creates a command block'
        Wait-For { (Own-Text $InputBox).Length -eq 0 } 'submitted command field cleared'
    }
    $before=Raw-Input
    Chord $InputBox 0x26 { [BksDirect]::Focus($own,$script:Case.Main) -eq $Journal } 'CtrlUp moves from native input into journal block' $true $false
    $last=[BksStage7]::Selection($own,$Journal)
    Chord $Journal 0x26 { ([BksStage7]::Selection($own,$Journal))[0] -lt $last[0] } 'journal CtrlUp reaches previous block' $true $false
    $previous=[BksStage7]::Selection($own,$Journal)
    Chord $Journal 0x28 { ([BksStage7]::Selection($own,$Journal))[0] -gt $previous[0] } 'journal CtrlDown reaches next block' $true $false
    Check 'ordinary CtrlUp CtrlDown block navigation changes journal selection without adding PTY bytes' ((Raw-Input) -ceq $before -and [BksDirect]::Focus($own,$Journal) -eq $Journal)
    $observations.Add(@{Case='ordinary journal block navigation';LastSelection=$last;PreviousSelection=$previous;NextSelection=[BksStage7]::Selection($own,$Journal);RawInput=(Raw-Input)})
    Save-OwnScreenshot $Journal 'ordinary-shell-regression.png'
    $checks.Add(@{Status='skipped';Name='Human NVDA native editing/speech acceptance remains separate from UIA and native selection checks'})
}
function Wait-SingleProjection([IntPtr]$Field,[IntPtr]$Output,[string]$Value,[string]$Phase,[string]$ScreenshotName='') {
    $own=$script:Case.Process.Id
    Wait-For {
        $selection=[BksStage7]::Selection($own,$Field)
        (Own-Text $Field) -ceq $Value -and $selection[0] -eq $Value.Length -and $selection[1] -eq $Value.Length -and (Own-Text $Output).Contains('BKS_SINGLE_OUTPUT_UNIQUE')
    } ('source-ack field text and UTF16 caret '+$Phase)
    if($ScreenshotName) { Save-OwnScreenshot $Field $ScreenshotName }
    $selection=[BksStage7]::Selection($own,$Field)
    Check ('shared field native text/UTF16 caret and UIA text agree '+$Phase) ((Own-Text $Field) -ceq $Value -and $selection[0] -eq $Value.Length -and $selection[1] -eq $Value.Length -and (Same-UiaText (Uia-Text $Field) $Value))
    if($SingleInput -and $PrivateDesktopName -and [BksQaDesktop]::CurrentDesktop -ceq $PrivateDesktopName) {
        # The classic EDIT client proxy queries the interactive input queue for
        # selection/caret ranges. Off-desktop it raises varying native errors.
        # Keep native EM_GETSEL and UIA text strict; this separate UIA capability
        # is unavailable here and must be checked on the interactive desktop.
        $checks.Add(@{Status='skipped';Name=('interactive EDIT UIA selection/caret unavailable on private desktop '+$Phase)})
    } else {
        $caret=[BksDirect]::UiaCaret($own,$Field)
        Check ('shared field UIA caret agrees '+$Phase) ($caret.Collapsed -and (Normalize-Text $caret.Prefix) -ceq (Normalize-Text $Value))
    }
    $text=Own-Text $Output
    Check ('projected upper output excludes the active composer and border '+$Phase) (-not $text.Contains($Value) -and -not $text.Contains([string][char]0x250c) -and -not $text.Contains([string][char]0x2514))
    foreach($line in @($Value -split "`r`n" | Where-Object {$_.Length -gt 0})) {Check ('upper output excludes each active input row '+$Phase) (-not $text.Contains($line))}
}
function Single-Character([IntPtr]$Window,[string]$Value,[string]$Phase) {
    Assert-ExpectedInput $script:ExpectedInput ('before '+$Phase)
    $before=$script:ExpectedInput
    foreach($unit in $Value.ToCharArray()) { [BksDirect]::Character($script:Case.Process.Id,$Window,$unit) }
    $bytes=([BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes($Value))).Replace('-','')
    Wait-For { (Raw-Input).Length -ge $before.Length+$bytes.Length } ($Phase+' reaches only the managed fixture')
    Start-Sleep -Milliseconds 150
    Confirm-Input $before $bytes ($Phase+' exact UTF8 once')
}
function Test-SingleSelection([IntPtr]$Field,[IntPtr]$Output) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main
    $source='A'+[char]0x0416+[char]::ConvertFromUtf32(0x1f600)+"`r`n"+[char]0x754c+'Z'
    foreach($key in @(8,0x2e)) {
        Request-Scene 'native-selection'
        Wait-SingleProjection $Field $Output $source ('before selected '+$key)
        [BksStage7]::Select($own,$Field,1,7)
        $before=$script:ExpectedInput
        [BksDirect]::Key($own,$Field,$key)
        Wait-For { $selection=[BksStage7]::Selection($own,$Field);(Own-Text $Field) -ceq 'AZ' -and $selection[0] -eq 1 -and $selection[1] -eq 1 } ('selected Unicode/multiline deletion '+$key)
        Start-Sleep -Milliseconds 150
        Confirm-Input $before ('1B5B44'+('7F'*4)) ('native selected '+$key+' moves once then deletes Unicode scalars and CRLF exactly once')
        Check ('native selected '+$key+' preserves unselected prefix and suffix through source acknowledgement') ((Own-Text $Field) -ceq 'AZ' -and (Same-UiaText (Uia-Text $Field) 'AZ'))
    }
    Request-Scene 'native-selection'
    Wait-SingleProjection $Field $Output $source 'before fast selected replacement'
    [BksStage7]::Select($own,$Field,1,7)
    Request-Scene 'hold'
    $before=$script:ExpectedInput
    [BksDirect]::Character($own,$Field,[char]'Q')
    Wait-For { (Raw-Input).Length -ge $before.Length+6 } 'selection replacement sends only its first source cursor key'
    Start-Sleep -Milliseconds 100
    Confirm-Input $before '1B5B44' 'held selection replacement positions the source exactly once'
    [BksDirect]::Character($own,$Field,[char]'R')
    Start-Sleep -Milliseconds 150
    $before=$script:ExpectedInput
    Check 'selected replacement waits for VT before changing readonly text or sending deletion/replacement' ((Own-Text $Field) -ceq $source -and ([BksStage7]::Selection($own,$Field))[0] -eq 1 -and ([BksStage7]::Selection($own,$Field))[1] -eq 7 -and (Raw-Input) -ceq $before)
    Request-Scene 'release'
    Wait-For { $selection=[BksStage7]::Selection($own,$Field);(Own-Text $Field) -ceq 'AQRZ' -and $selection[0] -eq 3 -and $selection[1] -eq 3 } 'queued replacement characters follow source-confirmed selected deletion'
    Start-Sleep -Milliseconds 150
    Confirm-Input $before (('7F'*4)+'5152') 'fast replacement characters retain their order and reach the source once'
    Check-Key $Field 8 '7F' 'collapsed selection keeps ordinary Backspace'
    Wait-For { (Own-Text $Field) -ceq 'AQZ' } 'ordinary Backspace source acknowledgement'
    Check-Key $Field 0x2e '1B5B337E' 'collapsed selection keeps ordinary Delete'
    Wait-For { (Own-Text $Field) -ceq 'AQ' } 'ordinary Delete source acknowledgement'

    Request-Scene 'native-selection'
    Wait-SingleProjection $Field $Output $source 'before selected replacement loses input focus'
    [BksStage7]::Select($own,$Field,1,7)
    Request-Scene 'hold';$before=$script:ExpectedInput
    [BksDirect]::Character($own,$Field,[char]'Q')
    Wait-For { (Raw-Input).Length -ge $before.Length+6 } 'held replacement cursor request before reading'
    Confirm-Input $before '1B5B44' 'held replacement has one outstanding source key before reading'
    [BksDirect]::Post($own,$main,0x111,60003)
    Wait-For { [BksDirect]::Focus($own,$main) -eq $Output } 'reading cancels a pending selection edit'
    Request-Scene 'release'
    Start-Sleep -Milliseconds 200
    Assert-ExpectedInput $script:ExpectedInput 'pending selection deletion/replacement never follows focus into the reader'
    [BksDirect]::Post($own,$main,0x111,60003)
    Wait-For { [BksDirect]::Focus($own,$main) -eq $Field } 'return from reading after selection cancellation'
    Check 'focus cancellation drops deferred replacement and leaves source text intact' ((Own-Text $Field) -ceq $source)
}
function Assert-StaleSingleInput([IntPtr]$Field,[IntPtr]$List,[string]$Phase) {
    $own=$script:Case.Process.Id;$before=$script:ExpectedInput
    foreach($window in @($Field,$List)) {
        # Deliberately target the old input HWND without focusing it. These are
        # stale queued messages, not an explicit command to the current reader.
        [BksDirect]::Post($own,$window,0x102,0x51)
        [BksDirect]::Post($own,$window,0x100,13)
        [BksDirect]::Post($own,$window,0x102,13)
    }
    Start-Sleep -Milliseconds 250
    Assert-ExpectedInput $before $Phase
    Check ($Phase+' consumes stale field/list characters and Return without PTY input') ((Raw-Input) -ceq $before)
}
function Wait-SingleListFocus([IntPtr]$List,[string]$Phase) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main
    try {
        Wait-For { [BksDirect]::Focus($own,$main) -eq $List } ('logical inline list focus '+$Phase)
    } catch {
        $focus=[BksDirect]::Focus($own,$main)
        $entry=@([BksStage7]::DialogTree($own,$main) | Where-Object {$_.Handle -eq $focus.ToInt64()} | Select-Object -First 1)
        $controlId=if($entry.Count){[int]$entry[0].ControlId}else{-1}
        $observations.Add(@{Case='inline list focus did not settle';Phase=$Phase;ExpectedControlId=1030;ExpectedHwnd=$List.ToInt64();ActualFocusId=$controlId;ActualFocusHwnd=$focus.ToInt64()})
        throw ('Inline list focus timeout at {0}: expected control ID=1030 HWND={1}; actual control ID={2} HWND={3}' -f $Phase,$List.ToInt64(),$controlId,$focus.ToInt64())
    }
}
function Wait-SingleSourceFocus([IntPtr]$Target,[string]$Phase) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main
    [BksStage7]::Own($own,$Target)
    if([BksDirect]::GetAncestor([BksDirect]::GetForegroundWindow(),2) -ne $main) {
        # A non-input desktop can have no foreground window. Production keeps
        # that speech/focus gate. Only explicit owned reader menu commands are
        # used here to select the logical input target; no focus API is called.
        $checks.Add(@{Status='skipped';Name=('foreground-gated automatic source focus unavailable on the private desktop '+$Phase)})
        $observations.Add(@{Case='private desktop foreground gate';Phase=$Phase;ForegroundHwnd=[BksDirect]::GetForegroundWindow().ToInt64();ForegroundPid=[BksDirect]::ForegroundProcess();LogicalFocusHwnd=[BksDirect]::Focus($own,$main).ToInt64()})
        if([BksDirect]::Focus($own,$main) -ne $Target) {
            $before=Raw-Input
            [BksDirect]::Post($own,$main,0x111,60003)
            Wait-For { [BksDirect]::Focus($own,$main) -eq (Field $main 1018) } ('explicit private reader entry '+$Phase)
            [BksDirect]::Post($own,$main,0x111,60003)
            Wait-For { [BksDirect]::Focus($own,$main) -eq $Target } ('explicit private reader return '+$Phase)
            Assert-ExpectedInput $before ('private logical focus menu commands '+$Phase)
        }
    }
    Wait-For { [BksDirect]::Focus($own,$main) -eq $Target } ('native source input focus '+$Phase)
}
function Assert-SingleProjectionContinuity([IntPtr]$Output,[string]$Size,[string]$Phase) {
    Check ('PTY dimensions stay unchanged '+$Phase) ((Own-Text (Field $script:Case.Main 1015)) -ceq $Size)
    $text=Own-Text $Output
    Check ('upper output keeps historical input and composer borders excluded '+$Phase) (-not $text.Contains('BKS_SINGLE_INPUT') -and -not $text.Contains('BKS_SINGLE_SECOND_ROW') -and -not $text.Contains([string][char]0x250c) -and -not $text.Contains([string][char]0x2514) -and -not $text.Contains([string][char]0x2502))
}
function Test-RuleComposer([IntPtr]$Field,[IntPtr]$Output,[IntPtr]$List,[string[]]$Labels) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main;$fieldHwnd=$Field;$before=Raw-Input
    $value='BKS_RULE_SOURCE '+[char]0x0416+[char]0x754c+'e'+[char]0x301+[char]::ConvertFromUtf32(0x1f600)
    Request-Scene 'native-rule-prompt'
    Wait-For { (Own-Text $Output).Contains('BKS_RULE_LAYOUT_READY') } 'source horizontal-rule layout arrives'
    Wait-SingleProjection $Field $Output $value 'right-side rule-bound composer' 'single-input-rule-field.png'
    Check 'rule-bound source keeps one shared native EDIT and removes both rules from upper output' ((Field $main 1002) -eq $fieldHwnd -and [BksStage7]::IsWindowVisible($Field) -and [BksStage7]::ReadOnly($own,$Field) -and -not (Own-Text $Output).Contains([string][char]0x2500) -and (Raw-Input) -ceq $before)
    Request-Scene 'hold'
    Single-Character $Field '/' 'held rule-bound slash'
    Check 'right-side rule field and menu wait for source acknowledgement' ((Own-Text $Field) -ceq $value -and -not [BksStage7]::IsWindowVisible($List))
    Request-Scene 'release';$value+='/'
    Wait-SingleProjection $Field $Output $value 'acknowledged rule-bound slash'
    Wait-For { [BksStage7]::IsWindowVisible($List) -and ([BksUnified]::ListItems($own,$List) -join "`n") -ceq ($Labels -join "`n") } 'generic source slash menu above rule-bound composer'
    Wait-SingleSourceFocus $List 'rule-bound source menu'
    Check 'rule-bound menu exposes source labels and native/UIA initial choice' ([BksUnified]::ListSelection($own,$List) -eq 0 -and (Uia-Selected $List) -ceq $Labels[0] -and ((Uia-Rows $List) -join "`n") -ceq ($Labels -join "`n"))
    Check 'upper output excludes the rule composer, rules and native menu copies' (-not (Own-Text $Output).Contains($value) -and -not (Own-Text $Output).Contains([string][char]0x2500) -and @($Labels | Where-Object {(Own-Text $Output).Contains($_)}).Count -eq 0)
    Save-OwnScreenshot $List 'single-input-rule-menu.png'
    Request-Scene 'hold'
    Check-Key $List 0x28 '1B5B42' 'held rule-menu Down'
    Check 'rule-menu selection remains source-owned before acknowledgement' ([BksUnified]::ListSelection($own,$List) -eq 0 -and (Uia-Selected $List) -ceq $Labels[0])
    Request-Scene 'release'
    Wait-For { [BksUnified]::ListSelection($own,$List) -eq 1 -and (Uia-Selected $List) -ceq $Labels[1] } 'rule-menu source acknowledges Bravo'
    Wait-SingleListFocus $List 'rule-menu Down acknowledgement'
    Check-Key $List 27 '1B' 'rule-menu Escape'
    Wait-For { -not [BksStage7]::IsWindowVisible($List) } 'source rule-menu Escape closes the list'
    Wait-SingleSourceFocus $Field 'rule-menu close'
    Check 'rule composer and menu add exactly slash, Down and Escape bytes once' ((Raw-Input) -ceq ($before+'2F1B5B421B') -and (Own-Text $Field) -ceq $value -and (Field $main 1002) -eq $fieldHwnd)
    $observations.Add(@{Case='generic rule-bound source with right-side prompt and slash menu';RawBefore=$before;RawAfter=(Raw-Input);NativeField=(Own-Text $Field);NativeOutput=(Own-Text $Output);FieldHwnd=$fieldHwnd.ToInt64();NoActualCli=$true})
}
function Test-NativeEditors([IntPtr]$Field,[IntPtr]$Output) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main;$fieldHwnd=$Field
    foreach($kind in @('padded','ruled')) {
        $before=Raw-Input
        Request-Scene ('native-'+$kind)
        Wait-For { (Own-Text $Field).Length -eq 0 -and (Own-Text $Output).Contains('BKS_NATIVE_EDITOR_'+$kind.ToUpperInvariant()) } ($kind+' empty editor with right-aligned footer')
        Check ($kind+' keeps the same visible readonly field and excludes the unentered hint') ((Field $main 1002) -eq $fieldHwnd -and [BksStage7]::IsWindowVisible($Field) -and [BksStage7]::ReadOnly($own,$Field) -and -not (Own-Text $Output).Contains('Generic unentered hint'))
        Assert-ExpectedInput $before ($kind+' projection sends no input')
        Wait-SingleSourceFocus $Field ($kind+' native editor')
        Request-Scene 'hold'
        Single-Character $Field ([string][char]0x0416) ($kind+' held character')
        Check ($kind+' waits for source acknowledgement') ((Own-Text $Field).Length -eq 0)
        Request-Scene 'release'
        $value=[string][char]0x0416
        Wait-SingleProjection $Field $Output $value ($kind+' first character')
        $more=[string][char]0x754c+[char]::ConvertFromUtf32(0x1f642)
        Single-Character $Field $more ($kind+' wide and surrogate characters')
        $value+=$more
        Wait-SingleProjection $Field $Output $value ($kind+' Unicode')
        Check-Key $Field 0x24 '1B5B48' ($kind+' Home')
        Wait-For { ([BksStage7]::Selection($own,$Field))[0] -eq 0 -and (Own-Text $Field) -ceq $value } ($kind+' source Home')
        Check-Key $Field 0x27 '1B5B43' ($kind+' Right')
        Wait-For { ([BksStage7]::Selection($own,$Field))[0] -eq 1 } ($kind+' source Right')
        [BksStage7]::Select($own,$Field,0,1)
        Check-Key $Field 8 '7F' ($kind+' selected Backspace')
        Wait-For { (Own-Text $Field) -ceq $more -and ([BksStage7]::Selection($own,$Field))[0] -eq 0 } ($kind+' selected character is removed in source')
        Check-Key $Field 0x23 '1B5B46' ($kind+' End')
        Wait-SingleProjection $Field $Output $more ($kind+' End after deletion')
        Check-Key $Field 8 '7F' ($kind+' delete surrogate glyph')
        Wait-For { (Own-Text $Field) -ceq ([string][char]0x754c) } ($kind+' surrogate deleted once')
        Check-Key $Field 8 '7F' ($kind+' clear final glyph')
        Wait-For { (Own-Text $Field).Length -eq 0 -and ([BksStage7]::Selection($own,$Field))[0] -eq 0 } ($kind+' empty source editor restored')
        Check ($kind+' empty editor keeps hint outside native value and upper output') ((Same-UiaText (Uia-Text $Field) '') -and -not (Own-Text $Output).Contains('Generic unentered hint'))
        Save-OwnScreenshot $Field ('native-'+$kind+'-empty.png')
    }
}
function Wait-TurnField([IntPtr]$Field,[string]$Value,[string]$Phase) {
    $own=$script:Case.Process.Id
    Wait-For { $selection=[BksStage7]::Selection($own,$Field);(Own-Text $Field) -ceq $Value -and $selection[0] -eq $Value.Length -and $selection[1] -eq $Value.Length } ('source-owned turn field '+$Phase)
    Check ('turn field native text/caret and UIA text agree '+$Phase) ([BksStage7]::ReadOnly($own,$Field) -and [BksStage7]::IsWindowVisible($Field) -and (Same-UiaText (Uia-Text $Field) $Value))
}
function Wait-TurnPhase([IntPtr]$Status,[string]$Prefix,[string]$Phase) {
    Wait-For { (Own-Text $Status).StartsWith($Prefix,[StringComparison]::Ordinal) } ('visible turn phase '+$Phase)
    $native=Own-Text $Status;$uiaName=(Element $Status).Current.Name
    Check ('visible turn phase and UIA name expose source progress '+$Phase) ([BksStage7]::IsWindowVisible($Status) -and $native.StartsWith($Prefix,[StringComparison]::Ordinal) -and $uiaName.Contains($Prefix))
    $observations.Add(@{Case='controlled turn phase';Phase=$Phase;NativeStatus=$native;UiaName=$uiaName;RawInput=(Raw-Input)})
    Assert-ExpectedInput $script:ExpectedInput ('source phase '+$Phase)
}
function Assert-TurnNotReady([IntPtr]$Status,[string]$Phase,[int]$Milliseconds=0) {
    $timer=[Diagnostics.Stopwatch]::StartNew()
    do {
        if((Own-Text $Status).StartsWith([BksUnified]::TurnReady,[StringComparison]::Ordinal)) {Check ('turn does not finish without source completion '+$Phase) $false}
        Assert-ExpectedInput $script:ExpectedInput ('unchanged source phase '+$Phase)
        if($timer.ElapsedMilliseconds -lt $Milliseconds) {Start-Sleep -Milliseconds 80}
    } while($timer.ElapsedMilliseconds -lt $Milliseconds)
    Check ('unchanged busy/source frame never fabricates answer completion '+$Phase) $true
}
function Invoke-TurnNavigation([IntPtr]$Output,[int]$Command,[string]$Body,[string]$Phase) {
    $own=$script:Case.Process.Id;$before=Raw-Input
    [BksDirect]::Post($own,$script:Case.Main,0x111,$Command)
    Wait-For {
        $text=Own-Text $Output;$selection=[BksStage7]::Selection($own,$Output)
        [BksDirect]::Focus($own,$script:Case.Main) -eq $Output -and $selection[0] -eq $selection[1] -and $selection[0] -lt $text.Length -and $text.Substring($selection[0]).StartsWith($Body,[StringComparison]::Ordinal)
    } ('answer-only navigation uses a collapsed native caret '+$Phase)
    $text=Own-Text $Output
    Check ('answer navigation uses native/UIA archive text and no PTY bytes '+$Phase) ((Same-UiaText (Uia-Text $Output) $text) -and (Raw-Input) -ceq $before)
    foreach($service in @('User: q','Thinking:','Searching:','Running:','Tool:','Status:','Footer:','BKS_TURN_FRAME_',([string][char]0x250c),([string][char]0x2502))) {
        Check ('answer archive excludes service/input chrome '+$service+' '+$Phase) (-not $text.Contains($service))
    }
    $observations.Add(@{Case='answer-only navigation';Phase=$Phase;Command=$Command;Text=$text;Selection=[BksStage7]::Selection($own,$Output);Viewport=[BksUnified]::Viewport($own,$Output);RawInput=(Raw-Input)})
}
function Test-RequestDisclosure([IntPtr]$Output) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main;$before=Raw-Input
    $original=Own-Text $Output;$closed=[BksUnified]::RequestCollapsed;$opened=[BksUnified]::RequestExpanded
    Check 'two accepted requests have separate collapsed headings in native/UIA text' ([regex]::Matches($original,[regex]::Escape($closed+'q')).Count -eq 2 -and (Same-UiaText (Uia-Text $Output) $original))
    $header=$original.IndexOf($closed,[StringComparison]::Ordinal)
    [BksStage7]::Select($own,$Output,$header,$header+$closed.Length)
    [BksDirect]::Key($own,$Output,13);Start-Sleep -Milliseconds 180
    Check 'selected request heading remains ordinary selectable text on Enter' ((Own-Text $Output) -ceq $original -and (Raw-Input) -ceq $before)
    $body=$original.IndexOf('BKS_TURN_001_BODY_HEAD',[StringComparison]::Ordinal)
    [BksStage7]::Select($own,$Output,$body,$body)
    [BksDirect]::Key($own,$Output,13);[BksDirect]::Key($own,$Output,32);Start-Sleep -Milliseconds 180
    Check 'Enter and Space inside an answer do not operate a request or write to the PTY' ((Own-Text $Output) -ceq $original -and (Raw-Input) -ceq $before)
    [BksStage7]::Select($own,$Output,$header,$header)
    [BksStage7]::ScrollTo($own,$Output,$header)
    [BksDirect]::Key($own,$Output,13)
    Wait-For { (Own-Text $Output).StartsWith($opened+'q'+"`r`nq`r`n",[StringComparison]::Ordinal) } 'Enter opens only the first request'
    $expanded=Own-Text $Output
    Check 'expanded request is native/UIA readable and remains readonly with reader focus' ((Same-UiaText (Uia-Text $Output) $expanded) -and [BksStage7]::ReadOnly($own,$Output) -and [BksDirect]::Focus($own,$main) -eq $Output -and $expanded.Contains($closed+'q'))
    Save-OwnScreenshot $Output 'tui-request-expanded.png'
    $selection=[BksStage7]::Selection($own,$Output)
    Check 'disclosure keeps the native caret on its heading' ($selection[0] -eq $header -and $selection[1] -eq $header)
    [BksUnified]::Repeat($own,$Output,13);[BksUnified]::Repeat($own,$Output,32);Start-Sleep -Milliseconds 180
    Check 'held Enter and Space do not repeatedly toggle the request' ((Own-Text $Output) -ceq $expanded)
    $requestStart=$expanded.IndexOf("`r`nq`r`n",[StringComparison]::Ordinal)+2
    [BksStage7]::Select($own,$Output,$requestStart,$requestStart+1)
    Check 'expanded request text can be selected without changing it' (([BksStage7]::Selection($own,$Output))[0] -eq $requestStart -and ([BksStage7]::Selection($own,$Output))[1] -eq $requestStart+1 -and (Own-Text $Output).Substring($requestStart,1) -ceq 'q')
    [BksStage7]::Select($own,$Output,$header,$header)
    [BksDirect]::Key($own,$Output,32)
    Wait-For { (Own-Text $Output) -ceq $original } 'Space collapses the same request'
    $last=$original.LastIndexOf($closed,[StringComparison]::Ordinal)
    [BksStage7]::Select($own,$Output,$last,$last)
    [BksDirect]::Key($own,$Output,13)
    Wait-For { (Own-Text $Output).Contains($opened+'q') } 'second request opens independently'
    Invoke-TurnNavigation $Output 60301 'BKS_TURN_001_BODY_HEAD' 'previous with second request expanded'
    Invoke-TurnNavigation $Output 60302 'BKS_TURN_002_BODY_HEAD' 'next with second request expanded'
    Check 'answer navigation preserves each request disclosure state' ((Own-Text $Output).StartsWith($closed+'q',[StringComparison]::Ordinal) -and (Own-Text $Output).Contains($opened+'q'))
    $expanded=Own-Text $Output;$last=$expanded.LastIndexOf($opened,[StringComparison]::Ordinal)
    [BksStage7]::Select($own,$Output,$last,$last)
    [BksDirect]::Key($own,$Output,32)
    Wait-For { (Own-Text $Output) -ceq $original } 'second request collapses independently'
    Check 'all request disclosure and reading actions emit zero PTY bytes' ((Raw-Input) -ceq $before -and [BksDirect]::Focus($own,$main) -eq $Output)
    Invoke-TurnNavigation $Output 60300 'BKS_TURN_002_BODY_HEAD' 'latest after request disclosure'
    $checks.Add(@{Status='skipped';Name='request disclosure NVDA state speech and physical keyboard gestures require human acceptance; clipboard was not changed'})
}
function Test-TuiTurns([IntPtr]$Field,[IntPtr]$Output,[IntPtr]$Journal,[IntPtr]$Legacy,[IntPtr]$Canvas,[IntPtr]$List,[IntPtr]$Status) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main;$fieldHwnd=$Field;$outputHwnd=$Output
    Check 'turn scenario requires its validated private desktop' ($PrivateDesktopName -and [BksQaDesktop]::CurrentDesktop -ceq $PrivateDesktopName)
    [BksDirect]::Post($own,$main,0x111,1009)
    Wait-For { [BksStage7]::ReadOnly($own,$Field) -and [BksStage7]::IsWindowVisible($Output) } 'explicit generic mode enables the turn fixture'
    Request-Scene 'turn-initial';Wait-TurnField $Field '' 'initial prompt'
    Check 'turn scene retains one native field/output and hides legacy surfaces' ((Field $main 1002) -eq $fieldHwnd -and (Field $main 1018) -eq $outputHwnd -and -not [BksStage7]::IsWindowVisible($Legacy) -and -not [BksStage7]::IsWindowVisible($Canvas))
    $size=Own-Text (Field $main 1015)
    $checks.Add(@{Status='skipped';Name='private turn fixture checks native EM_GETSEL and UIA text; interactive EDIT UIA selection/caret, spoken indices, ready sound and human NVDA are separate acceptance'})

    Single-Character $Field '/' 'turn slash-menu trigger'
    Wait-For { [BksStage7]::IsWindowVisible($List) } 'generic slash menu source appears'
    Wait-SingleSourceFocus $List 'turn slash menu'
    Check 'turn slash menu keeps strict native/UIA selection semantics' ([BksUnified]::ListSelection($own,$List) -eq 0 -and (Uia-Selected $List) -ceq 'Alpha > 1 '+[char]0x2192+' target')
    Check-Key $List 13 '0D' 'slash-menu confirmation is one explicit key'
    Wait-For { -not [BksStage7]::IsWindowVisible($List) } 'source slash-menu confirmation closes list'
    Wait-SingleSourceFocus $Field 'turn slash-menu close'
    Request-Scene 'turn-unarmed-done'
    Wait-For { (Own-Text $Output).Contains('BKS_TURN_000_BODY_HEAD') } 'unarmed source answer is visible as ordinary output'
    Assert-TurnNotReady $Status 'slash menu does not arm a request' 500
    $before=Raw-Input
    [BksDirect]::Post($own,$main,0x111,60300)
    Start-Sleep -Milliseconds 180
    Check 'answer navigation before a request sends zero PTY bytes' ((Raw-Input) -ceq $before)
    Request-Scene 'turn-initial'
    if([BksDirect]::Focus($own,$main) -eq $Output) {[BksDirect]::Post($own,$main,0x111,60003)}
    Wait-For { [BksDirect]::Focus($own,$main) -eq $Field } 'explicit return to native turn input'

    $firstStyle=$null
    for($turn=1;$turn -le 2;$turn++) {
        Wait-TurnField $Field '' ('before request '+$turn)
        Single-Character $Field 'q' ('plain request '+$turn)
        Wait-TurnField $Field 'q' ('acknowledged request '+$turn)
        Check-Key $Field 13 '0D' ('explicit request Enter '+$turn)
        Request-Scene 'turn-thinking';Wait-TurnPhase $Status ([BksUnified]::TurnThinking) ('thinking '+$turn)
        Wait-TurnField $Field '' ('busy composer absent '+$turn)
        if($turn -eq 2) {
            Invoke-TurnNavigation $Output 60300 'BKS_TURN_001_BODY_HEAD' 'read first answer while second request runs'
            $readingStart=([BksStage7]::Selection($own,$Output))[0]+4
            [BksStage7]::Select($own,$Output,$readingStart,$readingStart+5)
            [BksStage7]::ScrollTo($own,$Output,$readingStart)
            $pendingSelection=[BksStage7]::Selection($own,$Output);$pendingViewport=[BksUnified]::Viewport($own,$Output)
            $pendingText=(Own-Text $Output).Substring($pendingSelection[0],$pendingSelection[1]-$pendingSelection[0])
        }
        if($turn -eq 1) {Assert-TurnNotReady $Status 'thinking stable for at least 1.5 seconds' 1600}
        Request-Scene 'turn-searching';Wait-TurnPhase $Status ([BksUnified]::TurnSearching) ('search '+$turn)
        Request-Scene 'turn-running';Wait-TurnPhase $Status ([BksUnified]::TurnRunning) ('tool execution '+$turn)
        Request-Scene 'turn-answer'
        $head='BKS_TURN_'+$turn.ToString('D3')+'_BODY_HEAD'
        if($turn -eq 1) {Wait-For { (Own-Text $Output).Contains($head) } ('answer source begins streaming '+$turn)}
        Request-Scene 'turn-answer-more'
        if($turn -eq 1) {Wait-For { (Own-Text $Output).Contains('BKS_TURN_'+$turn.ToString('D3')+'_BODY_ROW_011') } ('answer source appends body '+$turn)}
        Assert-TurnNotReady $Status ('streamed answer still has visible busy '+$turn) 1600
        Request-Scene 'turn-done';Wait-TurnPhase $Status ([BksUnified]::TurnReady) ('completed answer '+$turn)
        Wait-TurnField $Field '' ('ready composer reappears '+$turn)
        $expectedFocus=if($turn -eq 1){$Field}else{$Output}
        Check ('source completion preserves current native focus and original dimensions '+$turn) ([BksDirect]::Focus($own,$main) -eq $expectedFocus -and (Own-Text (Field $main 1015)) -ceq $size -and (Field $main 1002) -eq $fieldHwnd)
        if($turn -eq 2) {
            $after=[BksStage7]::Selection($own,$Output)
            Check 'second answer completion preserves the first answer reader bookmark and viewport' ($after[0] -eq $pendingSelection[0] -and $after[1] -eq $pendingSelection[1] -and (Own-Text $Output).Substring($after[0],$after[1]-$after[0]) -ceq $pendingText -and ([BksUnified]::Viewport($own,$Output)).FirstLine -eq $pendingViewport.FirstLine)
        }
        $fieldSelection=[BksStage7]::Selection($own,$Field);$before=Raw-Input
        Request-Scene 'turn-passive';Start-Sleep -Milliseconds 250
        Check ('passive service update preserves current focus and source field caret '+$turn) ([BksDirect]::Focus($own,$main) -eq $expectedFocus -and ([BksStage7]::Selection($own,$Field))[0] -eq $fieldSelection[0] -and ([BksStage7]::Selection($own,$Field))[1] -eq $fieldSelection[1] -and (Raw-Input) -ceq $before)
        Invoke-TurnNavigation $Output 60300 $head ('last answer '+$turn)
        Check ('completed answer body is retained exactly once '+$turn) ([regex]::Matches((Own-Text $Output),[regex]::Escape($head)).Count -eq 1 -and (Own-Text $Output).Contains('BKS_TURN_'+$turn.ToString('D3')+'_BODY_TAIL'))
        $style=[BksUnified]::TextAttributes($own,$Output,$head)
        Check ('answer retains its source bold style '+$turn) ($style.Weight -ge 700 -and $style.Text -ceq $head)
        $observations.Add(@{Case='answer source style';Turn=$turn;Foreground=$style.Foreground;FontWeight=$style.Weight})
        if($turn -eq 1) {
            $firstStyle=$style
            $bookmark=[BksStage7]::Selection($own,$Output);$bookmarkStart=$bookmark[0]+4
            [BksStage7]::Select($own,$Output,$bookmarkStart,$bookmarkStart+5)
            [BksStage7]::ScrollTo($own,$Output,$bookmarkStart)
            $selection=[BksStage7]::Selection($own,$Output);$viewport=[BksUnified]::Viewport($own,$Output);$text=Own-Text $Output
            Assert-StaleSingleInput $Field $List 'turn answer reader'
            Request-Scene 'turn-initial';Start-Sleep -Milliseconds 300
            $after=[BksStage7]::Selection($own,$Output)
            Check 'source ED2/ED3 reset preserves archived answer, reader focus, selection and viewport' ((Own-Text $Output) -ceq $text -and $after[0] -eq $selection[0] -and $after[1] -eq $selection[1] -and ([BksUnified]::Viewport($own,$Output)).FirstLine -eq $viewport.FirstLine -and [BksDirect]::Focus($own,$main) -eq $Output)
            [BksDirect]::Post($own,$main,0x111,60003)
            Wait-For { [BksDirect]::Focus($own,$main) -eq $Field } 'explicit reader return before second request'
        } else {
            Check 'two source answers retain distinct colors' ($style.Foreground -ne $firstStyle.Foreground)
        }
    }
    Invoke-TurnNavigation $Output 60301 'BKS_TURN_001_BODY_HEAD' 'previous answer after second source reset'
    $preserved=[BksUnified]::TextAttributes($own,$Output,'BKS_TURN_001_BODY_HEAD')
    Check 'first archived answer style survives source reset and the second request' ($preserved.Foreground -eq $firstStyle.Foreground -and $preserved.Weight -eq $firstStyle.Weight)
    Invoke-TurnNavigation $Output 60302 'BKS_TURN_002_BODY_HEAD' 'next answer'
    Test-RequestDisclosure $Output
    $selection=[BksStage7]::Selection($own,$Output);$viewport=[BksUnified]::Viewport($own,$Output);$before=Raw-Input
    Request-Scene 'turn-passive';Start-Sleep -Milliseconds 250
    Check 'new service output preserves the answer reader position and focus' (([BksStage7]::Selection($own,$Output))[0] -eq $selection[0] -and ([BksStage7]::Selection($own,$Output))[1] -eq $selection[1] -and ([BksUnified]::Viewport($own,$Output)).FirstLine -eq $viewport.FirstLine -and [BksDirect]::Focus($own,$main) -eq $Output -and (Raw-Input) -ceq $before)
    Check 'turn scenario sends only slash confirmation and two explicit q+Enter requests' ((Raw-Input) -ceq '2F0D710D710D' -and $script:ExpectedInput -ceq '2F0D710D710D')
    Save-OwnScreenshot $Output 'tui-turns-answer-archive.png'
    Test-BulletTurns $Field $Output $List $Status
}
function Test-BulletTurns([IntPtr]$Field,[IntPtr]$Output,[IntPtr]$List,[IntPtr]$Status) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main;$before=Raw-Input
    [BksDirect]::Post($own,$main,0x111,60303)
    Start-Sleep -Milliseconds 150
    [BksDirect]::Post($own,$main,0x111,60003)
    Wait-For { [BksDirect]::Focus($own,$main) -eq $Field } 'explicit return after clearing the first acceptance archive'
    Check 'clear answer archive and return to source input emit zero PTY bytes' ((Raw-Input) -ceq $before -and $before -ceq '2F0D710D710D')
    Request-Scene 'bullet-initial';Wait-TurnField $Field '' 'fixed-footer bullet baseline'
    Wait-For { (Own-Text $Output).Contains('BKS_BULLET_SOURCE_HEADER') -and (Own-Text $Output).Contains('thinking: high') } 'fixed source footer baseline is visible'
    Assert-TurnNotReady $Status 'static thinking configuration is not activity or completion' 500
    $size=Own-Text (Field $main 1015);$fieldHwnd=$Field;$firstStyle=$null
    for($turn=1;$turn -le 2;$turn++) {
        Wait-TurnField $Field '' ('bullet request baseline '+$turn)
        Single-Character $Field 'q' ('bullet literal request '+$turn)
        Wait-TurnField $Field 'q' ('bullet source q echo '+$turn)
        Check-Key $Field 13 '0D' ('bullet explicit request Enter '+$turn)
        Request-Scene 'bullet-thinking';Wait-TurnPhase $Status ([BksUnified]::TurnThinking) ('bullet reasoning before retained footer '+$turn)
        Wait-TurnField $Field '' ('bullet busy source input '+$turn)
        if($turn -eq 2) {
            Invoke-TurnNavigation $Output 60300 'BKS_BULLET_001_BODY_HEAD' 'bullet first answer while second request runs'
            $start=([BksStage7]::Selection($own,$Output))[0]+4
            [BksStage7]::Select($own,$Output,$start,$start+5)
            [BksStage7]::ScrollTo($own,$Output,$start)
            $pendingSelection=[BksStage7]::Selection($own,$Output);$pendingViewport=[BksUnified]::Viewport($own,$Output)
            $pendingText=(Own-Text $Output).Substring($pendingSelection[0],$pendingSelection[1]-$pendingSelection[0])
        }
        Assert-TurnNotReady $Status ('fixed cancellation hint remains visible '+$turn) 1600
        Request-Scene 'bullet-answer'
        $head='BKS_BULLET_'+$turn.ToString('D3')+'_BODY_HEAD';$tail='BKS_BULLET_'+$turn.ToString('D3')+'_BODY_TAIL'
        if($turn -eq 1) {Wait-For { (Own-Text $Output).Contains($head) -and (Own-Text $Output).Contains('ctrl+c: cancel') } 'normal bullet answer is inserted above the same footer'}
        Request-Scene 'bullet-answer-more'
        if($turn -eq 1) {
            Wait-For { (Own-Text $Output).Contains('BKS_BULLET_001_BODY_ROW_005') } 'bullet body streams above retained footer'
            Save-OwnScreenshot $Output 'tui-turns-bullet-live-busy.png'
        }
        Assert-TurnNotReady $Status ('stable answer with cancellation hint must remain incomplete '+$turn) 1600
        Request-Scene 'bullet-done';Wait-TurnPhase $Status ([BksUnified]::TurnReady) ('cancel hint removed without Status done '+$turn)
        Wait-TurnField $Field '' ('empty bullet source composer after completion '+$turn)
        $expectedFocus=if($turn -eq 1){$Field}else{$Output}
        Check ('bullet completion preserves current focus, source field HWND and PTY geometry '+$turn) ([BksDirect]::Focus($own,$main) -eq $expectedFocus -and (Field $main 1002) -eq $fieldHwnd -and (Own-Text (Field $main 1015)) -ceq $size)
        if($turn -eq 2) {
            $after=[BksStage7]::Selection($own,$Output)
            Check 'persistently editable second source input never steals archived reader focus or bookmark' ($after[0] -eq $pendingSelection[0] -and $after[1] -eq $pendingSelection[1] -and (Own-Text $Output).Substring($after[0],$after[1]-$after[0]) -ceq $pendingText -and ([BksUnified]::Viewport($own,$Output)).FirstLine -eq $pendingViewport.FirstLine)
        }
        Invoke-TurnNavigation $Output 60300 $head ('latest bullet answer '+$turn)
        $text=Own-Text $Output
        foreach($service in @((''+[char]0x276f+' q'),'BKS_BULLET_REASON_','BKS_BULLET_SOURCE_HEADER','thinking: high','ctrl+c: cancel','context:','BKS_TURN_')) {
            Check ('bullet answer archive contains body only '+$service+' '+$turn) (-not $text.Contains($service))
        }
        Check ('bullet styled head and tail are retained exactly once '+$turn) ([regex]::Matches($text,[regex]::Escape($head)).Count -eq 1 -and [regex]::Matches($text,[regex]::Escape($tail)).Count -eq 1)
        $style=[BksUnified]::TextAttributes($own,$Output,$head);$tailStyle=[BksUnified]::TextAttributes($own,$Output,$tail)
        Check ('bullet head and tail retain source bold and matching foreground '+$turn) ($style.Weight -ge 700 -and $tailStyle.Weight -ge 700 -and $style.Foreground -eq $tailStyle.Foreground)
        $observations.Add(@{Case='generic bullet source answer';Turn=$turn;Head=$head;Tail=$tail;Foreground=$style.Foreground;FontWeight=$style.Weight;SourceBusyComposer=($turn -eq 2);RawInput=(Raw-Input)})
        if($turn -eq 1) {
            $firstStyle=$style;$selection=[BksStage7]::Selection($own,$Output);$viewport=[BksUnified]::Viewport($own,$Output);$text=Own-Text $Output;$before=Raw-Input
            Request-Scene 'bullet-reset';Start-Sleep -Milliseconds 300
            $after=[BksStage7]::Selection($own,$Output)
            Check 'bullet ED2 without ED3 preserves retained answer, selection, viewport and reader focus' ((Own-Text $Output) -ceq $text -and $after[0] -eq $selection[0] -and $after[1] -eq $selection[1] -and ([BksUnified]::Viewport($own,$Output)).FirstLine -eq $viewport.FirstLine -and [BksDirect]::Focus($own,$main) -eq $Output -and (Raw-Input) -ceq $before)
            [BksDirect]::Post($own,$main,0x111,60003)
            Wait-For { [BksDirect]::Focus($own,$main) -eq $Field } 'explicit return before second bullet request'
        } else {Check 'two bullet answers retain distinct source colors' ($style.Foreground -ne $firstStyle.Foreground)}
    }
    Invoke-TurnNavigation $Output 60301 'BKS_BULLET_001_BODY_HEAD' 'previous bullet body after source ED2'
    $preserved=[BksUnified]::TextAttributes($own,$Output,'BKS_BULLET_001_BODY_HEAD')
    Check 'first bullet color and bold survive ED2 and another request' ($preserved.Foreground -eq $firstStyle.Foreground -and $preserved.Weight -eq $firstStyle.Weight)
    Invoke-TurnNavigation $Output 60302 'BKS_BULLET_002_BODY_HEAD' 'next bullet body'
    $selection=[BksStage7]::Selection($own,$Output);$viewport=[BksUnified]::Viewport($own,$Output);$before=Raw-Input
    Request-Scene 'bullet-passive';Start-Sleep -Milliseconds 250
    Check 'unchanged bullet repaint preserves reader selection, viewport, focus and exact input ledger' (([BksStage7]::Selection($own,$Output))[0] -eq $selection[0] -and ([BksStage7]::Selection($own,$Output))[1] -eq $selection[1] -and ([BksUnified]::Viewport($own,$Output)).FirstLine -eq $viewport.FirstLine -and [BksDirect]::Focus($own,$main) -eq $Output -and (Raw-Input) -ceq $before)
    Check 'extended turn scenario sends exactly prior ledger plus two q+Enter requests' ((Raw-Input) -ceq '2F0D710D710D710D710D' -and $script:ExpectedInput -ceq '2F0D710D710D710D710D')
    Save-OwnScreenshot $Output 'tui-turns-bullet-answer-archive.png'
}
function Test-ShellEscape([IntPtr]$Field,[IntPtr]$Journal,[string]$Phase) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main
    $draft=Own-Text $Field;$history=Own-Text $Journal;$before=Raw-Input
    $selection=[BksStage7]::Selection($own,$Field)
    foreach($entry in @(@{Name='native shell field';Window=$Field},@{Name='native journal';Window=$Journal},@{Name='main window';Window=$main},@{Name='new-tab button';Window=(Field $main 1010)},@{Name='native shell field repeated';Window=$Field})) {
        # Use the actual queued-message path: SendMessage bypasses the dialog
        # preprocessor and did not reproduce this RichEdit WM_CLOSE regression.
        [BksDirect]::Post($own,$entry.Window,0x100,27)
        Start-Sleep -Milliseconds 150
        Check ($Phase+' posted Escape keeps '+$entry.Name+' and the main window alive') (-not $script:Case.Process.HasExited -and [BksStage7]::Responsive($own,$main,1000,2))
    }
    $after=[BksStage7]::Selection($own,$Field)
    Check ($Phase+' shell Escape preserves the draft, selection and journal') ((Own-Text $Field) -ceq $draft -and (Own-Text $Journal) -ceq $history -and $after[0] -eq $selection[0] -and $after[1] -eq $selection[1])
    [BksDirect]::Post($own,$main,0x111,60007)
    $query=Field $main 1020
    Wait-For { [BksStage7]::IsWindowVisible($query) } ($Phase+' own search opens')
    [BksDirect]::Post($own,$query,0x100,27)
    Wait-For { -not [BksStage7]::IsWindowVisible($query) } ($Phase+' Escape closes search')
    Check ($Phase+' search Escape preserves the main window and draft') (-not $script:Case.Process.HasExited -and [BksStage7]::Responsive($own,$main,1000,2) -and (Own-Text $Field) -ceq $draft)
    [BksDirect]::Post($own,$main,0x100,112)
    Wait-For { [BksStage7]::VisibleClass($own,'BksTermHelpWindow') -ne [IntPtr]::Zero } ($Phase+' own help opens')
    $help=[BksStage7]::VisibleClass($own,'BksTermHelpWindow')
    [BksDirect]::Post($own,(Field $help 1),0x100,27)
    Wait-For { [BksStage7]::VisibleClass($own,'BksTermHelpWindow') -eq [IntPtr]::Zero } ($Phase+' Escape closes help')
    Check ($Phase+' help Escape preserves the main window and draft') (-not $script:Case.Process.HasExited -and [BksStage7]::Responsive($own,$main,1000,2) -and (Own-Text $Field) -ceq $draft)
    Assert-ExpectedInput $before ($Phase+' shell Escape sends zero PTY bytes')
}
function Profile-ArgumentLabels([string[]]$Values) {
    for($index=0;$index -lt $Values.Count;$index++) {
        $label=if($Values[$index].Length -eq 0){'(пустой аргумент)'}else{$Values[$index]}
        '{0}. {1}' -f ($index+1),$label
    }
}
function Profile-Choice([uint32]$Own,[IntPtr]$Editor,[int]$Id,[int]$Index) {
    [BksStage7]::ComboChoice($Own,$Editor,(Field $Editor $Id),$Id,$Index)
    Start-Sleep -Milliseconds 150
}
function Test-LaunchProfileWorkflow {
    $own=$script:Case.Process.Id;$main=$script:Case.Main;$before=Raw-Input
    $path=Join-Path $script:Case.AppData 'BKS Term/profiles.json'
    [BksDirect]::Post($own,$main,0x111,60018)
    Wait-For { [BksStage7]::VisibleClass($own,'BksTermPreferences') -ne [IntPtr]::Zero } 'owned settings opens for profile workflow'
    $settingsWindow=[BksStage7]::VisibleClass($own,'BksTermPreferences')
    $profileButton=Field $settingsWindow 4613
    Check 'settings exposes an accessible native profiles button' ([BksStage7]::IsWindowVisible($profileButton) -and (Element $profileButton).Current.Name.Contains('Профили'))
    [BksDirect]::Post($own,$settingsWindow,0x111,4613)
    Wait-For { [BksStage7]::VisibleClass($own,'BksTermLaunchProfileEditor') -ne [IntPtr]::Zero } 'settings routes to owned launch profile editor'
    $editor=[BksStage7]::VisibleClass($own,'BksTermLaunchProfileEditor')
    $originalName=Own-Text (Field $editor 4310)
    [BksDirect]::Post($own,$editor,0x111,4303)
    Wait-For { (Own-Text (Field $editor 4310)) -cne $originalName } 'duplicate creates an editable profile variant'
    [BksStage7]::SetText($own,(Field $editor 4310),'Native workflow preset')
    Profile-Choice $own $editor 4305 1
    Profile-Choice $own $editor 4313 2
    Profile-Choice $own $editor 4316 1
    Profile-Choice $own $editor 4305 2
    $arguments=Field $editor 4317
    $expected=@('--single-input-session')
    foreach($argument in @('alpha','beta','','two words')) {
        [BksStage7]::SetText($own,(Field $editor 4318),$argument)
        [BksDirect]::Post($own,$editor,0x111,4319)
        $expected+=,$argument
        Wait-For { ([BksUnified]::ListItems($own,$arguments) -join '|') -ceq ((Profile-ArgumentLabels $expected) -join '|') } 'Add appends one literal argument without replacing its predecessor'
    }
    Check 'native profile argv preserves order, an empty argument and spaces' (([BksUnified]::ListItems($own,$arguments) -join '|') -ceq ((Profile-ArgumentLabels $expected) -join '|'))
    Save-OwnScreenshot $editor 'launch-profile-native-arguments.png'
    [BksDirect]::Post($own,(Field $editor 4301),0x14f,1)
    Wait-For { [BksStage7]::VisibleClass($own,'ComboLBox') -ne [IntPtr]::Zero } 'native shell dropdown opens without changing selection'
    Save-OwnScreenshot ([BksStage7]::VisibleClass($own,'ComboLBox')) 'launch-profile-native-shell-popup.png'
    [BksDirect]::Post($own,(Field $editor 4301),0x14f,0)
    Wait-For { [BksStage7]::VisibleClass($own,'ComboLBox') -eq [IntPtr]::Zero } 'native shell dropdown closes without executing a command'
    Profile-Choice $own $editor 4305 3
    [BksStage7]::SetText($own,(Field $editor 4324),'BKS_PROFILE_GUI_FIXTURE=fixture value')
    Profile-Choice $own $editor 4301 1
    [BksDirect]::Post($own,$editor,0x111,4306)
    Wait-For { [BksStage7]::VisibleClass($own,'BksTermLaunchProfileEditor') -eq [IntPtr]::Zero } 'native profile save returns to settings'
    $confirmedProfile=@{Document=$null}
    Wait-For {
        try {
            $document=Read-BksJson $path
            if($document.schema_version -ne 2 -or @($document.profiles | Where-Object {$_.name -ceq 'Native workflow preset'}).Count -ne 1) {return $false}
            $confirmedProfile.Document=$document
            return $true
        } catch {
            if($_.Exception -is [IO.IOException] -or $_.Exception.InnerException -is [IO.IOException]) {return $false}
            throw
        }
    } 'main persists the confirmed profile snapshot and releases its write lock'
    $saved=$confirmedProfile.Document
    $profile=@($saved.profiles | Where-Object {$_.name -ceq 'Native workflow preset'})
    Check 'profile workflow migrates its private file and preserves the selected global shell' ($saved.schema_version -eq 2 -and $saved.default_shell_profile_id -ceq 'windows-cmd' -and $profile.Count -eq 1)
    $profile=$profile[0]
    Check 'saved profile has the exact argv and adapter/exit choices' (($profile.args -join '|') -ceq ($expected -join '|') -and $profile.launch.adapter -ceq 'universal' -and $profile.launch.after_exit -ceq 'open_shell')
    $originalJson=@($saved.profiles | Where-Object {$_.id -ceq 'unified-fixture'})[0] | ConvertTo-Json -Depth 10 -Compress
    Check 'saved profile environment belongs only to that variant' ($profile.launch.environment.BKS_PROFILE_GUI_FIXTURE -ceq 'fixture value' -and -not $originalJson.Contains('BKS_PROFILE_GUI_FIXTURE'))
    Assert-ExpectedInput $before 'editing and saving launch profiles sends zero PTY bytes'
    [BksDirect]::Post($own,$main,0x111,60022)
    Wait-For { [BksStage7]::VisibleClass($own,'BksTermLaunchProfileEditor') -ne [IntPtr]::Zero } 'profile menu command reopens saved variants'
    $editor=[BksStage7]::VisibleClass($own,'BksTermLaunchProfileEditor')
    $ids=@($saved.profiles | Where-Object {$_.kind -ceq 'local'} | ForEach-Object {$_.id})
    Profile-Choice $own $editor 4300 ([Array]::IndexOf($ids,$profile.id))
    Wait-For { (Own-Text (Field $editor 4310)) -ceq 'Native workflow preset' } 'saved profile can be selected again'
    Profile-Choice $own $editor 4305 2
    Check 'reopened native profile retains the literal argv list' (([BksUnified]::ListItems($own,(Field $editor 4317)) -join '|') -ceq ((Profile-ArgumentLabels $expected) -join '|'))
    [BksDirect]::Post($own,$editor,0x111,4307)
    Wait-For { [BksStage7]::VisibleClass($own,'BksTermLaunchProfileEditor') -eq [IntPtr]::Zero } 'profile Cancel closes only its editor'
    [BksDirect]::Post($own,$settingsWindow,0x111,4608)
    Wait-For { [BksStage7]::VisibleClass($own,'BksTermPreferences') -eq [IntPtr]::Zero } 'settings Cancel restores the terminal'
    Check 'profile editing leaves the original terminal session alive' (-not $script:Case.Process.HasExited -and -not $script:Case.Fixture.HasExited -and [BksStage7]::Responsive($own,$main,1000,2))
    Assert-ExpectedInput $before 'the complete profile settings workflow never launches a configured command'
}
function Test-SingleInput([IntPtr]$Field,[IntPtr]$Output,[IntPtr]$Journal,[IntPtr]$Legacy,[IntPtr]$Canvas,[IntPtr]$List) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main;$fieldHwnd=$Field
    $seed='BKS_SINGLE_INPUT '+[char]0x0416+[char]0x754c+'e'+[char]0x301+[char]::ConvertFromUtf32(0x1f600)
    Test-LaunchProfileWorkflow
    $labels=@(('Alpha > 1 '+[char]0x2192+' target'),'Bravo | literal (2)','Charlie [on|off]')
    Wait-For { (Own-Text $Journal).Contains('BKS_FAKE_SHELL_READY') -and (Own-Text $Journal).Contains('fake> ') } 'single-field managed shell prompt'
    Check 'single input starts as the native editable shell field' ([BksStage7]::IsWindowVisible($Field) -and -not [BksStage7]::ReadOnly($own,$Field) -and [BksStage7]::IsWindowVisible($Journal) -and (Raw-Input).Length -eq 0)
    $size=Own-Text (Field $main 1015)
    $draft='BKS_UNSENT_NATIVE_DRAFT'
    [BksStage7]::SetText($own,$Field,$draft)
    Test-ShellEscape $Field $Journal 'before TUI'
    [BksDirect]::Post($own,$main,0x111,1009)
    Wait-For { [BksStage7]::ReadOnly($own,$Field) -and [BksStage7]::IsWindowVisible($Output) } 'explicit generic TUI mode keeps the same shared field'
    Request-Scene 'native-prompt'
    Wait-SingleProjection $Field $Output $seed 'initial boxed Unicode composer'
    Request-Scene 'native-multiline'
    Wait-SingleProjection $Field $Output ($seed+"`r`nBKS_SINGLE_SECOND_ROW") 'multiline composer' 'single-input-multiline.png'
    Request-Scene 'native-prompt'
    Wait-SingleProjection $Field $Output $seed 'restored single-line composer'
    Check 'TUI exposes one readonly shared EDIT and projected RichEdit while both old surfaces remain hidden' ((Field $main 1002) -eq $fieldHwnd -and [BksStage7]::IsWindowVisible($Field) -and [BksStage7]::IsTextControl($own,$Field) -and [BksStage7]::ReadOnly($own,$Field) -and [BksStage7]::ReadOnly($own,$Output) -and -not [BksStage7]::IsWindowVisible($Legacy) -and -not [BksStage7]::IsWindowVisible($Canvas) -and -not [BksStage7]::IsWindowVisible($List))
    Wait-For { [BksDirect]::Focus($own,$main) -eq $Field } 'TUI source acknowledgement retains native field focus'
    Assert-ExpectedInput '' 'mode and initial source projection'
    [BksDirect]::Watch($own,(Field $main 1026))
    Check-Key $Field 9 '09' 'Tab after same-HWND shell to TUI handoff'
    Check 'plain TUI Tab keeps the shared field focused before any popup exists' ([BksDirect]::Focus($own,$main) -eq $Field -and -not [BksStage7]::IsWindowVisible($List))

    Request-Scene 'hold'
    $held=Own-Text $Field;$heldSelection=[BksStage7]::Selection($own,$Field);$noticeStart=[BksDirect]::Notices().Length
    $omega=[string][char]0x03a9
    Single-Character $Field $omega 'held readonly-field Unicode'
    Check 'readonly shared field and caret do not echo speculatively before VT acknowledgement' ((Own-Text $Field) -ceq $held -and ([BksStage7]::Selection($own,$Field))[0] -eq $heldSelection[0] -and ([BksStage7]::Selection($own,$Field))[1] -eq $heldSelection[1] -and @([BksDirect]::Notices() | Select-Object -Skip $noticeStart | Where-Object {$_.Text -ceq $omega}).Count -eq 0)
    Request-Scene 'release'
    $value=$seed+$omega
    Wait-SingleProjection $Field $Output $value 'acknowledged Unicode'
    $emoji=[char]::ConvertFromUtf32(0x1f600)
    Single-Character $Field $emoji 'shared-field surrogate pair'
    $value+=$emoji
    Wait-SingleProjection $Field $Output $value 'acknowledged surrogate pair'

    [BksStage7]::Select($own,$Field,0,3)
    $copySelection=[BksStage7]::Selection($own,$Field);$before=Raw-Input
    Request-Scene 'passive-hidden'
    Wait-For { (Own-Text $Output).Contains('Tick: 000001') } 'passive upper output updates independently of the field'
    Start-Sleep -Milliseconds 150
    $pattern=$null
    if(-not (Element $Field).TryGetCurrentPattern([Windows.Automation.TextPattern]::Pattern,[ref]$pattern)) {throw 'Shared field has no native TextPattern'}
    Check 'passive output preserves a local copy selection without changing the TUI input' ((Own-Text $Field) -ceq $value -and ([BksStage7]::Selection($own,$Field))[0] -eq $copySelection[0] -and ([BksStage7]::Selection($own,$Field))[1] -eq $copySelection[1] -and (Own-Text $Field).Substring($copySelection[0],$copySelection[1]-$copySelection[0]) -ceq 'BKS' -and (Raw-Input) -ceq $before)
    Request-Scene 'passive'
    Wait-For { (Own-Text $Output).Contains('Tick: 000002') } 'passive repaint restores the source cursor'
    Check 'source cursor return keeps the local copy selection' (([BksStage7]::Selection($own,$Field))[0] -eq $copySelection[0] -and ([BksStage7]::Selection($own,$Field))[1] -eq $copySelection[1] -and (Raw-Input) -ceq $before)
    if($SingleInput -and $PrivateDesktopName -and [BksQaDesktop]::CurrentDesktop -ceq $PrivateDesktopName) {
        $checks.Add(@{Status='skipped';Name='interactive EDIT UIA selected range unavailable on private desktop; native selection and UIA text remain checked'})
    } else {
        Check 'UIA exposes the selected local copy range' ($pattern.GetSelection()[0].GetText(-1) -ceq 'BKS')
    }

    Test-SingleSelection $Field $Output
    Request-Scene 'native-prompt';$value=$seed
    Wait-SingleProjection $Field $Output $value 'restored composer after native selection edits'
    [BksStage7]::Select($own,$Field,$value.Length,$value.Length)

    Single-Character $Field '/' 'source menu trigger'
    $value+='/'
    Wait-For { [BksStage7]::IsWindowVisible($List) } 'acknowledged source menu exposes inline native LISTBOX'
    Wait-SingleSourceFocus $List 'first source menu'
    Wait-For { ([BksUnified]::ListItems($own,$List) -join "`n") -ceq ($labels -join "`n") -and [BksUnified]::ListSelection($own,$List) -eq 0 } 'generic menu labels and initial source selection'
    Save-OwnScreenshot $List 'single-input-menu.png'
    $textPattern=$null
    Check 'inline list provides selection semantics without an editable TextPattern' ((Element $List).Current.ControlType -eq [Windows.Automation.ControlType]::List -and -not (Element $List).TryGetCurrentPattern([Windows.Automation.TextPattern]::Pattern,[ref]$textPattern) -and (Uia-Selected $List) -ceq $labels[0] -and ((Uia-Rows $List) -join "`n") -ceq ($labels -join "`n"))
    Check 'projected upper output excludes native menu copies and active input' (-not (Own-Text $Output).Contains('Alpha > 1') -and -not (Own-Text $Output).Contains('Bravo | literal') -and -not (Own-Text $Output).Contains($value))
    [BksDirect]::Post($own,$main,0x111,60003)
    Wait-For { [BksDirect]::Focus($own,$main) -eq $Output } 'explicit native reader takes focus before stale input checks'
    Assert-StaleSingleInput $Field $List 'explicit reader'
    [BksDirect]::Post($own,$main,0x111,60003)
    Wait-For { [BksDirect]::Focus($own,$main) -eq $List } 'explicit reader return restores the source menu'
    Assert-SingleProjectionContinuity $Output $size 'after explicit reader closes'
    [BksDirect]::Post($own,$main,0x111,60007)
    $query=Field $main 1020
    Wait-For { [BksStage7]::IsWindowVisible($query) -and [BksDirect]::Focus($own,$main) -eq $query } 'explicit native search takes focus before stale input checks'
    Assert-StaleSingleInput $Field $List 'explicit search'
    [BksDirect]::Post($own,$main,0x111,1023)
    Wait-For { -not [BksStage7]::IsWindowVisible($query) -and [BksDirect]::Focus($own,$main) -eq $List } 'explicit search close restores the source menu'
    Assert-SingleProjectionContinuity $Output $size 'after explicit search closes'
    Request-Scene 'hold';$noticeStart=[BksDirect]::Notices().Length
    Check-Key $List 0x28 '1B5B42' 'held native-list Down'
    Check 'LISTBOX does not speculate its selection before a source frame' ([BksUnified]::ListSelection($own,$List) -eq 0 -and (Uia-Selected $List) -ceq $labels[0])
    Request-Scene 'release'
    Wait-For { [BksUnified]::ListSelection($own,$List) -eq 1 -and (Uia-Selected $List) -ceq $labels[1] } 'acknowledged native-list selection moves to Bravo'
    Start-Sleep -Milliseconds 220
    Save-OwnScreenshot $List 'single-input-menu-bravo.png'
    Check 'native menu selection emits no competing custom row feedback' (@([BksDirect]::Notices() | Select-Object -Skip $noticeStart | Where-Object {$_.Text.Contains('Bravo | literal')}).Count -eq 0)
    Request-Scene 'hold'
    Wait-SingleListFocus $List 'after Down acknowledgement before list character'
    Single-Character $List 'C' 'held list character'
    Check 'native LISTBOX typeahead does not consume a character or select Charlie' ([BksUnified]::ListSelection($own,$List) -eq 1 -and (Own-Text $Field) -ceq $value)
    Request-Scene 'release';$value+='C'
    Wait-For { (Own-Text $Field) -ceq $value -and [BksUnified]::ListSelection($own,$List) -eq 1 } 'list character is acknowledged in the actual source composer'
    Check-Key $List 0x26 '1B5B41' 'native-list Up'
    Wait-For { [BksUnified]::ListSelection($own,$List) -eq 0 } 'source Up acknowledgement'
    Check-Key $List 9 '09' 'native-list Tab'
    Request-Scene 'hold'
    Check-Key $List 13 '0D' 'held native-list Enter'
    Check 'Enter does not hide the inline list before source acknowledgement' ([BksStage7]::IsWindowVisible($List) -and [BksDirect]::Focus($own,$main) -eq $List)
    Request-Scene 'release'
    Wait-For { -not [BksStage7]::IsWindowVisible($List) -and (Own-Text $Field).Length -eq 0 } 'source closes menu and empties the same native field'
    Wait-SingleSourceFocus $Field 'source Enter closes menu'
    $before=Raw-Input
    Start-Sleep -Milliseconds 150
    Assert-ExpectedInput $before 'native menu source-close focus restoration'
    Single-Character $Field '/' 'second source menu trigger'
    Wait-For { [BksStage7]::IsWindowVisible($List) } 'second source menu opens'
    Wait-SingleSourceFocus $List 'second source menu'
    Check-Key $List 27 '1B' 'native-list Escape'
    Wait-For { -not [BksStage7]::IsWindowVisible($List) } 'source Escape acknowledgement closes the list'
    Wait-SingleSourceFocus $Field 'source Escape closes menu'

    Test-RuleComposer $Field $Output $List $labels
    Test-NativeEditors $Field $Output

    $before=Raw-Input
    Request-Scene 'native-unknown'
    Wait-For { (Own-Text $Field).Length -eq 0 -and (Own-Text $Output).Contains('BKS_SINGLE_FALLBACK_UNIQUE') -and (Own-Text $Output).Contains('> candidate one') -and (Own-Text $Output).Contains('> candidate two') -and -not [BksStage7]::IsWindowVisible($List) } 'complete ambiguous hidden-cursor source leaves the shared field empty'
    $observations.Add(@{Case='unknown source projection';InputName=(Element $Field).Current.Name;ExpectedName=[BksUnified]::SingleFieldName;Readonly=[BksStage7]::ReadOnly($own,$Field);ListVisible=[BksStage7]::IsWindowVisible($List);FieldLength=(Own-Text $Field).Length})
    Check 'unknown source retains its full text above an empty named direct field' ((@([BksUnified]::SingleFieldName,'Ввод TUI · клавиши передаются приложению') -ccontains (Element $Field).Current.Name) -and [BksStage7]::ReadOnly($own,$Field) -and (Own-Text $Output).Contains('> candidate one') -and (Own-Text $Output).Contains('> candidate two') -and -not [BksStage7]::IsWindowVisible($List))
    Check-Key $Field 0x2e '1B5B337E' 'unknown composer forwards ordinary Delete without synthesizing selection keys'
    $before=$script:ExpectedInput
    $secret='NEVER_EXPOSE_SOURCE'
    Request-Scene 'native-masked'
    Wait-For { $masked=Own-Text $Field;$body=Own-Text $Output; $masked.Length -eq $secret.Length -and [string]::IsNullOrWhiteSpace($masked) -and $body.LastIndexOf('BKS_SINGLE_OUTPUT_UNIQUE',[StringComparison]::Ordinal) -gt $body.LastIndexOf('BKS_SINGLE_FALLBACK_UNIQUE',[StringComparison]::Ordinal) } 'complete masked source field acknowledged while retaining earlier output history'
    Check 'concealed source text is absent from native and UIA field/output and custom notifications' (-not (Own-Text $Field).Contains($secret) -and -not (Uia-Text $Field).Contains($secret) -and -not (Own-Text $Output).Contains($secret) -and -not (Uia-Text $Output).Contains($secret) -and @([BksDirect]::Notices() | Where-Object {$_.Text.Contains($secret)}).Count -eq 0)
    Assert-ExpectedInput $before 'unknown and concealed source updates'
    [BksStage7]::Select($own,$Field,0,$secret.Length)
    Check-Key $Field 8 '7F' 'protected selection forwards one ordinary Backspace without synthesizing source edits'
    $before=$script:ExpectedInput
    Request-Scene 'single-shell'
    [BksDirect]::Post($own,$main,0x111,1009)
    Wait-For { -not [BksStage7]::ReadOnly($own,$Field) -and [BksStage7]::IsWindowVisible($Journal) -and (Own-Text $Field) -ceq $draft } 'manual source-mode return restores the separate unsent shell draft'
    Check 'manual TUI roundtrip preserves field HWND, shell draft, dimensions and exact fixture bytes' ((Field $main 1002) -eq $fieldHwnd -and [BksStage7]::IsWindowVisible($Field) -and -not [BksStage7]::IsWindowVisible($Legacy) -and -not [BksStage7]::IsWindowVisible($Canvas) -and (Own-Text (Field $main 1015)) -ceq $size -and (Raw-Input) -ceq $before)

    [BksStage7]::SetText($own,$Field,'kimi')
    $before=Raw-Input
    [BksDirect]::Post($own,$main,0x111,1003)
    Wait-For { (Raw-Input).Length -ge $before.Length+12 -and [BksStage7]::ReadOnly($own,$Field) -and (Own-Text $Output).Contains('BKS_FAKE_CLI_READY_001') } 'accepted bare synthetic command hands the same field to the managed fake CLI'
    Confirm-Input $before '6B696D690D0A' 'single-field shell submission sends literal kimi plus CRLF exactly once'
    Wait-SingleProjection $Field $Output $seed 'automatic synthetic CLI handoff'
    $before=Raw-Input
    Request-Scene 'cli-exit'
    Wait-For { -not [BksStage7]::ReadOnly($own,$Field) -and [BksStage7]::IsWindowVisible($Journal) -and (Own-Text $Journal).Contains('BKS_FAKE_CLI_DONE_001') -and (Own-Text $Field).Length -eq 0 } 'fresh parent prompt restores editable shell mode in the same input HWND'
    Assert-ExpectedInput $before 'automatic single-field CLI return sends no Enter or stale composer text'
    Check 'automatic handoff and return keep the original live fixture and native field' ((Field $main 1002) -eq $fieldHwnd -and -not $script:Case.Fixture.HasExited -and (Own-Text (Field $main 1015)) -ceq $size -and -not [BksStage7]::IsWindowVisible($List))
    Test-ShellEscape $Field $Journal 'after CLI return'
    $observations.Add(@{Case='single native input shell/TUI source-owned field and menu';FieldHwnd=$fieldHwnd.ToInt64();RawInput=(Raw-Input);NativeField=(Own-Text $Field);NativeJournal=(Own-Text $Journal);NativeMenuLabels=$labels;InputMechanism='own HWND posted WM_KEYDOWN/WM_KEYUP/WM_CHAR and explicit WM_COMMAND; no activation or keyboard-state modification';NvdaSpeech='requires human NVDA test'})
    Test-ShellOutput $Field $Journal $Legacy $Output
    Test-ShellBlocks $Field $Journal
    $checks.Add(@{Status='skipped';Name='Human NVDA speech and same-HWND pre-poll stale-Enter race are not proven by this isolated source/byte/UIA scenario; the race needs the pure main routing regression'})
}
function Test-CliHandoff([IntPtr]$InputBox,[IntPtr]$Journal,[IntPtr]$Direct,[IntPtr]$Reader) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main;$fixturePid=$script:Case.Fixture.Id;$canvas=Field $main 1017
    Wait-For { (Own-Text $Journal).Contains('BKS_FAKE_SHELL_READY') -and (Own-Text $Journal).Contains('fake> ') } 'managed shell initial OSC133 prompt is projected'
    Check 'keyless ordinary shell starts with editable native input and journal' ([BksStage7]::IsWindowVisible($InputBox) -and [BksStage7]::IsWindowVisible($Journal) -and -not [BksStage7]::IsWindowVisible($Direct) -and -not [BksStage7]::ReadOnly($own,$InputBox) -and (Raw-Input).Length -eq 0)
    Start-Sleep -Milliseconds 150
    $size=Own-Text (Field $main 1015)
    $observations.Add(@{Case='keyless CLI handoff initial shell';FixturePid=$fixturePid;PtySize=$size;NativeJournal=(Own-Text $Journal);InputMechanism='own WM_SETTEXT and WM_COMMAND1003 only; no focus/foreground/keyboard helpers'})
    foreach($iteration in 1..2) {
        $before=Raw-Input
        [BksStage7]::SetText($own,$InputBox,'kimi')
        [BksDirect]::Post($own,$main,0x111,1003)
        Wait-For { (Raw-Input).Length -ge $before.Length+12 } 'explicit own native submit reaches only managed fake shell'
        Wait-For { [BksStage7]::IsWindowVisible($Direct) -and -not [BksStage7]::IsWindowVisible($InputBox) -and (Own-Text $Direct).Contains(('BKS_FAKE_CLI_READY_'+$iteration.ToString('D3'))) } 'known bare user command automatically exposes direct TUI'
        Check-RawCanvasFocus $Direct $canvas ('after own submit WM_COMMAND cycle '+$iteration)
        Check-RawTextPatterns $Direct $canvas ('after own submit WM_COMMAND cycle '+$iteration)
        Start-Sleep -Milliseconds 150
        Confirm-Input $before '6B696D690D0A' 'each explicit kimi submission sends exactly literal UTF8 plus CRLF once to the managed fixture'
        Check 'automatic CLI handoff keeps the same live fixture and PTY dimensions' (-not $script:Case.Fixture.HasExited -and $script:Case.Fixture.Id -eq $fixturePid -and (Own-Text (Field $main 1015)) -ceq $size -and (Own-Text $InputBox).Length -eq 0)
        if($iteration -eq 1) {Save-OwnScreenshot $Direct 'cli-active.png'}
        if($iteration -eq 2) {
            # Verified main menu IDs: 60005 selects and opens the VT reader;
            # 60003 explicitly returns from reading. ID1010 is New Tab.
            [BksDirect]::Post($own,$main,0x111,60005)
            Wait-For { [BksStage7]::IsWindowVisible($Reader) -and (Own-Text $Reader).Contains('BKS_FAKE_CLI_READY_002') } 'own explicit menu command opens VT reader before fake CLI exit'
            $readerText=Own-Text $Reader
            $anchor=$readerText.LastIndexOf('BKS_FAKE_CLI_READY_002',[StringComparison]::Ordinal)
            [BksStage7]::Select($own,$Reader,$anchor,$anchor+22)
            [BksStage7]::ScrollTo($own,$Reader,$anchor)
            $selection=[BksStage7]::Selection($own,$Reader)
            $firstLine=[BksStage7]::FirstLine($own,$Reader)
            $selected=$readerText.Substring($selection[0],$selection[1]-$selection[0])
        }
        $expected=Raw-Input
        Request-Scene 'cli-exit'
        if($iteration -eq 1) {
            Wait-For { [BksStage7]::IsWindowVisible($InputBox) -and [BksStage7]::IsWindowVisible($Journal) -and -not [BksStage7]::IsWindowVisible($Direct) -and (Own-Text $Journal).Contains('BKS_FAKE_CLI_DONE_001') } 'parent OSC133 prompt returns automatically to native shell input'
        } else {
            Wait-For { (Own-Text $Reader).Contains('BKS_FAKE_CLI_DONE_002') -or (Own-Text $Journal).Contains('BKS_FAKE_CLI_DONE_002') } 'managed CLI exit and parent prompt reached the same session'
            Start-Sleep -Milliseconds 700
            $after=[BksStage7]::Selection($own,$Reader)
            Check 'parent prompt preserves the open reader selection and viewport until explicit return' ([BksStage7]::IsWindowVisible($Reader) -and -not [BksStage7]::IsWindowVisible($InputBox) -and $after[0] -eq $selection[0] -and $after[1] -eq $selection[1] -and (Own-Text $Reader).Substring($after[0],$after[1]-$after[0]) -ceq $selected -and [BksStage7]::FirstLine($own,$Reader) -eq $firstLine)
            $observations.Add(@{Case='parent prompt while VT reader is open';BeforeSelection=$selection;AfterSelection=$after;SelectedText=$selected;FirstVisibleLine=$firstLine;PtySize=(Own-Text (Field $main 1015));RawInput=(Raw-Input)})
            Save-OwnScreenshot $Reader 'cli-reader-deferred.png'
            [BksDirect]::Post($own,$main,0x111,60003)
            Wait-For { [BksStage7]::IsWindowVisible($InputBox) -and [BksStage7]::IsWindowVisible($Journal) -and -not [BksStage7]::IsWindowVisible($Direct) -and -not [BksStage7]::IsWindowVisible($Reader) } 'explicit own return applies the deferred native shell view'
        }
        Start-Sleep -Milliseconds 150
        Check 'CLI exit and reader actions add zero PTY bytes and preserve the same process and dimensions' ((Raw-Input) -ceq $expected -and -not $script:Case.Fixture.HasExited -and $script:Case.Fixture.Id -eq $fixturePid -and (Own-Text (Field $main 1015)) -ceq $size -and (Own-Text $InputBox).Length -eq 0)
    }
    Check 'complete keyless two-cycle CLI handoff sends no automatic Enter or extra command' ((Raw-Input) -ceq '6B696D690D0A6B696D690D0A' -and [BksStage7]::Responsive($own,$main,1000,2))
    $observations.Add(@{Case='keyless CLI handoff final shell';FixturePid=$fixturePid;PtySize=(Own-Text (Field $main 1015));RawInput=(Raw-Input);NativeJournal=(Own-Text $Journal);NvdaSpeech='not tested'})
    Test-ShellOutput $InputBox $Journal $Direct $Reader
}
function Assert-ShellBlockCaret([IntPtr]$Journal,[int]$Anchor,[string]$Line,[string]$Name) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main
    try {
        Wait-For {
            $selection=[BksStage7]::Selection($own,$Journal)
            [BksDirect]::Focus($own,$main) -eq $Journal -and $selection[0] -eq $Anchor -and $selection[1] -eq $Anchor
        } $Name
    } catch {
        $observations.Add(@{Case=$Name;ExpectedAnchor=$Anchor;ExpectedLine=$Line;ActualSelection=([BksStage7]::Selection($own,$Journal));ActualFocus=[BksDirect]::Focus($own,$main).ToInt64();JournalHwnd=$Journal.ToInt64();InputHwnd=(Field $main 1002).ToInt64();NativeJournal=(Own-Text $Journal)})
        throw
    }
    $pattern=$null
    if(-not (Element $Journal).TryGetCurrentPattern([Windows.Automation.TextPattern]::Pattern,[ref]$pattern)) {throw 'Journal has no native TextPattern'}
    $range=$pattern.GetSelection()[0].Clone()
    $range.ExpandToEnclosingUnit([Windows.Automation.Text.TextUnit]::Line)
    Check ($Name+' native caret and UIA line start at the actual output') ((Own-Text $Journal).Substring($Anchor).StartsWith($Line,[StringComparison]::Ordinal) -and (Normalize-Text $range.GetText(-1)).TrimEnd("`n") -ceq $Line)
}
function Test-ShellBlocks([IntPtr]$InputBox,[IntPtr]$Journal) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main
    $commands=@('ls','docker ps','ls');$anchors=@();$lines=@();$beforeAll=Raw-Input
    for($index=0;$index -lt $commands.Count;$index++) {
        $number=($index+1).ToString('D3')
        $line='    BKS_NAV_OUTPUT_'+$number+' '+[char]0x0416+[char]::ConvertFromUtf32(0x1f600)
        $tail='BKS_NAV_TAIL_'+$number
        $before=Raw-Input
        [BksStage7]::SetText($own,$InputBox,$commands[$index])
        [BksDirect]::Post($own,$main,0x111,1003)
        $bytes=([BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes($commands[$index]+"`r`n"))).Replace('-','')
        Wait-For { (Raw-Input).Length -ge $before.Length+$bytes.Length } ('three-result command '+$number+' input is accepted')
        Confirm-Input $before $bytes ('three-result command '+$number+' reaches only the fixture once')
        Wait-For { $text=Own-Text $Journal; $text.Contains($line) -and $text.Contains($tail) -and $text.TrimEnd().EndsWith('fake>') } ('three-result shell command '+$number+' completes')
        $anchors+=,(Own-Text $Journal).IndexOf($line,[StringComparison]::Ordinal)
        $lines+=,$line
    }
    Check 'three completed commands retain three distinct output positions' ($anchors[0] -ge 0 -and $anchors[0] -lt $anchors[1] -and $anchors[1] -lt $anchors[2])
    $draft='BKS_UNSENT_BLOCK_NAV_DRAFT'
    [BksStage7]::SetText($own,$InputBox,$draft)
    $before=Raw-Input
    [BksDirect]::Post($own,$InputBox,0x100,0x76)
    Assert-ShellBlockCaret $Journal $anchors[2] $lines[2] 'previous block from command input reaches the third result'
    foreach($index in @(1,0,0)) {
        [BksDirect]::Post($own,$Journal,0x100,0x76)
        Assert-ShellBlockCaret $Journal $anchors[$index] $lines[$index] ('previous block reaches result '+($index+1))
    }
    foreach($index in @(1,2,2)) {
        [BksDirect]::Post($own,$Journal,0x100,0x77)
        Assert-ShellBlockCaret $Journal $anchors[$index] $lines[$index] ('next block reaches result '+($index+1))
    }
    [BksDirect]::Post($own,$main,0x111,60003)
    Wait-For { [BksDirect]::Focus($own,$main) -eq $InputBox } 'return from third result to the preserved command draft'
    [BksDirect]::Post($own,$InputBox,0x100,0x77)
    Assert-ShellBlockCaret $Journal $anchors[0] $lines[0] 'next block from command input reaches the first result'
    Check 'shell block navigation preserves the native draft and sends no PTY bytes' ((Own-Text $InputBox) -ceq $draft -and (Raw-Input) -ceq $before)
    $observations.Add(@{Case='three shell results and native caret';Commands=$commands;Anchors=$anchors;FirstOutputLines=$lines;Draft=$draft;RawInputBefore=$beforeAll;RawInputAfter=(Raw-Input);Navigation='private F7/F8 aliases dispatch the same PreviousBlock/NextBlock actions as Ctrl+Up/Down; no global keys or keyboard-state changes';NvdaSpeech='requires human NVDA test'})
}
function Test-ShellOutput([IntPtr]$InputBox,[IntPtr]$Journal,[IntPtr]$Direct,[IntPtr]$Reader) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main
    $shellInputStart=$script:ExpectedInput
    Assert-ExpectedInput $shellInputStart 'before shell output and clear regression'
    $text=Own-Text $Journal;$history='BKS_HANDOFF_HISTORY_004';$anchor=$text.IndexOf($history,[StringComparison]::Ordinal)
    Check 'managed shell retains enough history for a real native scroll regression' ($anchor -ge 0 -and [regex]::Matches($text,'BKS_HANDOFF_HISTORY_[0-9]{3}').Count -eq 128)
    [BksStage7]::Select($own,$Journal,$anchor,$anchor+$history.Length)
    [BksStage7]::ScrollTo($own,$Journal,$anchor)
    $historySelection=[BksStage7]::Selection($own,$Journal);$historyViewport=[BksUnified]::Viewport($own,$Journal)
    Check 'pre-submit journal selection and viewport are in old history away from the tail' ($historySelection[0] -eq $anchor -and $historySelection[1] -eq $anchor+$history.Length -and -not $historyViewport.AtBottom)
    $observations.Add(@{Case='shell output before explicit submit';Selection=$historySelection;Viewport=$historyViewport;RawInput=(Raw-Input)})

    $command='BKS_OUTPUT_PROBE';$before=Raw-Input
    $bytes=([BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes($command+"`r`n"))).Replace('-','')
    [BksStage7]::SetText($own,$InputBox,$command)
    [BksDirect]::Post($own,$main,0x111,1003)
    Wait-For { (Raw-Input).Length -ge $before.Length+$bytes.Length } 'explicit native output command reaches only the managed shell'
    Confirm-Input $before $bytes 'output probe is sent once as an explicit literal plus CRLF'
    Wait-For { (Own-Text $Journal).Contains('BKS_SHELL_OUTPUT_TAIL_UNIQUE') } 'explicit native shell submit receives the complete long result'
    Wait-For { $text=Own-Text $Journal;$selection=[BksStage7]::Selection($own,$Journal);$selection[0] -eq $text.Length -and $selection[1] -eq $text.Length -and ([BksUnified]::Viewport($own,$Journal)).AtBottom } 'explicit shell command follows native journal EOF and viewport after redraw'
    $text=Own-Text $Journal;$selection=[BksStage7]::Selection($own,$Journal);$viewport=[BksUnified]::Viewport($own,$Journal)
    $unicode=[string][char]0x0416+[char]0x754c+'e'+[char]0x301+[char]::ConvertFromUtf32(0x1f600)
    Check 'explicit shell result is visible with command field focus and native viewport at EOF' ([BksStage7]::IsWindowVisible($Journal) -and [BksStage7]::IsWindowVisible($InputBox) -and -not [BksStage7]::IsWindowVisible($Direct) -and -not [BksStage7]::IsWindowVisible($Reader) -and [BksDirect]::Focus($own,$main) -eq $InputBox -and (Own-Text $InputBox).Length -eq 0 -and $selection[0] -eq $text.Length -and $selection[1] -eq $text.Length -and $viewport.AtBottom -and $viewport.FirstLine -gt $historyViewport.FirstLine)
    Check 'long native result retains all 128 Unicode rows once and exposes text without ANSI escapes' ([regex]::Matches($text,'BKS_SHELL_OUTPUT_ROW_[0-9]{3}').Count -eq 128 -and [regex]::Matches($text,'BKS_SHELL_OUTPUT_TAIL_UNIQUE').Count -eq 1 -and $text.Contains($unicode) -and -not $text.Contains([string][char]27) -and (Same-UiaText (Uia-Text $Journal) $text))
    $observations.Add(@{Case='explicit shell output followed after redraw';Selection=$selection;Viewport=$viewport;NativeUtf16=$text.Length;NativeJournal=$text;RawInput=(Raw-Input);NvdaSpeech='not tested'})
    Save-OwnScreenshot $Journal 'shell-output-follow.png'

    $expected=Raw-Input
    Request-Scene 'shell-follow'
    Wait-For { $text=Own-Text $Journal;$selection=[BksStage7]::Selection($own,$Journal);$text.Contains('BKS_SHELL_FOLLOW_TAIL') -and $selection[0] -eq $text.Length -and $selection[1] -eq $text.Length -and ([BksUnified]::Viewport($own,$Journal)).AtBottom } 'new passive output continues following the explicit command result'
    Check 'native follow continues without extra input or moving command field focus' ((Raw-Input) -ceq $expected -and [BksDirect]::Focus($own,$main) -eq $InputBox)

    $text=Own-Text $Journal;$history='BKS_SHELL_OUTPUT_ROW_004';$anchor=$text.IndexOf($history,[StringComparison]::Ordinal)
    Check 'passive-output bookmark exists in the retained command result' ($anchor -ge 0)
    [BksStage7]::Select($own,$Journal,$anchor,$anchor+$history.Length)
    [BksStage7]::ScrollTo($own,$Journal,$anchor)
    $historySelection=[BksStage7]::Selection($own,$Journal);$historyViewport=[BksUnified]::Viewport($own,$Journal)
    Check 'passive-output bookmark begins away from the native tail' (-not $historyViewport.AtBottom)
    Request-Scene 'shell-passive-history'
    Wait-For { (Own-Text $Journal).Contains('BKS_SHELL_PASSIVE_TAIL') } 'passive shell output reaches native journal while reading history'
    Start-Sleep -Milliseconds 150
    $text=Own-Text $Journal;$selection=[BksStage7]::Selection($own,$Journal);$viewport=[BksUnified]::Viewport($own,$Journal)
    Check 'passive shell output preserves historical selection and viewport without stealing focus' ($selection[0] -eq $historySelection[0] -and $selection[1] -eq $historySelection[1] -and $text.Substring($selection[0],$selection[1]-$selection[0]) -ceq $history -and $viewport.FirstLine -eq $historyViewport.FirstLine -and -not $viewport.AtBottom -and [BksDirect]::Focus($own,$main) -eq $InputBox -and (Raw-Input) -ceq $expected)
    $observations.Add(@{Case='passive shell output preserves history';BeforeSelection=$historySelection;AfterSelection=$selection;BeforeViewport=$historyViewport;AfterViewport=$viewport;RawInput=(Raw-Input)})
    Save-OwnScreenshot $Journal 'shell-passive-history.png'

    $command='BKS_CLEAR_PROBE';$before=Raw-Input
    $bytes=([BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes($command+"`r`n"))).Replace('-','')
    [BksStage7]::SetText($own,$InputBox,$command)
    [BksDirect]::Post($own,$main,0x111,1003)
    Wait-For { (Raw-Input).Length -ge $before.Length+$bytes.Length } 'explicit native clear command reaches only the managed shell'
    Confirm-Input $before $bytes 'clear probe is sent once as an explicit literal plus CRLF'
    Wait-For { (Own-Text $Journal).Length -eq 0 } 'split clear-only VT sequence empties native journal before any new prompt'
    Check 'clear-only output resets native journal and TextPattern while keeping the shell field focused' ((Own-Text $Journal).Length -eq 0 -and (Normalize-Text (Uia-Text $Journal)).TrimEnd("`n").Length -eq 0 -and [BksStage7]::IsWindowVisible($Journal) -and [BksStage7]::IsWindowVisible($InputBox) -and [BksDirect]::Focus($own,$main) -eq $InputBox -and (Own-Text $InputBox).Length -eq 0)
    Save-OwnScreenshot $Journal 'shell-clear-only.png'
    Request-Scene 'shell-clear-prompt'
    Wait-For { (Own-Text $Journal).Contains('BKS_SHELL_CLEARED_READY') -and (Own-Text $Journal).Contains('fake-cleared> ') } 'post-clear parent prompt appears in native journal'
    $text=Own-Text $Journal
    Check 'post-clear journal contains only the new output and one parent prompt without old history or duplicate appends' ((Normalize-Text $text).TrimEnd() -ceq "BKS_SHELL_CLEARED_READY`nfake-cleared>" -and (Same-UiaText (Uia-Text $Journal) $text))
    Check 'explicit clear returns to the visible shell field without stale TUI or reader focus' ([BksStage7]::IsWindowVisible($Journal) -and [BksStage7]::IsWindowVisible($InputBox) -and -not [BksStage7]::IsWindowVisible($Direct) -and -not [BksStage7]::IsWindowVisible($Reader) -and [BksDirect]::Focus($own,$main) -eq $InputBox)
    $allBytes=$shellInputStart+([BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes("BKS_OUTPUT_PROBE`r`nBKS_CLEAR_PROBE`r`n"))).Replace('-','')
    Check 'shell output and clear append only their two explicit literal commands to the exact prior input' ((Raw-Input) -ceq $allBytes -and $script:ExpectedInput -ceq $allBytes -and [BksStage7]::Responsive($own,$main,1000,2))
    $observations.Add(@{Case='keyless shell output and clear final';NativeJournal=$text;Selection=([BksStage7]::Selection($own,$Journal));RawInput=(Raw-Input);NvdaSpeech='not tested';InputMechanism='own WM_SETTEXT/WM_COMMAND plus fixture IPC only'})
    Save-OwnScreenshot $Journal 'shell-clear-prompt.png'
}

function Test-PaintIsolation([IntPtr]$Direct,[IntPtr]$Canvas) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main
    # Keyless inspection of private HWNDs only. The native provider is changed
    # programmatically to provoke native text painting/scrolling, never PTY input.
    if(-not [BksStage7]::IsWindowVisible($Direct)) { [BksDirect]::Post($own,$main,0x111,1009) }
    Wait-For { [BksStage7]::IsWindowVisible($Direct) -and [BksStage7]::IsWindowVisible($Canvas) } 'own provider and GDI overlay are visible'
    Check-RawCanvasFocus $Direct $Canvas 'after own startup/WM_COMMAND TUI entry'
    Check-RawTextPatterns $Direct $Canvas 'after own startup/WM_COMMAND TUI entry'
    Request-Scene 'paint'
    Wait-For { (Own-Text $Direct).Contains('BKS_PAINT_READY') -and (Own-Text $Direct).Contains('Tick: 000000') } 'own stable paint fixture is current'
    $element=Element $Direct;$provider=Own-Text $Direct
    Check 'overlay keeps visible same-area native provider and GDI canvas' ([BksStage7]::IsTextControl($own,$Direct) -and -not [BksStage7]::IsTextControl($own,$Canvas) -and [BksStage7]::IsWindowVisible($Direct) -and [BksStage7]::IsWindowVisible($Canvas) -and (Rect-Key ([BksDirect]::Bounds($own,$Direct))) -ceq (Rect-Key ([BksDirect]::Bounds($own,$Canvas))))
    Check 'canvas is above provider with both WS_CLIPSIBLINGS, canvas WS_TABSTOP and no provider WS_TABSTOP' ([BksUnified]::OverlayProtected($own,$Direct,$Canvas))
    Check 'occluded provider stays on-screen read-only with a complete named TextPattern' (-not $element.Current.IsOffscreen -and [BksStage7]::ReadOnly($own,$Direct) -and $element.Current.Name -ceq [BksDirect]::DirectName -and (Same-UiaText (Uia-Text $Direct) $provider) -and $provider.Contains('VT canvas only') -and $provider.Contains('Stable source; native provider is underneath'))
    Request-Scene 'hold'
    $original=Own-Text $Direct;$selection=[BksStage7]::Selection($own,$Direct)
    $vtBefore=Shared-Text (Join-Path $script:Case.Task 'fixture-output.vt');$before=Raw-Input
    $baselinePath=Join-Path $runDirectory 'canvas-before-native-patch.png'
    $patchedPath=Join-Path $runDirectory 'canvas-after-native-patch.png'
    $spinnerPath=Join-Path $runDirectory 'canvas-after-vt-spinner.png'
    Save-OwnScreenshot $Canvas 'canvas-before-native-patch.png'
    $patched=$false
    try {
        $patched=[BksUnified]::PatchProvider($own,$Direct)
        $observations.Add(@{Case='native-only provider replacement and caret scroll';PatchObserved=$patched;NativeText=(Own-Text $Direct);FirstVisibleLine=([BksStage7]::FirstLine($own,$Direct));RawInput=(Raw-Input)})
        Save-OwnScreenshot $Canvas 'canvas-after-native-patch.png'
        Check 'native provider replacement and scrolling leave the VT source unchanged and emit zero PTY bytes' ((Shared-Text (Join-Path $script:Case.Task 'fixture-output.vt')) -ceq $vtBefore -and (Raw-Input) -ceq $before)
        Check 'native provider patch cannot change canvas visibility or sibling clipping' ([BksStage7]::IsWindowVisible($Direct) -and [BksStage7]::IsWindowVisible($Canvas) -and [BksUnified]::OverlayProtected($own,$Direct,$Canvas))
    } finally {
        [BksStage7]::SetText($own,$Direct,$original)
        [BksStage7]::Select($own,$Direct,$selection[0],$selection[1])
    }
    Check-RawCanvasFocus $Direct $Canvas 'after native provider replacement/selection/scroll'
    Request-Scene 'release'
    Request-Scene 'passive'
    Wait-For { (Own-Text $Direct).Contains('Spinner: / Tick: 000001') } 'positive control receives a real VT spinner update'
    Save-OwnScreenshot $Canvas 'canvas-after-vt-spinner.png'
    if($patched -and [IO.File]::Exists($baselinePath) -and [IO.File]::Exists($patchedPath) -and [IO.File]::Exists($spinnerPath)) {
        $stable=[BksUnified]::CompareCanvas($own,$Canvas,$baselinePath,$patchedPath)
        $changed=[BksUnified]::CompareCanvas($own,$Canvas,$baselinePath,$spinnerPath)
        $observations.Add(@{Case='canvas pixels under stable VT and real VT positive control';StableSource=$stable;ChangedVtSource=$changed;CanvasBounds=([BksDirect]::Bounds($own,$Canvas));ProviderRestorationMayOccurOnTimer=$true})
        if($stable.DistinctBefore -gt 1 -and $changed.Changed -gt 0) {
            Check 'stable VT canvas pixels are unchanged by EM_SETSEL EM_REPLACESEL and EM_SCROLLCARET' ($stable.Changed -eq 0)
            Check 'pixel comparison positively observes changed VT content' ($changed.Changed -gt 0)
        } else { $checks.Add(@{Status='skipped';Name='PrintWindow did not establish rendered VT with a changing positive control; no pixel isolation verdict'}) }
    } else { $checks.Add(@{Status='skipped';Name='Provider patch was restored before observation or PrintWindow unavailable; no pixel isolation verdict'}) }
    $frames=New-Object 'System.Collections.Generic.List[object]'
    foreach($tick in 2..8) {
        Request-Scene 'passive'
        $marker='Tick: '+$tick.ToString('D6')
        Wait-For { (Own-Text $Direct).Contains($marker) } ('own streamed VT update '+$tick)
        $frames.Add(@{Tick=$tick;CanvasVisible=[BksStage7]::IsWindowVisible($Canvas);ProviderVisible=[BksStage7]::IsWindowVisible($Direct);Protected=[BksUnified]::OverlayProtected($own,$Direct,$Canvas);ProviderOffscreen=(Element $Direct).Current.IsOffscreen;TextAgrees=(Same-UiaText (Uia-Text $Direct) (Own-Text $Direct))})
    }
    Check 'bounded spinner updates preserve visible clipped overlay and readable on-screen provider' (@($frames | Where-Object {-not $_.CanvasVisible -or -not $_.ProviderVisible -or -not $_.Protected -or $_.ProviderOffscreen -or -not $_.TextAgrees}).Count -eq 0)
    Check-RawCanvasFocus $Direct $Canvas 'after bounded VT updates'
    Check-RawTextPatterns $Direct $Canvas 'after bounded VT updates'
    Assert-ExpectedInput $before 'complete keyless paint isolation scenario'
    Check 'keyless native painting and VT updates emit zero PTY bytes' ($before.Length -eq 0 -and (Raw-Input).Length -eq 0)
    $observations.Add(@{Case='keyless paint isolation final';Frames=$frames.ToArray();NativeText=(Own-Text $Direct);UiaText=(Uia-Text $Direct);RawInput=(Raw-Input);NvdaSpeech='not tested';PerceivedFlicker='not tested'})
    $checks.Add(@{Status='skipped';Name='Architectural clipping and captured pixels are checked; perceived flicker and human NVDA require manual verification'})
}

try {
    Add-Type -ReferencedAssemblies $references -TypeDefinition ($nativeSource+"`r`n"+$directSource+"`r`n"+$extraSource)
    if($privateScenario) {
        [BksQaDesktop]::InitializeUiaOnWorker([BksStage7])
        Check 'private desktop PowerShell caller and UIA MTA worker attach before provider initialization' ([BksQaDesktop]::CurrentDesktop -ceq $PrivateDesktopName -and [BksQaDesktop]::WorkerDesktop -ceq $PrivateDesktopName)
    } else { [BksStage7]::InitializeUia() }
    if(-not $CliHandoff -and -not $PaintIsolation -and -not $privateScenario) {
        $probe=[BksDirect]::ProbeKeyUp()
        Check 'own queue key-up contract avoids duplicate Return' (-not $probe.NativeKeyUpGeneratedReturn)
    }
    $templateExe=Join-Path $runDirectory 'unified-fixture.exe'
    $fixtureFile=Join-Path $runDirectory 'unified-fixture.cs'
    [IO.File]::WriteAllText($fixtureFile,$fixtureSource,(New-Object Text.UTF8Encoding $false))
    & (Join-Path $env:SystemRoot 'Microsoft.NET/Framework64/v4.0.30319/csc.exe') /nologo /target:exe /platform:x64 /warnaserror+ /reference:System.Web.Extensions.dll ('/out:'+$templateExe) $fixtureFile
    if($LASTEXITCODE -ne 0) { throw 'Own unified fixture compilation failed' }
    Check 'native/UIA helpers and own unified fixture compile' (Test-Path -LiteralPath $templateExe)
    $selfArgument=if($TuiTurns){'--tui-turns-self-test'}else{'--self-test'}
    $selfStart=New-Object Diagnostics.ProcessStartInfo($templateExe,$selfArgument)
    $selfStart.UseShellExecute=$false;$selfStart.CreateNoWindow=$true;$selfStart.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $selfStart.RedirectStandardOutput=$true;$selfStart.RedirectStandardError=$true
    $self=[Diagnostics.Process]::Start($selfStart)
    try {
        if(-not $self.WaitForExit(5000)) { $self.Kill();throw 'Own fixture self-test timeout' }
        [IO.File]::WriteAllText((Join-Path $runDirectory 'fixture-self-test.txt'),$self.StandardOutput.ReadToEnd()+$self.StandardError.ReadToEnd())
        $selfCheckName=if($TuiTurns){'fixture controlled generic phases, styled answer stream, source completion and two-request reset self-test'}else{'fixture split keys, Unicode echo, shell output/follow, split clear and single-field source/menu/masked self-test'}
        Check $selfCheckName ($self.ExitCode -eq 0)
    } finally { $self.Dispose() }
    if($privateScenario) {
        $probeDirectory=Join-Path $runDirectory 'desktop-probe'
        $probeAppData=Join-Path $probeDirectory 'appdata'
        $null=[IO.Directory]::CreateDirectory($probeAppData)
        $unicode=[string][char]0x0416+[char]0x754c+'e'+[char]0x301+[char]::ConvertFromUtf32(0x1f600)
        $probeStart=New-Object Diagnostics.ProcessStartInfo($templateExe,'--desktop-probe')
        $probeStart.UseShellExecute=$false;$probeStart.CreateNoWindow=$true;$probeStart.WorkingDirectory=$probeDirectory
        $probeStart.EnvironmentVariables.Clear()
        foreach($entry in @{'SystemRoot'=$env:SystemRoot;'WINDIR'=$env:SystemRoot;'APPDATA'=$probeAppData;'BKS_QA_DESKTOP'=$PrivateDesktopName;'BKS_QA_PROBE'=$probeDirectory;'BKS_QA_UNICODE'=$unicode}.GetEnumerator()) {$probeStart.EnvironmentVariables[$entry.Key]=$entry.Value}
        $probeProcess=[BksQaDesktop]::StartApplication($probeStart)
        try {
            if(-not $probeProcess.WaitForExit(5000)) {$probeProcess.Kill();throw 'Own private desktop launch probe timed out'}
            $probeIdentity=Read-BksJson (Join-Path $probeDirectory 'desktop.json')
            Check 'native CreateProcessW fixture launch preserves private desktop, Unicode environment, APPDATA and cwd' ($probeProcess.ExitCode -eq 0 -and $probeIdentity.Pid -eq $probeProcess.Id -and $probeIdentity.Executable -ieq $templateExe -and $probeIdentity.Desktop -ceq $PrivateDesktopName -and $probeIdentity.Cwd -ieq $probeDirectory -and $probeIdentity.AppData -ieq $probeAppData -and $probeIdentity.Unicode -ceq $unicode)
        } finally {$probeProcess.Dispose()}
    }
    if($PrepareOnly) {
        $checks.Add(@{Status='skipped';Name='PrepareOnly: no BKS window, ConPTY session, input-queue attachment or UI interaction launched'})
    } else {
        $Exe=Get-BksFullPath $Exe;Assert-BksNoReparse $Exe
        Start-Case
        $main=$script:Case.Main;$pidOwn=$script:Case.Process.Id
        $direct=Field $main 1011;$reader=Field $main 1018;$command=Field $main 1002;$canvas=Field $main 1017;$journal=Field $main 1001
        if($TuiTurns) {
            Test-TuiTurns $command $reader $journal $direct $canvas (Field $main 1030) (Field $main 1031)
        } elseif($SingleInput) {
            Test-SingleInput $command $reader $journal $direct $canvas (Field $main 1030)
        } elseif($CliHandoff) {
            Test-CliHandoff $command $journal $direct $reader
        } elseif($PaintIsolation) {
            Test-PaintIsolation $direct $canvas
        } elseif($ShellRegression) {
            Wait-For { (Own-Text $journal).Contains('BKS_UNIFIED_READY') } 'ordinary journal receives own fixture startup output'
            Check 'ordinary managed profile starts in editable command field and native journal' ([BksStage7]::IsWindowVisible($command) -and [BksStage7]::IsWindowVisible($journal) -and -not [BksStage7]::IsWindowVisible($direct) -and -not [BksStage7]::ReadOnly($pidOwn,$command) -and [BksStage7]::ReadOnly($pidOwn,$journal))
            Activate-OwnOnce $command
            [BksDirect]::Watch($pidOwn,(Field $main 1026))
            Test-ShellRegression $command $journal $direct
        } else {
        # The managed fixture is deliberately an ordinary profile. Generic TUI
        # mode is selected explicitly here; shell defaults are checked separately.
        if(-not [BksStage7]::IsWindowVisible($direct)) {
            [BksDirect]::Post($pidOwn,$main,0x111,1009)
        }
        Wait-For { [BksStage7]::IsWindowVisible($direct) -and (Own-Text $direct).Contains('BKS_UNIFIED_READY') } 'explicit TUI direct surface with fixture VT'
        Check 'explicit TUI mode overlays GDI canvas over a full-area read-only native provider without a lower command field' ([BksStage7]::IsTextControl($pidOwn,$direct) -and [BksStage7]::ReadOnly($pidOwn,$direct) -and -not [BksStage7]::IsWindowVisible($command) -and [BksStage7]::IsWindowVisible($canvas) -and [BksUnified]::OverlayProtected($pidOwn,$direct,$canvas) -and (Rect-Key ([BksDirect]::Bounds($pidOwn,$direct))) -ceq (Rect-Key ([BksDirect]::Bounds($pidOwn,$canvas))))
        Check 'native TextPattern and Unicode document agree' ((Element $direct).Current.Name -ceq [BksDirect]::DirectName -and (Same-UiaText (Uia-Text $direct) (Own-Text $direct)))
        Check-RawCanvasFocus $direct $canvas 'before any explicit harness activation or keys'
        Check-RawTextPatterns $direct $canvas 'before any explicit harness activation or keys'
        Check 'native direct input HWND is Unicode before numeric surrogate-pair delivery' ([BksUnified]::UnicodeWindow($pidOwn,$direct))
        Save-OwnScreenshot $direct 'direct-surface.png'
        # Exactly one initial activation, after the private process/executable
        # handshake. Every later focus/key operation requires own foreground.
        if(-not $TuiSelection) { Activate-OwnOnce $direct }
        [BksDirect]::Watch($pidOwn,(Field $main 1026))
        if($TuiSelection) {
            Test-TuiSelection $direct $reader
        } elseif($ReadNavigation) {
            Test-ReadNavigation $direct $reader
        } else {
        Check-Key $direct 9 '09' 'Tab immediately after explicit TUI selection'
        if(-not $Minimal) {
        $noticeStart=[BksDirect]::Notices().Length;$before=Raw-Input
        Chord $direct 0x49 { @([BksDirect]::Notices() | Select-Object -Skip $noticeStart | Where-Object {$_.Text -ceq [BksDirect]::CaptureOffNotice}).Count -gt 0 } 'CtrlShiftI disables capture'
        [BksDirect]::Key($pidOwn,$direct,9)
        Wait-For { [BksDirect]::Focus($pidOwn,$main) -ne $direct } 'capture-off Tab stays GUI navigation'
        Assert-ExpectedInput $before 'capture-off GUI Tab'
        Focus-Own $direct -ForceUia
        Check-Key $direct 9 '09' 'Tab after direct focus autocaptures'
        foreach($arrow in @(0x26,0x28)) {
            $before=Raw-Input
            Chord $direct $arrow { [BksDirect]::Focus($pidOwn,$main) -eq $reader } 'TUI CtrlArrow enters the native reader' $true $false
            Assert-OwnReaderKey $reader 27
            Wait-For { [BksDirect]::Focus($pidOwn,$main) -eq $direct } 'Escape returns from TUI CtrlArrow reading'
            Assert-ExpectedInput $before 'TUI CtrlArrow reading sends no PTY bytes'
        }
        $area=Rect-Key ([BksDirect]::Bounds($pidOwn,$direct));$size=Own-Text (Field $main 1015);$before=Raw-Input
        Chord $direct 0x20 { [BksDirect]::Focus($pidOwn,$main) -eq $reader } 'CtrlShiftSpace enters independent reader'
        Check 'reader retains full terminal dimensions' ((Rect-Key ([BksDirect]::Bounds($pidOwn,$reader))) -ceq $area -and (Own-Text (Field $main 1015)) -ceq $size)
        [BksDirect]::Key($pidOwn,$reader,27)
        Wait-For { [BksDirect]::Focus($pidOwn,$main) -eq $direct } 'Esc returns from reader'
        Assert-ExpectedInput $before 'reader Esc return'
        Check-Key $direct 9 '09' 'Tab after reader Esc autocaptures'
        Chord $direct 0x20 { [BksDirect]::Focus($pidOwn,$main) -eq $reader } 'CtrlShiftSpace opens reader again'
        $before=Raw-Input
        Chord $reader 0x20 { [BksDirect]::Focus($pidOwn,$main) -eq $direct } 'CtrlShiftSpace returns to captured direct input'
        Assert-ExpectedInput $before 'reader toggle return'
        } else {
            $checks.Add(@{Status='skipped';Name='Minimal: extra capture toggles, CtrlArrow and duplicate reader-toggle paths remain in the full scenario'})
        }

        # Unknown CLI: snapshot reading stays independent until explicit opt-in.
        Chord $direct 0x44 { [BksStage7]::VisibleClass($pidOwn,'BksTermTuiDialog') -ne [IntPtr]::Zero } 'CtrlShiftD opens native generic overview'
        $dialog=[BksStage7]::VisibleClass($pidOwn,'BksTermTuiDialog')
        Wait-For { [BksStage7]::GetDlgItem($dialog,4400) -ne [IntPtr]::Zero -and [BksStage7]::GetDlgItem($dialog,4401) -ne [IntPtr]::Zero -and [BksStage7]::GetDlgItem($dialog,4405) -ne [IntPtr]::Zero -and [BksStage7]::GetDlgItem($dialog,4404) -ne [IntPtr]::Zero } 'native overview controls initialized'
        $context=Field $dialog 4400;$list=Field $dialog 4401;$control=Field $dialog 4405;$status=Field $dialog 4404
        Focus-Own $list -Root $dialog -ForceUia
        $before=Raw-Input;$snapshot=Own-Text $context;$selection=Uia-Selected $list
        [BksDirect]::Key($pidOwn,$list,0x28);[BksDirect]::Key($pidOwn,$list,13)
        Start-Sleep -Milliseconds 150
        Assert-ExpectedInput $before 'snapshot arrows and Enter'
        $item=(Element $list).FindAll([Windows.Automation.TreeScope]::Children,[Windows.Automation.Condition]::TrueCondition) | Where-Object {$_.Current.ControlType -eq [Windows.Automation.ControlType]::ListItem} | Select-Object -First 1
        $selectionPattern=$null
        if($item.TryGetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern,[ref]$selectionPattern)) { $selectionPattern.Select() }
        Request-Scene 'passive'
        Wait-For { (Own-Text $status).Contains([BksDirect]::StaleOverview) } 'snapshot reports passive update'
        Check 'snapshot context and UIA list selection never generate PTY input' ((Own-Text $context) -ceq $snapshot -and (Raw-Input) -ceq $before)
        [BksDirect]::Unwatch();[BksDirect]::Watch($pidOwn,$status)
        Invoke-Own $control;Wait-Live $status
        Focus-Own $list -Root $dialog -ForceUia
        Live-Key $list $status 0x28 '1B5B42' 'live Down'
        Check 'live Down projects actual visible Bravo marker' ((Uia-Selected $list).Contains('Bravo'))
        Save-OwnScreenshot $dialog 'live-menu.png'
        if(-not $Minimal) {
        Live-Key $list $status 0x26 '1B5B41' 'live Up'
        Live-Key $list $status 13 '0D' 'live Enter'
        $before=Raw-Input
        [BksUnified]::Repeat($pidOwn,$list,13);Start-Sleep -Milliseconds 150
        Assert-ExpectedInput $before 'held Enter after new frame'
        Live-Key $list $status 9 '09' 'live Tab'
        Live-Key $list $status 27 '1B' 'live Escape'
        Check 'live controls preserve dialog and use its own notification provider' ([BksStage7]::VisibleClass($pidOwn,'BksTermTuiDialog') -eq $dialog -and [BksDirect]::Notices().Length -gt 0)
        Request-Scene 'hold'
        $clock=[Diagnostics.Stopwatch]::StartNew();$before=Raw-Input
        [BksDirect]::Key($pidOwn,$list,0x26)
        Wait-For { (Raw-Input).Length -gt $before.Length } 'held-frame key reaches fixture'
        Wait-For { (Own-Text $status).Contains([BksUnified]::Pending) } 'live pending feedback is visible'
        Wait-Live $status;$clock.Stop()
        Confirm-Input $before '1B5B41' 'unchanged-frame deadline never repeats a key'
        Check 'unchanged frame releases pending around the 700ms deadline' ($clock.ElapsedMilliseconds -ge 500 -and $clock.ElapsedMilliseconds -lt 1700)
        Request-Scene 'release';Start-Sleep -Milliseconds 200;Wait-Live $status
        } else {
            $checks.Add(@{Status='skipped';Name='Minimal: remaining literal menu keys, held Enter and pending deadline remain in the full scenario'})
        }
        $before=Raw-Input
        Invoke-Own $control
        Wait-For { (Element $control).Current.Name -ceq [BksUnified]::ControlButton } 'explicit control button freezes snapshot'
        $snapshot=Own-Text $context
        Focus-Own $list -Root $dialog -ForceUia
        [BksDirect]::Key($pidOwn,$list,0x28);[BksDirect]::Key($pidOwn,$list,13)
        Request-Scene 'passive';Start-Sleep -Milliseconds 200
        Check 'stopped live mode keeps snapshot stable and sends no input' ((Own-Text $context) -ceq $snapshot -and (Raw-Input) -ceq $before)
        if(-not $Minimal) {
        Invoke-Own $control;Wait-Live $status;Focus-Own $list -Root $dialog -ForceUia
        Request-Scene 'storm';$before=Raw-Input
        [BksDirect]::Key($pidOwn,$list,13)
        Wait-For { (Own-Text $status).Contains([BksUnified]::KeyRejected) } 'stale Enter freezes without sending'
        Assert-ExpectedInput $before 'stale Enter'
        Check 'stale Enter sends no byte and disables live forwarding' ((Element $control).Current.Name -ceq [BksUnified]::ControlButton)
        Wait-For { [IO.File]::Exists((Join-Path $script:Case.Task 'storm.done')) } 'bounded private repaint ends'
        } else {
            $checks.Add(@{Status='skipped';Name='Minimal: stale Enter under bounded repaint remains in the full scenario'})
        }
        [BksDirect]::Post($pidOwn,$dialog,0x10,0)
        Wait-For { [BksStage7]::VisibleClass($pidOwn,'BksTermTuiDialog') -eq [IntPtr]::Zero } 'own overview closes'
        [BksDirect]::Unwatch();[BksDirect]::Watch($pidOwn,(Field $main 1026))
        Focus-Own $direct -ForceUia

        # Confirmed Unicode echo: no speculative notification before fixture VT.
        Request-Scene 'prompt'
        Wait-For { (Own-Text $direct).Contains('BKS_UNIFIED_PROMPT_READY') } 'controlled prompt scene'
        Wait-For { $position=[BksDirect]::UiaCaret($pidOwn,$direct);$position.Collapsed -and $position.Prefix.TrimEnd().EndsWith('Input>',[StringComparison]::Ordinal) } 'controlled prompt cursor is stable before typing'
        Request-Scene 'hold';$noticeStart=[BksDirect]::Notices().Length;$before=Raw-Input
        [BksDirect]::Character($pidOwn,$direct,[char]0x0416)
        Wait-For { (Raw-Input).Length -gt $before.Length } 'typed Unicode byte reaches held fixture'
        Start-Sleep -Milliseconds 160
        Check 'typing has no speculative Unicode notification before VT confirmation' (@([BksDirect]::Notices() | Select-Object -Skip $noticeStart | Where-Object {$_.Text.Contains([string][char]0x0416)}).Count -eq 0)
        Confirm-Input $before 'D096' 'held Unicode is forwarded once'
        Request-Scene 'release'
        Wait-For { @([BksDirect]::Notices() | Select-Object -Skip $noticeStart | Where-Object {$_.Text -ceq [string][char]0x0416}).Count -eq 1 } 'VT confirms exact Unicode echo'
        $emoji=[char]::ConvertFromUtf32(0x1f600);$before=Raw-Input
        [BksUnified]::EmojiPair($pidOwn,$direct)
        Wait-For { (Raw-Input).Length -ge $before.Length+8 } 'surrogate pair reaches UTF8 fixture'
        Confirm-Input $before 'F09F9880' 'emoji forwarded once'
        $typed=[string][char]0x0416+$emoji
        Wait-For { (Own-Text $direct).Contains('Input> '+$typed) } 'VT confirms complete input text'
        $text=Own-Text $direct;$expected=$text.LastIndexOf('Input> '+$typed,[StringComparison]::Ordinal)+('Input> '+$typed).Length
        Wait-For { ([BksStage7]::Selection($pidOwn,$direct))[0] -eq $expected } 'native caret follows confirmed Unicode'
        $caret=[BksDirect]::UiaCaret($pidOwn,$direct)
        Check 'UIA collapsed caret agrees with full-area confirmed Unicode position' ($caret.Collapsed -and (Normalize-Text $caret.Prefix) -ceq (Normalize-Text $text.Substring(0,$expected)))
        $noticeStart=[BksDirect]::Notices().Length;$before=Raw-Input
        Chord $direct 0x4c { @([BksDirect]::Notices() | Select-Object -Skip $noticeStart | Where-Object {$_.Text.Contains($typed)}).Count -gt 0 } 'CtrlShiftL reads current confirmed input'
        Assert-ExpectedInput $before 'read-current-input shortcut'
        Check-Key $direct 13 '0D' 'explicit Enter before new output'
        Wait-For { (Own-Text $direct).Contains('BKS_UNIFIED_NEW_OUTPUT_001') } 'new output appears only after explicit Enter'
        $before=Raw-Input
        Chord $direct 0x52 { [BksDirect]::Focus($pidOwn,$main) -eq $reader } 'CtrlShiftR opens new output reader'
        $readerText=Own-Text $reader;$expected=$readerText.LastIndexOf('BKS_UNIFIED_NEW_OUTPUT_001',[StringComparison]::Ordinal)
        Check 'new-output reader starts at the first new logical line' ($expected -ge 0 -and ([BksStage7]::Selection($pidOwn,$reader))[0] -eq $expected)
        [BksDirect]::Key($pidOwn,$reader,27)
        Wait-For { [BksDirect]::Focus($pidOwn,$main) -eq $direct } 'new-output Esc returns to captured input'
        Assert-ExpectedInput $before 'read-new-output and Esc return'
        Check-Key $direct 9 '09' 'Tab after new-output Esc autocaptures'
        Check 'unknown CLI remains alive and responsive after all reader/menu actions' (-not $script:Case.Fixture.HasExited -and [BksStage7]::Responsive($pidOwn,$main,1000,2))
        if(-not $Minimal) {
        Menu 1009
        Wait-For { [BksStage7]::IsWindowVisible($command) } 'optional command mode is still explicit'
        Menu 1009
        Wait-For { [BksStage7]::IsWindowVisible($direct) -and [BksDirect]::Focus($pidOwn,$main) -eq $direct } 'optional command mode returns to direct-ready input'
        }
        Assert-ExpectedInput $script:ExpectedInput 'complete unified UI scenario'
        $observations.Add(@{RawInput=Raw-Input;Notifications=[BksDirect]::Notices();OwnedTree=[BksStage7]::DialogTree($pidOwn,$main)})
        $checks.Add(@{Status='skipped';Name='Adapter-failure process isolation remains covered by existing Rust transport regressions; no production fault flag injected here'})
        }
        }
    }
} catch {
    $failure=$_.Exception.Message
    if(-not $failure.StartsWith('FAILED:')) { $checks.Add(@{Status='error';Name=$failure}) }
    $observations.Add(@{Failure=$failure;Stack=$_.ScriptStackTrace})
    if($script:Case -and -not $script:Case.Process.HasExited) {
        try {
            $ownedDirect=Field $script:Case.Main 1011
            $diagnostic=@{RawInput=Raw-Input;Notifications=[BksDirect]::Notices();OwnedTree=[BksStage7]::DialogTree($script:Case.Process.Id,$script:Case.Main)}
            if(-not $CliHandoff -and -not $PaintIsolation -and -not $privateScenario) {$diagnostic.Focus=Focus-Diagnostics $ownedDirect}
            $observations.Add($diagnostic)
            $diagnosticIds=if($TuiTurns){@(1001,1002,1011,1018,1026,1030,1031)}elseif($SingleInput){@(1001,1002,1011,1018,1026,1030)}else{@(1001,1002,1011,1018,1026)}
            foreach($id in $diagnosticIds) {
                $window=Field $script:Case.Main $id
                [IO.File]::WriteAllText((Join-Path $runDirectory ('owned-control-'+$id+'.txt')),(Own-Text $window),(New-Object Text.UTF8Encoding $false))
            }
            $ownedDialog=[BksStage7]::VisibleClass($script:Case.Process.Id,'BksTermTuiDialog')
            if($ownedDialog -ne [IntPtr]::Zero) {
                $observations.Add(@{OwnedDialogTree=[BksStage7]::DialogTree($script:Case.Process.Id,$ownedDialog);Context=(Own-Text (Field $ownedDialog 4400));Status=(Own-Text (Field $ownedDialog 4404))})
            }
        } catch { $observations.Add(@{DiagnosticError=$_.Exception.Message}) }
    }
} finally {
    try { if('BksDirect' -as [type]) { [BksDirect]::Unwatch() } } catch { $checks.Add(@{Status='error';Name=('Notification cleanup: '+$_.Exception.Message)}) }
    try { Stop-Case } catch { $checks.Add(@{Status='error';Name=('Owned cleanup: '+$_.Exception.Message)}) }
    if($privateScenario) {try {[BksQaDesktop]::Disconnect();Check 'private UIA worker stops and the calling thread restores its startup desktop' ([BksQaDesktop]::CurrentDesktop -ceq $PrivateDesktopName)} catch {$checks.Add(@{Status='error';Name=('Private desktop cleanup: '+$_.Exception.Message)})}}
    $counts=@{passed=@($checks|Where-Object {$_.Status -eq 'passed'}).Count;failed=@($checks|Where-Object {$_.Status -eq 'failed'}).Count;skipped=@($checks|Where-Object {$_.Status -eq 'skipped'}).Count;error=@($checks|Where-Object {$_.Status -eq 'error'}).Count}
    $scope=if($privateScenario){'Only own PID/HWND, private WinSta0 desktop, APPDATA and managed fixture; posted own WM_KEYDOWN/WM_KEYUP/WM_CHAR, native provider messages/WM_COMMAND and fixture IPC; no SwitchDesktop, activation/focus helper, SendInput, physical/global keys, keyboard-state changes, clipboard, real CLI, model, network or approval; UIA is not human NVDA'}elseif($CliHandoff -or $PaintIsolation){'Keyless own PID/HWND/private APPDATA and managed fixture only; own native provider messages/WM_COMMAND and fixture IPC; no activation/focus/keyboard-state/keystroke helpers, real CLI, model, network or approval; UIA is not human NVDA'}else{'Only own PID/HWND/private APPDATA and fixture; no SendInput/global keyboard events/real CLI/network/approval; modified keyboard state is limited to attached own UI queue and restored in finally; UIA is not human NVDA'}
    Write-BksJson (Join-Path $runDirectory 'summary.json') @{Scope=$scope;PreparedOnly=[bool]$PrepareOnly;Minimal=[bool]$Minimal;ReadNavigation=[bool]$ReadNavigation;ShellRegression=[bool]$ShellRegression;CliHandoff=[bool]$CliHandoff;PaintIsolation=[bool]$PaintIsolation;TuiSelection=[bool]$TuiSelection;SingleInput=[bool]$SingleInput;TuiTurns=[bool]$TuiTurns;Executable=$Exe;Counts=$counts;Checks=$checks.ToArray();Observations=$observations.ToArray()}
    [IO.File]::WriteAllLines((Join-Path $runDirectory 'results.txt'),[string[]]@($checks|ForEach-Object {$_.Status+': '+$_.Name}),(New-Object Text.UTF8Encoding $false))
    if($privateScenario -and $PrivateDesktopResult) {Write-BksJson $PrivateDesktopResult @{RunDirectory=$runDirectory;PrivateDesktop=$PrivateDesktopName;Counts=$counts;Failure=$failure}}
}
Write-Output "Unified TUI: passed=$($counts.passed) failed=$($counts.failed) skipped=$($counts.skipped) error=$($counts.error)"
Write-Output "Artifacts: $runDirectory"
if($failure -or $counts.failed -or $counts.error) { throw $(if($failure){$failure}else{'Unified checks or cleanup failed'}) }
