using SessionAtlas.Models;

namespace SessionAtlas.Tests;

public class ProjectPathSemanticsTests
{
    public static TheoryData<ProjectPathFlavor, string, string> NormalizedPaths => new()
    {
        { ProjectPathFlavor.Windows, @"C:\", @"C:\" },
        { ProjectPathFlavor.Windows, @"c:/Repo/", @"C:\Repo" },
        { ProjectPathFlavor.Windows, @"C:\repo\.\child\..\", @"C:\repo" },
        { ProjectPathFlavor.Windows, @"\\server\share", @"\\server\share" },
        { ProjectPathFlavor.Windows, @"\\server\share\repo\", @"\\server\share\repo" },
        { ProjectPathFlavor.Unix, "/", "/" },
        { ProjectPathFlavor.Unix, "/repo/", "/repo" },
        { ProjectPathFlavor.Unix, "/repo/./child/../", "/repo" },
    };

    [Theory]
    [MemberData(nameof(NormalizedPaths))]
    public void NormalizeProjectPathPreservesRootsAndResolvesSegments(
        ProjectPathFlavor flavor,
        string input,
        string expected)
    {
        Assert.True(ProjectPathSemantics.TryNormalize(input, flavor, out var actual));
        Assert.Equal(expected, actual);
    }

    [Theory]
    [InlineData(ProjectPathFlavor.Windows, "")]
    [InlineData(ProjectPathFlavor.Windows, "repo")]
    [InlineData(ProjectPathFlavor.Windows, "C:repo")]
    [InlineData(ProjectPathFlavor.Windows, @"\\server")]
    [InlineData(ProjectPathFlavor.Unix, "repo")]
    [InlineData(ProjectPathFlavor.Unix, "")]
    public void NormalizeProjectPathRejectsNonAbsoluteOrIncompletePaths(
        ProjectPathFlavor flavor,
        string input)
    {
        Assert.False(ProjectPathSemantics.TryNormalize(input, flavor, out var actual));
        Assert.Equal("", actual);
    }

    [Fact]
    public void FlavorComparersMatchPlatformCaseRules()
    {
        Assert.True(ProjectPathSemantics.GetComparer(ProjectPathFlavor.Windows)
            .Equals(@"C:\Repo", @"c:\repo"));
        Assert.False(ProjectPathSemantics.GetComparer(ProjectPathFlavor.Unix)
            .Equals("/Repo", "/repo"));
    }

    [Theory]
    [InlineData(ProjectPathFlavor.Windows, @"C:\", @"C:\")]
    [InlineData(ProjectPathFlavor.Windows, @"\\server\share", @"\\server\share")]
    [InlineData(ProjectPathFlavor.Windows, @"C:\repo", "repo")]
    [InlineData(ProjectPathFlavor.Unix, "/", "/")]
    [InlineData(ProjectPathFlavor.Unix, "/repo", "repo")]
    public void DisplayNameIsNeverEmptyForAValidRoot(
        ProjectPathFlavor flavor,
        string path,
        string expected)
    {
        Assert.Equal(expected, ProjectPathSemantics.GetDisplayName(path, flavor));
    }
}
