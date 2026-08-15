using SessionAtlas.Desktop.Services;
using SessionAtlas.Models;

namespace SessionAtlas.Desktop.Tests;

public class ProjectServiceTests
{
    [Fact]
    public void ExactUsageLookupDoesNotDependOnTheRecentListWindow()
    {
        var expected = new ToolUsage { ToolKey = "codex", LastUsedAt = DateTime.UtcNow };
        var store = new FakeStore
        {
            ExactProject = new Project
            {
                Path = Path.GetFullPath(Path.Combine(Path.GetTempPath(), "Target")),
                ToolUsages = [expected],
            },
        };
        var service = new ProjectService(store);
        var lookup = store.ExactProject.Path + Path.DirectorySeparatorChar;
        if (OperatingSystem.IsWindows()) lookup = lookup.ToUpperInvariant();

        Assert.Equal("codex", service.GetLastUsedTool(lookup));
        Assert.Same(expected, Assert.Single(service.GetToolUsages(lookup)));
        Assert.Equal(2, store.ExactLookupCount);
        Assert.Equal(0, store.ListCount);
    }

    [Fact]
    public void MissingExactProjectReturnsEmptyUsageState()
    {
        var service = new ProjectService(new FakeStore());
        Assert.Null(service.GetLastUsedTool(Path.GetTempPath()));
        Assert.Empty(service.GetToolUsages(Path.GetTempPath()));
    }

    private sealed class FakeStore : IProjectStore
    {
        public Project? ExactProject { get; init; }
        public int ExactLookupCount { get; private set; }
        public int ListCount { get; private set; }
        public List<Project> ListProjects(string? search = null, string? toolKey = null, int limit = 100)
        {
            ListCount++;
            return [];
        }
        public Project? GetProjectByPath(string path)
        {
            ExactLookupCount++;
            if (ExactProject is null) return null;
            return ProjectPathSemantics.NativeComparer.Equals(
                ProjectPathSemantics.NormalizeNative(path),
                ProjectPathSemantics.NormalizeNative(ExactProject.Path))
                ? ExactProject
                : null;
        }
        public void ReplaceToolSnapshots(IReadOnlyCollection<Project> projects, IReadOnlyCollection<string> scannedToolKeys) { }
        public void RecordSession(Session session) { }
        public List<Session> GetRecentSessions(int limit = 10) => [];
        public void Dispose() { }
    }
}
