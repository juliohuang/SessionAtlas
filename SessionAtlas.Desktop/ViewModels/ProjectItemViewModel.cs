using System.Collections.Generic;
using System.Windows.Input;
using CommunityToolkit.Mvvm.Input;
using SessionAtlas.Desktop.Services;
using SessionAtlas.Models;

namespace SessionAtlas.Desktop.ViewModels;

public partial class ProjectItemViewModel : ViewModelBase
{
    private readonly ProjectItem _item;
    private readonly ProjectService _service;
    private readonly MainWindowViewModel _mainVm;

    public string DisplayName => _item.DisplayName;
    public string Path => _item.Path;
    public string LastAccessed => _item.LastAccessed;
    public string ToolTags => _item.ToolTags;
    public string ToolIcons => _item.ToolIcons;
    public List<ToolUsage> ToolUsages => _item.ToolUsages;
    public string GitBranch => _item.Project.GitBranch ?? "";

    public ICommand OpenWithLastToolCommand { get; }

    public ProjectItemViewModel(ProjectItem item, ProjectService service, MainWindowViewModel mainVm)
    {
        _item = item;
        _service = service;
        _mainVm = mainVm;
        OpenWithLastToolCommand = new RelayCommand(() => _mainVm.OpenWithLastTool(this));
    }
}
