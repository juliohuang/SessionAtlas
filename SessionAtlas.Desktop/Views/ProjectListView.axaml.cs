using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Markup.Xaml;
using SessionAtlas.Desktop.ViewModels;
using CommunityToolkit.Mvvm.Input;

namespace SessionAtlas.Desktop.Views;

public partial class ProjectListView : UserControl
{
    private MainWindowViewModel? _vm;

    public ProjectListView()
    {
        InitializeComponent();
    }

    private void InitializeComponent()
    {
        AvaloniaXamlLoader.Load(this);
        var listBox = this.FindControl<ListBox>("ProjectList");
        if (listBox != null)
        {
            listBox.DoubleTapped += OnProjectDoubleTapped;
            listBox.ContextRequested += OnContextRequested;
        }
    }

    protected override void OnDataContextChanged(EventArgs e)
    {
        base.OnDataContextChanged(e);
        _vm = DataContext as MainWindowViewModel;
    }

    private void OnProjectDoubleTapped(object? sender, TappedEventArgs e)
    {
        if (_vm?.SelectedProject == null) return;
        _vm.OpenWithLastTool(_vm.SelectedProject);
    }

    private void OnContextRequested(object? sender, ContextRequestedEventArgs e)
    {
        if (_vm?.SelectedProject == null) return;
        e.Handled = true;

        var project = _vm.SelectedProject;
        var menu = new ContextMenu();

        // 使用上次工具
        var lastToolUsage = project.ToolUsages.OrderByDescending(u => u.LastUsedAt).FirstOrDefault();
        var lastToolName = lastToolUsage?.ToolName ?? "默认";

        menu.Items.Add(new MenuItem
        {
            Header = $"使用上次工具打开 ({lastToolName})",
            Command = new RelayCommand(() => _vm.OpenWithLastTool(project))
        });

        // 使用指定工具
        var openWith = new MenuItem { Header = "使用指定工具打开" };
        var allTools = new[] { ("claude", "Claude Code"), ("codex", "Codex CLI"), ("kimi", "Kimi CLI"), ("opencode", "OpenCode"), ("aider", "Aider") };
        foreach (var (key, name) in allTools)
        {
            var k = key;
            openWith.Items.Add(new MenuItem
            {
                Header = $"{GetToolIcon(key)} {name}",
                Command = new RelayCommand(() => _vm.OpenProject(project, k))
            });
        }
        menu.Items.Add(openWith);

        // 会话历史
        var sessions = new MenuItem { Header = "历史会话" };
        var recent = project.ToolUsages.OrderByDescending(u => u.LastUsedAt).Take(5);
        foreach (var s in recent)
        {
            var sid = s.LastSessionId;
            var tkey = s.ToolKey;
            sessions.Items.Add(new MenuItem
            {
                Header = $"{s.ToolName} ({s.LastUsedAt:MM-dd HH:mm})",
                Command = new RelayCommand(() => _vm.OpenProject(project, tkey, sid)),
                IsEnabled = !string.IsNullOrEmpty(sid)
            });
        }
        if (!sessions.Items.Any())
            sessions.Items.Add(new MenuItem { Header = "无历史会话", IsEnabled = false });
        menu.Items.Add(sessions);

        menu.Items.Add(new Separator());

        menu.Items.Add(new MenuItem
        {
            Header = "在资源管理器中打开",
            Command = new RelayCommand(() => _vm.OpenInExplorerCommand.Execute(project))
        });

        menu.Open(this);
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
}
