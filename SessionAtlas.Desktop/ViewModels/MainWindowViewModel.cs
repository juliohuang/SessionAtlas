using System;
using System.Collections.ObjectModel;
using System.Linq;
using System.Threading.Tasks;
using System.Windows.Input;
using CommunityToolkit.Mvvm.Input;
using SessionAtlas.Core.Launcher;
using SessionAtlas.Core.Process;
using SessionAtlas.Desktop.Services;
using SessionAtlas.Models;

namespace SessionAtlas.Desktop.ViewModels;

public partial class MainWindowViewModel : ViewModelBase
{
    private readonly ProjectService _projectService;
    private readonly AgentSessionManager _sessionManager;
    private readonly IToolResolver _toolResolver;
    private readonly IUiDispatcher _dispatcher;
    private int _searchGeneration;

    public ObservableCollection<ProjectItemViewModel> Projects { get; }
    public ObservableCollection<AgentSessionViewModel> AgentTabs { get; }

    private string _searchQuery = "";
    public string SearchQuery
    {
        get => _searchQuery;
        set
        {
            if (SetProperty(ref _searchQuery, value))
            {
                _ = SearchProjectsAsync(value, Interlocked.Increment(ref _searchGeneration), true);
            }
        }
    }

    private string _statusMessage = "";
    public string StatusMessage
    {
        get => _statusMessage;
        set => SetProperty(ref _statusMessage, value);
    }

    private ProjectItemViewModel? _selectedProject;
    public ProjectItemViewModel? SelectedProject
    {
        get => _selectedProject;
        set => SetProperty(ref _selectedProject, value);
    }

    private AgentSessionViewModel? _selectedTab;
    public AgentSessionViewModel? SelectedTab
    {
        get => _selectedTab;
        set => SetProperty(ref _selectedTab, value);
    }

    public IAsyncRelayCommand ScanCommand { get; }
    public IAsyncRelayCommand SearchCommand { get; }
    public IRelayCommand<ProjectItemViewModel> OpenWithLastToolCommand { get; }
    public IRelayCommand<ProjectItemViewModel> OpenInExplorerCommand { get; }

    public MainWindowViewModel(
        ProjectService? projectService = null,
        AgentSessionManager? sessionManager = null,
        IToolResolver? toolResolver = null,
        IUiDispatcher? dispatcher = null)
    {
        _projectService = projectService ?? new ProjectService();
        _sessionManager = sessionManager ?? new AgentSessionManager();
        _toolResolver = toolResolver ?? new CliToolResolver();
        _dispatcher = dispatcher ?? new AvaloniaUiDispatcher();
        _sessionManager.SessionStarted += session =>
            _projectService.RecordSession(session.ProjectPath, session.ToolKey);

        Projects = new ObservableCollection<ProjectItemViewModel>();
        AgentTabs = new ObservableCollection<AgentSessionViewModel>();

        ScanCommand = new AsyncRelayCommand(ScanAsync);
        SearchCommand = new AsyncRelayCommand(SearchAsync);
        OpenWithLastToolCommand = new RelayCommand<ProjectItemViewModel>(OpenWithLastTool);
        OpenInExplorerCommand = new RelayCommand<ProjectItemViewModel>(OpenInExplorer);

        LoadProjects();
    }

    private void LoadProjects()
    {
        var items = _projectService.QueryProjectsAsync(null).GetAwaiter().GetResult();
        // This method runs during the Avalonia UI initialization path and is
        // also called by the scan command on the UI thread. Waiting for the
        // dispatcher here can deadlock before the first window is shown when
        // CheckAccess() is not yet established. Initial publication is already
        // on the UI thread; async searches marshal through the dispatcher below.
        PublishProjects(items, null);
    }

    private async Task ScanAsync()
    {
        StatusMessage = "正在扫描...";
        var progress = new Progress<string>(msg => StatusMessage = msg);
        await _projectService.ScanAsync(progress);
        LoadProjects();
    }

    private async Task SearchAsync()
    {
        await SearchProjectsAsync(
            _searchQuery,
            Interlocked.Increment(ref _searchGeneration),
            false);
    }

    public Task SearchNowAsync(string query) => SearchProjectsAsync(
        query,
        Interlocked.Increment(ref _searchGeneration),
        false);

    private async Task SearchProjectsAsync(string query, int generation, bool debounce)
    {
        if (debounce)
            await Task.Delay(150);
        var items = await _projectService.QueryProjectsAsync(query);
        if (generation != Volatile.Read(ref _searchGeneration)) return;
        await _dispatcher.InvokeAsync(() =>
        {
            if (generation != Volatile.Read(ref _searchGeneration)) return;
            PublishProjects(items, query);
        });
    }

    private void PublishProjects(IReadOnlyCollection<ProjectItem> items, string? query)
    {
        Projects.Clear();
        foreach (var item in items)
            Projects.Add(new ProjectItemViewModel(item, _projectService, this));
        StatusMessage = string.IsNullOrWhiteSpace(query)
            ? $"共 {Projects.Count} 个项目"
            : $"搜索 '{query}' 找到 {Projects.Count} 个项目";
    }

    public void OpenWithLastTool(ProjectItemViewModel? project)
    {
        if (project == null) return;
        var lastTool = _projectService.GetLastUsedTool(project.Path);
        if (string.IsNullOrEmpty(lastTool) || !_toolResolver.IsAvailable(lastTool))
            lastTool = GuessDefaultTool(project, _toolResolver);
        if (lastTool is null)
        {
            StatusMessage = "没有可用的 AI CLI 工具，请先安装或启用一个工具。";
            return;
        }
        OpenProject(project, lastTool);
    }

    public void OpenProject(
        ProjectItemViewModel project,
        string toolKey,
        string? sessionId = null)
    {
        string validatedTool;
        try
        {
            validatedTool = CommandSecurity.ValidateToolKey(toolKey);
            if (!_toolResolver.IsAvailable(validatedTool))
                throw new InvalidOperationException("工具命令不可用");
            _ = _toolResolver.BuildArguments(validatedTool, sessionId);
        }
        catch (Exception ex) when (ex is ArgumentException or InvalidOperationException)
        {
            StatusMessage = $"工具标识无效: {ex.Message}";
            return;
        }

        var session = _sessionManager.StartSession(project.Path, validatedTool, sessionId);

        var vm = new AgentSessionViewModel(session, _sessionManager, this);
        AgentTabs.Add(vm);
        SelectedTab = vm;
        StatusMessage = $"已启动 {session.ToolName} 在 {System.IO.Path.GetFileName(project.Path)}";
    }

    public void CloseTab(AgentSessionViewModel? tab)
    {
        if (tab == null) return;
        _sessionManager.CloseSession(tab.Id);
        AgentTabs.Remove(tab);
        SelectedTab = AgentTabs.LastOrDefault();
        StatusMessage = $"已关闭 {tab.DisplayTitle}";
    }

    public void OpenInExplorer(ProjectItemViewModel? project)
    {
        if (project == null) return;
        try
        {
            DirectoryOpener.Open(project.Path);
        }
        catch (Exception ex)
        {
            StatusMessage = $"打开目录失败: {ex.Message}";
        }
    }

    public static string? GuessDefaultTool(ProjectItemViewModel project, IToolResolver resolver)
    {
        return project.ToolUsages
            .Select(u => u.ToolKey)
            .FirstOrDefault(resolver.IsAvailable);
    }

    public Task CloseAllAsync() => _sessionManager.CloseAllAsync();
}
