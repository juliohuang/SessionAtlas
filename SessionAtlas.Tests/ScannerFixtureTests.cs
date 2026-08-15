using System.Text.Json;
using SessionAtlas.Core.Scanner;

namespace SessionAtlas.Tests;

public class ScannerFixtureTests
{
    private static string FixturePath(params string[] parts)
    {
        var all = new[] { AppContext.BaseDirectory, "Fixtures" }.Concat(parts).ToArray();
        return Path.Combine(all);
    }

    [Fact]
    public void CodexFixtureMatchesNestedSessionMetaShape()
    {
        var line = File.ReadLines(FixturePath(
            "codex", "sessions", "2026", "07", "30", "rollout-demo.jsonl")).First();
        using var document = JsonDocument.Parse(line);

        Assert.Equal("session_meta", document.RootElement.GetProperty("type").GetString());
        var payload = document.RootElement.GetProperty("payload");
        Assert.Equal("session-codex-demo", payload.GetProperty("id").GetString());
        Assert.Equal("{{PROJECT_PATH}}", payload.GetProperty("cwd").GetString());
    }

    [Fact]
    public void ClaudeFixtureContainsOnlyTheFieldsNeededForDiscovery()
    {
        var line = File.ReadLines(FixturePath(
            "claude", "projects", "demo-project", "session-claude-demo.jsonl")).First();
        using var document = JsonDocument.Parse(line);

        Assert.Equal("user", document.RootElement.GetProperty("type").GetString());
        Assert.Equal("session-claude-demo", document.RootElement.GetProperty("sessionId").GetString());
        Assert.Equal("{{PROJECT_PATH}}", document.RootElement.GetProperty("cwd").GetString());
    }

    [Fact]
    public void KimiFixtureMatchesBucketSessionStateShape()
    {
        var statePath = FixturePath(
            "kimi-code", "sessions", "demo-worktree", "session-kimi-demo", "state.json");
        var json = File.ReadAllText(statePath);
        using var document = JsonDocument.Parse(json);

        Assert.Equal("session-kimi-demo", Directory.GetParent(statePath)!.Name);
        Assert.Equal("{{PROJECT_PATH}}", document.RootElement.GetProperty("workDir").GetString());
    }

    [Fact]
    public void OpenCodeFixtureDefinesCurrentProjectAndSessionTables()
    {
        var sql = File.ReadAllText(FixturePath("opencode", "opencode-schema.sql"));

        Assert.Contains("CREATE TABLE project", sql);
        Assert.Contains("CREATE TABLE session", sql);
        Assert.Contains("worktree TEXT NOT NULL", sql);
        Assert.Contains("directory TEXT NOT NULL", sql);
    }

    [Fact]
    public void FixturesDoNotContainTheCurrentUserHome()
    {
        var fixtureRoot = FixturePath();
        var realHome = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);

        foreach (var file in Directory.EnumerateFiles(fixtureRoot, "*", SearchOption.AllDirectories))
        {
            var text = File.ReadAllText(file);
            Assert.DoesNotContain(realHome, text, StringComparison.OrdinalIgnoreCase);
        }
    }

    [Fact]
    public void ScannerHomeCanBeRedirectedToATemporaryDirectory()
    {
        using var temporaryHome = new TemporaryDirectory();
        var previous = Environment.GetEnvironmentVariable("SESSIONATLAS_HOME");
        try
        {
            Environment.SetEnvironmentVariable("SESSIONATLAS_HOME", temporaryHome.Path);
            Assert.Equal(
                Path.GetFullPath(temporaryHome.Path),
                ScannerRegistry.GetHomeDirectory());
        }
        finally
        {
            Environment.SetEnvironmentVariable("SESSIONATLAS_HOME", previous);
        }
    }
}
