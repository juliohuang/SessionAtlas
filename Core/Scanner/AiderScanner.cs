namespace SessionAtlas.Core.Scanner;

/// <summary>
/// Aider has no central session database. Discover projects from the presence
/// and modification time of .aider.chat.history without reading its content.
/// </summary>
public sealed class AiderScanner : ProjectScannerBase
{
    public AiderScanner(Func<bool>? isCommandAvailable = null)
        : base(isCommandAvailable ?? (() => ScannerRegistry.CommandExists("aider")))
    {
    }

    public override string ToolKey => "aider";
    public override string ToolName => "Aider";

    public override ScanOutcome Scan()
    {
        var home = ScannerRegistry.GetHomeDirectory();
        var candidates = new[]
        {
            Path.Combine(home, "work"),
            Path.Combine(home, "projects"),
            Path.Combine(home, "dev"),
            Path.Combine(home, "src")
        };
        var roots = new List<string>();
        foreach (var candidate in candidates)
        {
            var sourceProbe = ProbeDirectory(candidate);
            if (sourceProbe == SourceProbe.Failed)
                return SourceReadFailure("an Aider search root");
            if (sourceProbe == SourceProbe.Exists)
                roots.Add(candidate);
        }

        if (roots.Count == 0)
            return MissingSource();

        var projects = new List<ScannedProject>();
        var diagnostics = new List<ScanDiagnostic>();
        var seenPaths = new HashSet<string>(
            OperatingSystem.IsWindows()
                ? StringComparer.OrdinalIgnoreCase
                : StringComparer.Ordinal);
        var options = ScannerParsing.RecursiveFileEnumeration();

        try
        {
            foreach (var root in roots.OrderBy(path => path, StringComparer.Ordinal))
            {
                foreach (var historyFile in Directory.EnumerateFiles(
                    root,
                    ".aider.chat.history",
                    options))
                {
                    var projectPath = Path.GetDirectoryName(historyFile);
                    if (projectPath is null ||
                        !seenPaths.Add(Path.GetFullPath(projectPath)))
                    {
                        continue;
                    }

                    try
                    {
                        projects.Add(new ScannedProject
                        {
                            Path = Path.GetFullPath(projectPath),
                            LastAccessedAt = File.GetLastWriteTimeUtc(historyFile),
                            SessionId = null
                        });
                    }
                    catch (Exception error) when (
                        error is IOException or
                        UnauthorizedAccessException)
                    {
                        diagnostics.Add(Diagnostic(
                            ScanDiagnosticSeverity.Warning,
                            "session_read_failed",
                            "An Aider history marker could not be inspected and was skipped."));
                    }
                }
            }
        }
        catch (Exception error) when (
            error is IOException or
            UnauthorizedAccessException)
        {
            return SourceReadFailure("the configured Aider search roots");
        }

        return ScanOutcome.Succeeded(projects, diagnostics);
    }
}
