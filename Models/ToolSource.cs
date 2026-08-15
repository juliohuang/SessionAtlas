namespace SessionAtlas.Models;

/// <summary>
/// AI CLI 工具来源定义
/// </summary>
public class ToolSource
{
    /// <summary>唯一标识，如 claude, codex, kimi</summary>
    public string Key { get; set; } = string.Empty;

    /// <summary>显示名称</summary>
    public string Name { get; set; } = string.Empty;

    /// <summary>CLI 命令，如 claude, codex, kimi</summary>
    public string CliCommand { get; set; } = string.Empty;

    /// <summary>数据目录（各平台路径解析后的绝对路径）</summary>
    public string DataDirectory { get; set; } = string.Empty;

    /// <summary>扫描器类型</summary>
    public string ScannerType { get; set; } = string.Empty;

    /// <summary>是否已安装（命令在 PATH 中）</summary>
    public bool IsInstalled { get; set; }

    /// <summary>是否已启用扫描</summary>
    public bool IsEnabled { get; set; } = true;

    /// <summary>打开命令模板，可用变量: {projectPath}, {sessionId}</summary>
    public string OpenCommandTemplate { get; set; } = "cd \"{projectPath}\" && {cliCommand}";
}
