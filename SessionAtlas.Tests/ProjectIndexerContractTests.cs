using SessionAtlas.Core.Indexer;
using SessionAtlas.Core.Scanner;

namespace SessionAtlas.Tests;

public class ProjectIndexerContractTests
{
    [Fact]
    public void IndexerMergesToolsAndCountsDistinctSessionsPerProject()
    {
        using var root = new TemporaryDirectory();
        var projectPath = root.Combine("sample-project");
        Directory.CreateDirectory(projectPath);
        var first = new FakeScanner("claude", "Claude Code");
        var second = new FakeScanner("codex", "Codex CLI");
        var t1 = new DateTime(2026, 7, 29, 10, 0, 0, DateTimeKind.Utc);
        var t2 = new DateTime(2026, 7, 30, 10, 0, 0, DateTimeKind.Utc);

        var result = new ProjectIndexer().BuildIndex(new()
        {
            (first, new()
            {
                new() { Path = projectPath, LastAccessedAt = t1, SessionId = "claude-1" },
                new() { Path = projectPath, LastAccessedAt = t2, SessionId = "claude-2" },
            }),
            (second, new()
            {
                new() { Path = projectPath, LastAccessedAt = t1, SessionId = "codex-1" },
            }),
        });

        var project = Assert.Single(result);
        Assert.Equal(t2, project.LastAccessedAt);
        Assert.Equal(2, project.ToolUsages.Single(u => u.ToolKey == "claude").SessionCount);
        Assert.Equal(1, project.ToolUsages.Single(u => u.ToolKey == "codex").SessionCount);
    }

    [Fact]
    public void IndexerCountsDistinctNativeSessionIdsAndKeepsLatestSessionIdentity()
    {
        using var root = new TemporaryDirectory();
        var projectPath = root.Combine("sample-project");
        var scanner = new FakeScanner("codex", "Codex CLI");
        var older = new DateTime(2026, 7, 29, 10, 0, 0, DateTimeKind.Utc);
        var newer = new DateTime(2026, 7, 30, 10, 0, 0, DateTimeKind.Utc);

        var result = new ProjectIndexer().BuildIndex(new()
        {
            (scanner, new()
            {
                new() { Path = projectPath, LastAccessedAt = newer, SessionId = "latest" },
                new() { Path = projectPath, LastAccessedAt = older, SessionId = "duplicate" },
                new() { Path = projectPath, LastAccessedAt = older, SessionId = "duplicate" },
            })
        });

        var usage = Assert.Single(Assert.Single(result).ToolUsages);
        Assert.Equal(2, usage.SessionCount);
        Assert.Equal("latest", usage.LastSessionId);
        Assert.Equal(newer, usage.LastUsedAt);
    }

    [Fact]
    public void IndexerReportsZeroKnownSessionsWhenSourceHasNoNativeSessionId()
    {
        using var root = new TemporaryDirectory();
        var scanner = new FakeScanner("aider", "Aider");

        var result = new ProjectIndexer().BuildIndex(new()
        {
            (scanner, new()
            {
                new()
                {
                    Path = root.Combine("sample-project"),
                    LastAccessedAt = new DateTime(2026, 7, 30, 10, 0, 0, DateTimeKind.Utc)
                }
            })
        });

        Assert.Equal(0, Assert.Single(Assert.Single(result).ToolUsages).SessionCount);
    }

    private sealed class FakeScanner(string key, string name) : IProjectScanner
    {
        public string ToolKey => key;
        public string ToolName => name;
        public bool IsAvailable() => true;
        public ScanOutcome Scan() => ScanOutcome.Succeeded();
    }
}
