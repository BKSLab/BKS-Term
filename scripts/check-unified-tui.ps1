[CmdletBinding()]
param([string]$Exe,[string]$Artifacts,[switch]$PrepareOnly,[switch]$Minimal,[switch]$ReadNavigation,[switch]$ShellRegression,[switch]$CliHandoff,[switch]$PaintIsolation,[switch]$TuiSelection)

$ErrorActionPreference='Stop'
# Resolve defaults in the body: Windows PowerShell may not populate PSScriptRoot
# while parameter defaults are being evaluated.
if(-not $Exe) { $Exe=Join-Path $PSScriptRoot '../target/release/bks-app.exe' }
if(-not $Artifacts) { $Artifacts=Join-Path $PSScriptRoot $(if($PaintIsolation){'../target/paint-isolation'}elseif($TuiSelection){'../target/tui-selection'}else{'../target/unified-tui'}) }
if($PSVersionTable.PSEdition -eq 'Core' -or [Threading.Thread]::CurrentThread.GetApartmentState() -ne 'MTA') {
    $arguments=@('-NoProfile','-Mta','-ExecutionPolicy','Bypass','-File',$PSCommandPath,'-Exe',$Exe,'-Artifacts',$Artifacts)
    if($PrepareOnly) { $arguments+='-PrepareOnly' }
    if($Minimal) { $arguments+='-Minimal' }
    if($ReadNavigation) { $arguments+='-ReadNavigation' }
    if($ShellRegression) { $arguments+='-ShellRegression' }
    if($CliHandoff) { $arguments+='-CliHandoff' }
    if($PaintIsolation) { $arguments+='-PaintIsolation' }
    if($TuiSelection) { $arguments+='-TuiSelection' }
    & "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe" @arguments
    exit $LASTEXITCODE
}
. (Join-Path $PSScriptRoot '../packaging/common.ps1')
$artifactRoot=Get-BksFullPath $Artifacts
Assert-BksNoReparse $artifactRoot
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
    [DllImport("user32.dll",SetLastError=true)] private static extern bool SetWindowPos(IntPtr window,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll",EntryPoint="GetWindowLongPtrW")] private static extern IntPtr WindowStyle(IntPtr window,int index);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern IntPtr SendMessageTimeoutW(IntPtr window,uint message,IntPtr wparam,IntPtr lparam,uint flags,uint timeout,out IntPtr result);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageTimeoutW",SetLastError=true)] private static extern IntPtr SendTextTimeout(IntPtr window,uint message,IntPtr wparam,StringBuilder text,uint flags,uint timeout,out IntPtr result);
    [DllImport("user32.dll")] private static extern bool RedrawWindow(IntPtr window,IntPtr rect,IntPtr region,uint flags);
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr window,uint command);
    [StructLayout(LayoutKind.Sequential)] private struct Gui {
        public uint Size,Flags; public IntPtr Active,Focus,Capture,Menu,MoveSize,Caret; public BksDirect.Rect CaretRect;
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
           (WindowStyle(canvas,-16).ToInt64()&0x10000)!=0 ||
           String.Equals(BksStage7.ClassName(canvas),"Edit",StringComparison.OrdinalIgnoreCase))return false;
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
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool ReadConsoleW(IntPtr input,[Out] char[] buffer,uint capacity,out uint read,IntPtr reserved);
    static readonly object Sync=new object();
    static readonly UTF8Encoding Utf8=new UTF8Encoding(false);
    static readonly JavaScriptSerializer Json=new JavaScriptSerializer();
    static readonly List<byte> Pending=new List<byte>();
    static readonly string[] Labels={"Alpha","Bravo","Charlie"};
    static TextWriter output;
    static StreamWriter raw;
    static string root,scene="menu",typed="",last="none";
    static int selected,events,tick,replies,storm;
    static bool hold,shellCr,handoff;
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
    static void Event(string kind) {
        last=kind; events++;
        if(scene=="shell") {
            if(kind.StartsWith("Text:",StringComparison.Ordinal)) {typed+=kind.Substring(5);output.Write(kind.Substring(5));}
            else if(kind=="Enter") {
                replies++;
                if(handoff && typed=="kimi") {
                    scene="fake-cli";typed="";
                    output.Write("\x1b]133;C\x07\r\nBKS_FAKE_CLI_READY_"+replies.ToString("D3")+"\r\nManaged fake CLI; no executable lookup or command execution\r\nFAKE CLI> ");
                } else {typed="";output.Write("\r\nBKS_FAKE_RESULT_"+replies.ToString("D3")+"\r\nfake> ");}
            }
            output.Flush();return;
        }
        if(scene=="fake-cli")return;
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
                        kind=final=='A'?"Up":final=='B'?"Down":final=='C'?"Right":final=='D'?"Left":final=='H'?"Home":final=='F'?"End":final=='Z'?"ShiftTab":sequence;
                        if(sequence.Contains(";5"))kind="Ctrl"+kind;
                    } else kind="Escape";
                } else if(!expire)break; else kind="Escape";
            } else if(Pending[0]==13 || Pending[0]==10)kind="Enter";
            else if(Pending[0]==9)kind="Tab";
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
    static bool Request(string request) {
        if(request=="cli-exit" && handoff && scene=="fake-cli") {
            scene="shell";typed="";shellCr=false;
            output.Write("\r\nBKS_FAKE_CLI_DONE_"+replies.ToString("D3")+"\r\n\x1b]133;A\u0007fake> \x1b]133;B\u0007");output.Flush();
        }
        else if(request=="shell") {scene="shell";typed="";hold=false;shellCr=false;output.Write("\x1b[?25h\r\nBKS_SHELL_READY\r\nfake> ");output.Flush();}
        else if(request=="menu" || request=="prompt" || request=="paint" || request=="selection") {scene=request;typed="";selected=0;hold=false;Render(true);}
        else if(request=="hold")hold=true;
        else if(request=="release") {hold=false;Render(false);}
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
        Request("paint");if(!output.ToString().Contains("BKS_PAINT_READY") || !output.ToString().EndsWith("\x1b[?25l",StringComparison.Ordinal))return 14;
        string painted=output.ToString();Request("hold");Request("passive");if(output.ToString()!=painted)return 15;
        Request("release");if(output.ToString()==painted || !output.ToString().Contains("Spinner: / Tick:"))return 16;
        Request("selection");if(!output.ToString().Contains("Input> \u0416\u754ce\u0301\U0001f600 alpha\x1b[7m \x1b[0mbeta tail") ||
            !output.ToString().EndsWith("\x1b[1;1H\x1b[?25l",StringComparison.Ordinal))return 17;
        Console.WriteLine("unified fixture parser, confirmed echo hold/release and appended-output self-test passed");return 0;
    }
    public static int Main(string[] args) {
        if(args.Length==1 && args[0]=="--self-test")return SelfTest();
        if(args.Length!=1 || (args[0]!="--session" && args[0]!="--handoff-session"))return 10;
        handoff=args[0]=="--handoff-session";
        root=Path.Combine(Environment.GetEnvironmentVariable("APPDATA"),"BKSUnifiedFixture");Directory.CreateDirectory(root);
        uint im,om;var ih=GetStdHandle(-10);var oh=GetStdHandle(-11);
        if(!GetConsoleMode(ih,out im)||!SetConsoleMode(ih,(im&~7u)|0x200u)||!GetConsoleMode(oh,out om)||!SetConsoleMode(oh,om|4u))return 11;
        SetConsoleCP(65001);SetConsoleOutputCP(65001);
        output=new CapturingWriter(new StreamWriter(Console.OpenStandardOutput(),Utf8){AutoFlush=true},Path.Combine(root,"fixture-output.vt"));
        raw=new StreamWriter(new FileStream(Path.Combine(root,"input.jsonl"),FileMode.Create,FileAccess.Write,FileShare.ReadWrite),Utf8){AutoFlush=true};
        string identity=Path.Combine(root,"identity.json");
        File.WriteAllText(identity+".pending",Json.Serialize(new {Pid=Process.GetCurrentProcess().Id,Executable=Process.GetCurrentProcess().MainModule.FileName,Cwd=Environment.CurrentDirectory}),Utf8);File.Move(identity+".pending",identity);
        if(handoff) {
            scene="shell";
            for(int i=0;i<12;i++)output.Write("BKS_HANDOFF_HISTORY_"+i.ToString("D3")+"\r\n");
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
    $fixtureArgument=if($CliHandoff){'--handoff-session'}else{'--session'}
    Write-BksJson (Join-Path $configuration 'profiles.json') @{schema_version=1;default_profile_id='unified-fixture';profiles=@(@{id='unified-fixture';name='Unified fixture';kind='local';executable=$mock;args=@($fixtureArgument);working_directory=$directory})}
    Write-BksJson (Join-Path $configuration 'settings.json') @{speech_mode='important';last_tab='keep_window';remember_workspace=$false;shortcuts=@{}}
    $start=New-Object Diagnostics.ProcessStartInfo($Exe)
    $start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.WorkingDirectory=$directory;$start.Arguments='--profile unified-fixture'
    $start.EnvironmentVariables.Clear()
    foreach($entry in @{'SystemRoot'=$env:SystemRoot;'WINDIR'=$env:SystemRoot;'COMSPEC'=(Join-Path $env:SystemRoot 'System32\cmd.exe');'APPDATA'=$appData;'LOCALAPPDATA'=(Join-Path $directory 'local');'USERPROFILE'=(Join-Path $directory 'home');'HOME'=(Join-Path $directory 'home');'TEMP'=(Join-Path $directory 'temp');'TMP'=(Join-Path $directory 'temp');'CODEX_HOME'=(Join-Path $directory 'codex');'PATH'=($bin+';'+$env:SystemRoot+'\System32')}.GetEnumerator()) { $start.EnvironmentVariables[$entry.Key]=$entry.Value }
    foreach($name in @('local','home','temp','codex')) { $null=[IO.Directory]::CreateDirectory((Join-Path $directory $name)) }
    $process=[Diagnostics.Process]::Start($start)
    $script:Case=@{Process=$process;Main=[IntPtr]::Zero;Task=$task;Mock=$mock;Fixture=$null;AppData=$appData}
    Wait-For { [BksStage7]::MainWindow($process.Id) -ne [IntPtr]::Zero } 'own unified main window'
    $script:Case.Main=[BksStage7]::MainWindow($process.Id)
    Wait-For { [IO.File]::Exists((Join-Path $task 'identity.json')) } 'own fixture identity'
    $identity=Read-BksJson (Join-Path $task 'identity.json')
    $fixture=[Diagnostics.Process]::GetProcessById($identity.Pid)
    if($identity.Executable -ine $mock -or $fixture.MainModule.FileName -ine $mock) { throw 'Unknown fixture identity refused' }
    $script:Case.Fixture=$fixture
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
    Check 'explicit reader is focused named read-only native EDIT with TextPattern' ([BksStage7]::ClassName($Reader) -ieq 'Edit' -and [BksStage7]::ReadOnly($script:Case.Process.Id,$Reader) -and $element.Current.IsKeyboardFocusable -and $element.Current.HasKeyboardFocus -and $element.Current.Name.Length -gt 0 -and (Same-UiaText (Uia-Text $Reader) $text))
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
function Test-CliHandoff([IntPtr]$InputBox,[IntPtr]$Journal,[IntPtr]$Direct,[IntPtr]$Reader) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main;$fixturePid=$script:Case.Fixture.Id
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
}

function Test-PaintIsolation([IntPtr]$Direct,[IntPtr]$Canvas) {
    $own=$script:Case.Process.Id;$main=$script:Case.Main
    # Keyless inspection of private HWNDs only. The native provider is changed
    # programmatically to provoke EDIT painting/scrolling, never PTY input.
    if(-not [BksStage7]::IsWindowVisible($Direct)) { [BksDirect]::Post($own,$main,0x111,1009) }
    Wait-For { [BksStage7]::IsWindowVisible($Direct) -and [BksStage7]::IsWindowVisible($Canvas) } 'own provider and GDI overlay are visible'
    Request-Scene 'paint'
    Wait-For { (Own-Text $Direct).Contains('BKS_PAINT_READY') -and (Own-Text $Direct).Contains('Tick: 000000') } 'own stable paint fixture is current'
    $element=Element $Direct;$provider=Own-Text $Direct
    Check 'overlay keeps visible same-area native provider and GDI canvas' ([BksStage7]::ClassName($Direct) -ieq 'Edit' -and [BksStage7]::ClassName($Canvas) -ine 'Edit' -and [BksStage7]::IsWindowVisible($Direct) -and [BksStage7]::IsWindowVisible($Canvas) -and (Rect-Key ([BksDirect]::Bounds($own,$Direct))) -ceq (Rect-Key ([BksDirect]::Bounds($own,$Canvas))))
    Check 'canvas is above provider with both WS_CLIPSIBLINGS and without canvas WS_TABSTOP' ([BksUnified]::OverlayProtected($own,$Direct,$Canvas))
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
    Assert-ExpectedInput $before 'complete keyless paint isolation scenario'
    Check 'keyless native painting and VT updates emit zero PTY bytes' ($before.Length -eq 0 -and (Raw-Input).Length -eq 0)
    $observations.Add(@{Case='keyless paint isolation final';Frames=$frames.ToArray();NativeText=(Own-Text $Direct);UiaText=(Uia-Text $Direct);RawInput=(Raw-Input);NvdaSpeech='not tested';PerceivedFlicker='not tested'})
    $checks.Add(@{Status='skipped';Name='Architectural clipping and captured pixels are checked; perceived flicker and human NVDA require manual verification'})
}

try {
    Add-Type -ReferencedAssemblies $references -TypeDefinition ($nativeSource+"`r`n"+$directSource+"`r`n"+$extraSource)
    [BksStage7]::InitializeUia()
    if(-not $CliHandoff -and -not $PaintIsolation) {
        $probe=[BksDirect]::ProbeKeyUp()
        Check 'own queue key-up contract avoids duplicate Return' (-not $probe.NativeKeyUpGeneratedReturn)
    }
    $templateExe=Join-Path $runDirectory 'unified-fixture.exe'
    $fixtureFile=Join-Path $runDirectory 'unified-fixture.cs'
    [IO.File]::WriteAllText($fixtureFile,$fixtureSource,(New-Object Text.UTF8Encoding $false))
    & (Join-Path $env:SystemRoot 'Microsoft.NET/Framework64/v4.0.30319/csc.exe') /nologo /target:exe /platform:x64 /warnaserror+ /reference:System.Web.Extensions.dll ('/out:'+$templateExe) $fixtureFile
    if($LASTEXITCODE -ne 0) { throw 'Own unified fixture compilation failed' }
    Check 'native/UIA helpers and own unified fixture compile' (Test-Path -LiteralPath $templateExe)
    $selfStart=New-Object Diagnostics.ProcessStartInfo($templateExe,'--self-test')
    $selfStart.UseShellExecute=$false;$selfStart.CreateNoWindow=$true;$selfStart.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $selfStart.RedirectStandardOutput=$true;$selfStart.RedirectStandardError=$true
    $self=[Diagnostics.Process]::Start($selfStart)
    try {
        if(-not $self.WaitForExit(5000)) { $self.Kill();throw 'Own fixture self-test timeout' }
        [IO.File]::WriteAllText((Join-Path $runDirectory 'fixture-self-test.txt'),$self.StandardOutput.ReadToEnd()+$self.StandardError.ReadToEnd())
        Check 'fixture split keys, Unicode width, held/released VT echo and appended output self-test' ($self.ExitCode -eq 0)
    } finally { $self.Dispose() }
    if($PrepareOnly) {
        $checks.Add(@{Status='skipped';Name='PrepareOnly: no BKS window, ConPTY session, input-queue attachment or UI interaction launched'})
    } else {
        $Exe=Get-BksFullPath $Exe;Assert-BksNoReparse $Exe
        Start-Case
        $main=$script:Case.Main;$pidOwn=$script:Case.Process.Id
        $direct=Field $main 1011;$reader=Field $main 1018;$command=Field $main 1002;$canvas=Field $main 1017;$journal=Field $main 1001
        if($CliHandoff) {
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
        Check 'explicit TUI mode overlays GDI canvas over a full-area read-only native provider without a lower command field' ([BksStage7]::ClassName($direct) -ieq 'Edit' -and [BksStage7]::ReadOnly($pidOwn,$direct) -and -not [BksStage7]::IsWindowVisible($command) -and [BksStage7]::IsWindowVisible($canvas) -and [BksUnified]::OverlayProtected($pidOwn,$direct,$canvas) -and (Rect-Key ([BksDirect]::Bounds($pidOwn,$direct))) -ceq (Rect-Key ([BksDirect]::Bounds($pidOwn,$canvas))))
        Check 'native TextPattern and Unicode document agree' ((Element $direct).Current.Name -ceq [BksDirect]::DirectName -and (Same-UiaText (Uia-Text $direct) (Own-Text $direct)))
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
            if(-not $CliHandoff -and -not $PaintIsolation) {$diagnostic.Focus=Focus-Diagnostics $ownedDirect}
            $observations.Add($diagnostic)
            foreach($id in @(1001,1002,1011,1018,1026)) {
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
    $counts=@{passed=@($checks|Where-Object {$_.Status -eq 'passed'}).Count;failed=@($checks|Where-Object {$_.Status -eq 'failed'}).Count;skipped=@($checks|Where-Object {$_.Status -eq 'skipped'}).Count;error=@($checks|Where-Object {$_.Status -eq 'error'}).Count}
    $scope=if($CliHandoff -or $PaintIsolation){'Keyless own PID/HWND/private APPDATA and managed fixture only; own native provider messages/WM_COMMAND and fixture IPC; no activation/focus/keyboard-state/keystroke helpers, real CLI, model, network or approval; UIA is not human NVDA'}else{'Only own PID/HWND/private APPDATA and fixture; no SendInput/global keyboard events/real CLI/network/approval; modified keyboard state is limited to attached own UI queue and restored in finally; UIA is not human NVDA'}
    Write-BksJson (Join-Path $runDirectory 'summary.json') @{Scope=$scope;PreparedOnly=[bool]$PrepareOnly;Minimal=[bool]$Minimal;ReadNavigation=[bool]$ReadNavigation;ShellRegression=[bool]$ShellRegression;CliHandoff=[bool]$CliHandoff;PaintIsolation=[bool]$PaintIsolation;TuiSelection=[bool]$TuiSelection;Executable=$Exe;Counts=$counts;Checks=$checks.ToArray();Observations=$observations.ToArray()}
    [IO.File]::WriteAllLines((Join-Path $runDirectory 'results.txt'),[string[]]@($checks|ForEach-Object {$_.Status+': '+$_.Name}),(New-Object Text.UTF8Encoding $false))
}
Write-Output "Unified TUI: passed=$($counts.passed) failed=$($counts.failed) skipped=$($counts.skipped) error=$($counts.error)"
Write-Output "Artifacts: $runDirectory"
if($failure -or $counts.failed -or $counts.error) { throw $(if($failure){$failure}else{'Unified checks or cleanup failed'}) }
