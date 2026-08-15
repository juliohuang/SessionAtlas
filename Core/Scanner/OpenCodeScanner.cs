using Microsoft.Data.Sqlite;

namespace SessionAtlas.Core.Scanner;

/// <summary>
/// OpenCode scanner for its SQLite project/session store.
/// </summary>
public sealed class OpenCodeScanner : ProjectScannerBase
{
    public OpenCodeScanner(Func<bool>? isCommandAvailable = null)
        : base(isCommandAvailable ?? (() => ScannerRegistry.CommandExists("opencode")))
    {
    }

    public override string ToolKey => "opencode";
    public override string ToolName => "OpenCode";

    public override ScanOutcome Scan()
    {
        var candidatePaths = CandidateDatabasePaths()
            .Distinct(PathComparer())
            .OrderBy(path => path, StringComparer.Ordinal)
            .ToArray();
        var databasePaths = new List<string>();
        foreach (var candidatePath in candidatePaths)
        {
            var sourceProbe = ProbeFile(candidatePath);
            if (sourceProbe == SourceProbe.Failed)
                return SourceReadFailure("an OpenCode database path");
            if (sourceProbe == SourceProbe.Exists)
                databasePaths.Add(candidatePath);
        }
        if (databasePaths.Count == 0)
            return MissingSource();

        var projects = new List<ScannedProject>();
        var diagnostics = new List<ScanDiagnostic>();
        foreach (var databasePath in databasePaths)
        {
            if (!TryReadDatabase(databasePath, projects, diagnostics))
            {
                diagnostics.Add(Diagnostic(
                    ScanDiagnosticSeverity.Error,
                    "source_read_failed",
                    "Could not safely inspect the OpenCode database; the previous index is preserved."));
                return ScanOutcome.Failed(diagnostics);
            }
        }

        return ScanOutcome.Succeeded(projects, diagnostics);
    }

    private static IEnumerable<string> CandidateDatabasePaths()
    {
        var home = ScannerRegistry.GetHomeDirectory();
        yield return Path.Combine(home, ".local", "share", "opencode", "opencode.db");
        yield return Path.Combine(home, ".opencode", "opencode.db");

        if (string.IsNullOrWhiteSpace(
            Environment.GetEnvironmentVariable("SESSIONATLAS_HOME")))
        {
            var xdgDataHome = Environment.GetEnvironmentVariable("XDG_DATA_HOME");
            if (!string.IsNullOrWhiteSpace(xdgDataHome))
                yield return Path.Combine(xdgDataHome, "opencode", "opencode.db");
        }
    }

    private bool TryReadDatabase(
        string databasePath,
        List<ScannedProject> projects,
        List<ScanDiagnostic> diagnostics)
    {
        try
        {
            var connectionString = new SqliteConnectionStringBuilder
            {
                DataSource = databasePath,
                Mode = SqliteOpenMode.ReadOnly,
                Pooling = false
            }.ToString();
            using var connection = new SqliteConnection(connectionString);
            connection.Open();

            using var command = connection.CreateCommand();
            command.CommandText = """
                SELECT
                    session.id,
                    CASE
                        WHEN TRIM(session.directory) <> '' THEN session.directory
                        ELSE project.worktree
                    END,
                    session.time_updated,
                    project.time_updated
                FROM session
                JOIN project ON project.id = session.project_id
                ORDER BY session.id
                """;
            using var reader = command.ExecuteReader();
            while (reader.Read())
            {
                var projectPath = reader.GetString(1);
                if (!ScannerParsing.TryNormalizeProjectPath(
                    projectPath,
                    Path.GetDirectoryName(databasePath)!,
                    out var normalizedPath))
                {
                    diagnostics.Add(Diagnostic(
                        ScanDiagnosticSeverity.Warning,
                        "missing_project_path",
                        "An OpenCode session did not contain a safe absolute directory and was skipped."));
                    continue;
                }

                DateTime timestamp;
                if (!ScannerParsing.TryReadUnixTimestamp(reader.GetInt64(2), out timestamp) &&
                    !ScannerParsing.TryReadUnixTimestamp(reader.GetInt64(3), out timestamp))
                {
                    timestamp = File.GetLastWriteTimeUtc(databasePath);
                    diagnostics.Add(Diagnostic(
                        ScanDiagnosticSeverity.Warning,
                        "timestamp_fallback",
                        "An OpenCode session had no valid activity timestamp; database modification time was used."));
                }

                projects.Add(new ScannedProject
                {
                    Path = normalizedPath,
                    LastAccessedAt = timestamp,
                    SessionId = reader.GetString(0)
                });
            }
            return true;
        }
        catch (Exception error) when (
            error is SqliteException or
            IOException or
            UnauthorizedAccessException or
            InvalidOperationException)
        {
            return false;
        }
    }

    private static StringComparer PathComparer() =>
        OperatingSystem.IsWindows()
            ? StringComparer.OrdinalIgnoreCase
            : StringComparer.Ordinal;
}
