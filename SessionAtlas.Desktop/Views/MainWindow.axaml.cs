using Avalonia.Controls;
using Avalonia.Markup.Xaml;

namespace SessionAtlas.Desktop.Views;

public partial class MainWindow : Window
{
    private bool _closeCompleted;

    public MainWindow()
    {
        InitializeComponent();
        Closing += OnClosing;
    }

    private async void OnClosing(object? sender, WindowClosingEventArgs e)
    {
        if (_closeCompleted) return;
        e.Cancel = true;
        if (DataContext is ViewModels.MainWindowViewModel viewModel)
            await viewModel.CloseAllAsync();
        _closeCompleted = true;
        Close();
    }

    private void InitializeComponent()
    {
        AvaloniaXamlLoader.Load(this);
    }
}
