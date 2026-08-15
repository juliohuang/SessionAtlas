using SessionAtlas.Core.Store;
using SessionAtlas.Models;
using Microsoft.Data.Sqlite;

namespace SessionAtlas.Tests;

public class SqliteSnapshotTests
{
    [Fact]
    public void RepeatingIdenticalSnapshotKeepsIdentityAndSingleUsage()
    {
        using var root = new TemporaryDirectory();
        var databasePath = root.Combine("index.db");
        var projectPath = root.Combine("project");
        var firstSeen = Utc(2026, 7, 1);
        var lastUsed = Utc(2026, 7, 30);

        using var store = new SqliteStore(databasePath);
        store.ReplaceToolSnapshots(
            [ProjectAt(projectPath, "incoming-one", firstSeen, Usage("codex", lastUsed, 2, "session-2"))],
            ["codex"]);

        var first = Assert.Single(store.ListProjects());

        store.ReplaceToolSnapshots(
            [ProjectAt(projectPath, "incoming-two", Utc(2026, 7, 31), Usage("codex", lastUsed, 2, "session-2"))],
            ["codex"]);

        var second = Assert.Single(store.ListProjects());
        Assert.Equal(first.Id, second.Id);
        Assert.Equal(firstSeen, second.FirstSeenAt);
        var usage = Assert.Single(second.ToolUsages);
        Assert.Equal(2, usage.SessionCount);
        Assert.Equal("session-2", usage.LastSessionId);
    }

    [Fact]
    public void PartialAndEmptySnapshotsReplaceOnlyScannedToolsAndRemoveOrphans()
    {
        using var root = new TemporaryDirectory();
        var databasePath = root.Combine("index.db");
        var projectPath = root.Combine("project");
        var firstSeen = Utc(2026, 7, 1);

        using var store = new SqliteStore(databasePath);
        store.ReplaceToolSnapshots(
            [
                ProjectAt(
                    projectPath,
                    "project-one",
                    firstSeen,
                    Usage("claude", Utc(2026, 7, 20), 3, "claude-3"),
                    Usage("codex", Utc(2026, 7, 21), 2, "codex-2"))
            ],
            ["claude", "codex"]);

        store.ReplaceToolSnapshots(
            [
                ProjectAt(
                    projectPath,
                    "ignored-new-id",
                    Utc(2026, 7, 31),
                    Usage("codex", Utc(2026, 7, 25), 4, "codex-4"))
            ],
            ["codex"]);

        var afterPartial = Assert.Single(store.ListProjects());
        Assert.Equal(firstSeen, afterPartial.FirstSeenAt);
        Assert.Equal(Utc(2026, 7, 25), afterPartial.LastAccessedAt);
        Assert.Equal(2, afterPartial.ToolUsages.Count);
        Assert.Equal(3, afterPartial.ToolUsages.Single(u => u.ToolKey == "claude").SessionCount);
        Assert.Equal(4, afterPartial.ToolUsages.Single(u => u.ToolKey == "codex").SessionCount);

        store.ReplaceToolSnapshots([], ["codex"]);
        var afterCodexEmpty = Assert.Single(store.ListProjects());
        Assert.Equal(Utc(2026, 7, 20), afterCodexEmpty.LastAccessedAt);
        Assert.Equal("claude", Assert.Single(afterCodexEmpty.ToolUsages).ToolKey);

        store.ReplaceToolSnapshots([], ["claude"]);
        Assert.Empty(store.ListProjects());
        Assert.Empty(store.ListProjects(search: "project"));
    }

