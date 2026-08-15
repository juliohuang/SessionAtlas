using System.ComponentModel;
using Spectre.Console;
using Spectre.Console.Cli;
using SessionAtlas.Core.Store;
using SessionAtlas.CLI.Prompts;

namespace SessionAtlas.CLI.Commands;

[Description("列出已索引的所有项目")]
public class ListCommand : AsyncCommand<ListCommand.Settings>
{
    public class Settings : CommandSettings
    {
        [CommandOption("-t|--tool")]
        [Description("只显示指定工具的项目")]
        public string? ToolKey { get; set; }

        [CommandOption("-l|--limit")]
        [Description("显示数量限制")]
        public int Limit { get; set; } = 50;

        [CommandOption("--interactive")]
        [Description("交互式选择并打开")]
        public bool Interactive { get; set; } = false;

        public override ValidationResult Validate() => Limit is >= 1 and <= 10000
            ? ValidationResult.Success()
            : ValidationResult.Error("--limit 必须在 1 到 10000 之间。");
    }

    public override async Task<int> ExecuteAsync(CommandContext context, Settings settings)
    {
        using var store = new SqliteStore();
        var projects = store.ListProjects(toolKey: settings.ToolKey, limit: settings.Limit);

        if (projects.Count == 0)
        {
            AnsiConsole.MarkupLine("[yellow]暂无项目索引，请先运行 [bold]sessionatlas scan[/][/]");
            return 0;
        }

        if (settings.Interactive)
        {
            var selected = ProjectSelector.PromptSelect(projects, "选择要打开的项目:");
            if (selected != null)
            {
                var openCmd = new OpenCommand();
                return await openCmd.ExecuteAsync(context, new OpenCommand.Settings
                {
                    ProjectPath = selected.Path,
                    Interactive = true
                });
            }
            return 0;
        }

        var table = new Table()
            .Border(TableBorder.Rounded)
            .AddColumn(new TableColumn("#").Width(4).RightAligned())
            .AddColumn(new TableColumn("项目").Width(25))
            .AddColumn(new TableColumn("工具").Width(18))
            .AddColumn(new TableColumn("路径").Width(40))
            .AddColumn(new TableColumn("分支").Width(15))
            .AddColumn(new TableColumn("最后访问").Width(12));

        for (int i = 0; i < projects.Count; i++)
        {
            var p = projects[i];
            var timeStr = FormatRelativeTime(p.LastAccessedAt);
            var branch = p.GitBranch ?? "-";
            var tools = p.ToolTags;
            var name = p.Name;
            var path = Truncate(p.Path, 38);
            table.AddRow($"[dim]{i + 1}[/]", $"[bold]{EscapeMarkup(name)}[/]", $"[cyan]{EscapeMarkup(tools)}[/]", $"[dim]{EscapeMarkup(path)}[/]", $"[yellow]{EscapeMarkup(branch)}[/]", $"[dim]{EscapeMarkup(timeStr)}[/]");
        }

        AnsiConsole.Write(table);
        AnsiConsole.MarkupLine($"\n[dim]共 {projects.Count} 个项目 (使用 --interactive 交互式打开)[/]");
        return 0;
    }

    private static string EscapeMarkup(string input) => Markup.Escape(input);

    private static string Truncate(string value, int maxLength)
    {
        if (value.Length <= maxLength) return value;
        return "..." + value.Substring(value.Length - maxLength + 3);
    }

    private static string FormatRelativeTime(DateTime dt)
    {
        var diff = DateTime.UtcNow - dt;
        if (diff.TotalMinutes < 1) return "刚刚";
        if (diff.TotalHours < 1) return $"{(int)diff.TotalMinutes}m";
        if (diff.TotalDays < 1) return $"{(int)diff.TotalHours}h";
        if (diff.TotalDays < 7) return $"{(int)diff.TotalDays}d";
        return $"{(int)(diff.TotalDays / 7)}w";
    }
}
