using System.Text.Json;

namespace SessionAtlas.Core.Scanner;

/// <summary>
/// Codex CLI scanner for date-nested rollout JSONL files under
/// ~/.codex/sessions/YYYY/MM/DD/.
/// </summary>
public sealed class CodexScanner : ProjectScannerBase
{
    public CodexScanner(Func<bool>? isCommandAvailable = null)
        : base(isCommandAvailable ?? (() => ScannerRegistry.CommandExists("codex")))
    {
    }

    public override string ToolKey => "codex";
    public override string ToolName => "Codex CLI";

    public override ScanOutcome Scan()
    {
        var codexHome = Path.Combine(
            ScannerRegistry.GetHomeDirectory(),
            ".codex");
        var sessionsDir = Path.Combine(codexHome, "sessions");
        var sourceProbe = ProbeDirectory(sessionsDir);
        if (sourceProbe == SourceProbe.Missing)
            return MissingSource();
        if (sourceProbe == SourceProbe.Failed)
            return SourceReadFailure("the Codex sessions directory");

        string[] sessionFiles;
        try
        {
            sessionFiles = Directory.GetFiles(
                sessionsDir,
                "*.jsonl",
                ScannerParsing.RecursiveFileEnumeration());
        }
        catch (Exception error) when (
            error is IOException or
            UnauthorizedAccessException)
        {
            return SourceReadFailure("the Codex sessions directory");
        }

        var projects = new List<ScannedProject>();
        var diagnostics = new List<ScanDiagnostic>();
        foreach (var sessionFile in sessionFiles.OrderBy(path => path, StringComparer.Ordinal))
        {
            ParseSessionFile(sessionFile, codexHome, projects, diagnostics);
        }

        return CompleteSessionFiles(sessionFiles.Length, projects, diagnostics);
    }

    private void ParseSessionFile(
        string sessionFile,
        string codexHome,
        List<ScannedProject> projects,
        List<ScanDiagnostic> diagnostics)
    {
        string? projectPath = null;
        string? sessionId = null;
        DateTime? latestActivity = null;
        var malformedLines = 0;

        try
        {
            foreach (var line in File.ReadLines(sessionFile))
            {
                if (string.IsNullOrWhiteSpace(line))
                    continue;

                try
                {
                    using var document = JsonDocument.Parse(line);
                    var root = document.RootElement;
                    if (root.TryGetProperty("timestamp", out var timestampElement) &&
                        ScannerParsing.TryReadUtcTimestamp(timestampElement, out var timestamp) &&
                        (latestActivity is null || timestamp > latestActivity))
                    {
                        latestActivity = timestamp;
                    }

                    if (!root.TryGetProperty("type", out var typeElement) ||
                        typeElement.GetString() != "session_meta" ||
                        !root.TryGetProperty("payload", out var payload))
                    {
                        continue;
                    }

                    if (payload.TryGetProperty("id", out var idElement))
                        sessionId = idElement.GetString();
                    if (payload.TryGetProperty("cwd", out var cwdElement))
                        projectPath = cwdElement.GetString();
                    if (payload.TryGetProperty("timestamp", out var payloadTimestamp) &&
                        ScannerParsing.TryReadUtcTimestamp(payloadTimestamp, out timestamp) &&
                        (latestActivity is null || timestamp > latestActivity))
                    {
                        latestActivity = timestamp;
                    }
                }
                catch (JsonException)
                {
                    malformedLines++;
                }
            }
        }
        catch (Exception error) when (
            error is IOException or
            UnauthorizedAccessException)
        {
            diagnostics.Add(Diagnostic(
                ScanDiagnosticSeverity.Warning,
                "session_read_failed",
                "A Codex session file could not be read and was skipped."));
            return;
        }

        if (malformedLines > 0)
        {
            diagnostics.Add(Diagnostic(
                ScanDiagnosticSeverity.Warning,
                "malformed_session_record",
                $"A Codex session contained {malformedLines} malformed record(s); valid records were retained."));
        }

        if (!ScannerParsing.TryNormalizeProjectPath(
            projectPath,
            codexHome,
            out var normalizedPath))
        {
            diagnostics.Add(Diagnostic(
                ScanDiagnosticSeverity.Warning,
                "missing_project_path",
                "A Codex session did not contain a safe absolute project path and was skipped."));
            return;
        }

        if (string.IsNullOrWhiteSpace(sessionId))
        {
            diagnostics.Add(Diagnostic(
                ScanDiagnosticSeverity.Warning,
                "missing_session_id",
                "A Codex session did not contain a native session ID and was skipped."));
            return;
        }

        if (latestActivity is null)
        {
            latestActivity = File.GetLastWriteTimeUtc(sessionFile);
            diagnostics.Add(Diagnostic(
                ScanDiagnosticSeverity.Warning,
                "timestamp_fallback",
                "A Codex session had no valid activity timestamp; file modification time was used."));
        }

        projects.Add(new ScannedProject
        {
            Path = normalizedPath,
            LastAccessedAt = latestActivity.Value,
            SessionId = sessionId
        });
    }
}
