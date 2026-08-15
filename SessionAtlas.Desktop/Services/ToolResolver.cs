using SessionAtlas.Core.Launcher;

namespace SessionAtlas.Desktop.Services;

public interface IToolResolver
{
    bool IsAvailable(string toolKey);
    IReadOnlyList<string> BuildArguments(string toolKey, string? sessionId = null);
}

public sealed class CliToolResolver : IToolResolver
{
    private readonly CliLauncher _launcher;
    public CliToolResolver(CliLauncher? launcher = null) =>
        _launcher = launcher ?? new CliLauncher();
    public bool IsAvailable(string toolKey) => _launcher.IsToolAvailable(toolKey);
    public IReadOnlyList<string> BuildArguments(string toolKey, string? sessionId = null) =>
        _launcher.BuildToolArguments(toolKey, sessionId);
}
