using SessionAtlas.Core.Process;

namespace SessionAtlas.Tests;

public class CommandSecurityTests
{
    [Fact]
    public void SafeCommandParserKeepsQuotedArgumentsTogether()
    {
        var arguments = CommandSecurity.ParseSafeCommand(
            "npx \"@scope/agent package\" --mode safe");

        Assert.Equal(
            ["npx", "@scope/agent package", "--mode", "safe"],
            arguments);
    }

    [Theory]
    [InlineData("cmd /c calc")]
    [InlineData("powershell -Command calc")]
    [InlineData("agent && calc")]
    [InlineData("agent\nwhoami")]
    [InlineData("agent \"unterminated")]
    public void UnsafeCommandSyntaxIsRejected(string command)
    {
        Assert.Throws<ArgumentException>(() => CommandSecurity.ParseSafeCommand(command));
    }

    [Fact]
    public void PosixQuotingPreservesApostrophes()
    {
        Assert.Equal(
            "'/srv/alice'\"'\"'s repo'",
            CommandSecurity.QuotePosix("/srv/alice's repo"));
    }

    [Fact]
    public void DisplayLabelsRejectTerminalControlCharacters()
    {
        Assert.Equal("Fixture Agent", CommandSecurity.ValidateDisplayLabel(" Fixture Agent "));
        Assert.Throws<ArgumentException>(
            () => CommandSecurity.ValidateDisplayLabel("Fixture\rAgent"));
    }
}
