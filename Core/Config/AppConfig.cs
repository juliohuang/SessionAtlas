using System.Security.Cryptography;
using System.Text.Json;
using System.Text.Json.Serialization;
using SessionAtlas.Models;
using SessionAtlas.Core.Scanner;

namespace SessionAtlas.Core.Config;

public sealed class ConfigConflictException(string message) : IOException(message);
public sealed class ConfigBusyException(string message, Exception? inner = null) : IOException(message, inner);

/// <summary>User configuration with atomic, cross-process-safe mutation.</summary>
public class AppConfig
{
    internal static Action<string, string>? BeforeAtomicReplaceForTests { get; set; }
    private static readonly JsonSerializerOptions ReadOptions = new()
    {
        PropertyNameCaseInsensitive = true,
    };
    private static readonly JsonSerializerOptions WriteOptions = new()
    {
        WriteIndented = true,
    };
    private const int DefaultLockTimeoutMilliseconds = 5000;
    private const string MissingFingerprint = "<missing>";

    [JsonIgnore]
    private string? _sourcePath;
    [JsonIgnore]
    private string? _sourceFingerprint;

    private static string GetConfigPath()
    {
        var home = ScannerRegistry.GetHomeDirectory();
        return Path.Combine(home, ".sessionatlas", "config.json");
    }

    public List<ToolSource> CustomTools { get; set; } = new();
    public Dictionary<string, string> PreferredToolsByPath { get; set; } = new();
    public string DefaultTerminal { get; set; } = "auto";

    public static AppConfig Load() => Load(GetConfigPath());

    public static AppConfig Load(string configPath)
    {
        if (!TryLoad(configPath, out var config))
            throw new JsonException("Configuration is unreadable or contains invalid JSON.");
        return config;
    }

    public static bool TryLoad(out AppConfig config) =>
        TryLoad(GetConfigPath(), out config);

    public static bool TryLoad(string configPath, out AppConfig config)
    {
        var path = NormalizeConfigPath(configPath);
        try
        {
            var bytes = ReadConfigBytes(path);
            config = bytes is null
                ? new AppConfig()
                : JsonSerializer.Deserialize<AppConfig>(bytes, ReadOptions) ?? new AppConfig();
            config.TrackSource(path, Fingerprint(bytes));
            return true;
        }
        catch (Exception error) when (
            error is IOException or
            UnauthorizedAccessException or
            JsonException)
        {
            config = new AppConfig();
            return false;
        }
    }

    public static AppConfig Update(Action<AppConfig> mutation) =>
        Update(GetConfigPath(), mutation);

    public static AppConfig Update(
        string configPath,
        Action<AppConfig> mutation,
        TimeSpan? lockTimeout = null)
    {
        ArgumentNullException.ThrowIfNull(mutation);
        var path = NormalizeConfigPath(configPath);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        using var configLock = AcquireLock(path, lockTimeout ?? TimeSpan.FromMilliseconds(DefaultLockTimeoutMilliseconds));
        CleanupStaleTemps(path, DateTime.UtcNow);
        var config = LoadLocked(path);
        mutation(config);
        var bytes = Serialize(config);
        AtomicWrite(path, bytes);
        config.TrackSource(path, Fingerprint(bytes));
        return config;
    }

    public void Save() => Save(_sourcePath ?? GetConfigPath());

    public void Save(string configPath, TimeSpan? lockTimeout = null)
    {
        var path = NormalizeConfigPath(configPath);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        using var configLock = AcquireLock(path, lockTimeout ?? TimeSpan.FromMilliseconds(DefaultLockTimeoutMilliseconds));
        CleanupStaleTemps(path, DateTime.UtcNow);

        if (_sourcePath is not null
            && string.Equals(_sourcePath, path, NativePathComparison())
            && !string.Equals(_sourceFingerprint, ReadFingerprint(path), StringComparison.Ordinal))
        {
            throw new ConfigConflictException(
                "Configuration changed after it was loaded; reload and retry the mutation.");
        }

        var bytes = Serialize(this);
        AtomicWrite(path, bytes);
        TrackSource(path, Fingerprint(bytes));
    }

    private static AppConfig LoadLocked(string path)
    {
        var bytes = ReadConfigBytes(path);
        var config = bytes is null
            ? new AppConfig()
            : JsonSerializer.Deserialize<AppConfig>(bytes, ReadOptions)
                ?? throw new JsonException("Configuration JSON contains no object.");
        config.TrackSource(path, Fingerprint(bytes));
        return config;
    }

