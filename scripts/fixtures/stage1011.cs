// Hermetic terminal/session and Codex 0.164.0 stdio peer. Never executes tools.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;

internal static class Stage1011Fixture
{
    private static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
    private static readonly UTF8Encoding Utf8 = new UTF8Encoding(false);
    private static string root;
    private static string id;
    private static TextWriter output;
    private static void Reply(object value) { output.WriteLine(Json.Serialize(value)); output.Flush(); }
    private static bool SessionArguments(string[] arguments)
    {
        return arguments.Length == 0 || (arguments.Length == 1 && arguments[0] == "--session");
    }
    private static void Marker(string kind, string[] arguments)
    {
        string path = Path.Combine(root, kind + "-" + id + ".json");
        File.WriteAllText(path + ".pending", Json.Serialize(new {
            Pid = Process.GetCurrentProcess().Id, Cwd = Environment.CurrentDirectory,
            Executable = Process.GetCurrentProcess().MainModule.FileName, Arguments = arguments
        }), Utf8);
        File.Move(path + ".pending", path);
    }
    public static int Main(string[] arguments)
    {
        if (arguments.Length == 1 && arguments[0] == "--self-test")
        {
            if (!SessionArguments(new string[0]) || !SessionArguments(new[] { "--session" }) ||
                SessionArguments(new[] { "--version" }) || SessionArguments(new[] { "app-server", "--listen", "stdio://" }) ||
                SessionArguments(new[] { "unexpected" })) return 20;
            Console.WriteLine("stage1011 fixture harmless session argument self-test passed");
            return 0;
        }
        if (arguments.Length == 1 && arguments[0] == "--version") { Console.WriteLine("codex-cli 0.164.0"); return 0; }
        output = new StreamWriter(Console.OpenStandardOutput(), Utf8) { AutoFlush = true };
        root = Path.Combine(Environment.GetEnvironmentVariable("APPDATA"), "BKSStage1011");
        Directory.CreateDirectory(root);
        id = Process.GetCurrentProcess().Id.ToString();
        if (SessionArguments(arguments))
        {
            // A discovered CLI profile supplies no default arguments. It must
            // still become only this controlled peer, never a shell or tool.
            Marker("session", arguments);
            output.WriteLine("BKS1011_CWD=" + Environment.CurrentDirectory);
            output.WriteLine("BKS1011_SESSION_READY");
            output.Flush();
            while (!File.Exists(Path.Combine(root, "exit-" + id))) Thread.Sleep(30);
            return 0;
        }
        if (arguments.Length != 3 || arguments[0] != "app-server" || arguments[1] != "--listen" || arguments[2] != "stdio://") return 3;
        Marker("server", arguments);
        string log = Path.Combine(root, "server-" + id + ".ndjson");
        var input = new StreamReader(Console.OpenStandardInput(), Utf8);
        string line;
        while ((line = input.ReadLine()) != null)
        {
            using (var stream = new FileStream(log, FileMode.Append, FileAccess.Write, FileShare.ReadWrite))
            { byte[] data = Utf8.GetBytes(line + "\n"); stream.Write(data, 0, data.Length); }
            var request = Json.DeserializeObject(line) as Dictionary<string, object>;
            if (request == null) return 4;
            object methodValue;
            string method = request.TryGetValue("method", out methodValue) ? methodValue as string : null;
            object requestId;
            request.TryGetValue("id", out requestId);
            if (method == "initialize") Reply(new { id = requestId, result = new {
                codexHome = "fixture-only", userAgent = "fixture/0.164.0", platformFamily = "windows", platformOs = "windows"
            }});
            else if (method == "thread/start") Reply(new { id = requestId, result = new {
                thread = new { id = "qa-thread" }, approvalPolicy = "untrusted", approvalsReviewer = "user",
                sandbox = new { type = "readOnly", networkAccess = false }, cwd = Environment.CurrentDirectory, model = "fixture-model"
            }});
            else if (method == "turn/start")
            {
                Reply(new { id = requestId, result = new { turn = new { id = "qa-turn", status = "inProgress" } } });
                Reply(new { method = "turn/started", @params = new { threadId = "qa-thread", turn = new { id = "qa-turn", status = "inProgress" } } });
                Reply(new { method = "item/agentMessage/delta", @params = new { threadId = "qa-thread", turnId = "qa-turn", itemId = "qa-message", delta = "STAGE1011_RESPONSE \u0416" } });
                Reply(new { method = "item/completed", @params = new { threadId = "qa-thread", turnId = "qa-turn", item = new { type = "agentMessage", id = "qa-message", text = "STAGE1011_RESPONSE \u0416" } } });
                Reply(new { id = "qa-approval", method = "item/commandExecution/requestApproval", @params = new {
                    threadId = "qa-thread", turnId = "qa-turn", itemId = "qa-command", startedAtMs = 123,
                    command = "echo fixture-only", cwd = Environment.CurrentDirectory, reason = "synthetic local QA; never execute", kind = "command"
                }});
            }
            else if (method == "turn/interrupt")
            {
                Reply(new { id = requestId, result = new { } });
                Reply(new { method = "turn/completed", @params = new { threadId = "qa-thread", turn = new { id = "qa-turn", status = "interrupted" } } });
            }
            // Decisions, if any, are merely recorded. No command or approval is executed.
        }
        return 0;
    }
}
