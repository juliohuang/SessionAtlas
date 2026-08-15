using System.Runtime.InteropServices;
using Avalonia.Controls;
using Avalonia.Interactivity;
using SessionAtlas.Desktop.ViewModels;
using SessionAtlas.Core.Launcher;
using SessionAtlas.Core.Process;
using Iciclecreek.Terminal;

namespace SessionAtlas.Desktop.Views;

/// <summary>
/// 内嵌终端会话视图：在 Tab 内通过 PTY 运行 AI CLI（claude/codex 等），
/// 类似 VSCode 的 integrated terminal，而非弹出外部终端窗口。
/// </summary>
public partial class AgentSessionView : UserControl
{
    private AgentSessionViewModel? _vm;
    private bool _launched;

    public AgentSessionView()
    {
        InitializeComponent();
        Loaded += OnLoaded;
    }

    private void OnLoaded(object? sender, RoutedEventArgs e)
    {
        // Loaded 触发时若 DataContext 已就绪则刷新 _vm
        _vm ??= DataContext as AgentSessionViewModel;
        TryLaunch();
    }

    protected override void OnDataContextChanged(EventArgs e)
    {
        base.OnDataContextChanged(e);
        _vm = DataContext as AgentSessionViewModel;
        TryLaunch();
    }

    /// <summary>
    /// 终端控件加载且 DataContext 就绪后，启动 AI CLI 进程（PTY）。
    /// </summary>
    private void TryLaunch()
    {
        if (_launched || _vm == null || Terminal == null)
            return;

        _launched = true;
        try
        {
            var (process, args) = BuildLaunchCommand(_vm.ToolKey, _vm.ResumeSessionId);
            Terminal.LaunchProcess(_vm.ProjectPath, process, args);
            // Pid 在进程刚启动后可能尚未就绪，容错读取
            int pid = 0;
            try { pid = Terminal.Pid; } catch { }
            _vm.OnTerminalStarted(pid);
            _vm.CloseRequested += OnCloseRequested;
        }
        catch (Exception ex)
        {
            _vm.OnTerminalError(ex.Message);
        }
    }

    /// <summary>
    /// 根据工具 key 构造 PTY 启动命令。
    /// Windows 上 claude/codex 等是 .cmd 脚本，需通过 cmd.exe /K 启动；Unix 直接 exec。
    /// </summary>
    internal static (string process, string[] args) BuildLaunchCommand(
        string toolKey,
        string? sessionId = null)
    {
        var toolArgs = new CliLauncher().BuildToolArguments(
            string.IsNullOrWhiteSpace(toolKey) ? "claude" : toolKey,
            sessionId);
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
            return (
                "cmd.exe",
                new[] { "/D", "/K", CommandSecurity.BuildWindowsCommand(toolArgs) });
        return (toolArgs[0], toolArgs.Skip(1).ToArray());
    }

    private void OnProcessExited(object? sender, ProcessExitedEventArgs e)
    {
        _vm?.OnTerminalExited(e.ExitCode);
    }

    private void OnCloseRequested()
    {
        try { Terminal?.Kill(); } catch { }
    }
}
