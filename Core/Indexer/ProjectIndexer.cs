using SessionAtlas.Models;
using SessionAtlas.Core.Scanner;

namespace SessionAtlas.Core.Indexer;

/// <summary>
/// 项目索引构建器 - 去重合并、路径标准化
/// </summary>
public class ProjectIndexer
{
    public List<Project> BuildIndex(List<(IProjectScanner Scanner, List<ScannedProject> Results)> scanResults)
    {
        var pathComparer = ProjectPathSemantics.NativeComparer;
        var projectMap = new Dictionary<string, Project>(pathComparer);
        var sessionIds = new Dictionary<(string Path, string Tool), HashSet<string>>(
            new ProjectToolIdentityComparer(pathComparer));

        foreach (var (scanner, results) in scanResults)
        {
            foreach (var r in results)
            {
                var normalizedPath = NormalizePath(r.Path);
                if (string.IsNullOrEmpty(normalizedPath))
                    continue;
                var lastAccessedAt = AsUtc(r.LastAccessedAt);

                if (!projectMap.TryGetValue(normalizedPath, out var project))
                {
                    project = new Project
                    {
                        Id = Guid.NewGuid().ToString("N"),
                        Path = normalizedPath,
                        LastAccessedAt = lastAccessedAt,
                        FirstSeenAt = DateTime.UtcNow,
                        GitBranch = r.GitBranch
                    };
                    projectMap[normalizedPath] = project;
                }
                else
                {
                    if (lastAccessedAt > project.LastAccessedAt)
                    {
                        project.LastAccessedAt = lastAccessedAt;
                        project.GitBranch = r.GitBranch ?? project.GitBranch;
                    }
                }

                var usageIdentity = (normalizedPath, scanner.ToolKey);
                if (!sessionIds.TryGetValue(usageIdentity, out var knownSessionIds))
                {
                    knownSessionIds = new HashSet<string>(StringComparer.Ordinal);
                    sessionIds[usageIdentity] = knownSessionIds;
                }
                if (!string.IsNullOrWhiteSpace(r.SessionId))
                    knownSessionIds.Add(r.SessionId);

                var existing = project.ToolUsages.FirstOrDefault(u =>
                    u.ToolKey.Equals(scanner.ToolKey, StringComparison.OrdinalIgnoreCase));
                if (existing == null)
                {
                    project.ToolUsages.Add(new ToolUsage
                    {
                        ToolName = scanner.ToolName,
                        ToolKey = scanner.ToolKey,
                        LastUsedAt = lastAccessedAt,
                        SessionCount = knownSessionIds.Count,
                        LastSessionId = r.SessionId
                    });
                }
                else
                {
                    existing.SessionCount = knownSessionIds.Count;
                    if (IsLaterObservation(
                        lastAccessedAt,
                        r.SessionId,
                        existing.LastUsedAt,
                        existing.LastSessionId))
                    {
                        existing.LastUsedAt = lastAccessedAt;
                        existing.LastSessionId = r.SessionId;
                    }
                }
            }
        }

        // 尝试获取 Git 分支信息
        foreach (var project in projectMap.Values)
        {
            try
            {
                var gitHead = Path.Combine(project.Path, ".git", "HEAD");
                if (File.Exists(gitHead))
                {
                    var headContent = File.ReadAllText(gitHead).Trim();
                    if (headContent.StartsWith("ref: "))
                        project.GitBranch = headContent.Substring(5).Replace("refs/heads/", "");
                    else
                        project.GitBranch = headContent.Length > 7
                            ? headContent[..7]
                            : headContent;
                }
            }
            catch (Exception error) when (
                error is IOException or
                UnauthorizedAccessException)
            {
                // Git metadata is optional and does not affect scan integrity.
            }
        }

        return projectMap.Values.OrderByDescending(p => p.LastAccessedAt).ToList();
    }

    private static string NormalizePath(string path)
    {
        if (string.IsNullOrWhiteSpace(path))
            return "";

        if (path == "~" ||
            path.StartsWith($"~{Path.DirectorySeparatorChar}", StringComparison.Ordinal) ||
            path.StartsWith($"~{Path.AltDirectorySeparatorChar}", StringComparison.Ordinal))
        {
            var home = ScannerRegistry.GetHomeDirectory();
            path = Path.Combine(
                home,
                path[1..].TrimStart(
                    Path.DirectorySeparatorChar,
                    Path.AltDirectorySeparatorChar));
        }

        if (!Path.IsPathRooted(path))
            return "";

        return ProjectPathSemantics.TryNormalizeNative(path, out var normalized)
            ? normalized
            : "";
    }

    private static DateTime AsUtc(DateTime value) =>
        value.Kind switch
        {
            DateTimeKind.Utc => value,
            DateTimeKind.Local => value.ToUniversalTime(),
            _ => DateTime.SpecifyKind(value, DateTimeKind.Utc)
        };

    private static bool IsLaterObservation(
        DateTime candidateTime,
        string? candidateSessionId,
        DateTime currentTime,
        string? currentSessionId)
    {
        var timeComparison = candidateTime.CompareTo(currentTime);
        if (timeComparison != 0)
            return timeComparison > 0;

        return string.Compare(
            candidateSessionId,
            currentSessionId,
            StringComparison.Ordinal) > 0;
    }

    private sealed class ProjectToolIdentityComparer(StringComparer pathComparer)
        : IEqualityComparer<(string Path, string Tool)>
    {
        public bool Equals(
            (string Path, string Tool) left,
            (string Path, string Tool) right) =>
            pathComparer.Equals(left.Path, right.Path) &&
            StringComparer.OrdinalIgnoreCase.Equals(left.Tool, right.Tool);

        public int GetHashCode((string Path, string Tool) value) =>
            HashCode.Combine(
                pathComparer.GetHashCode(value.Path),
                StringComparer.OrdinalIgnoreCase.GetHashCode(value.Tool));
    }
}
