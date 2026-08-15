using System.Diagnostics;
using System.Runtime.InteropServices;

namespace SessionAtlas.Core.Process;

/// <summary>
/// Opens a directory with the platform file manager without delegating the
/// directory path to shell parsing.
/// </summary>
public static class DirectoryOpener
{
    public static void Open(string directoryPath, IProcessRunner? processRunner = null)
    {
        if (!Directory.Exists(directoryPath))
            throw new DirectoryNotFoundException($"目录不存在: {directoryPath}");

        var fullPath = Path.GetFullPath(directoryPath);
        var startInfo = new ProcessStartInfo(
            RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
                ? "explorer.exe"
                : RuntimeInformation.IsOSPlatform(OSPlatform.OSX)
                    ? "open"
                    : "xdg-open")
        {
            UseShellExecute = false
        };
        startInfo.ArgumentList.Add(fullPath);
        (processRunner ?? new SystemProcessRunner()).Start(startInfo);
    }
}
