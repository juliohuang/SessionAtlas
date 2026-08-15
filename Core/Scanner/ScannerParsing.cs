using System.Globalization;
using System.Text.Json;
using SessionAtlas.Models;

namespace SessionAtlas.Core.Scanner;

internal static class ScannerParsing
{
    public static EnumerationOptions RecursiveFileEnumeration() =>
        new()
        {
            RecurseSubdirectories = true,
            IgnoreInaccessible = false,
            ReturnSpecialDirectories = false,
            AttributesToSkip = FileAttributes.ReparsePoint
        };

    public static bool TryReadUtcTimestamp(JsonElement element, out DateTime value)
    {
        if (element.ValueKind == JsonValueKind.String)
        {
            var text = element.GetString();
            if (DateTimeOffset.TryParse(
                text,
                CultureInfo.InvariantCulture,
                DateTimeStyles.RoundtripKind,
                out var parsed))
            {
                value = parsed.UtcDateTime;
                return true;
            }
        }
        else if (element.ValueKind == JsonValueKind.Number &&
                 element.TryGetInt64(out var numeric))
        {
            return TryReadUnixTimestamp(numeric, out value);
        }

        value = default;
        return false;
    }

    public static bool TryReadUnixTimestamp(long value, out DateTime timestamp)
    {
        try
        {
            var parsed = Math.Abs(value) >= 100_000_000_000
                ? DateTimeOffset.FromUnixTimeMilliseconds(value)
                : DateTimeOffset.FromUnixTimeSeconds(value);
            timestamp = parsed.UtcDateTime;
            return true;
        }
        catch (ArgumentOutOfRangeException)
        {
            timestamp = default;
            return false;
        }
    }

    public static bool TryNormalizeProjectPath(
        string? candidate,
        string sourceRoot,
        out string normalized)
    {
        normalized = "";
        if (string.IsNullOrWhiteSpace(candidate))
            return false;

        var path = candidate.Trim();
        if (path == "~" ||
            path.StartsWith($"~{Path.DirectorySeparatorChar}", StringComparison.Ordinal) ||
            path.StartsWith($"~{Path.AltDirectorySeparatorChar}", StringComparison.Ordinal))
        {
            path = Path.Combine(ScannerRegistry.GetHomeDirectory(), path[1..].TrimStart(
                Path.DirectorySeparatorChar,
                Path.AltDirectorySeparatorChar));
        }

        if (!Path.IsPathRooted(path))
            return false;

        try
        {
            normalized = ProjectPathSemantics.NormalizeNative(path);
            var normalizedSource = ProjectPathSemantics.NormalizeNative(sourceRoot);
            if (IsSameOrChildPath(normalized, normalizedSource))
            {
                normalized = "";
                return false;
            }
            return true;
        }
        catch (Exception error) when (
            error is ArgumentException or
            NotSupportedException or
            PathTooLongException)
        {
            normalized = "";
            return false;
        }
    }

    public static string TrimTrailingSeparatorsExceptRoot(string path)
    {
        return ProjectPathSemantics.TryNormalizeNative(path, out var normalized)
            ? normalized
            : path.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
    }

    private static bool IsSameOrChildPath(string candidate, string parent)
    {
        return ProjectPathSemantics.IsSameOrChild(candidate, parent);
    }
}
