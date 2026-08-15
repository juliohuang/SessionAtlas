using System.ComponentModel;
using Spectre.Console;
using Spectre.Console.Cli;
using SessionAtlas.Core.Store;

namespace SessionAtlas.CLI.Commands;

[Description("模糊搜索项目")]
public class SearchCommand : AsyncCommand<SearchCommand.Settings>
{
    public class Settings : CommandSettings
    {
        [CommandArgument(0, "<QUERY>")]
        [Description("搜索关键词")]
        public string Query { get; set; } = "";

        [CommandOption("--interactive")]
        [Description("交互式选择并打开")]
        public bool Interactive { get; set; } = true;
    }

    public override async Task<int> ExecuteAsync(CommandContext context, Settings settings)
    {
        using var store = new SqliteStore();
        var projects = store.ListProjects(search: settings.Query, limit: 50);

        if (projects.Count == 0)
        {
            AnsiConsole.MarkupLine(
                $"[yellow]未找到匹配 '{Markup.Escape(settings.Query)}' 的项目[/]");
            return 0;
        }

        AnsiConsole.MarkupLine($"[green]找到 {projects.Count} 个匹配项目:[/]\n");

        var table = new Table()
            .Border(TableBorder.Rounded)
            .AddColumn("#")
            .AddColumn("项目")
            .AddColumn("工具")
            .AddColumn("路径")
            .AddColumn("最后访问");

        for (int i = 0; i < projects.Count; i++)
        {
            var p = projects[i];
            table.AddRow(
                (i + 1).ToString(),
                EscapeMarkup(p.Name),
                EscapeMarkup(p.ToolTags),
                EscapeMarkup(p.Path),
                p.LastAccessedAt.ToString("yyyy-MM-dd HH:mm")
            );
        }

        AnsiConsole.Write(table);

        if (settings.Interactive)
        {
            var cancelIndex = projects.Count;
            var selectedIndex = AnsiConsole.Prompt(
                new SelectionPrompt<int>()
                    .Title("\n选择要打开的项目 (或取消):")
                    .UseConverter(index =>
                    {
                        if (index == cancelIndex)
                        {
                            return "[取消]";
                        }

                        var project = projects[index];
                        return $"{Markup.Escape(project.Name)} " +
                               $"[dim]({Markup.Escape(project.ToolTags)})[/]";
                    })
                    .AddChoices(Enumerable.Range(0, projects.Count + 1)));

            if (selectedIndex != cancelIndex)
            {
                var selected = projects[selectedIndex];
                var openCmd = new OpenCommand();
                return await openCmd.ExecuteAsync(context, new OpenCommand.Settings
                {
                    ProjectPath = selected.Path,
                    Interactive = true
                });
            }
        }

        return 0;
    }

    private static string EscapeMarkup(string input) => Markup.Escape(input);
}