    [Fact]
    public void SnapshotFailureRollsBackEveryProjectUsageAndFtsChange()
    {
        using var root = new TemporaryDirectory();
        var databasePath = root.Combine("index.db");
        var baselinePath = root.Combine("baseline");
        var rejectedPath = root.Combine("must-not-survive");

        using var store = new SqliteStore(databasePath);
        store.ReplaceToolSnapshots(
            [ProjectAt(baselinePath, "baseline-id", Utc(2026, 7, 1), Usage("codex", Utc(2026, 7, 20), 1, "base"))],
            ["codex"]);

        using (var connection = Open(databasePath))
        {
            using var command = connection.CreateCommand();
            command.CommandText = """
                CREATE TRIGGER reject_blocked_usage
                BEFORE INSERT ON tool_usages
                WHEN NEW.tool_key = 'blocked'
                BEGIN
                    SELECT RAISE(ABORT, 'forced snapshot failure');
                END;
                """;
            command.ExecuteNonQuery();
        }

        Assert.Throws<SqliteException>(() => store.ReplaceToolSnapshots(
            [
                ProjectAt(
                    rejectedPath,
                    "rejected-id",
                    Utc(2026, 7, 2),
                    Usage("codex", Utc(2026, 7, 30), 1, "new"),
                    Usage("blocked", Utc(2026, 7, 30), 1, "blocked"))
            ],
            ["codex", "blocked"]));

        var remaining = Assert.Single(store.ListProjects());
        Assert.Equal(Path.GetFullPath(baselinePath), remaining.Path);
        Assert.Empty(store.ListProjects(search: "must"));
        Assert.Single(store.ListProjects(search: "baseline"));
    }

    [Fact]
    public void OpeningLegacyDatabaseCollapsesDuplicateToolUsages()
    {
        using var root = new TemporaryDirectory();
        var databasePath = root.Combine("legacy.db");
        CreateLegacyDatabaseWithDuplicateUsages(databasePath);

        using var store = new SqliteStore(databasePath);

        var project = Assert.Single(store.ListProjects());
        var usage = Assert.Single(project.ToolUsages);
        Assert.Equal(5, usage.SessionCount);
        Assert.Equal(Utc(2026, 7, 30), usage.LastUsedAt);
        Assert.Equal("newest", usage.LastSessionId);
    }

    [Fact]
    public void RejectsSnapshotUsageOutsideDeclaredSuccessfulToolsWithoutMutation()
    {
        using var root = new TemporaryDirectory();
        var databasePath = root.Combine("index.db");
        var projectPath = root.Combine("project");

        using var store = new SqliteStore(databasePath);

        Assert.Throws<ArgumentException>(() => store.ReplaceToolSnapshots(
            [ProjectAt(projectPath, "id", Utc(2026, 7, 1), Usage("claude", Utc(2026, 7, 30), 1, "s1"))],
            ["codex"]));

        Assert.Empty(store.ListProjects());
    }

    [Fact]
    public void WindowsPathIdentityRemainsStableWhenPathCasingChanges()
    {
        if (!OperatingSystem.IsWindows())
            return;

        using var root = new TemporaryDirectory();
        var databasePath = root.Combine("index.db");
        var projectPath = root.Combine("MixedCaseProject");

        using var store = new SqliteStore(databasePath);
        store.ReplaceToolSnapshots(
            [ProjectAt(projectPath, "stable-id", Utc(2026, 7, 1), Usage("codex", Utc(2026, 7, 20), 1, "old"))],
            ["codex"]);
        store.ReplaceToolSnapshots(
            [ProjectAt(projectPath.ToUpperInvariant(), "new-id", Utc(2026, 7, 2), Usage("codex", Utc(2026, 7, 30), 2, "new"))],
            ["codex"]);

        var project = Assert.Single(store.ListProjects());
        Assert.Equal("stable-id", project.Id);
        Assert.Equal(2, Assert.Single(project.ToolUsages).SessionCount);
    }

    [Fact]
    public void SnapshotAcceptsUsageWithNoKnownNativeSessionId()
    {
        using var root = new TemporaryDirectory();
        var databasePath = root.Combine("index.db");

        using var store = new SqliteStore(databasePath);
        store.ReplaceToolSnapshots(
            [
                ProjectAt(
                    root.Combine("aider-project"),
                    "aider-id",
                    Utc(2026, 7, 1),
                    Usage("aider", Utc(2026, 7, 30), 0, null))
            ],
            ["aider"]);

        var usage = Assert.Single(Assert.Single(store.ListProjects()).ToolUsages);
        Assert.Equal(0, usage.SessionCount);
    }

