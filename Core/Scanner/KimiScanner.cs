using System.Text.Json;

namespace SessionAtlas.Core.Scanner;

/// <summary>
/// Kimi Code scanner for
/// ~/.kimi-code/sessions/&lt;worktree-key&gt;/&lt;session-id&gt;/state.json.
/// </summary>
public sealed class KimiScanner : ProjectScannerBase
{
    public KimiScanner(Func<bool>? isCommandAvailable = null)
        : base(isCommandAvailable ?? (() => ScannerRegistry.CommandExists("kimi")))
    {
    }

    public override string ToolKey => "kimi";
    public override string ToolName => "Kimi CLI";

    public override ScanOutcome Scan()
    {
        var kimiHome = ResolveKimiHome();
        var sessionsDir = Path.Combine(kimiHome, "sessions");
        var sourceProbe = ProbeDirectory(sessionsDir);
        if (sourceProbe == SourceProbe.Missing)
            return MissingSource();
        if (sourceProbe == SourceProbe.Failed)
            return SourceReadFailure("the Kimi Code sessions directory");

        string[] stateFiles;
        try
        {
            stateFiles = Directory.GetFiles(
                sessionsDir,
                "state.json",
                ScannerParsing.RecursiveFileEnumeration());
        }
        catch (Exception error) when (
            error is IOException or
            UnauthorizedAccessException)
        {
            return SourceReadFailure("the Kimi Code sessions directory");
        }

        var projects = new List<ScannedProject>();
        var diagnostics = new List<ScanDiagnostic>();
        foreach (var stateFile in stateFiles.OrderBy(path => path, StringComparer.Ordinal))
        {
            ParseStateFile(stateFile, kimiHome, projects, diagnostics);
        }

        return CompleteSessionFiles(stateFiles.Length, projects, diagnostics);
    }

    private static string ResolveKimiHome()
    {
        var applicationHome = ScannerRegistry.GetHomeDirectory();
        if (!string.IsNullOrWhiteSpace(
            Environment.GetEnvironmentVariable("SESSIONATLAS_HOME")))
        {
            return Path.Combine(applicationHome, ".kimi-code");
        }

        var configured = Environment.GetEnvironmentVariable("KIMI_CODE_HOME");
        return string.IsNullOrWhiteSpace(configured)
            ? Path.Combine(applicationHome, ".kimi-code")
            : Path.GetFullPath(configured);
    }

    private void ParseStateFile(
        string stateFile,
        string kimiHome,
        List<ScannedProject> projects,
        List<ScanDiagnostic> diagnostics)
    {
        try
        {
            using var document = JsonDocument.Parse(File.ReadAllText(stateFile));
            var root = document.RootElement;
            var workDir = root.TryGetProperty("workDir", out var workDirElement)
                ? workDirElement.GetString()
                : null;
            if (!ScannerParsing.TryNormalizeProjectPath(
                workDir,
                kimiHome,
                out var normalizedPath))
            {
                diagnostics.Add(Diagnostic(
                    ScanDiagnosticSeverity.Warning,
                    "missing_project_path",
                    "A Kimi Code session did not contain a safe absolute workDir and was skipped."));
                return;
            }

            var timestamp = ReadTimestamp(root);
            if (timestamp is null)
            {
                timestamp = File.GetLastWriteTimeUtc(stateFile);
                diagnostics.Add(Diagnostic(
                    ScanDiagnosticSeverity.Warning,
                    "timestamp_fallback",
                    "A Kimi Code session had no valid activity timestamp; state-file modification time was used."));
            }

            projects.Add(new ScannedProject
            {
                Path = normalizedPath,
                LastAccessedAt = timestamp.Value,
                SessionId = Directory.GetParent(stateFile)?.Name
            });
        }
        catch (JsonException)
        {
            diagnostics.Add(Diagnostic(
                ScanDiagnosticSeverity.Warning,
                "malformed_session_record",
                "A Kimi Code state file contained malformed JSON and was skipped."));
        }
        catch (Exception error) when (
            error is IOException or
            UnauthorizedAccessException)
        {
            diagnostics.Add(Diagnostic(
                ScanDiagnosticSeverity.Warning,
                "session_read_failed",
                "A Kimi Code state file could not be read and was skipped."));
        }
    }

    private static DateTime? ReadTimestamp(JsonElement root)
    {
        foreach (var propertyName in new[] { "updatedAt", "lastUpdatedAt", "timestamp" })
        {
            if (root.TryGetProperty(propertyName, out var element) &&
                ScannerParsing.TryReadUtcTimestamp(element, out var timestamp))
            {
                return timestamp;
            }
        }

        return null;
    }
}
