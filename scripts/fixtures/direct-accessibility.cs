// Local ConPTY peer for the direct-input accessibility check. Executes no commands.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Web.Script.Serialization;

internal static class DirectAccessibilityFixture
{
    [DllImport("kernel32.dll")] private static extern IntPtr GetStdHandle(int kind);
    [DllImport("kernel32.dll")] private static extern bool GetConsoleMode(IntPtr handle, out uint mode);
    [DllImport("kernel32.dll")] private static extern bool SetConsoleMode(IntPtr handle, uint mode);
    [DllImport("kernel32.dll")] private static extern bool SetConsoleCP(uint codePage);
    [DllImport("kernel32.dll")] private static extern bool SetConsoleOutputCP(uint codePage);
    private static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
    private static readonly UTF8Encoding Utf8 = new UTF8Encoding(false);
    private static readonly object Sync = new object();
    private static readonly List<byte> Pending = new List<byte>();
    private static readonly string[] Labels = { "Alpha", "Bravo", "Charlie" };
    private static TextWriter output;
    private static StreamWriter rawLog;
    private static string root;
    private static int selected;
    private static int events;
    private static int tick;
    private static string last = "none";
    private static string typed = "";
    private static string scene = "marker";
    private static long escapeAt;

    private static void Render()
    {
        // Keep the original full-clear baseline. Style scenes repaint in place:
        // CSI 2J may push the previous frame into terminal scrollback, which
        // would otherwise create text changes unrelated to the highlight.
        output.Write("\x1b[?25l" + (scene == "marker" ? "\x1b[2J" : "") + "\x1b[H");
        output.Write("BKS_DIRECT_READY \u0416\u754ce\u0301\U0001f600\r\n");
        output.Write("Fixture menu: Up/Down; Enter and Esc only record input.\r\n");
        for (int row = 0; row < Labels.Length; row++)
        {
            if (row == selected && scene == "background") output.Write("\x1b[44m");
            if (row == selected && scene == "inverse") output.Write("\x1b[7m");
            output.Write((row == selected && scene == "marker" ? "> " : "  ") + "\u754c\U0001f600e\u0301 " + Labels[row] + "\x1b[0m\x1b[K\r\n");
        }
        // Style scenes deliberately keep every character fixed during an
        // arrow key. Raw input is still recorded in the private input log.
        string eventText = scene == "marker" ? events.ToString("D4") + " " + last : "recorded only in private input log";
        output.Write("\r\nInput: " + typed + "\r\nEvent: " + eventText + "\r\nTick: " + tick.ToString("D6") + " Mode: " + scene + "\x1b[K");
        // Column 7 follows a wide CJK cell and a surrogate-pair emoji.
        // Its reading-document offset is five UTF-16 units into this row.
        output.Write("\x1b[" + (3 + selected) + ";7H" + (scene == "marker" ? "\x1b[?25h" : "\x1b[?25l"));
        output.Flush();
    }

    private static bool ApplyScene(string requested)
    {
        if (requested == "passive") tick++;
        else if (requested == "marker" || requested == "background" || requested == "inverse")
        {
            scene = requested;
            selected = 0;
        }
        else return false;
        Render();
        return true;
    }

    private static void Event(string kind)
    {
        if (kind == "Up") selected = Math.Max(0, selected - 1);
        if (kind == "Down") selected = Math.Min(Labels.Length - 1, selected + 1);
        if (kind.StartsWith("Text:", StringComparison.Ordinal)) typed += kind.Substring(5);
        last = kind;
        events++;
    }

    private static bool Drain(bool expireEscape)
    {
        bool changed = false;
        while (Pending.Count > 0)
        {
            int length = 1;
            string kind;
            if (Pending[0] == 27)
            {
                if (Pending.Count >= 3 && (Pending[1] == (byte)'[' || Pending[1] == (byte)'O') &&
                    (Pending[2] == (byte)'A' || Pending[2] == (byte)'B'))
                {
                    kind = Pending[2] == (byte)'A' ? "Up" : "Down";
                    length = 3;
                }
                else if (!expireEscape) break;
                else kind = "Escape";
            }
            else if (Pending[0] == 13 || Pending[0] == 10) kind = "Enter";
            else
            {
                byte first = Pending[0];
                length = first < 0x80 ? 1 : first < 0xe0 ? 2 : first < 0xf0 ? 3 : 4;
                if (Pending.Count < length) break;
                kind = "Text:" + Utf8.GetString(Pending.GetRange(0, length).ToArray());
            }
            Pending.RemoveRange(0, length);
            Event(kind);
            changed = true;
        }
        return changed;
    }

    private static void ReadInput()
    {
        try
        {
            Stream input = Console.OpenStandardInput();
            byte[] bytes = new byte[256];
            int count;
            while ((count = input.Read(bytes, 0, bytes.Length)) > 0)
            {
                lock (Sync)
                {
                    byte[] chunk = new byte[count];
                    Array.Copy(bytes, chunk, count);
                    rawLog.WriteLine(Json.Serialize(new { Bytes = BitConverter.ToString(chunk).Replace("-", ""), Count = count }));
                    rawLog.Flush();
                    Pending.AddRange(chunk);
                    escapeAt = Stopwatch.GetTimestamp();
                    if (Drain(false)) Render();
                }
            }
        }
        catch (IOException) { }
    }

