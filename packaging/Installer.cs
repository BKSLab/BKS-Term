// Build with the .NET Framework C# compiler shipped with Windows.
// Embedded files: the verified portable ZIP, install.ps1 and common.ps1.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Text;
using System.Windows.Forms;

internal static class Installer
{
    [STAThread]
    private static int Main(string[] arguments)
    {
        bool extractOnly = arguments.Length == 2 && arguments[0] == "--extract-only";
        bool quiet = extractOnly;
        bool skipIntegration = false;
        string installDirectory = null;
        string temporary = null;
        try
        {
            if (extractOnly)
            {
                string target = FullLocalDirectory(arguments[1]);
                AssertNoReparse(target);
                if (Directory.Exists(target) && Directory.GetFileSystemEntries(target).Length != 0)
                    throw new InvalidOperationException("Extraction requires an empty directory.");
                Directory.CreateDirectory(target);
                WriteResources(target);
                return 0;
            }
            for (int index = 0; index < arguments.Length; index++)
            {
                if (arguments[index] == "--quiet") quiet = true;
                else if (arguments[index] == "--skip-integration") skipIntegration = true;
                else if (arguments[index] == "--install-directory" && index + 1 < arguments.Length)
                    installDirectory = FullLocalDirectory(arguments[++index]);
                else throw new ArgumentException("Unknown setup argument: " + arguments[index]);
            }
            temporary = Path.Combine(Path.GetTempPath(), ".bks-setup-" + Guid.NewGuid().ToString("N"));
            AssertNoReparse(temporary);
            Directory.CreateDirectory(temporary);
            WriteResources(temporary);
            string powershell = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows),
                @"System32\WindowsPowerShell\v1.0\powershell.exe");
            if (!File.Exists(powershell)) throw new FileNotFoundException("Windows PowerShell 5.1 is required.", powershell);
            string command = "-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File " +
                Quote(Path.Combine(temporary, "install.ps1")) + " -PayloadArchive " + Quote(Path.Combine(temporary, "payload.zip"));
            if (installDirectory != null) command += " -InstallDirectory " + Quote(installDirectory);
            if (skipIntegration) command += " -SkipIntegration";
            var start = new ProcessStartInfo(powershell, command);
            start.UseShellExecute = false;
            start.CreateNoWindow = true;
            start.WindowStyle = ProcessWindowStyle.Hidden;
            start.WorkingDirectory = temporary;
            start.RedirectStandardOutput = true;
            start.RedirectStandardError = true;
            var output = new StringBuilder();
            using (var process = new Process())
            {
                process.StartInfo = start;
                DataReceivedEventHandler collect = delegate(object sender, DataReceivedEventArgs message)
                {
                    if (message.Data == null) return;
                    lock (output) { if (output.Length < 128 * 1024) output.AppendLine(message.Data); }
                };
                process.OutputDataReceived += collect;
                process.ErrorDataReceived += collect;
                if (!process.Start()) throw new InvalidOperationException("Could not start the per-user installer.");
                process.BeginOutputReadLine();
                process.BeginErrorReadLine();
                process.WaitForExit();
                if (process.ExitCode != 0)
                    throw new InvalidOperationException("Install failed (exit " + process.ExitCode + ").\r\n" + output.ToString());
            }
            if (!quiet) MessageBox.Show("BKS Term is installed for the current user.\r\nOpen BKS Term from the Start menu.\r\nProfiles and settings were preserved.",
                "BKS Term setup", MessageBoxButtons.OK, MessageBoxIcon.Information);
            return 0;
        }
        catch (Exception error)
        {
            if (quiet) Console.Error.WriteLine(error.Message);
            else MessageBox.Show(error.Message, "BKS Term setup failed", MessageBoxButtons.OK, MessageBoxIcon.Error);
            return 1;
        }
        finally
        {
            if (temporary != null)
            {
                try { DeletePrivateTemporaryDirectory(temporary); }
                catch (Exception error) { Console.Error.WriteLine("Temporary setup files retained: " + error.Message); }
            }
        }
    }

    private static string Quote(string value)
    {
        if (value.IndexOf('"') >= 0) throw new ArgumentException("Double quotes are not allowed in setup paths.");
        return "\"" + value + "\"";
    }

    private static string FullLocalDirectory(string value)
    {
        string full = Path.GetFullPath(value).TrimEnd(Path.DirectorySeparatorChar);
        if (full.Length < 4 || full[1] != ':' || full[2] != '\\' || full.IndexOf('"') >= 0)
            throw new ArgumentException("A local directory below a drive root is required.");
        return full;
    }

    private static void AssertNoReparse(string value)
    {
        string current = Path.GetFullPath(value);
        while (!String.IsNullOrEmpty(current))
        {
            if ((Directory.Exists(current) || File.Exists(current)) &&
                (File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0)
                throw new IOException("Refusing a junction or symbolic link: " + current);
            current = Path.GetDirectoryName(current.TrimEnd(Path.DirectorySeparatorChar));
        }
    }

    private static void WriteResources(string root)
    {
        string[] resources = { "Payload", "Install", "Common" };
        string[] files = { "payload.zip", "install.ps1", "common.ps1" };
        for (int index = 0; index < resources.Length; index++)
        {
            string target = Path.Combine(root, files[index]);
            AssertNoReparse(target);
            using (Stream source = Assembly.GetExecutingAssembly().GetManifestResourceStream("BksTerm." + resources[index]))
            {
                if (source == null) throw new InvalidDataException("Missing setup resource: " + resources[index]);
                using (var destination = new FileStream(target, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                    source.CopyTo(destination);
            }
        }
    }

    private static void DeletePrivateTemporaryDirectory(string root)
    {
        string full = FullLocalDirectory(root);
        string parent = FullLocalDirectory(Path.GetTempPath());
        if (!String.Equals(Path.GetDirectoryName(full), parent, StringComparison.OrdinalIgnoreCase) ||
            !Path.GetFileName(full).StartsWith(".bks-setup-", StringComparison.Ordinal))
            throw new IOException("Refusing cleanup outside the private setup directory.");
        var pending = new Stack<string>();
        var directories = new List<string>();
        var files = new List<string>();
        pending.Push(full);
        while (pending.Count != 0)
        {
            string directory = pending.Pop();
            AssertNoReparse(directory);
            directories.Add(directory);
            foreach (string entry in Directory.GetFileSystemEntries(directory))
            {
                AssertNoReparse(entry);
                if (Directory.Exists(entry)) pending.Push(entry); else files.Add(entry);
            }
        }
        foreach (string file in files) File.Delete(file);
        for (int index = directories.Count - 1; index >= 0; index--) Directory.Delete(directories[index], false);
    }
}