    private static FileStream AcquireLock(string configPath, TimeSpan timeout)
    {
        if (timeout < TimeSpan.Zero)
            throw new ArgumentOutOfRangeException(nameof(timeout));
        var lockPath = configPath + ".lock";
        var deadline = DateTime.UtcNow + timeout;
        Exception? lastError = null;
        do
        {
            try
            {
                return new FileStream(
                    lockPath,
                    FileMode.OpenOrCreate,
                    FileAccess.ReadWrite,
                    FileShare.None,
                    bufferSize: 1,
                    FileOptions.None);
            }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException)
            {
                lastError = error;
                if (DateTime.UtcNow >= deadline)
                    break;
                Thread.Sleep(25);
            }
        } while (true);

        throw new ConfigBusyException(
            "Configuration is busy; retry after the other writer finishes.",
            lastError);
    }

    private static void AtomicWrite(string path, byte[] bytes)
    {
        var directory = Path.GetDirectoryName(path)!;
        var prefix = Path.GetFileName(path) + ".tmp.";
        var tempPath = Path.Combine(
            directory,
            $"{prefix}{Environment.ProcessId}.{Guid.NewGuid():N}");
        try
        {
            using (var stream = new FileStream(
                tempPath,
                FileMode.CreateNew,
                FileAccess.Write,
                FileShare.None,
                bufferSize: 16 * 1024,
                FileOptions.SequentialScan))
            {
                stream.Write(bytes);
                stream.Flush(flushToDisk: true);
            }
            BeforeAtomicReplaceForTests?.Invoke(tempPath, path);
            if (OperatingSystem.IsWindows() && File.Exists(path))
                File.Replace(tempPath, path, destinationBackupFileName: null, ignoreMetadataErrors: true);
            else
                File.Move(tempPath, path, overwrite: true);
        }
        finally
        {
            try
            {
                if (File.Exists(tempPath))
                    File.Delete(tempPath);
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
        }
    }

    private static void CleanupStaleTemps(string configPath, DateTime utcNow)
    {
        var directory = Path.GetDirectoryName(configPath)!;
        var prefix = Path.GetFileName(configPath) + ".tmp.";
        foreach (var candidate in Directory.EnumerateFiles(directory, prefix + "*", SearchOption.TopDirectoryOnly))
        {
            var name = Path.GetFileName(candidate);
            if (!name.StartsWith(prefix, StringComparison.Ordinal)
                || !IsGeneratedTempName(name[prefix.Length..])
                || (File.GetAttributes(candidate) & FileAttributes.ReparsePoint) != 0
                || utcNow - File.GetLastWriteTimeUtc(candidate) <= TimeSpan.FromHours(24))
                continue;
            try { File.Delete(candidate); }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
        }
    }

    private static bool IsGeneratedTempName(string suffix)
    {
        var parts = suffix.Split('.', StringSplitOptions.None);
        return parts.Length == 2
            && int.TryParse(parts[0], out var processId)
            && processId > 0
            && Guid.TryParseExact(parts[1], "N", out _);
    }

    private void TrackSource(string path, string fingerprint)
    {
        _sourcePath = path;
        _sourceFingerprint = fingerprint;
    }

    private static byte[] Serialize(AppConfig config) =>
        JsonSerializer.SerializeToUtf8Bytes(config, WriteOptions);

    private static string ReadFingerprint(string path) =>
        Fingerprint(ReadConfigBytes(path));

    private static byte[]? ReadConfigBytes(string path)
    {
        if (!File.Exists(path))
            return null;
        Exception? lastError = null;
        for (var attempt = 0; attempt < 5; attempt++)
        {
            try
            {
                using var stream = new FileStream(
                    path,
                    FileMode.Open,
                    FileAccess.Read,
                    FileShare.ReadWrite | FileShare.Delete);
                using var buffer = new MemoryStream();
                stream.CopyTo(buffer);
                return buffer.ToArray();
            }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException)
            {
                lastError = error;
                if (attempt < 4)
                    Thread.Sleep(5);
            }
        }
        throw lastError!;
    }

    private static string Fingerprint(byte[]? bytes) => bytes is null
        ? MissingFingerprint
        : Convert.ToHexString(SHA256.HashData(bytes));

    private static string NormalizeConfigPath(string path)
    {
        if (string.IsNullOrWhiteSpace(path))
            throw new ArgumentException("Configuration path cannot be empty.", nameof(path));
        return Path.GetFullPath(path);
    }

    private static StringComparison NativePathComparison() => OperatingSystem.IsWindows()
        ? StringComparison.OrdinalIgnoreCase
        : StringComparison.Ordinal;
}
