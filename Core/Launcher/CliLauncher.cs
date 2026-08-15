using System.Diagnostics;
using System.Runtime.InteropServices;
using SessionAtlas.Core.Config;
using SessionAtlas.Core.Process;
using SessionAtlas.Core.Scanner;

namespace SessionAtlas.Core.Launcher;

/// <summary>
/// Launches a configured AI CLI in a terminal without placing project paths
/// or unvalidated scanner metadata into shell syntax.
/// </summary>
public class CliLauncher
{
    private readonly Dictionary<string, string> _commands;
    private readonly IProcessRunner _processRunner;

    public CliLauncher(IProcessRunner? processRunner = null, AppConfig? config = null)
    {
        _processRunner = processRunner ?? new SystemProcessRunner();
        _commands = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
        {
            ["claude"] = "claude",
            ["codex"] = "codex",
            ["kimi"] = "kimi",
            ["opencode"] = "opencode",
            ["aider"] = "aider",
        };

        config ??= LoadConfigSafely();
        foreach (var tool in config.CustomTools.Where(tool => tool.IsEnabled))
        {
            if (string.IsNullOrWhiteSpace(tool.Key) || string.IsNullOrWhiteSpace(tool.CliCommand))
                continue;
            try
            {
                var key = CommandSecurity.ValidateToolKey(tool.Key);
                _ = CommandSecurity.ParseSafeCommand(tool.CliCommand);
                // Custom config cannot replace a built-in executable identity.
                _commands.TryAdd(key, tool.CliCommand);
            }
            catch (ArgumentException)
            {
                // A hand-edited invalid entry is not an executable source.
            }
        }
    }

    public void Launch(string projectPath, string toolKey, string? sessionId = null)
    {
        if (!Directory.Exists(projectPath))
            throw new DirectoryNotFoundException($"项目目录不存在: {projectPath}");

        var arguments = BuildToolArguments(toolKey, sessionId);
        LaunchInTerminal(projectPath, arguments);
    }

    public IReadOnlyList<string> BuildToolArguments(
        string toolKey,
        string? sessionId = null)
    {
        var validatedKey = CommandSecurity.ValidateToolKey(toolKey);
        if (!_commands.TryGetValue(validatedKey, out var commandText))
            throw new InvalidOperationException($"未配置可启动的工具: {validatedKey}");
        var arguments = CommandSecurity.ParseSafeCommand(commandText).ToList();
        if (!string.IsNullOrWhiteSpace(sessionId))
        {
            arguments.Add("--resume");
            arguments.Add(CommandSecurity.ValidateSessionId(sessionId));
        }
        return arguments;
    }

    public bool IsToolAvailable(string toolKey)
    {
        try
        {
            var validatedKey = CommandSecurity.ValidateToolKey(toolKey);
            return _commands.TryGetValue(validatedKey, out var commandText) &&
                   ScannerRegistry.CommandExists(commandText, _processRunner);
        }
        catch (ArgumentException)
        {
            return false;
        }
    }

    private static AppConfig LoadConfigSafely()
    {
        try
        {
            return AppConfig.Load();
        }
        catch
        {
            return new AppConfig();
        }
    }

    private void LaunchInTerminal(string projectPath, IReadOnlyList<string> toolArguments)
    {
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            LaunchOnWindows(projectPath, toolArguments);
            return;
        }
        if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
        {
            LaunchOnMac(projectPath, toolArguments);
            return;
        }
        LaunchOnLinux(projectPath, toolArguments);
    }

    private void LaunchOnWindows(string projectPath, IReadOnlyList<string> toolArguments)
    {
        var command = CommandSecurity.BuildWindowsCommand(toolArguments);
        var wtPath = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "Microsoft", "WindowsApps", "wt.exe");

        if (File.Exists(wtPath))
        {
            var terminal = NewStartInfo(wtPath, projectPath);
            terminal.ArgumentList.Add("-d");
            terminal.ArgumentList.Add(projectPath);
            terminal.ArgumentList.Add("cmd.exe");
            terminal.ArgumentList.Add("/D");
            terminal.ArgumentList.Add("/K");
            terminal.ArgumentList.Add(command);
            _processRunner.Start(terminal);
            return;
        }

        var fallback = NewStartInfo("cmd.exe", projectPath);
        fallback.ArgumentList.Add("/D");
        fallback.ArgumentList.Add("/K");
        fallback.ArgumentList.Add(command);
        _processRunner.Start(fallback);
    }

    private void LaunchOnMac(string projectPath, IReadOnlyList<string> toolArguments)
    {
        var command = $"cd {CommandSecurity.QuotePosix(projectPath)} && exec " +
                      CommandSecurity.BuildPosixCommand(toolArguments);
        const string script = """
            on run argv
              tell application "Terminal"
                activate
                do script (item 1 of argv)
              end tell
            end run
            """;
        var startInfo = NewStartInfo("osascript", projectPath);
        startInfo.ArgumentList.Add("-e");
        startInfo.ArgumentList.Add(script);
        startInfo.ArgumentList.Add("--");
        startInfo.ArgumentList.Add(command);
        _processRunner.Start(startInfo);
    }

    private void LaunchOnLinux(string projectPath, IReadOnlyList<string> toolArguments)
    {
        var terminals = new[]
        {
            "gnome-terminal", "konsole", "xfce4-terminal", "alacritty", "kitty", "xterm"
        };
        foreach (var terminalName in terminals)
        {
            if (!ScannerRegistry.CommandExists(terminalName))
                continue;

            var startInfo = NewStartInfo(terminalName, projectPath);
            startInfo.ArgumentList.Add(terminalName == "gnome-terminal" ? "--" : "-e");
            foreach (var argument in toolArguments)
                startInfo.ArgumentList.Add(argument);
            _processRunner.Start(startInfo);
            return;
        }

        Console.WriteLine(
            "无法找到图形终端。请在项目目录中运行: " +
            CommandSecurity.BuildPosixCommand(toolArguments));
    }

    private static ProcessStartInfo NewStartInfo(string fileName, string workingDirectory)
    {
        return new ProcessStartInfo(fileName)
        {
            UseShellExecute = false,
            WorkingDirectory = workingDirectory,
        };
    }
}
