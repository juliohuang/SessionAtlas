using SessionAtlas.Core.Config;
using SessionAtlas.Core.Scanner;
using System.Collections.Concurrent;
using System.Text.Json;

namespace SessionAtlas.Tests;

public class AppConfigTests
{
    [Fact]
    public void ConfigPathFollowsTheCurrentIsolatedHome()
    {
        using var firstHome = new TemporaryDirectory();
        using var secondHome = new TemporaryDirectory();
        var previous = Environment.GetEnvironmentVariable("SESSIONATLAS_HOME");

        try
        {
            Environment.SetEnvironmentVariable("SESSIONATLAS_HOME", firstHome.Path);
            new AppConfig { DefaultTerminal = "first-terminal" }.Save();

            Environment.SetEnvironmentVariable("SESSIONATLAS_HOME", secondHome.Path);
            new AppConfig { DefaultTerminal = "second-terminal" }.Save();

            Assert.True(File.Exists(firstHome.Combine(".sessionatlas", "config.json")));
            Assert.True(File.Exists(secondHome.Combine(".sessionatlas", "config.json")));
            Assert.Equal("second-terminal", AppConfig.Load().DefaultTerminal);
        }
        finally
        {
            Environment.SetEnvironmentVariable("SESSIONATLAS_HOME", previous);
        }
    }

    [Fact]
    public void RegistryReportsMalformedCustomToolConfiguration()
    {
        using var home = new TemporaryDirectory();
        var previous = Environment.GetEnvironmentVariable("SESSIONATLAS_HOME");

        try
        {
            Environment.SetEnvironmentVariable("SESSIONATLAS_HOME", home.Path);
            var configPath = home.Combine(".sessionatlas", "config.json");
            Directory.CreateDirectory(Path.GetDirectoryName(configPath)!);
            File.WriteAllText(configPath, "{not-valid-json");

            var registry = new ScannerRegistry();

            Assert.Equal(5, registry.All.Count);
            Assert.Contains(registry.Diagnostics, diagnostic =>
                diagnostic.Code == "config_read_failed" &&
                diagnostic.Severity == ScanDiagnosticSeverity.Warning);
        }
        finally
        {
            Environment.SetEnvironmentVariable("SESSIONATLAS_HOME", previous);
        }
    }

    [Fact]
    public void StaleLoadedInstanceCannotOverwriteANewerSave()
    {
        using var root = new TemporaryDirectory();
        var path = root.Combine("config.json");
        new AppConfig { DefaultTerminal = "initial" }.Save(path);
        var first = AppConfig.Load(path);
        var stale = AppConfig.Load(path);
        first.DefaultTerminal = "first";
        first.Save();
        stale.DefaultTerminal = "stale";

        Assert.Throws<ConfigConflictException>(() => stale.Save());
        Assert.Equal("first", AppConfig.Load(path).DefaultTerminal);
    }

    [Fact]
    public async Task ConcurrentUpdatesDoNotLoseSuccessfulMutationsOrExposePartialJson()
    {
        using var root = new TemporaryDirectory();
        var path = root.Combine("config.json");
        new AppConfig().Save(path);
        using var stop = new CancellationTokenSource();
        var readErrors = new ConcurrentQueue<Exception>();
        var reader = Task.Run(() =>
        {
            while (!stop.IsCancellationRequested)
            {
                try
                {
                    if (!AppConfig.TryLoad(path, out _))
                        throw new JsonException("invalid config read");
                }
                catch (Exception error)
                {
                    readErrors.Enqueue(error);
                }
            }
        });

        var writers = Enumerable.Range(0, 2).Select(writer => Task.Run(() =>
        {
            for (var iteration = 0; iteration < 50; iteration++)
            {
                var key = $"writer-{writer}-{iteration}";
                AppConfig.Update(path, config => config.PreferredToolsByPath[key] = "codex");
            }
        })).ToArray();
        await Task.WhenAll(writers);
        stop.Cancel();
        await reader;

        Assert.Empty(readErrors);
        var final = AppConfig.Load(path);
        Assert.Equal(100, final.PreferredToolsByPath.Count);
    }

    [Fact]
    public void BusyLockIsBoundedAndDoesNotModifyTheConfig()
    {
        using var root = new TemporaryDirectory();
        var path = root.Combine("config.json");
        new AppConfig { DefaultTerminal = "old" }.Save(path);
        using var held = new FileStream(
            path + ".lock", FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None);

        Assert.Throws<ConfigBusyException>(() => AppConfig.Update(
            path,
            config => config.DefaultTerminal = "new",
            TimeSpan.FromMilliseconds(75)));
        Assert.Equal("old", AppConfig.Load(path).DefaultTerminal);
    }

    [Fact]
    public void ReplaceFailureKeepsOldJsonAndCleansOnlyTheCurrentTemp()
    {
        using var root = new TemporaryDirectory();
        var path = root.Combine("config.json");
        new AppConfig { DefaultTerminal = "old" }.Save(path);
        var config = AppConfig.Load(path);
        config.DefaultTerminal = "sensitive-placeholder";
        AppConfig.BeforeAtomicReplaceForTests = (_, _) => throw new IOException("forced replace failure");
        try
        {
            Assert.Throws<IOException>(() => config.Save());
        }
        finally
        {
            AppConfig.BeforeAtomicReplaceForTests = null;
        }

        Assert.Equal("old", AppConfig.Load(path).DefaultTerminal);
        Assert.Empty(Directory.EnumerateFiles(root.Path, "config.json.tmp.*"));
        Assert.DoesNotContain("sensitive-placeholder", File.ReadAllText(path), StringComparison.Ordinal);
    }

    [Fact]
    public void LockedCleanupDeletesOnlyStrictOldGeneratedTemps()
    {
        using var root = new TemporaryDirectory();
        var path = root.Combine("config.json");
        new AppConfig().Save(path);
        var oldGenerated = path + $".tmp.123.{Guid.NewGuid():N}";
        var newGenerated = path + $".tmp.124.{Guid.NewGuid():N}";
        var similar = path + ".tmp.not-a-generated-name";
        File.WriteAllText(oldGenerated, "old");
        File.WriteAllText(newGenerated, "new");
        File.WriteAllText(similar, "similar");
        File.SetLastWriteTimeUtc(oldGenerated, DateTime.UtcNow.AddHours(-25));
        File.SetLastWriteTimeUtc(similar, DateTime.UtcNow.AddHours(-25));

        AppConfig.Update(path, config => config.DefaultTerminal = "updated");

        Assert.False(File.Exists(oldGenerated));
        Assert.True(File.Exists(newGenerated));
        Assert.True(File.Exists(similar));
    }
}
