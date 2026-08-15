namespace SessionAtlas.Core.Scanner;

/// <summary>
/// Shared availability and diagnostic behavior for built-in scanners.
/// Historical data remains discoverable even when the CLI executable is gone.
/// </summary>
public abstract class ProjectScannerBase : IProjectScanner
{
    protected enum SourceProbe
    {
        Exists,
        Missing,
        Failed
    }

    private readonly Func<bool> _isCommandAvailable;

    protected ProjectScannerBase(Func<bool> isCommandAvailable)
    {
        _isCommandAvailable = isCommandAvailable;
    }

    public abstract string ToolKey { get; }
    public abstract string ToolName { get; }

    public bool IsAvailable() => _isCommandAvailable();
    public abstract ScanOutcome Scan();

    protected ScanOutcome MissingSource()
    {
        if (IsAvailable())
            return ScanOutcome.Succeeded();

        return ScanOutcome.Unavailable(
        [
            Diagnostic(
                ScanDiagnosticSeverity.Info,
                "source_unavailable",
                "The CLI executable and its local session source were not found; the previous index is preserved.")
        ]);
    }

    protected ScanDiagnostic Diagnostic(
        ScanDiagnosticSeverity severity,
        string code,
        string message) =>
        new(ToolKey, severity, code, message);

    protected ScanOutcome SourceReadFailure(string sourceDescription) =>
        ScanOutcome.Failed(
        [
            Diagnostic(
                ScanDiagnosticSeverity.Error,
                "source_read_failed",
                $"Could not safely inspect {sourceDescription}; the previous index is preserved.")
        ]);

    protected ScanOutcome CompleteSessionFiles(
        int sourceFileCount,
        IReadOnlyCollection<ScannedProject> projects,
        List<ScanDiagnostic> diagnostics)
    {
        if (sourceFileCount > 0 && projects.Count == 0)
        {
            diagnostics.Add(Diagnostic(
                ScanDiagnosticSeverity.Error,
                "no_valid_sessions",
                "Session files were present but none produced a safe project record; the previous index is preserved."));
            return ScanOutcome.Failed(diagnostics);
        }

        return ScanOutcome.Succeeded(projects, diagnostics);
    }

    protected static SourceProbe ProbeDirectory(string path) =>
        ProbePath(path, FileAttributes.Directory);

    protected static SourceProbe ProbeFile(string path)
    {
        var result = ProbePath(path, FileAttributes.Normal);
        if (result != SourceProbe.Exists)
            return result;

        try
        {
            return File.GetAttributes(path).HasFlag(FileAttributes.Directory)
                ? SourceProbe.Failed
                : SourceProbe.Exists;
        }
        catch (Exception error) when (
            error is FileNotFoundException or
            DirectoryNotFoundException)
        {
            return SourceProbe.Missing;
        }
        catch (Exception error) when (
            error is IOException or
            UnauthorizedAccessException)
        {
            return SourceProbe.Failed;
        }
    }

    private static SourceProbe ProbePath(string path, FileAttributes expectedAttribute)
    {
        try
        {
            var attributes = File.GetAttributes(path);
            return expectedAttribute == FileAttributes.Directory &&
                   !attributes.HasFlag(FileAttributes.Directory)
                ? SourceProbe.Failed
                : SourceProbe.Exists;
        }
        catch (Exception error) when (
            error is FileNotFoundException or
            DirectoryNotFoundException)
        {
            return SourceProbe.Missing;
        }
        catch (Exception error) when (
            error is IOException or
            UnauthorizedAccessException)
        {
            return SourceProbe.Failed;
        }
    }
}