    private static int SelfTest()
    {
        Pending.AddRange(new byte[] { 27, 91, 66 });
        if (!Drain(false) || selected != 1 || events != 1) return 2;
        Pending.AddRange(new byte[] { 27, 91 });
        if (Drain(false)) return 3;
        Pending.Add(65);
        if (!Drain(false) || selected != 0 || events != 2) return 4;
        Pending.Add(27);
        if (Drain(false) || !Drain(true) || last != "Escape") return 5;
        Pending.AddRange(Utf8.GetBytes("\u0416"));
        if (!Drain(false) || typed != "\u0416") return 6;
        Pending.Add(13);
        if (!Drain(false) || last != "Enter" || events != 5) return 7;
        foreach (string mode in new[] { "background", "inverse" })
        {
            output = new StringWriter();
            if (!ApplyScene(mode)) return 20;
            string before = output.ToString();
            output = new StringWriter();
            Event("Down");
            Render();
            string after = output.ToString();
            const string ansi = @"\x1b\[[0-?]*[ -/]*[@-~]";
            if (before == after || Regex.Replace(before, ansi, "") != Regex.Replace(after, ansi, "")) return 21;
            string highlight = mode == "background" ? "\x1b[44m" : "\x1b[7m";
            if (!before.Contains(highlight + "  \u754c\U0001f600e\u0301 Alpha") ||
                !after.Contains(highlight + "  \u754c\U0001f600e\u0301 Bravo") || !after.EndsWith("\x1b[?25l")) return 22;
        }
        output = new StringWriter();
        if (ApplyScene("unknown")) return 23;
        Console.WriteLine("direct fixture parser and text-invariant style-scene self-test passed");
        return 0;
    }

    public static int Main(string[] arguments)
    {
        if (arguments.Length == 1 && arguments[0] == "--self-test") return SelfTest();
        if (arguments.Length != 1 || arguments[0] != "--session") return 10;
        root = Path.Combine(Environment.GetEnvironmentVariable("APPDATA"), "BKSDirectFixture");
        Directory.CreateDirectory(root);
        uint inputMode, outputMode;
        IntPtr inputHandle = GetStdHandle(-10), outputHandle = GetStdHandle(-11);
        if (!GetConsoleMode(inputHandle, out inputMode) || !SetConsoleMode(inputHandle, (inputMode & ~7u) | 0x200u)) return 11;
        if (!GetConsoleMode(outputHandle, out outputMode) || !SetConsoleMode(outputHandle, outputMode | 4u)) return 12;
        if (!SetConsoleCP(65001) || !SetConsoleOutputCP(65001)) return 13;
        output = new StreamWriter(Console.OpenStandardOutput(), Utf8) { AutoFlush = true };
        rawLog = new StreamWriter(new FileStream(Path.Combine(root, "input.jsonl"), FileMode.Create, FileAccess.Write, FileShare.ReadWrite), Utf8) { AutoFlush = true };
        string identity = Path.Combine(root, "identity.json");
        File.WriteAllText(identity + ".pending", Json.Serialize(new {
            Pid = Process.GetCurrentProcess().Id,
            Executable = Process.GetCurrentProcess().MainModule.FileName,
            Cwd = Environment.CurrentDirectory,
            InputMode = (inputMode & ~7u) | 0x200u
        }), Utf8);
        File.Move(identity + ".pending", identity);
        for (int row = 0; row < 70; row++) output.Write("BKS_DIRECT_HISTORY_" + row.ToString("D3") + "\r\n");
        Render();
        new Thread(ReadInput) { IsBackground = true }.Start();
        int repaint = 0;
        while (!File.Exists(Path.Combine(root, "exit")))
        {
            lock (Sync)
            {
                if (Pending.Count > 0 && (Stopwatch.GetTimestamp() - escapeAt) * 1000 / Stopwatch.Frequency >= 100)
                    if (Drain(true)) Render();
                string sceneRequest = Path.Combine(root, "scene.request");
                if (File.Exists(sceneRequest))
                {
                    string requested = File.ReadAllText(sceneRequest, Utf8).Trim();
                    if (ApplyScene(requested))
                    {
                        File.Delete(sceneRequest);
                        File.WriteAllText(Path.Combine(root, "scene.done"), requested, Utf8);
                    }
                }
                if (repaint == 0 && File.Exists(Path.Combine(root, "repaint.request"))) repaint = 100;
                if (repaint > 0)
                {
                    tick++;
                    Render();
                    repaint--;
                    if (repaint == 0)
                    {
                        File.Delete(Path.Combine(root, "repaint.request"));
                        File.WriteAllText(Path.Combine(root, "repaint.done"), tick.ToString(), Utf8);
                    }
                }
            }
            Thread.Sleep(20);
        }
        output.Write("\x1b[?25h\r\nBKS_DIRECT_FIXTURE_EXIT\r\n");
        output.Flush();
        return 0;
    }
}
