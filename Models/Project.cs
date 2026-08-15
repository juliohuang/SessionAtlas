using System;
using System.Collections.Generic;
using System.Linq;

namespace SessionAtlas.Models;

/// <summary>
/// 项目实体 - 聚合跨多个 AI CLI 工具的工作记录
/// </summary>
public class Project
{
    public string Id { get; set; } = Guid.NewGuid().ToString("N");
    
    /// <summary>项目路径（绝对路径，唯一标识）</summary>
    public string Path { get; set; } = string.Empty;
    
    /// <summary>项目名称（取目录名）</summary>
    public string Name => ProjectPathSemantics.GetDisplayName(Path);
    
    /// <summary>最后访问时间</summary>
    public DateTime LastAccessedAt { get; set; }
    
    /// <summary>首次发现时间</summary>
    public DateTime FirstSeenAt { get; set; } = DateTime.UtcNow;
    
    /// <summary>Git 分支（如果存在 .git）</summary>
    public string? GitBranch { get; set; }
    
    /// <summary>Git 远程 URL</summary>
    public string? GitRemoteUrl { get; set; }
    
    /// <summary>此项目被哪些 AI CLI 工具编辑过</summary>
    public List<ToolUsage> ToolUsages { get; set; } = new();
    
    /// <summary>去重显示标签</summary>
    public string ToolTags => string.Join(", ", ToolUsages.Select(t => t.ToolName).Distinct().OrderBy(t => t));

    public override string ToString() => $"{Name} [{ToolTags}] @ {Path}";
}

/// <summary>
/// 某个 AI CLI 工具在特定项目上的使用记录
/// </summary>
public class ToolUsage
{
    public string ToolName { get; set; } = string.Empty;
    public string ToolKey { get; set; } = string.Empty;
    public DateTime LastUsedAt { get; set; }
    public int SessionCount { get; set; }
    public string? LastSessionId { get; set; }
}
