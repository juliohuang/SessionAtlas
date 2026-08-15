using SessionAtlas.Core.Scanner;
using Microsoft.Data.Sqlite;
using System.Text.Json;

namespace SessionAtlas.Tests;

public class ScannerBehaviorTests
{
    [Fact]
    public void CodexScannerReadsDateNestedSessionMetadataAndActivityTime()
    {
        using var home = new ScannerTestHome();
        var projectPath = home.CreateProject("codex-project");
        home.CopyTextFixture(
            ["codex", "sessions", "2026", "07", "30", "rollout-demo.jsonl"],
            [".codex", "sessions", "2026", "07", "30", "rollout-demo.jsonl"],
            projectPath);

        var outcome = home.Scan(new CodexScanner(() => false));

        Assert.Equal(ScanStatus.Succeeded, outcome.Status);
        var project = Assert.Single(outcome.Projects);
        Assert.Equal(projectPath, project.Path);
        Assert.Equal("session-codex-demo", project.SessionId);
        Assert.Equal(Utc(2026, 7, 30, 10, 0, 1), project.LastAccessedAt);
    }

    [Fact]
    public void ClaudeScannerReadsCurrentJsonlWithoutDecodingBucketName()
    {
        using var home = new ScannerTestHome();
        var projectPath = home.CreateProject("claude-project");
        home.CopyTextFixture(
            ["claude", "projects", "demo-project", "session-claude-demo.jsonl"],
            [".claude", "projects", "lossy-bucket-name", "session-claude-demo.jsonl"],
            projectPath);

        var outcome = home.Scan(new ClaudeCodeScanner(() => false));

        Assert.Equal(ScanStatus.Succeeded, outcome.Status);
        var project = Assert.Single(outcome.Projects);
        Assert.Equal(projectPath, project.Path);
        Assert.Equal("session-claude-demo", project.SessionId);
        Assert.Equal("main", project.GitBranch);
        Assert.Equal(Utc(2026, 7, 30, 9, 0, 0), project.LastAccessedAt);
    }

    [Fact]
    public void KimiScannerReadsKimiCodeStateAndUsesDocumentedTimestampFallback()
    {
        using var home = new ScannerTestHome();
        var projectPath = home.CreateProject("kimi-project");
        var statePath = home.CopyTextFixture(
            ["kimi-code", "sessions", "demo-worktree", "session-kimi-demo", "state.json"],
            [".kimi-code", "sessions", "demo-worktree", "session-kimi-demo", "state.json"],
            projectPath);
        var fallbackTime = Utc(2026, 7, 30, 11, 0, 0);
        File.SetLastWriteTimeUtc(statePath, fallbackTime);

        var outcome = home.Scan(new KimiScanner(() => false));

        Assert.Equal(ScanStatus.Succeeded, outcome.Status);
        var project = Assert.Single(outcome.Projects);
        Assert.Equal(projectPath, project.Path);
        Assert.Equal("session-kimi-demo", project.SessionId);
        Assert.Equal(fallbackTime, project.LastAccessedAt);
        Assert.Contains(outcome.Diagnostics, diagnostic => diagnostic.Code == "timestamp_fallback");
    }

    [Fact]
    public void OpenCodeScannerReadsProjectAndSessionTablesReadOnly()
    {
        using var home = new ScannerTestHome();
        var projectPath = home.CreateProject("opencode-project");
        var databasePath = home.Combine(".local", "share", "opencode", "opencode.db");
        Directory.CreateDirectory(Path.GetDirectoryName(databasePath)!);

        using (var connection = Open(databasePath))
        {
            using var command = connection.CreateCommand();
            command.CommandText = File.ReadAllText(ScannerTestHome.Fixture(
                "opencode", "opencode-schema.sql"));
            command.ExecuteNonQuery();

            var updated = new DateTimeOffset(Utc(2026, 7, 30, 12, 0, 0)).ToUnixTimeMilliseconds();
            command.CommandText = """
                INSERT INTO project (id, worktree, name, time_created, time_updated)
                VALUES ('project-demo', @path, 'Demo', @updated, @updated);
                INSERT INTO session
                    (id, project_id, directory, title, version, time_created, time_updated)
                VALUES
                    ('session-opencode-demo', 'project-demo', @path, 'Fixture', '0.0.0', @updated, @updated);
                """;
            command.Parameters.AddWithValue("@path", projectPath);
            command.Parameters.AddWithValue("@updated", updated);
            command.ExecuteNonQuery();
        }

        var outcome = home.Scan(new OpenCodeScanner(() => false));

        Assert.Equal(ScanStatus.Succeeded, outcome.Status);
        var project = Assert.Single(outcome.Projects);
        Assert.Equal(projectPath, project.Path);
        Assert.Equal("session-opencode-demo", project.SessionId);
        Assert.Equal(Utc(2026, 7, 30, 12, 0, 0), project.LastAccessedAt);
    }

    [Fact]
    public void OpenCodeSchemaFailureIsNotReportedAsSuccessfulEmptySnapshot()
    {
        using var home = new ScannerTestHome();
        var databasePath = home.Combine(".local", "share", "opencode", "opencode.db");
        Directory.CreateDirectory(Path.GetDirectoryName(databasePath)!);
        using (var connection = Open(databasePath))
        {
            using var command = connection.CreateCommand();
            command.CommandText = "CREATE TABLE unrelated (id TEXT)";
            command.ExecuteNonQuery();
        }

        var outcome = home.Scan(new OpenCodeScanner(() => true));

        Assert.Equal(ScanStatus.Failed, outcome.Status);
        Assert.Empty(outcome.Projects);
        Assert.Contains(outcome.Diagnostics, diagnostic =>
            diagnostic.Severity == ScanDiagnosticSeverity.Error &&
            diagnostic.Code == "source_read_failed");
    }