    [Fact]
    public void ExactPathLookupIsNotLimitedByRecencyOrListWindow()
    {
        using var root = new TemporaryDirectory();
        var databasePath = root.Combine("index.db");
        var oldestPath = root.Combine("oldest-project");

        using var store = new SqliteStore(databasePath);
        store.ReplaceToolSnapshots(
            Enumerable.Range(0, 125)
                .Select(index => ProjectAt(
                    index == 0 ? oldestPath : root.Combine($"project-{index:D3}"),
                    $"project-{index:D3}",
                    Utc(2026, 7, 1),
                    Usage(
                        index == 0 ? "fixture" : "codex",
                        Utc(2026, 7, 1).AddMinutes(index),
                        1,
                        $"session-{index:D3}")))
                .ToArray(),
            ["fixture", "codex"]);

        var lookupPath = oldestPath + Path.DirectorySeparatorChar;
        if (OperatingSystem.IsWindows())
            lookupPath = lookupPath.ToUpperInvariant();

        var project = Assert.IsType<Project>(store.GetProjectByPath(lookupPath));
        Assert.Equal(Path.GetFullPath(oldestPath), project.Path);
        var usage = Assert.Single(project.ToolUsages);
        Assert.Equal("fixture", usage.ToolKey);
        Assert.Equal("session-000", usage.LastSessionId);
    }

    [Fact]
    public void SearchTreatsFtsOperatorsAndPunctuationAsLiteralSeparators()
    {
        using var root = new TemporaryDirectory();
        var databasePath = root.Combine("index.db");

        using var store = new SqliteStore(databasePath);
        store.ReplaceToolSnapshots(
            [
                ProjectAt(
                    root.Combine("alpha-beta"),
                    "alpha-beta",
                    Utc(2026, 7, 1),
                    Usage("codex", Utc(2026, 7, 30), 1, "session"))
            ],
            ["codex"]);

        Assert.Single(store.ListProjects(search: "alpha-beta"));
        Assert.Empty(store.ListProjects(search: "\" OR * -"));
    }

    private static Project ProjectAt(
        string path,
        string id,
        DateTime firstSeen,
        params ToolUsage[] usages)
    {
        return new Project
        {
            Id = id,
            Path = Path.GetFullPath(path),
            FirstSeenAt = firstSeen,
            LastAccessedAt = usages.Max(u => u.LastUsedAt),
            ToolUsages = [.. usages]
        };
    }

    private static ToolUsage Usage(
        string key,
        DateTime lastUsed,
        int sessionCount,
        string? sessionId)
    {
        return new ToolUsage
        {
            ToolKey = key,
            ToolName = key,
            LastUsedAt = lastUsed,
            SessionCount = sessionCount,
            LastSessionId = sessionId
        };
    }

    private static DateTime Utc(int year, int month, int day) =>
        new(year, month, day, 12, 0, 0, DateTimeKind.Utc);

    private static SqliteConnection Open(string databasePath)
    {
        var connection = new SqliteConnection(new SqliteConnectionStringBuilder
        {
            DataSource = databasePath,
            Pooling = false
        }.ToString());
        connection.Open();
        return connection;
    }

    private static void CreateLegacyDatabaseWithDuplicateUsages(string databasePath)
    {
        using var connection = Open(databasePath);
        using var command = connection.CreateCommand();
        command.CommandText = """
            CREATE TABLE projects (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                last_accessed_at TEXT,
                first_seen_at TEXT,
                git_branch TEXT,
                git_remote_url TEXT
            );
            CREATE TABLE tool_usages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                tool_key TEXT NOT NULL,
                last_used_at TEXT,
                session_count INTEGER DEFAULT 1,
                last_session_id TEXT
            );
            INSERT INTO projects
                (id, path, last_accessed_at, first_seen_at)
            VALUES
                ('project-id', 'C:\demo\project', '2026-07-30T12:00:00.0000000Z', '2026-07-01T12:00:00.0000000Z');
            INSERT INTO tool_usages
                (project_id, tool_name, tool_key, last_used_at, session_count, last_session_id)
            VALUES
                ('project-id', 'Codex', 'codex', '2026-07-20T12:00:00.0000000Z', 5, 'older'),
                ('project-id', 'Codex', 'codex', '2026-07-30T12:00:00.0000000Z', 2, 'newest');
            """;
        command.ExecuteNonQuery();
    }
}
