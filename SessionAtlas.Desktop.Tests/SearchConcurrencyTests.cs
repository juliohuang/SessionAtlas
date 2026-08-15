using SessionAtlas.Desktop.Services;
using SessionAtlas.Desktop.ViewModels;
using SessionAtlas.Models;

namespace SessionAtlas.Desktop.Tests;

public class SearchConcurrencyTests
{
    [Fact]
    public async Task SearchesAreSerializedAndOnlyTheLatestGenerationPublishesOnDispatcher()
    {
        var store = new BlockingStore();
        var dispatcher = new RecordingDispatcher();
        var main = new MainWindowViewModel(
            new ProjectService(store),
            new AgentSessionManager(),
            new NoTools(),
            dispatcher);
        var offDispatcherMutation = false;
        main.Projects.CollectionChanged += (_, _) =>
            offDispatcherMutation |= !dispatcher.InContext;
        main.PropertyChanged += (_, args) =>
        {
            if (args.PropertyName == nameof(main.StatusMessage))
                offDispatcherMutation |= !dispatcher.InContext;
        };

        var first = main.SearchNowAsync("first");
        Assert.True(store.FirstStarted.Wait(TimeSpan.FromSeconds(2)));
        var second = main.SearchNowAsync("second");
        store.ReleaseFirst.Set();
        await Task.WhenAll(first, second);

        Assert.Equal(1, store.MaxInFlight);
        Assert.Equal("second", Assert.Single(main.Projects).DisplayName);
        Assert.Contains("second", main.StatusMessage, StringComparison.Ordinal);
        Assert.False(offDispatcherMutation);
        Assert.True(dispatcher.CallCount >= 1);
    }

    private sealed class RecordingDispatcher : IUiDispatcher
    {
        public bool InContext { get; private set; }
        public int CallCount { get; private set; }
        public Task InvokeAsync(Action action)
        {
            CallCount++;
            InContext = true;
            try { action(); }
            finally { InContext = false; }
            return Task.CompletedTask;
        }
    }

    private sealed class NoTools : IToolResolver
    {
        public bool IsAvailable(string toolKey) => false;
        public IReadOnlyList<string> BuildArguments(string toolKey, string? sessionId = null) =>
            throw new InvalidOperationException();
    }

    private sealed class BlockingStore : IProjectStore
    {
        private int _inFlight;
        public int MaxInFlight { get; private set; }
        public ManualResetEventSlim FirstStarted { get; } = new(false);
        public ManualResetEventSlim ReleaseFirst { get; } = new(false);

        public List<Project> ListProjects(string? search = null, string? toolKey = null, int limit = 100)
        {
            var current = Interlocked.Increment(ref _inFlight);
            MaxInFlight = Math.Max(MaxInFlight, current);
            try
            {
                if (search == "first")
                {
                    FirstStarted.Set();
                    ReleaseFirst.Wait(TimeSpan.FromSeconds(5));
                }
                if (string.IsNullOrEmpty(search)) return [];
                return
                [
                    new Project
                    {
                        Id = search,
                        Path = Path.GetFullPath(Path.Combine(Path.GetTempPath(), search)),
                        LastAccessedAt = DateTime.UtcNow,
                    }
                ];
            }
            finally
            {
                Interlocked.Decrement(ref _inFlight);
            }
        }
        public Project? GetProjectByPath(string path) => null;
        public void ReplaceToolSnapshots(IReadOnlyCollection<Project> projects, IReadOnlyCollection<string> scannedToolKeys) { }
        public void RecordSession(Session session) { }
        public List<Session> GetRecentSessions(int limit = 10) => [];
        public void Dispose() { }
    }
}
