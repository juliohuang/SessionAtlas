using System.ComponentModel;
using System.Text.Json;
using Spectre.Console;
using Spectre.Console.Cli;
using SessionAtlas.Core.Config;
using SessionAtlas.Core.Process;
using SessionAtlas.Core.Scanner;
using SessionAtlas.Models;

namespace SessionAtlas.CLI.Commands;

[Description("管理配置和自定义工具")]
public class ConfigCommand : AsyncCommand<ConfigCommand.Settings>
{
    public class Settings : CommandSettings
    {
        [CommandArgument(0, "[ACTION]")]
        [Description("show | add-tool | set-default-terminal")]
        public string Action { get; set; } = "show";
    }

    public override Task<int> ExecuteAsync(CommandContext context, Settings settings)
    {
        if (!AppConfig.TryLoad(out var config))
        {
            AnsiConsole.MarkupLine(
                "[red]配置文件无法读取或格式无效；为避免覆盖，未执行任何修改。[/]");
            return Task.FromResult(1);
        }

        try
        {
            return Task.FromResult(settings.Action.ToLowerInvariant() switch
            {
                "show" => ShowConfig(config),
                "add-tool" => AddCustomTool(config),
                "set-default-terminal" => SetDefaultTerminal(),
                _ => ShowUnknownAction(settings.Action),
            });
        }
        catch (ConfigConflictException)
        {
            AnsiConsole.MarkupLine("[red]配置已被其他进程更新，请重新运行命令。[/]");
        }
        catch (ConfigBusyException)
        {
            AnsiConsole.MarkupLine("[red]配置正被其他进程使用，请稍后重试。[/]");
        }
        catch (JsonException)
        {
            AnsiConsole.MarkupLine("[red]配置格式无效，未写入任何修改。[/]");
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException)
        {
            AnsiConsole.MarkupLine("[red]配置保存失败，请检查文件权限和磁盘状态。[/]");
        }
        return Task.FromResult(1);
    }

    private static int ShowConfig(AppConfig config)
    {
        AnsiConsole.MarkupLine("[bold]当前配置:[/]");
        AnsiConsole.MarkupLine($"默认终端: {Markup.Escape(config.DefaultTerminal)}");
        AnsiConsole.MarkupLine($"自定义工具数量: {config.CustomTools.Count}");
        foreach (var tool in config.CustomTools)
        {
            AnsiConsole.MarkupLine(
                $"  - {Markup.Escape(tool.Name)} ({Markup.Escape(tool.Key)}): " +
                Markup.Escape(tool.DataDirectory));
        }
        return 0;
    }

    private static int AddCustomTool(AppConfig config)
    {
        var name = AnsiConsole.Ask<string>("工具显示名称:");
        var key = AnsiConsole.Ask<string>("工具唯一标识 (如 my-custom-agent):");
        var cli = AnsiConsole.Ask<string>("CLI 命令:");
        var dir = AnsiConsole.Ask<string>("数据目录 (绝对路径，可用 ~ 表示 home):");
        try
        {
            name = CommandSecurity.ValidateDisplayLabel(name);
            key = CommandSecurity.ValidateToolKey(key);
            _ = CommandSecurity.ParseSafeCommand(cli);
        }
        catch (ArgumentException error)
        {
            AnsiConsole.MarkupLine($"[red]{Markup.Escape(error.Message)}[/]");
            return 1;
        }
        var reservedKeys = new[] { "claude", "codex", "kimi", "opencode", "aider" };
        if (reservedKeys.Contains(key, StringComparer.OrdinalIgnoreCase) ||
            config.CustomTools.Any(tool =>
                tool.Key.Equals(key, StringComparison.OrdinalIgnoreCase)))
        {
            AnsiConsole.MarkupLine(
                $"[red]工具标识 '{Markup.Escape(key)}' 已存在或属于内置工具[/]");
            return 1;
        }

        var home = ScannerRegistry.GetHomeDirectory();
        var dataDirectory = dir == "~"
            ? home
            : dir.StartsWith("~/", StringComparison.Ordinal) ||
              dir.StartsWith("~\\", StringComparison.Ordinal)
                ? Path.Combine(home, dir[2..])
                : dir;

        var newTool = new ToolSource
        {
            Key = key,
            Name = name,
            CliCommand = cli,
            DataDirectory = dataDirectory,
            IsEnabled = true
        };
        AppConfig.Update(latest =>
        {
            if (reservedKeys.Contains(key, StringComparer.OrdinalIgnoreCase) ||
                latest.CustomTools.Any(tool =>
                    tool.Key.Equals(key, StringComparison.OrdinalIgnoreCase)))
            {
                throw new ConfigConflictException(
                    "The custom tool key was added by another writer.");
            }
            latest.CustomTools.Add(newTool);
        });
        AnsiConsole.MarkupLine("[green]自定义工具已添加并保存[/]");
        return 0;
    }

    private static int SetDefaultTerminal()
    {
        var term = AnsiConsole.Prompt(
            new SelectionPrompt<string>()
                .Title("选择默认终端:")
                .AddChoices("auto", "windows-terminal", "cmd", "iterm2", "terminal", "gnome-terminal", "konsole"));
        AppConfig.Update(config => config.DefaultTerminal = term);
        AnsiConsole.MarkupLine($"[green]默认终端已设置为: {Markup.Escape(term)}[/]");
        return 0;
    }

    private static int ShowUnknownAction(string action)
    {
        AnsiConsole.MarkupLine($"[red]未知操作: {Markup.Escape(action)}[/]");
        AnsiConsole.MarkupLine("[dim]可用操作: show, add-tool, set-default-terminal[/]");
        return 1;
    }
}
