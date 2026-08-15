using System.Diagnostics;

namespace SessionAtlas.Core.Process;

/// <summary>
/// Boundary around operating-system process execution. Production uses
/// <see cref="SystemProcessRunner"/>; tests supply a recording fake so they
/// never start terminals, tools, git, ssh, or browsers.
/// </summary>
public interface IProcessRunner
{
    ProcessExecutionResult Run(ProcessStartInfo startInfo);
    void Start(ProcessStartInfo startInfo);
}

public sealed record ProcessExecutionResult(
    int ExitCode,
    string StandardOutput = "",
    string StandardError = "");

public sealed class SystemProcessRunner : IProcessRunner
{
    public ProcessExecutionResult Run(ProcessStartInfo startInfo)
    {
        using var process = System.Diagnostics.Process.Start(startInfo)
            ?? throw new InvalidOperationException($"Failed to start process: {startInfo.FileName}");
        var stdout = startInfo.RedirectStandardOutput
            ? process.StandardOutput.ReadToEndAsync()
            : Task.FromResult("");
        var stderr = startInfo.RedirectStandardError
            ? process.StandardError.ReadToEndAsync()
            : Task.FromResult("");
        process.WaitForExit();
        return new ProcessExecutionResult(
            process.ExitCode,
            stdout.GetAwaiter().GetResult(),
            stderr.GetAwaiter().GetResult());
    }

    public void Start(ProcessStartInfo startInfo)
    {
        _ = System.Diagnostics.Process.Start(startInfo)
            ?? throw new InvalidOperationException($"Failed to start process: {startInfo.FileName}");
    }
}
