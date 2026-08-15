using SessionAtlas.Core.Store;
using SessionAtlas.Models;
using Microsoft.Data.Sqlite;

namespace SessionAtlas.Tests;

public class SqlitePathSemanticsTests
{
    [Fact]
    public void NativeRootRoundTripsThroughSnapshotLookupAndFtsRebuild()
    {
        using var temp = new TemporaryDirectory();
        using var store = new SqliteStore(temp.Combine("index.db"));
        var root = Path.GetPathRoot(temp.Path)!;
        var project = ProjectAt(root, "root-id");

        store.ReplaceToolSnapshots([project], ["codex"]);
        store.RebuildSearchIndex();
        store.RebuildSearchIndex();

        var listed = Assert.Single(store.ListProjects());
        Assert.Equal(ProjectPathSemantics.NormalizeNative(root), listed.Path);
        Assert.NotEmpty(listed.Name);
        Assert.Equal("root-id", Assert.IsType<Project>(store.GetProjectByPath(root)).Id);
        Assert.Single(store.ListProjects(search: listed.Name));
    }

    [Fact]
    public void SnapshotDuplicateValidationUsesFinalNativePathBeforeMutation()
    {
        using var temp = new TemporaryDirectory();
        using var store = new SqliteStore(temp.Combine("index.db"));
        var path = temp.Combine("duplicate");

        Assert.Throws<ArgumentException>(() => store.ReplaceToolSnapshots(
            [ProjectAt(path, "one"), ProjectAt(path + Path.DirectorySeparatorChar, "two")],
            ["codex"]));

        Assert.Empty(store.ListProjects());
    }

    [Fact]
    public void LegacyUpsertAndSessionRecordingUseCanonicalNativePaths()
    {
        using var temp = new TemporaryDirectory();
        using var store = new SqliteStore(temp.Combine("index.db"));
        var path = temp.Combine("MixedCase");
        store.UpsertProject(ProjectAt(path + Path.DirectorySeparatorChar, "stable"));
        store.UpsertProject(ProjectAt(
            OperatingSystem.IsWindows() ? path.ToUpperInvariant() : path,
            "replacement"));

        var project = Assert.Single(store.ListProjects());
        Assert.Equal("stable", project.Id);
        Assert.Equal(ProjectPathSemantics.NormalizeNative(path), project.Path);

        store.RecordSession(new Session
        {
            Id = "session",
            ProjectPath = path + Path.DirectorySeparatorChar,
            ToolKey = "codex",
            ToolName = "Codex",
        });
        Assert.Equal(
            ProjectPathSemantics.NormalizeNative(path),
            Assert.Single(store.GetRecentSessions()).ProjectPath);
    }

    [Fact]
    public void LegacyAnomalyInspectionIsReadOnlyAndReportsInvalidAndCollidingRows()
    {
        if (!OperatingSystem.IsWindows())
            return;

        using var temp = new TemporaryDirectory();
        var databasePath = temp.Combine("index.db");
        using (var store = new SqliteStore(databasePath)) { }
        using (var connection = Open(databasePath))
        {
            foreach (var row in new[]
            {
                ("invalid", "C:"),
                ("upper", @"C:\Repo"),
                ("lower", @"c:\repo\"),
            })
            {
                using var command = connection.CreateCommand();
                command.CommandText = "INSERT INTO projects (id, path) VALUES (@id, @path)";
                command.Parameters.AddWithValue("@id", row.Item1);
                command.Parameters.AddWithValue("@path", row.Item2);
                command.ExecuteNonQuery();
            }
        }

        using (var store = new SqliteStore(databasePath))
        {
            var anomalies = store.InspectProjectPathAnomalies();
            Assert.Contains(anomalies, item => item.Contains("invalid legacy path", StringComparison.Ordinal));
            Assert.Contains(anomalies, item => item.Contains("collide", StringComparison.Ordinal));
        }
        using (var connection = Open(databasePath))
        using (var command = connection.CreateCommand())
        {
            command.CommandText = "SELECT COUNT(*) FROM projects";
            Assert.Equal(3L, (long)command.ExecuteScalar()!);
        }
    }

    private static Project ProjectAt(string path, string id) => new()
    {
        Id = id,
        Path = path,
        FirstSeenAt = new DateTime(2026, 8, 1, 0, 0, 0, DateTimeKind.Utc),
        LastAccessedAt = new DateTime(2026, 8, 2, 0, 0, 0, DateTimeKind.Utc),
        ToolUsages =
        [
            new ToolUsage
            {
                ToolKey = "codex",
                ToolName = "Codex",
                LastUsedAt = new DateTime(2026, 8, 2, 0, 0, 0, DateTimeKind.Utc),
                SessionCount = 1,
                LastSessionId = "fixture",
            }
        ],
    };

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
