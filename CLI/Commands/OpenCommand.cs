using System.ComponentModel;
using System.Diagnostics;
using Spectre.Console;
using Spectre.Console.Cli;
using SessionAtlas.Core.Config;
using SessionAtlas.Core.Launcher;
using SessionAtlas.Core.Process;
using SessionAtlas.Core.Scanner;
using SessionAtlas.Core.Store;
using SessionAtlas.Models;

namespace SessionAtlas.CLI.Commands;

[Description("打开项目并启动指定 AI CLI 工具")]
public class OpenCommand : AsyncCommand<OpenCommand.Settings>
{
    public class Settings : CommandSettings
    {
        [CommandArgument(0, "[PATH]")]
        [Description("项目路径（支持模糊匹配，留空进入交互选择）")]
        public string? ProjectPath { get; set; }

        [CommandOption("-t|--tool")]
        [Description("指定工具: claude, codex, kimi, opencode, aider")]
        public string? Tool { get; set; }

        [CommandOption("--interactive")]
        [Description("交互式选择工具")]
        public bool Interactive { get; set; } = false;

        [CommandOption("--recent")]
        [Description("直接打开最近使用的项目")]
        public bool Recent { get; set; } = false;

        /// <summary>Exact already-resolved session selected by a typed caller.</summary>
        public string? ResolvedSessionId { get; set; }
    }

    public override Task<int> ExecuteAsync(CommandContext context, Settings settings) =>
        Task.FromResult(ExecuteCore(context, settings));

    private static int ExecuteCore(CommandContext context, Settings settings)
    {
        using var store = new SqliteStore();
        var config = AppConfig.Load();
        var launcher = new CliLauncher(config: config);

        string? resolvedPath = null;

        // 模式1: --recent 直接打开最近
        if (settings.Recent)
        {
            var recentSession = store.GetRecentSessions(1).FirstOrDefault();
            if (recentSession == null)
            {
                AnsiConsole.MarkupLine("[red]没有最近会话记录[/]");
                return 1;
            }
            resolvedPath = recentSession.ProjectPath;
            settings.Tool ??= recentSession.ToolKey;
        }
        // 模式2: 通过路径模糊匹配
        else if (!string.IsNullOrEmpty(settings.ProjectPath))
        {
            if (Directory.Exists(settings.ProjectPath))
            {
                resolvedPath = System.IO.Path.GetFullPath(settings.ProjectPath);
            }
            else
            {
                // 模糊搜索
                var matches = store.ListProjects(search: settings.ProjectPath, limit: 10);
                if (matches.Count == 1)
                {
                    resolvedPath = matches[0].Path;
                }
                else if (matches.Count > 1)
                {
                    var choice = AnsiConsole.Prompt(
                        new SelectionPrompt<Project>()
                            .Title("找到多个匹配项目，请选择:")
                            .UseConverter(project =>
                                $"{Markup.Escape(project.Name)} ({Markup.Escape(project.Path)})")
                            .AddChoices(matches));
                    resolvedPath = choice.Path;
                }
                else
                {
                    AnsiConsole.MarkupLine(
                        $"[red]未找到匹配路径 '{Markup.Escape(settings.ProjectPath)}' 的项目，" +
                        "请运行 'sessionatlas scan' 或检查路径[/]");
                    return 1;
                }
            }
        }
        // 模式3: 完全交互式
        else
        {
            var allProjects = store.ListProjects(limit: 100);
            if (allProjects.Count == 0)
            {
                AnsiConsole.MarkupLine("[yellow]暂无项目，请先运行 [bold]sessionatlas scan[/][/]");
                return 0;
            }
            var choice = AnsiConsole.Prompt(
                new SelectionPrompt<Project>()
                    .Title("选择项目:")
                    .PageSize(15)
                    .UseConverter(project =>
                        $"{Markup.Escape(project.Name)} [dim]({Markup.Escape(project.ToolTags)})[/] " +
                        Markup.Escape(project.Path))
                    .AddChoices(allProjects));
            resolvedPath = choice.Path;
        }

        if (string.IsNullOrEmpty(resolvedPath) || !Directory.Exists(resolvedPath))
        {
            AnsiConsole.MarkupLine("[red]无法解析项目路径[/]");
            return 1;
        }

        // 确定工具
        var toolKey = settings.Tool;
        if (string.IsNullOrEmpty(toolKey) || settings.Interactive)
        {
            var project = store.GetProjectByPath(resolvedPath);
            var availableTools = project?.ToolUsages.Select(u => u.ToolKey).ToList() ?? new List<string> { "claude", "codex", "kimi", "opencode", "aider" };

            // 检查命令是否存在
            availableTools = availableTools.Where(launcher.IsToolAvailable).ToList();

            if (availableTools.Count == 0)
            {
                AnsiConsole.MarkupLine("[red]未在 PATH 中找到任何可用的 AI CLI 工具命令[/]");
                return 1;
            }

            toolKey = AnsiConsole.Prompt(
                new SelectionPrompt<string>()
                    .Title(
                        $"选择要在 [bold]{Markup.Escape(System.IO.Path.GetFileName(resolvedPath))}[/] " +
                        "中使用的 CLI 工具:")
                    .AddChoices(availableTools.ToArray()));
        }

        try
        {
            toolKey = CommandSecurity.ValidateToolKey(toolKey);
        }
        catch (ArgumentException error)
        {
            AnsiConsole.MarkupLine($"[red]{Markup.Escape(error.Message)}[/]");
            return 1;
        }

        // 查找该项目最近一次使用该工具的会话 ID（用于 --resume 恢复）
        string? resumeSessionId = settings.ResolvedSessionId;
        if (resumeSessionId is null && settings.Recent)
        {
            // --recent 模式直接用最近会话的 ID
            resumeSessionId = store.GetRecentSessions(1).FirstOrDefault()?.SessionIdFromTool;
        }
        else
        {
            // 普通模式：取项目该工具上次的 last_session_id
            var project = store.GetProjectByPath(resolvedPath);
            resumeSessionId = project?.ToolUsages
                .FirstOrDefault(u => u.ToolKey.Equals(toolKey, StringComparison.OrdinalIgnoreCase))
                ?.LastSessionId;
        }
        if (!string.IsNullOrWhiteSpace(resumeSessionId))
        {
            try
            {
                resumeSessionId = CommandSecurity.ValidateSessionId(resumeSessionId);
            }
            catch (ArgumentException)
            {
                AnsiConsole.MarkupLine(
                    "[yellow]索引中的会话 ID 格式无效；将启动新会话。[/]");
                resumeSessionId = null;
            }
        }

        // 启动
        AnsiConsole.MarkupLine(
            $"[green]正在启动 [bold]{Markup.Escape(toolKey)}[/] 在 " +
            $"[bold]{Markup.Escape(resolvedPath)}[/]...[/]");
        if (!string.IsNullOrEmpty(resumeSessionId))
            AnsiConsole.MarkupLine(
                $"[dim]尝试恢复会话 {Markup.Escape(resumeSessionId)}" +
                "（若工具不支持 --resume 将忽略）[/]");

        try
        {
            launcher.Launch(resolvedPath, toolKey, resumeSessionId);
        }
        catch (Exception ex)
        {
            AnsiConsole.MarkupLine($"[red]启动失败: {Markup.Escape(ex.Message)}[/]");
            return 1;
        }

        // Record only after the canonical tool was resolved and the terminal
        // process accepted the launch request.
        store.RecordSession(new Session
        {
            ProjectPath = resolvedPath,
            ToolKey = toolKey,
            ToolName = toolKey
        });

        return 0;
    }
}
