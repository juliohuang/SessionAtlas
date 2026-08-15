using System.Text.Json;
using SessionAtlas.Core.Config;
using SessionAtlas.Models;

namespace SessionAtlas.Core.Scanner;

/// <summary>
/// Generic scanner for a user-configured directory whose direct children are
/// projects or contain metadata.json pointing at the real project.
/// </summary>
public sealed class CustomToolScanner : ProjectScannerBase
{
    private readonly ToolSource _tool;

    public CustomToolScanner(
        ToolSource tool,
        Func<bool>? isCommandAvailable = null)
        : base(isCommandAvailable ?? (() =>
            ScannerRegistry.CommandExists(tool.CliCommand)))
    {
        _tool = tool;
    }

    public override string ToolKey => _tool.Key;
    public override string ToolName => _tool.Name;

    public override ScanOutcome Scan()
    {
        var dataDirectory = ResolveDataDirectory();
        if (dataDirectory is null)
            return MissingSource();
        var sourceProbe = ProbeDirectory(dataDirectory);
        if (sourceProbe == SourceProbe.Missing)
            return MissingSource();
        if (sourceProbe == SourceProbe.Failed)
            return SourceReadFailure("the configured custom-tool data directory");

        string[] directories;
        try
        {
            directories = Directory.GetDirectories(dataDirectory);
        }
        catch (Exception error) when (
            error is IOException or
            UnauthorizedAccessException)
        {
            return SourceReadFailure("the configured custom-tool data directory");
        }

        var projects = new List<ScannedProject>();
        var diagnostics = new List<ScanDiagnostic>();
        foreach (var directory in directories.OrderBy(path => path, StringComparer.Ordinal))
        {
            ParseProject(directory, projects, diagnostics);
        }

        return ScanOutcome.Succeeded(projects, diagnostics);
    }

    private string? ResolveDataDirectory()
    {
        if (string.IsNullOrWhiteSpace(_tool.DataDirectory))
            return null;

        var value = _tool.DataDirectory.Trim();
        if (value == "~" ||
            value.StartsWith($"~{Path.DirectorySeparatorChar}", StringComparison.Ordinal) ||
            value.StartsWith($"~{Path.AltDirectorySeparatorChar}", StringComparison.Ordinal))
        {
            value = Path.Combine(
                ScannerRegistry.GetHomeDirectory(),
                value[1..].TrimStart(
                    Path.DirectorySeparatorChar,
                    Path.AltDirectorySeparatorChar));
        }

        try
        {
            return Path.GetFullPath(value);
        }
        catch (Exception error) when (
            error is ArgumentException or
            NotSupportedException or
            PathTooLongException)
        {
            return null;
        }
    }

    private void ParseProject(
        string directory,
        List<ScannedProject> projects,
        List<ScanDiagnostic> diagnostics)
    {
        var projectPath = Path.GetFullPath(directory);
        var lastAccessed = Directory.GetLastWriteTimeUtc(directory);
        string? sessionId = null;
        var metadataPath = Path.Combine(directory, "metadata.json");

        if (File.Exists(metadataPath))
        {
            try
            {
                using var document = JsonDocument.Parse(File.ReadAllText(metadataPath));
                var root = document.RootElement;
                if (root.TryGetProperty("project_path", out var pathElement) ||
                    root.TryGetProperty("cwd", out pathElement))
                {
                    var configuredPath = pathElement.GetString();
                    if (!string.IsNullOrWhiteSpace(configuredPath) &&
                        Path.IsPathRooted(configuredPath))
                    {
                        projectPath = Path.GetFullPath(configuredPath);
                    }
                }
                if (root.TryGetProperty("last_accessed", out var timestampElement) &&
                    ScannerParsing.TryReadUtcTimestamp(timestampElement, out var timestamp))
                {
                    lastAccessed = timestamp;
                }
                if (root.TryGetProperty("id", out var idElement))
                    sessionId = idElement.GetString();
            }
            catch (JsonException)
            {
                diagnostics.Add(Diagnostic(
                    ScanDiagnosticSeverity.Warning,
                    "malformed_session_record",
                    "A custom-tool metadata file contained malformed JSON; directory metadata was used."));
            }
            catch (Exception error) when (
                error is IOException or
                UnauthorizedAccessException)
            {
                diagnostics.Add(Diagnostic(
                    ScanDiagnosticSeverity.Warning,
                    "session_read_failed",
                    "A custom-tool metadata file could not be read; directory metadata was used."));
            }
        }

        projects.Add(new ScannedProject
        {
            Path = projectPath,
            LastAccessedAt = lastAccessed,
            SessionId = sessionId
        });
    }
}
