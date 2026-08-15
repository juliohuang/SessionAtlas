using System.Runtime.InteropServices;
using SessionAtlas.Models;
using SessionAtlas.Core.Process;

namespace SessionAtlas.Core.Scanner;

/// <summary>
/// 扫描器注册表 - 管理所有内置扫描器
/// </summary>
public class ScannerRegistry
{
    private readonly List<IProjectScanner> _scanners = new();
    private readonly List<ScanDiagnostic> _diagnostics = new();

    public ScannerRegistry()
    {
        // 注册内置扫描器
        Register(new ClaudeCodeScanner());
        Register(new KimiScanner());
        Register(new CodexScanner());
        Register(new OpenCodeScanner());
        Register(new AiderScanner());

        // 注册用户通过 `config add-tool` 配置的自定义工具扫描规则
        if (Core.Config.AppConfig.TryLoad(out var config))
        {
            foreach (var tool in config.CustomTools.Where(t => t.IsEnabled))
            {
                // 避免与内置扫描器 Key 冲突导致重复注册
                if (_scanners.Any(s => s.ToolKey.Equals(tool.Key, StringComparison.OrdinalIgnoreCase)))
                    continue;
                Register(new CustomToolScanner(tool));
            }
        }
        else
        {
            _diagnostics.Add(new ScanDiagnostic(
                "config",
                ScanDiagnosticSeverity.Warning,
                "config_read_failed",
                "The custom-tool configuration could not be read; built-in scanners remain available."));
        }
    }

    public void Register(IProjectScanner scanner) => _scanners.Add(scanner);
    /// <summary>All configured sources, including historical data for uninstalled CLIs.</summary>
    public IReadOnlyList<IProjectScanner> All => _scanners;
    public IReadOnlyList<ScanDiagnostic> Diagnostics => _diagnostics;
    /// <summary>Sources whose CLI executable can currently be launched.</summary>
    public IReadOnlyList<IProjectScanner> Launchable =>
        _scanners.Where(scanner => scanner.IsAvailable()).ToList();

    [Obsolete("Use All for discovery or Launchable for executable availability.")]
    public IReadOnlyList<IProjectScanner> Available => Launchable;

    public static string GetHomeDirectory()
    {
        // Explicit override used by isolated tests and portable installations.
        // Production behavior is unchanged when the variable is absent.
        var overrideHome = Environment.GetEnvironmentVariable("SESSIONATLAS_HOME");
        if (!string.IsNullOrWhiteSpace(overrideHome))
            return Path.GetFullPath(overrideHome);

        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
            return Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        return Environment.GetEnvironmentVariable("HOME") ?? "/";
    }

    public static bool CommandExists(string command, IProcessRunner? processRunner = null)
    {
        try
        {
            var fileName = RuntimeInformation.IsOSPlatform(OSPlatform.Windows) ? "where" : "which";
            var executable = CommandSecurity.ParseSafeCommand(command)[0];
            var psi = new System.Diagnostics.ProcessStartInfo(fileName)
            {
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false
            };
            psi.ArgumentList.Add(executable);
            var result = (processRunner ?? new SystemProcessRunner()).Run(psi);
            return result.ExitCode == 0;
        }
        catch
        {
            return false;
        }
    }
}