    [Fact]
    public void AiderScannerDiscoversHistoryMarkerWithoutReadingPromptContent()
    {
        using var home = new ScannerTestHome();
        var projectPath = home.CreateProject(Path.Combine("projects", "aider-project"));
        var markerPath = home.CopyTextFixture(
            ["aider", "projects", "demo-project", ".aider.chat.history"],
            ["projects", "aider-project", ".aider.chat.history"],
            projectPath);
        var modified = Utc(2026, 7, 30, 13, 0, 0);
        File.SetLastWriteTimeUtc(markerPath, modified);

        var outcome = home.Scan(new AiderScanner(() => false));

        Assert.Equal(ScanStatus.Succeeded, outcome.Status);
        var project = Assert.Single(outcome.Projects);
        Assert.Equal(projectPath, project.Path);
        Assert.Null(project.SessionId);
        Assert.Equal(modified, project.LastAccessedAt);
    }

    [Fact]
    public void MissingSourceAndMissingExecutableIsUnavailableRatherThanEmptySuccess()
    {
        using var home = new ScannerTestHome();

        var outcome = home.Scan(new CodexScanner(() => false));

        Assert.Equal(ScanStatus.Unavailable, outcome.Status);
        Assert.Empty(outcome.Projects);
        Assert.Contains(outcome.Diagnostics, diagnostic => diagnostic.Code == "source_unavailable");
    }

    [Fact]
    public void MissingSourceForInstalledToolIsSuccessfulEmptySnapshot()
    {
        using var home = new ScannerTestHome();

        var outcome = home.Scan(new CodexScanner(() => true));

        Assert.Equal(ScanStatus.Succeeded, outcome.Status);
        Assert.Empty(outcome.Projects);
    }

    [Fact]
    public void ExistingButUnparseableSessionsDoNotBecomeAnEmptySnapshot()
    {
        using var home = new ScannerTestHome();
        home.WriteText(
            [".codex", "sessions", "2026", "07", "30", "broken.jsonl"],
            "{not-valid-json prompt-secret-sentinel");

        var outcome = home.Scan(new CodexScanner(() => true));

        Assert.Equal(ScanStatus.Failed, outcome.Status);
        Assert.Empty(outcome.Projects);
        Assert.Contains(outcome.Diagnostics, diagnostic =>
            diagnostic.Code == "malformed_session_record");
        Assert.Contains(outcome.Diagnostics, diagnostic =>
            diagnostic.Code == "no_valid_sessions" &&
            diagnostic.Severity == ScanDiagnosticSeverity.Error);
        Assert.All(outcome.Diagnostics, diagnostic =>
            Assert.DoesNotContain("prompt-secret-sentinel", diagnostic.Message));
    }

    [Fact]
    public void InaccessibleShapeAtSourcePathIsFailureRatherThanEmptySuccess()
    {
        using var home = new ScannerTestHome();
        home.WriteText([".codex", "sessions"], "not-a-directory");

        var outcome = home.Scan(new CodexScanner(() => true));

        Assert.Equal(ScanStatus.Failed, outcome.Status);
        Assert.Contains(outcome.Diagnostics, diagnostic =>
            diagnostic.Code == "source_read_failed");
    }

    private static SqliteConnection Open(string path)
    {
        var connection = new SqliteConnection(new SqliteConnectionStringBuilder
        {
            DataSource = path,
            Pooling = false
        }.ToString());
        connection.Open();
        return connection;
    }

    private static DateTime Utc(
        int year,
        int month,
        int day,
        int hour,
        int minute,
        int second) =>
        new(year, month, day, hour, minute, second, DateTimeKind.Utc);

    private sealed class ScannerTestHome : IDisposable
    {
        private readonly TemporaryDirectory _directory = new();
        private readonly string? _previousHome =
            Environment.GetEnvironmentVariable("SESSIONATLAS_HOME");

        public ScannerTestHome()
        {
            Environment.SetEnvironmentVariable("SESSIONATLAS_HOME", _directory.Path);
        }

        public static string Fixture(params string[] parts) =>
            Path.Combine([AppContext.BaseDirectory, "Fixtures", .. parts]);

        public string Combine(params string[] parts) =>
            Path.Combine([_directory.Path, .. parts]);

        public string CreateProject(string relativePath)
        {
            var path = Combine(relativePath);
            Directory.CreateDirectory(path);
            return Path.GetFullPath(path);
        }

        public string CopyTextFixture(
            string[] fixtureParts,
            string[] destinationParts,
            string projectPath)
        {
            var destination = Combine(destinationParts);
            Directory.CreateDirectory(Path.GetDirectoryName(destination)!);
            var escapedProjectPath = JsonSerializer.Serialize(projectPath)[1..^1];
            var content = File.ReadAllText(Fixture(fixtureParts))
                .Replace(
                    "{{PROJECT_PATH}}",
                    escapedProjectPath,
                    StringComparison.Ordinal);
            File.WriteAllText(destination, content);
            return destination;
        }

        public string WriteText(string[] destinationParts, string content)
        {
            var destination = Combine(destinationParts);
            Directory.CreateDirectory(Path.GetDirectoryName(destination)!);
            File.WriteAllText(destination, content);
            return destination;
        }

        public ScanOutcome Scan(IProjectScanner scanner) => scanner.Scan();

        public void Dispose()
        {
            Environment.SetEnvironmentVariable("SESSIONATLAS_HOME", _previousHome);
            _directory.Dispose();
        }
    }
}
