$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading.Tasks;
public static class OpenMindBoundedCapture {
    public sealed class Result { public int exitCode; public bool truncated; public bool timedOut; }
    static bool Drain(StreamReader input, string path, int maxChars) {
        bool truncated = false;
        int stored = 0;
        char[] buffer = new char[4096];
        using (var output = new StreamWriter(path, false, new UTF8Encoding(false))) {
            int count;
            while ((count = input.Read(buffer, 0, buffer.Length)) > 0) {
                int keep = Math.Min(count, Math.Max(0, maxChars - stored));
                if (keep > 0) output.Write(buffer, 0, keep);
                stored += keep;
                if (keep < count) truncated = true;
            }
        }
        return truncated;
    }
    public static Result Run(string encoded, string cwd, string stdout, string stderr, int limit, int timeoutMs) {
        var start = new ProcessStartInfo("powershell.exe", "-NoLogo -NoProfile -NonInteractive -EncodedCommand " + encoded);
        start.UseShellExecute = false;
        start.CreateNoWindow = true;
        start.RedirectStandardOutput = true;
        start.RedirectStandardError = true;
        start.StandardOutputEncoding = Encoding.UTF8;
        start.StandardErrorEncoding = Encoding.UTF8;
        start.WorkingDirectory = cwd;
        using (var process = Process.Start(start)) {
            // Drain both pipes concurrently, including all bytes beyond the cap.
            var outTask = Task.Run(() => Drain(process.StandardOutput, stdout, limit));
            var errTask = Task.Run(() => Drain(process.StandardError, stderr, limit));
            bool timedOut = !process.WaitForExit(timeoutMs);
            if (timedOut) {
                using (var kill = Process.Start(new ProcessStartInfo("taskkill.exe", "/PID " + process.Id + " /T /F") { UseShellExecute = false, CreateNoWindow = true })) { kill.WaitForExit(); }
                process.WaitForExit();
            }
            Task.WaitAll(outTask, errTask);
            return new Result { exitCode = timedOut ? -1 : process.ExitCode, truncated = outTask.Result || errTask.Result, timedOut = timedOut };
        }
    }
}
'@
try {
  $summary = [OpenMindBoundedCapture]::Run('__ENCODED__', '__CWD__', '__STDOUT__', '__STDERR__', __LIMIT__, __TIMEOUT__)
  $summary | ConvertTo-Json -Compress | Set-Content -LiteralPath '__RESULT__.pending' -Encoding UTF8
} catch {
  # Never copy an unbounded exception dump into the mapped host folder.
  '{"exitCode":1,"truncated":false,"timedOut":false}' | Set-Content -LiteralPath '__RESULT__.pending' -Encoding UTF8
}
Move-Item -LiteralPath '__RESULT__.pending' -Destination '__RESULT__' -Force
__SHUTDOWN__
