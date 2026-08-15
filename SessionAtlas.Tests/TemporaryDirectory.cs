namespace SessionAtlas.Tests;

internal sealed class TemporaryDirectory : IDisposable
{
    public TemporaryDirectory()
    {
        Path = System.IO.Path.Combine(
            System.IO.Path.GetTempPath(),
            "sessionatlas-tests",
            Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(Path);
    }

    public string Path { get; }

    public string Combine(params string[] parts)
    {
        var result = Path;
        foreach (var part in parts)
            result = System.IO.Path.Combine(result, part);
        return result;
    }

    public void Dispose()
    {
        if (Directory.Exists(Path))
            Directory.Delete(Path, recursive: true);
    }
}
