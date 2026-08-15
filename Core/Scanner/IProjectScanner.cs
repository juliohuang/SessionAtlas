namespace SessionAtlas.Core.Scanner;

/// <summary>
/// 项目扫描器接口 - 每个 AI CLI 工具一个实现
/// </summary>
public interface IProjectScanner
{
    string ToolKey { get; }
    string ToolName { get; }
    bool IsAvailable();
    ScanOutcome Scan();
}

public enum ScanStatus
{
    Succeeded,
    Unavailable,
    Failed
}

public enum ScanDiagnosticSeverity
{
    Info,
    Warning,
    Error
}

/// <summary>
/// A scanner diagnostic is intentionally structured and must not contain
/// prompt text, message bodies, credentials, or other session content.
/// </summary>
public sealed record ScanDiagnostic(
    string ToolKey,
    ScanDiagnosticSeverity Severity,
    string Code,
    string Message);

/// <summary>
/// Distinguishes a trustworthy empty snapshot from a source that could not be
/// inspected. Only Succeeded outcomes are allowed to replace stored data.
/// </summary>
public sealed class ScanOutcome
{
    private ScanOutcome(
        ScanStatus status,
        IReadOnlyList<ScannedProject> projects,
        IReadOnlyList<ScanDiagnostic> diagnostics)
    {
        Status = status;
        Projects = projects;
        Diagnostics = diagnostics;
    }

    public ScanStatus Status { get; }
    public IReadOnlyList<ScannedProject> Projects { get; }
    public IReadOnlyList<ScanDiagnostic> Diagnostics { get; }
    public bool IsSuccessful => Status == ScanStatus.Succeeded;

    public static ScanOutcome Succeeded(
        IEnumerable<ScannedProject>? projects = null,
        IEnumerable<ScanDiagnostic>? diagnostics = null) =>
        new(
            ScanStatus.Succeeded,
            projects?.ToArray() ?? [],
            diagnostics?.ToArray() ?? []);

    public static ScanOutcome Unavailable(IEnumerable<ScanDiagnostic> diagnostics) =>
        new(ScanStatus.Unavailable, [], diagnostics.ToArray());

    public static ScanOutcome Failed(IEnumerable<ScanDiagnostic> diagnostics) =>
        new(ScanStatus.Failed, [], diagnostics.ToArray());
}

/// <summary>
/// 扫描原始结果
/// </summary>
public class ScannedProject
{
    public string Path { get; set; } = string.Empty;
    public DateTime LastAccessedAt { get; set; }
    public string? SessionId { get; set; }
    public string? GitBranch { get; set; }
}
