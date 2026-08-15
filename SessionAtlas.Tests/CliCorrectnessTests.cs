using SessionAtlas.CLI.Commands;
using SessionAtlas.CLI.Prompts;
using SessionAtlas.Core.Config;
using SessionAtlas.Core.Launcher;
using SessionAtlas.Core.Store;
using SessionAtlas.Models;
using Microsoft.Data.Sqlite;
using Spectre.Console;

namespace SessionAtlas.Tests;

public class CliCorrectnessTests
{
    [Fact]
    public void RecentAndProjectChoicesEscapeEveryMarkupField()
    {
        var session = new Session
        {
            ToolName = "[red]tool[/]",
            ProjectPath = Path.Combine(Path.GetTempPath(), "[project]"),
        };
        var recent = RecentCommand.FormatChoice(new RecentCommand.RecentSessionChoice(session));
        Assert.Equal(
            $"{session.ToolName} | {session.ProjectPath}",
            Markup.Remove(recent));

        var project = new Project
        {
            Path = Path.Combine(Path.GetTempPath(), "[name]"),
            ToolUsages = [new ToolUsage { ToolName = "[blue]tool[/]" }],
        };
        var formatted = ProjectSelector.FormatChoice(new ProjectSelector.ProjectChoice(project));
        Assert.Contains("[name]", Markup.Remove(formatted), StringComparison.Ordinal);
        Assert.Contains("[blue]tool[/]", Markup.Remove(formatted), StringComparison.Ordinal);
    }

    [Fact]
    public void TypedRecentChoicePreservesExactOlderSessionIdentity()
    {
        var older = new Session
        {
            ProjectPath = Path.GetTempPath(),
            ToolKey = "codex",
            ToolName = "Codex",
            SessionIdFromTool = "older-session",
        };
        var newer = new Session
        {
            ProjectPath = older.ProjectPath,
            ToolKey = older.ToolKey,
            ToolName = older.ToolName,
            SessionIdFromTool = "newer-session",
        };
        var choices = new[]
        {
            new RecentCommand.RecentSessionChoice(older),
            new RecentCommand.RecentSessionChoice(newer),
        };

        var selected = choices[0].Session!;
        var settings = new OpenCommand.Settings
        {
            ProjectPath = selected.ProjectPath,
            Tool = selected.ToolKey,
            ResolvedSessionId = selected.SessionIdFromTool,
        };
        Assert.Equal("older-session", settings.ResolvedSessionId);
        Assert.Equal(
            ["codex", "--resume", "older-session"],
            new CliLauncher(config: new AppConfig())
                .BuildToolArguments(settings.Tool, settings.ResolvedSessionId));
    }

    [Theory]
    [InlineData(0, false)]
    [InlineData(-1, false)]
    [InlineData(1, true)]
    [InlineData(10000, true)]
    [InlineData(10001, false)]
    public void ProjectLimitIsValidatedByCliAndStore(int limit, bool valid)
    {
        Assert.Equal(valid, new ListCommand.Settings { Limit = limit }.Validate().Successful);
        using var temp = new TemporaryDirectory();
        using var store = new SqliteStore(temp.Combine("index.db"));
        if (valid)
            Assert.Empty(store.ListProjects(limit: limit));
        else
            Assert.Throws<ArgumentOutOfRangeException>(() => store.ListProjects(limit: limit));
    }

    [Theory]
    [InlineData(0, false)]
    [InlineData(-1, false)]
    [InlineData(1, true)]
    [InlineData(1000, true)]
    [InlineData(1001, false)]
    public void SessionCountIsValidatedByCliAndStore(int count, bool valid)
    {
        Assert.Equal(valid, new RecentCommand.Settings { Count = count }.Validate().Successful);
        using var temp = new TemporaryDirectory();
        using var store = new SqliteStore(temp.Combine("index.db"));
        if (valid)
            Assert.Empty(store.GetRecentSessions(count));
        else
            Assert.Throws<ArgumentOutOfRangeException>(() => store.GetRecentSessions(count));
    }

    [Fact]
    public void ToolFilterIsCaseInsensitiveAndUsesTheNoCaseIndex()
    {
        using var temp = new TemporaryDirectory();
        var databasePath = temp.Combine("index.db");
        using var store = new SqliteStore(databasePath);
        store.UpsertProject(new Project
        {
            Id = "project",
            Path = temp.Combine("project"),
            LastAccessedAt = DateTime.UtcNow,
            ToolUsages =
            [
                new ToolUsage
                {
                    ToolKey = "Codex",
                    ToolName = "Codex",
                    LastUsedAt = DateTime.UtcNow,
                    SessionCount = 1,
                }
            ],
        });

        var lower = Assert.Single(store.ListProjects(toolKey: "codex"));
        Assert.Equal("Codex", Assert.Single(lower.ToolUsages).ToolKey);
        Assert.Single(store.ListProjects(toolKey: "CODEX"));

        using var connection = Open(databasePath);
        using var command = connection.CreateCommand();
        command.CommandText = @"
            EXPLAIN QUERY PLAN
            SELECT project_id FROM tool_usages
            WHERE tool_key = @toolKey COLLATE NOCASE
        ";
        command.Parameters.AddWithValue("@toolKey", "codex");
        var details = new List<string>();
        using var reader = command.ExecuteReader();
        while (reader.Read())
            details.Add(reader.GetString(3));
        Assert.Contains(details, detail =>
            detail.Contains("idx_usages_tool_nocase", StringComparison.OrdinalIgnoreCase));
    }

    private static SqliteConnection Open(string path)
    {
        var connection = new SqliteConnection(new SqliteConnectionStringBuilder
        {
            DataSource = path,
            Pooling = false,
        }.ToString());
        connection.Open();
        return connection;
    }
}
