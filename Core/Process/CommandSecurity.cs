namespace SessionAtlas.Core.Process;

/// <summary>
/// Validation and quoting rules for values that cross into a command
/// interpreter. Project paths stay in <see cref="System.Diagnostics.ProcessStartInfo.WorkingDirectory"/>
/// or a dedicated argv item; only validated tool/config tokens are rendered
/// into a shell command.
/// </summary>
public static class CommandSecurity
{
    private const int MaxToolKeyLength = 64;
    private const int MaxSessionIdLength = 512;
    private static readonly char[] ShellMetacharacters = ['&', '|', '<', '>', '^', '%', '!'];

    public static string ValidateToolKey(string value)
    {
        var trimmed = value?.Trim() ?? "";
        if (trimmed.Length is 0 or > MaxToolKeyLength ||
            !char.IsAsciiLetterOrDigit(trimmed[0]) ||
            trimmed.Any(c => !char.IsAsciiLetterOrDigit(c) && c is not ('.' or '_' or '+' or '-')))
        {
            throw new ArgumentException("工具标识包含不支持的字符", nameof(value));
        }
        return trimmed;
    }

    public static string ValidateSessionId(string value)
    {
        var trimmed = value?.Trim() ?? "";
        if (trimmed.Length is 0 or > MaxSessionIdLength ||
            trimmed.Any(c => !char.IsAsciiLetterOrDigit(c) && c is not ('.' or '_' or ':' or '+' or '-')))
        {
            throw new ArgumentException("会话 ID 包含不支持的字符", nameof(value));
        }
        return trimmed;
    }

    public static string ValidateDisplayLabel(string value)
    {
        var trimmed = value?.Trim() ?? "";
        if (trimmed.Length is 0 or > 128 || trimmed.Any(char.IsControl))
            throw new ArgumentException("显示名称包含不支持的字符", nameof(value));
        return trimmed;
    }

    public static IReadOnlyList<string> ParseSafeCommand(string command)
    {
        if (string.IsNullOrWhiteSpace(command))
            throw new ArgumentException("CLI 命令不能为空", nameof(command));
        if (command.Any(c => c is '\0' or '\r' or '\n' || ShellMetacharacters.Contains(c)))
            throw new ArgumentException("CLI 命令不能包含 shell 控制字符", nameof(command));

        var tokens = new List<string>();
        var current = new System.Text.StringBuilder();
        var quoted = false;
        var started = false;
        foreach (var character in command)
        {
            if (character == '"')
            {
                quoted = !quoted;
                started = true;
            }
            else if (char.IsWhiteSpace(character) && !quoted)
            {
                if (!started) continue;
                tokens.Add(current.ToString());
                current.Clear();
                started = false;
            }
            else
            {
                current.Append(character);
                started = true;
            }
        }

        if (quoted)
            throw new ArgumentException("CLI 命令包含未闭合的双引号", nameof(command));
        if (started)
            tokens.Add(current.ToString());
        if (tokens.Count == 0 || tokens[0].StartsWith('-') || IsShellProgram(tokens[0]))
            throw new ArgumentException("CLI 命令不能直接调用 shell 或脚本包装器", nameof(command));
        return tokens;
    }

    public static string BuildWindowsCommand(IEnumerable<string> arguments)
    {
        var tokens = arguments.ToArray();
        EnsureSafeTokens(tokens);
        return string.Join(" ", tokens.Select(token => $"\"{token}\""));
    }

    public static string BuildPosixCommand(IEnumerable<string> arguments)
    {
        var tokens = arguments.ToArray();
        EnsureSafeTokens(tokens);
        return string.Join(" ", tokens.Select(QuotePosix));
    }

    public static string QuotePosix(string value)
    {
        if (value.Contains('\0'))
            throw new ArgumentException("命令参数包含 NUL 字节", nameof(value));
        return $"'{value.Replace("'", "'\"'\"'")}'";
    }

    private static void EnsureSafeTokens(IEnumerable<string> tokens)
    {
        foreach (var token in tokens)
        {
            if (token.Length == 0 || token.Contains('"') ||
                token.Any(c => c is '\0' or '\r' or '\n' || ShellMetacharacters.Contains(c)))
            {
                throw new ArgumentException("命令参数包含不支持的 shell 字符", nameof(tokens));
            }
        }
    }

    private static bool IsShellProgram(string program)
    {
        var fileName = program.Replace('\\', '/').Split('/').Last().ToLowerInvariant();
        var extension = Path.GetExtension(fileName);
        if (extension is ".cmd" or ".bat" or ".ps1")
            return true;
        return fileName is
            "cmd" or "cmd.exe" or
            "powershell" or "powershell.exe" or
            "pwsh" or "pwsh.exe" or
            "sh" or "bash" or "zsh" or "fish" or
            "wsl" or "wsl.exe" or "osascript";
    }
}
