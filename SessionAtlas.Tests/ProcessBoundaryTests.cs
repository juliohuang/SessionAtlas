using System.Diagnostics;
using SessionAtlas.Core.Launcher;
using SessionAtlas.Core.Config;
using SessionAtlas.Core.Process;
using SessionAtlas.Core.Scanner;
using SessionAtlas.Models;

namespace SessionAtlas.Tests;

public class ProcessBoundaryTests
{
    [Fact]
    public void CommandDetectionCanUseAFakeRunner()
    {
        var runner = new RecordingProcessRunner
        {
            NextResult = new ProcessExecutionResult(0, "fixture-tool")
        };

        Assert.True(ScannerRegistry.CommandExists("fixture-tool", runner));
        var request = Assert.Single(runner.RunRequests);
        Assert.Contains("fixture-tool", request.ArgumentList);
        Assert.False(ScannerRegistry.CommandExists("-oProxyCommand=calc", runner));
        Assert.Single(runner.RunRequests);
    }

    [Fact]
    public void CliLauncherCanBeVerifiedWithoutStartingATerminal()
    {
        using var root = new TemporaryDirectory();
        var runner = new RecordingProcessRunner();
        var projectPath = root.Combine("fixture & safe");
        Directory.CreateDirectory(projectPath);

        new CliLauncher(runner, new AppConfig())
            .Launch(projectPath, "codex", "session-demo");

        var request = Assert.Single(runner.StartRequests);
        Assert.Equal(projectPath, request.WorkingDirectory);
        Assert.DoesNotContain(
            request.ArgumentList,
            value => value != projectPath &&
                     value.Contains(projectPath, StringComparison.Ordinal));
        Assert.Contains(request.ArgumentList, value => value.Contains("codex", StringComparison.Ordinal));
        Assert.Contains(
            request.ArgumentList,
            value => value.Contains("session-demo", StringComparison.Ordinal));
    }

    [Theory]
    [InlineData("codex & calc", null)]
    [InlineData("codex", "session\rwhoami")]
    public void CliLauncherRejectsScannerMetadataContainingShellSyntax(
        string toolKey,
        string? sessionId)
    {
        using var root = new TemporaryDirectory();
        var runner = new RecordingProcessRunner();
        var launcher = new CliLauncher(runner, new AppConfig());

        Assert.Throws<ArgumentException>(() => launcher.Launch(root.Path, toolKey, sessionId));
        Assert.Empty(runner.StartRequests);
    }

    [Fact]
    public void CliLauncherRejectsUnsafeCustomCommand()
    {
        using var root = new TemporaryDirectory();
        var runner = new RecordingProcessRunner();
        var config = new AppConfig
        {
            CustomTools =
            [
                new ToolSource
                {
                    Key = "fixture",
                    Name = "Fixture",
                    CliCommand = "codex & calc",
                }
            ]
        };

        var launcher = new CliLauncher(runner, config);

        Assert.Throws<InvalidOperationException>(
            () => launcher.Launch(root.Path, "fixture"));
        Assert.Empty(runner.StartRequests);
    }

    [Fact]
    public void CliLauncherRequiresCanonicalToolConfiguration()
    {
        using var root = new TemporaryDirectory();
        var runner = new RecordingProcessRunner();
        var config = new AppConfig
        {
            CustomTools =
            [
                new ToolSource
                {
                    Key = "codex",
                    Name = "Override",
                    CliCommand = "calc",
                    IsEnabled = true,
                },
                new ToolSource
                {
                    Key = "disabled-agent",
                    Name = "Disabled",
                    CliCommand = "disabled-agent",
                    IsEnabled = false,
                }
            ]
        };
        var launcher = new CliLauncher(runner, config);

        launcher.Launch(root.Path, "codex");
        var request = Assert.Single(runner.StartRequests);
        Assert.Contains(request.ArgumentList, value => value.Contains("codex"));
        Assert.DoesNotContain(request.ArgumentList, value => value.Contains("calc"));

        Assert.Throws<InvalidOperationException>(
            () => launcher.Launch(root.Path, "disabled-agent"));
        Assert.Throws<InvalidOperationException>(
            () => launcher.Launch(root.Path, "unknown-agent"));
        Assert.Single(runner.StartRequests);
    }

    [Fact]
    public void CustomToolAvailabilityUsesConfiguredExecutableInsteadOfToolKey()
    {
        var runner = new RecordingProcessRunner
        {
            NextResult = new ProcessExecutionResult(0, "fixture-cli")
        };
        var launcher = new CliLauncher(
            runner,
            new AppConfig
            {
                CustomTools =
                [
                    new ToolSource
                    {
                        Key = "friendly-key",
                        Name = "Fixture",
                        CliCommand = "fixture-cli --profile safe",
                        IsEnabled = true,
                    }
                ]
            });

        Assert.True(launcher.IsToolAvailable("friendly-key"));
        var request = Assert.Single(runner.RunRequests);
        Assert.Contains("fixture-cli", request.ArgumentList);
        Assert.DoesNotContain("friendly-key", request.ArgumentList);
        Assert.Equal(
            ["fixture-cli", "--profile", "safe", "--resume", "session-123"],
            launcher.BuildToolArguments("friendly-key", "session-123"));
    }

    [Fact]
    public void DirectoryOpenerPassesShellPunctuationAsOneArgument()
    {
        using var root = new TemporaryDirectory();
        var runner = new RecordingProcessRunner();
        var path = root.Combine("fixture & safe");
        Directory.CreateDirectory(path);

        DirectoryOpener.Open(path, runner);

        var request = Assert.Single(runner.StartRequests);
        Assert.False(request.UseShellExecute);
        Assert.Equal(Path.GetFullPath(path), Assert.Single(request.ArgumentList));
        if (OperatingSystem.IsWindows())
            Assert.Equal("explorer.exe", request.FileName);
    }

    private sealed class RecordingProcessRunner : IProcessRunner
    {
        public ProcessExecutionResult NextResult { get; init; } = new(0);
        public List<ProcessStartInfo> RunRequests { get; } = new();
        public List<ProcessStartInfo> StartRequests { get; } = new();

        public ProcessExecutionResult Run(ProcessStartInfo startInfo)
        {
            RunRequests.Add(startInfo);
            return NextResult;
        }

        public void Start(ProcessStartInfo startInfo)
        {
            StartRequests.Add(startInfo);
        }
    }
}
