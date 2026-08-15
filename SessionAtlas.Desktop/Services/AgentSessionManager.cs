using System;
using System.Collections.ObjectModel;
using System.Linq;
using SessionAtlas.Core.Process;

namespace SessionAtlas.Desktop.Services;

public class AgentSessionManager
{
    public ObservableCollection<AgentSession> Sessions { get; } = new();
    public event Action<AgentSession>? SessionStarted;

    /// <summary>
    /// 创建会话元数据。实际的 CLI 进程由 AgentSessionView 中的内嵌终端（PTY）启动与管理，
    /// 这里不再启动外部终端进程。
    /// </summary>
    public AgentSession StartSession(string projectPath, string toolKey, string? sessionId = null)
    {
        var validatedTool = CommandSecurity.ValidateToolKey(toolKey);
        var session = new AgentSession
        {
            Id = Guid.NewGuid().ToString(),
            ProjectPath = projectPath,
            ToolKey = validatedTool,
            ToolName = GetToolDisplayName(validatedTool),
            ToolIcon = GetToolIcon(validatedTool),
            ResumeSessionId = sessionId,
            StartTime = DateTime.Now,
            Status = AgentStatus.Starting
        };
        Sessions.Add(session);
        return session;
    }

    public void CloseSession(string sessionId)
    {
        var session = Sessions.FirstOrDefault(s => s.Id == sessionId);
        if (session == null) return;
        session.RequestClose();
        session.Status = AgentStatus.Closed;
        Sessions.Remove(session);
    }

    public void MarkStarted(AgentSession session, int processId)
    {
        session.ProcessId = processId;
        session.Status = AgentStatus.Running;
        if (session.StartPublished) return;
        session.StartPublished = true;
        SessionStarted?.Invoke(session);
    }

    public Task CloseAllAsync()
    {
        foreach (var session in Sessions.ToArray())
            CloseSession(session.Id);
        return Task.CompletedTask;
    }

    private static string GetToolDisplayName(string key) => key.ToLower() switch
    {
        "claude" => "Claude Code",
        "codex" => "Codex CLI",
        "kimi" => "Kimi CLI",
        "opencode" => "OpenCode",
        "aider" => "Aider",
        _ => key
    };

    private static string GetToolIcon(string key) => key.ToLower() switch
    {
        "claude" => "🅲",
        "codex" => "🆇",
        "kimi" => "🅺",
        "opencode" => "🅾",
        "aider" => "🅰",
        _ => "❓"
    };
}

public class AgentSession
{
    public string Id { get; set; } = "";
    public string ProjectPath { get; set; } = "";
    public string ToolKey { get; set; } = "";
    public string ToolName { get; set; } = "";
    public string ToolIcon { get; set; } = "";
    public DateTime StartTime { get; set; }
    public AgentStatus Status { get; set; }
    public string? ErrorMessage { get; set; }
    public int ProcessId { get; set; }
    public string? ResumeSessionId { get; set; }
    internal bool StartPublished { get; set; }
    internal bool ClosePublished { get; set; }
    public event Action? CloseRequested;
    internal void RequestClose()
    {
        if (ClosePublished) return;
        ClosePublished = true;
        CloseRequested?.Invoke();
    }
    public string DisplayTitle => $"{ToolIcon} {ToolName} - {System.IO.Path.GetFileName(ProjectPath)}";
}

public enum AgentStatus { Starting, Running, Error, Closed }
