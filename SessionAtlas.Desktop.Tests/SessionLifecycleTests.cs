using SessionAtlas.Desktop.Services;
using SessionAtlas.Desktop.ViewModels;
using SessionAtlas.Models;

namespace SessionAtlas.Desktop.Tests;

public class SessionLifecycleTests
{
    [Fact]
    public void ExactResumeFlowsToTabAndRecordsOnlyOnceAfterTerminalStarts()
    {
        var project = ProjectWithUsage("custom-key");
        var store = new FakeStore(project);
        var resolver = new FakeResolver("custom-key");
        var manager = new AgentSessionManager();
        var main = new MainWindowViewModel(new ProjectService(store), manager, resolver, new ImmediateDispatcher());
        var item = Assert.Single(main.Projects);

        main.OpenProject(item, "custom-key", "older-session");

        var tab = Assert.Single(main.AgentTabs);
        Assert.Equal("older-session", tab.ResumeSessionId);
        Assert.Equal(("custom-key", "older-session"), resolver.LastBuild);
        Assert.Empty(store.RecordedSessions);
        tab.OnTerminalStarted(42);
        tab.OnTerminalStarted(42);
        var recorded = Assert.Single(store.RecordedSessions);
        Assert.Equal("custom-key", recorded.ToolKey);
        Assert.Equal(project.Path, recorded.ProjectPath);
    }

    [Fact]
    public void NoAvailableHistoricalToolDoesNotCreateATabOrFallback()
    {
        var project = ProjectWithUsage("missing-tool");
        var main = new MainWindowViewModel(
            new ProjectService(new FakeStore(project)),
            new AgentSessionManager(),
            new FakeResolver(),
            new ImmediateDispatcher());

        main.OpenWithLastTool(Assert.Single(main.Projects));

        Assert.Empty(main.AgentTabs);
        Assert.Contains("没有可用", main.StatusMessage, StringComparison.Ordinal);
    }

    [Fact]
    public void InitialLoadPublishesOnTheCurrentUiThreadWithoutDispatcherWait()
    {
        var dispatcher = new CountingDispatcher();
        var main = new MainWindowViewModel(
            new ProjectService(new FakeStore(ProjectWithUsage("codex"))),
            new AgentSessionManager(),
            new FakeResolver("codex"),
            dispatcher);

        Assert.Single(main.Projects);
        Assert.Equal(0, dispatcher.CallCount);
    }

    [Fact]
    public async Task ExplicitCloseIsIdempotentAndCloseAllAffectsEveryRemainingSession()
    {
        var manager = new AgentSessionManager();
        var first = manager.StartSession(Path.GetTempPath(), "codex");
        var second = manager.StartSession(Path.GetTempPath(), "claude");
        var firstClose = 0;
        var secondClose = 0;
        first.CloseRequested += () => firstClose++;
        second.CloseRequested += () => secondClose++;

        manager.CloseSession(first.Id);
        manager.CloseSession(first.Id);
        Assert.Equal(1, firstClose);
        Assert.Equal(0, secondClose);
        Assert.Single(manager.Sessions);

        await manager.CloseAllAsync();
        await manager.CloseAllAsync();
        Assert.Equal(1, firstClose);
        Assert.Equal(1, secondClose);
        Assert.Empty(manager.Sessions);
    }

    private static Project ProjectWithUsage(string toolKey) => new()
    {
        Id = "project",
        Path = Path.GetFullPath(Path.Combine(Path.GetTempPath(), "desktop-project")),
        LastAccessedAt = DateTime.UtcNow,
        ToolUsages =
        [
            new ToolUsage
            {
                ToolKey = toolKey,
                ToolName = toolKey,
                LastUsedAt = DateTime.UtcNow,
                LastSessionId = "newer-session",
            }
        ],
    };

    private sealed class FakeResolver(params string[] available) : IToolResolver
    {
        private readonly HashSet<string> _available = new(available, StringComparer.OrdinalIgnoreCase);
        public (string Tool, string? Session)? LastBuild { get; private set; }
        public bool IsAvailable(string toolKey) => _available.Contains(toolKey);
        public IReadOnlyList<string> BuildArguments(string toolKey, string? sessionId = null)
        {
            LastBuild = (toolKey, sessionId);
            return sessionId is null ? [toolKey] : [toolKey, "--resume", sessionId];
        }
    }

    private sealed class ImmediateDispatcher : IUiDispatcher
    {
        public Task InvokeAsync(Action action) { action(); return Task.CompletedTask; }
    }

    private sealed class CountingDispatcher : IUiDispatcher
    {
        public int CallCount { get; private set; }
        public Task InvokeAsync(Action action)
        {
            CallCount++;
            action();
            return Task.CompletedTask;
        }
    }

    private sealed class FakeStore(Project project) : IProjectStore
    {
        public List<Session> RecordedSessions { get; } = [];
        public List<Project> ListProjects(string? search = null, string? toolKey = null, int limit = 100) => [project];
        public Project? GetProjectByPath(string path) => project;
        public void ReplaceToolSnapshots(IReadOnlyCollection<Project> projects, IReadOnlyCollection<string> scannedToolKeys) { }
        public void RecordSession(Session session) => RecordedSessions.Add(session);
        public List<Session> GetRecentSessions(int limit = 10) => [];
        public void Dispose() { }
    }
}
