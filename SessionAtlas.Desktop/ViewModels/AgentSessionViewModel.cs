using System;
using SessionAtlas.Core.Process;
using CommunityToolkit.Mvvm.Input;
using SessionAtlas.Desktop.Services;

namespace SessionAtlas.Desktop.ViewModels;

public partial class AgentSessionViewModel : ViewModelBase
{
    private readonly AgentSession _session;
    private readonly AgentSessionManager _manager;
    private readonly MainWindowViewModel _mainVm;

    public string Id => _session.Id;
    public string DisplayTitle => _session.DisplayTitle;
    public string ToolKey => _session.ToolKey;
    public string ToolName => _session.ToolName;
    public string ToolIcon => _session.ToolIcon;
    public string ProjectPath => _session.ProjectPath;
    public string? ResumeSessionId => _session.ResumeSessionId;
    public string StartTime => _session.StartTime.ToString("HH:mm:ss");
    public string? ErrorMessage => _session.ErrorMessage;

    public string Status => _session.Status.ToString();

    public IRelayCommand CloseCommand { get; }
    public IRelayCommand OpenDirectoryCommand { get; }

    public AgentSessionViewModel(AgentSession session, AgentSessionManager manager, MainWindowViewModel mainVm)
    {
        _session = session;
        _manager = manager;
        _mainVm = mainVm;
        CloseCommand = new RelayCommand(() => _mainVm.CloseTab(this));
        OpenDirectoryCommand = new RelayCommand(OpenDirectory);
    }

    /// <summary>内嵌终端进程已启动，更新状态与 PID。</summary>
    public void OnTerminalStarted(int pid)
    {
        _manager.MarkStarted(_session, pid);
        OnPropertyChanged(nameof(Status));
    }

    public event Action? CloseRequested
    {
        add => _session.CloseRequested += value;
        remove => _session.CloseRequested -= value;
    }

    /// <summary>内嵌终端进程退出。</summary>
    public void OnTerminalExited(int exitCode)
    {
        _session.Status = AgentStatus.Closed;
        OnPropertyChanged(nameof(Status));
        StatusMessage = exitCode == 0
            ? $"会话已结束（退出码 0）"
            : $"会话已结束（退出码 {exitCode}）";
    }

    /// <summary>内嵌终端启动失败。</summary>
    public void OnTerminalError(string message)
    {
        _session.Status = AgentStatus.Error;
        _session.ErrorMessage = message;
        OnPropertyChanged(nameof(Status));
        OnPropertyChanged(nameof(ErrorMessage));
        StatusMessage = $"启动失败: {message}";
    }

    private void OpenDirectory()
    {
        try
        {
            DirectoryOpener.Open(_session.ProjectPath);
        }
        catch (Exception ex)
        {
            StatusMessage = $"打开目录失败: {ex.Message}";
        }
    }

    public event Action<string>? StatusMessageChanged;
    private string _statusMessage = "";
    public string StatusMessage
    {
        get => _statusMessage;
        set
        {
            if (SetProperty(ref _statusMessage, value))
                StatusMessageChanged?.Invoke(value);
        }
    }
}
