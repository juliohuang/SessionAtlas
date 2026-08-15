using System.Text.Json;

namespace SessionAtlas.Core.Scanner;

/// <summary>
/// Claude Code scanner for project-bucket JSONL sessions under
/// ~/.claude/projects/.
/// </summary>
public sealed class ClaudeCodeScanner : ProjectScannerBase
{
    public ClaudeCodeScanner(Func<bool>? isCommandAvailable = null)
        : base(isCommandAvailable ?? (() => ScannerRegistry.CommandExists("claude")))
    {
    }

    public override string ToolKey => "claude";
    public override string ToolName => "Claude Code";

    public override ScanOutcome Scan()
    {
        var claudeHome = Path.Combine(
            ScannerRegistry.GetHomeDirectory(),
            ".claude");
        var projectsDir = Path.Combine(claudeHome, "projects");
        var sourceProbe = ProbeDirectory(projectsDir);
        if (sourceProbe == SourceProbe.Missing)
            return MissingSource();
        if (sourceProbe == SourceProbe.Failed)
            return SourceReadFailure("the Claude Code projects directory");

        string[] sessionFiles;
        try
        {
            sessionFiles = Directory.GetFiles(
                projectsDir,
                "*.jsonl",
                ScannerParsing.RecursiveFileEnumeration());
        }
        catch (Exception error) when (
            error is IOException or
            UnauthorizedAccessException)
        {
            return SourceReadFailure("the Claude Code projects directory");
        }

        var projects = new List<ScannedProject>();
        var diagnostics = new List<ScanDiagnostic>();
        foreach (var sessionFile in sessionFiles.OrderBy(path => path, StringComparer.Ordinal))
        {
            ParseSessionFile(sessionFile, claudeHome, projects, diagnostics);
        }

        return CompleteSessionFiles(sessionFiles.Length, projects, diagnostics);
    }

    private void ParseSessionFile(
        string sessionFile,
        string claudeHome,
        List<ScannedProject> projects,
        List<ScanDiagnostic> diagnostics)
    {
        string? projectPath = null;
        string? sessionId = null;
        string? gitBranch = null;
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
                    if (projectPath is null &&
                        root.TryGetProperty("cwd", out var cwdElement))
                    {
                        projectPath = cwdElement.GetString();
                    }
                    if (sessionId is null &&
                        root.TryGetProperty("sessionId", out var sessionElement))
                    {
                        sessionId = sessionElement.GetString();
                    }
                    if (root.TryGetProperty("gitBranch", out var branchElement))
                    {
                        gitBranch = branchElement.GetString() ?? gitBranch;
                    }
                    if (root.TryGetProperty("timestamp", out var timestampElement) &&
                        ScannerParsing.TryReadUtcTimestamp(timestampElement, out var timestamp) &&
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
                "A Claude Code session file could not be read and was skipped."));
            return;
        }

        if (malformedLines > 0)
        {
            diagnostics.Add(Diagnostic(
                ScanDiagnosticSeverity.Warning,
                "malformed_session_record",
                $"A Claude Code session contained {malformedLines} malformed record(s); valid records were retained."));
        }

        if (!ScannerParsing.TryNormalizeProjectPath(
            projectPath,
            claudeHome,
            out var normalizedPath))
        {
            diagnostics.Add(Diagnostic(
                ScanDiagnosticSeverity.Warning,
                "missing_project_path",
                "A Claude Code session did not contain a safe absolute project path and was skipped."));
            return;
        }

        sessionId ??= Path.GetFileNameWithoutExtension(sessionFile);
        if (latestActivity is null)
        {
            latestActivity = File.GetLastWriteTimeUtc(sessionFile);
            diagnostics.Add(Diagnostic(
                ScanDiagnosticSeverity.Warning,
                "timestamp_fallback",
                "A Claude Code session had no valid activity timestamp; file modification time was used."));
        }

        projects.Add(new ScannedProject
        {
            Path = normalizedPath,
            LastAccessedAt = latestActivity.Value,
            SessionId = sessionId,
            GitBranch = gitBranch
        });
    }
}
