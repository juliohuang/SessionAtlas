using System.ComponentModel;
using Spectre.Console;
using Spectre.Console.Cli;
using SessionAtlas.Core.Store;
using SessionAtlas.Models;

namespace SessionAtlas.CLI.Commands;

[Description("查看最近会话记录")]
public class RecentCommand : AsyncCommand<RecentCommand.Settings>
{
    public class Settings : CommandSettings
    {
        [CommandOption("-n|--count")]
        [Description("显示数量")]
        public int Count { get; set; } = 10;

        [CommandOption("--open")]
        [Description("交互式选择并打开")]
        public bool Open { get; set; } = false;

        public override ValidationResult Validate() => Count is >= 1 and <= 1000
            ? ValidationResult.Success()
            : ValidationResult.Error("--count 必须在 1 到 1000 之间。");
    }

    public sealed record RecentSessionChoice(Session? Session, bool IsCancel = false);

    public static string FormatChoice(RecentSessionChoice choice) => choice.IsCancel
        ? "[取消]"
        : $"{Markup.Escape(choice.Session!.ToolName)} | {Markup.Escape(choice.Session.ProjectPath)}";

    public override async Task<int> ExecuteAsync(CommandContext context, Settings settings)
    {
        using var store = new SqliteStore();
        var sessions = store.GetRecentSessions(settings.Count);

        if (sessions.Count == 0)
        {
            AnsiConsole.MarkupLine("[yellow]暂无会话记录[/]");
            return 0;
        }

        var table = new Table()
            .Border(TableBorder.Rounded)
            .AddColumn("时间")
            .AddColumn("工具")
            .AddColumn("项目路径");

        foreach (var s in sessions)
        {
            table.AddRow(
                s.StartedAt.ToString("MM-dd HH:mm"),
                $"[cyan]{Markup.Escape(s.ToolName)}[/]",
                Markup.Escape(s.ProjectPath)
            );
        }

        AnsiConsole.Write(table);

        if (settings.Open)
        {
            var choices = sessions
                .Select(session => new RecentSessionChoice(session))
                .Append(new RecentSessionChoice(null, IsCancel: true));
            var choice = AnsiConsole.Prompt(
                new SelectionPrompt<RecentSessionChoice>()
                    .Title("\n选择会话恢复:")
                    .UseConverter(FormatChoice)
                    .AddChoices(choices));

            if (!choice.IsCancel)
            {
                var selected = choice.Session!;
                var openCmd = new OpenCommand();
                return await openCmd.ExecuteAsync(context, new OpenCommand.Settings
                {
                    ProjectPath = selected.ProjectPath,
                    Tool = selected.ToolKey,
                    ResolvedSessionId = selected.SessionIdFromTool,
                });
            }
        }

        return 0;
    }
}
