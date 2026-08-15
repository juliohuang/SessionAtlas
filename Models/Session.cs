namespace SessionAtlas.Models;

/// <summary>
/// 会话记录 - 用于 recent 列表和 resume 功能
/// </summary>
public class Session
{
    public string Id { get; set; } = Guid.NewGuid().ToString("N");
    public string ProjectPath { get; set; } = string.Empty;
    public string ToolKey { get; set; } = string.Empty;
    public string ToolName { get; set; } = string.Empty;
    public DateTime StartedAt { get; set; } = DateTime.UtcNow;
    public DateTime? EndedAt { get; set; }
    public string? SessionIdFromTool { get; set; }
}
