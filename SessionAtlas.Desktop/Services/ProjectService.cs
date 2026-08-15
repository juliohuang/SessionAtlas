using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using SessionAtlas.Core.Indexer;
using SessionAtlas.Core.Scanner;
using SessionAtlas.Core.Store;
using SessionAtlas.Models;

namespace SessionAtlas.Desktop.Services;

public interface IProjectStore : IDisposable
{
    List<Project> ListProjects(string? search = null, string? toolKey = null, int limit = 100);
    Project? GetProjectByPath(string path);
    void ReplaceToolSnapshots(IReadOnlyCollection<Project> projects, IReadOnlyCollection<string> scannedToolKeys);
    void RecordSession(Session session);
    List<Session> GetRecentSessions(int limit = 10);
}

public sealed class SqliteProjectStore : IProjectStore
{
    private readonly SqliteStore _store;
    public SqliteProjectStore(SqliteStore? store = null) => _store = store ?? new SqliteStore();
    public List<Project> ListProjects(string? search = null, string? toolKey = null, int limit = 100) =>
        _store.ListProjects(search, toolKey, limit);
    public Project? GetProjectByPath(string path) => _store.GetProjectByPath(path);
    public void ReplaceToolSnapshots(IReadOnlyCollection<Project> projects, IReadOnlyCollection<string> scannedToolKeys) =>
        _store.ReplaceToolSnapshots(projects, scannedToolKeys);
    public void RecordSession(Session session) => _store.RecordSession(session);
    public List<Session> GetRecentSessions(int limit = 10) => _store.GetRecentSessions(limit);
    public void Dispose() => _store.Dispose();
}

public class ProjectService
{
    private readonly IProjectStore _store;
    private readonly ScannerRegistry _registry;
    private readonly ProjectIndexer _indexer;
    private readonly SemaphoreSlim _storeGate = new(1, 1);

    public ProjectService(
        IProjectStore? store = null,
        ScannerRegistry? registry = null,
        ProjectIndexer? indexer = null)
    {
        _store = store ?? new SqliteProjectStore();
        _registry = registry ?? new ScannerRegistry();
        _indexer = indexer ?? new ProjectIndexer();
    }

    public async Task<List<ProjectItem>> QueryProjectsAsync(
        string? query,
        CancellationToken cancellationToken = default)
    {
        // QueryProjectsAsync is also used by the Avalonia startup path, where
        // the initial result is synchronously awaited before the first window
        // is shown. Do not capture the UI synchronization context here: doing
        // so would deadlock that startup wait while the continuation waits for
        // the blocked UI thread. UI callers marshal publication explicitly.
        await _storeGate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var projects = await Task.Run(() => string.IsNullOrWhiteSpace(query)
                ? _store.ListProjects(limit: 500)
                : _store.ListProjects(search: query, limit: 500), cancellationToken).ConfigureAwait(false);
            return projects.Select(project => new ProjectItem(project)).ToList();
        }
        finally
        {
            _storeGate.Release();
        }
    }

    public async Task ScanAsync(IProgress<string> progress)
    {
        var scanResults = new List<(IProjectScanner Scanner, List<ScannedProject> Results)>();
        foreach (var diagnostic in _registry.Diagnostics)
            progress.Report($"配置: {diagnostic.Message}");

        foreach (var scanner in _registry.All)
        {
            progress.Report($"正在扫描 {scanner.ToolName}...");
            try
            {
                var outcome = await Task.Run(() => scanner.Scan());
                foreach (var diagnostic in outcome.Diagnostics)
                    progress.Report($"{scanner.ToolName}: {diagnostic.Message}");
                if (outcome.IsSuccessful)
                    scanResults.Add((scanner, outcome.Projects.ToList()));
            }
            catch (Exception)
            {
                progress.Report($"{scanner.ToolName}: 扫描失败，已保留上一份索引");
            }
        }

        if (scanResults.Count == 0)
        {
            progress.Report("没有工具产生可信快照，索引未发生变化");
            return;
        }

        var projects = await Task.Run(() => _indexer.BuildIndex(scanResults));
        await _storeGate.WaitAsync();
        try
        {
            _store.ReplaceToolSnapshots(
                projects,
                scanResults
                    .Select(result => result.Scanner.ToolKey)
                    .Distinct(StringComparer.OrdinalIgnoreCase)
                    .ToArray());
        }
        finally
        {
            _storeGate.Release();
        }
        progress.Report($"扫描完成，共 {projects.Count} 个项目");
    }

    public string? GetLastUsedTool(string projectPath)
    {
        var project = WithStore(() => _store.GetProjectByPath(projectPath));
        if (project == null) return null;
        return project.ToolUsages.OrderByDescending(u => u.LastUsedAt).FirstOrDefault()?.ToolKey;
    }

    public List<ToolUsage> GetToolUsages(string projectPath)
    {
        var project = WithStore(() => _store.GetProjectByPath(projectPath));
        return project?.ToolUsages ?? new List<ToolUsage>();
    }

    public void RecordSession(string projectPath, string toolKey)
    {
        WithStore(() => _store.RecordSession(new Session
        {
            ProjectPath = projectPath,
            ToolKey = toolKey,
            ToolName = toolKey
        }));
    }

    public List<Session> GetRecentSessions(int limit = 10)
    {
        return WithStore(() => _store.GetRecentSessions(limit));
    }

    private T WithStore<T>(Func<T> action)
    {
        _storeGate.Wait();
        try { return action(); }
        finally { _storeGate.Release(); }
    }

    private void WithStore(Action action)
    {
        _storeGate.Wait();
        try { action(); }
        finally { _storeGate.Release(); }
    }
}

public class ProjectItem
{
    public Project Project { get; }
    public string DisplayName => Project.Name;
    public string Path => Project.Path;
    public string LastAccessed => FormatRelativeTime(Project.LastAccessedAt);
    public string ToolTags => Project.ToolTags;
    public List<ToolUsage> ToolUsages => Project.ToolUsages;

    public string ToolIcons => string.Join("  ", ToolUsages.Select(u => GetToolIcon(u.ToolKey)));

    public ProjectItem(Project project)
    {
        Project = project;
    }

    private static string GetToolIcon(string key) => key.ToLower() switch
    {
        "claude" => "🅲",
        "codex" => "🆇",
        "kimi" => "🅺",
        "opencode" => "🅾",
        "aider" => "🅰",
        _ => "❓"
    };

    private static string FormatRelativeTime(DateTime dt)
    {
        var diff = DateTime.UtcNow - dt;
        if (diff.TotalMinutes < 1) return "刚刚";
        if (diff.TotalHours < 1) return $"{(int)diff.TotalMinutes}m";
        if (diff.TotalDays < 1) return $"{(int)diff.TotalHours}h";
        if (diff.TotalDays < 7) return $"{(int)diff.TotalDays}d";
        return $"{(int)(diff.TotalDays / 7)}w";
    }
}
