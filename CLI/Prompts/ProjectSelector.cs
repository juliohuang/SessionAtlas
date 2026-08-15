using SessionAtlas.Models;
using Spectre.Console;

namespace SessionAtlas.CLI.Prompts;

/// <summary>
/// 项目选择器 - 交互式 TUI 选择
/// </summary>
public static class ProjectSelector
{
    public static Project? PromptSelect(List<Project> projects, string title)
    {
        if (projects.Count == 0)
            return null;

        var choices = projects.Select(p => new ProjectChoice(p)).ToList();
        var selected = AnsiConsole.Prompt(
            new SelectionPrompt<ProjectChoice>()
                .Title(title)
                .PageSize(15)
                .AddChoices(choices)
                .UseConverter(FormatChoice));

        return selected?.Project;
    }

    private static string Truncate(string value, int maxLength)
    {
        if (value.Length <= maxLength) return value;
        return "..." + value.Substring(value.Length - maxLength + 3);
    }

    public static string FormatChoice(ProjectChoice choice) =>
        $"{Markup.Escape(choice.Project.Name)} " +
        $"[dim]({Markup.Escape(choice.Project.ToolTags)})[/] " +
        Markup.Escape(Truncate(choice.Project.Path, 40));

    public sealed class ProjectChoice(Project project)
    {
        public Project Project { get; } = project;
        public override string ToString() => Project.Name;
    }
}
