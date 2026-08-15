using SessionAtlas.Core.Store;

namespace SessionAtlas.Tests;

public class SqliteStoreIsolationTests
{
    [Fact]
    public void ExplicitDatabasePathCreatesOnlyTheTemporaryDatabase()
    {
        using var root = new TemporaryDirectory();
        var databasePath = root.Combine("data", "index.db");

        using (var store = new SqliteStore(databasePath))
        {
            Assert.Empty(store.ListProjects());
        }

        Assert.True(File.Exists(databasePath));
        Assert.StartsWith(root.Path, Path.GetFullPath(databasePath), StringComparison.OrdinalIgnoreCase);
    }
}
