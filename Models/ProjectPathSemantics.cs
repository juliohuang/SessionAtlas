namespace SessionAtlas.Models;

public enum ProjectPathFlavor
{
    Windows,
    Unix,
}

/// <summary>
/// Canonical, root-safe project path operations. Explicit flavors are lexical
/// so both platform contracts can be tested on every host; native inputs first
/// pass through <see cref="Path.GetFullPath(string)"/>.
/// </summary>
public static class ProjectPathSemantics
{
    public static ProjectPathFlavor NativeFlavor => OperatingSystem.IsWindows()
        ? ProjectPathFlavor.Windows
        : ProjectPathFlavor.Unix;

    public static StringComparer NativeComparer => GetComparer(NativeFlavor);

    public static StringComparer GetComparer(ProjectPathFlavor flavor) =>
        flavor == ProjectPathFlavor.Windows
            ? StringComparer.OrdinalIgnoreCase
            : StringComparer.Ordinal;

    public static bool TryNormalizeNative(string? candidate, out string normalized)
    {
        normalized = "";
        if (string.IsNullOrWhiteSpace(candidate))
            return false;
        if (OperatingSystem.IsWindows()
            && candidate.Length >= 2
            && char.IsAsciiLetter(candidate[0])
            && candidate[1] == ':'
            && (candidate.Length < 3 || candidate[2] is not ('\\' or '/')))
            return false;
        if (!Path.IsPathRooted(candidate))
            return false;

        try
        {
            return TryNormalize(Path.GetFullPath(candidate), NativeFlavor, out normalized);
        }
        catch (Exception error) when (
            error is ArgumentException or
            NotSupportedException or
            PathTooLongException)
        {
            return false;
        }
    }

    public static string NormalizeNative(string candidate)
    {
        if (!TryNormalizeNative(candidate, out var normalized))
            throw new ArgumentException("Project path must be a valid absolute path.", nameof(candidate));
        return normalized;
    }

    public static bool TryNormalize(
        string? candidate,
        ProjectPathFlavor flavor,
        out string normalized)
    {
        normalized = "";
        if (string.IsNullOrWhiteSpace(candidate) || candidate.IndexOf('\0') >= 0)
            return false;

        return flavor == ProjectPathFlavor.Windows
            ? TryNormalizeWindows(candidate, out normalized)
            : TryNormalizeUnix(candidate, out normalized);
    }

    public static string GetDisplayName(string? path) =>
        GetDisplayName(path, NativeFlavor);

    public static string GetDisplayName(string? path, ProjectPathFlavor flavor)
    {
        if (!TryNormalize(path, flavor, out var normalized))
            return "";

        if (flavor == ProjectPathFlavor.Unix)
            return normalized == "/" ? "/" : normalized[(normalized.LastIndexOf('/') + 1)..];

        if (normalized.Length == 3 && normalized[1] == ':' && normalized[2] == '\\')
            return normalized;
        if (normalized.StartsWith(@"\\", StringComparison.Ordinal)
            && normalized.Count(character => character == '\\') == 3)
            return normalized;
        return normalized[(normalized.LastIndexOf('\\') + 1)..];
    }

    public static bool IsSameOrChild(string candidate, string parent)
    {
        var comparison = NativeFlavor == ProjectPathFlavor.Windows
            ? StringComparison.OrdinalIgnoreCase
            : StringComparison.Ordinal;
        if (string.Equals(candidate, parent, comparison))
            return true;
        var separator = NativeFlavor == ProjectPathFlavor.Windows ? '\\' : '/';
        return candidate.StartsWith(
            parent.EndsWith(separator) ? parent : parent + separator,
            comparison);
    }

    private static bool TryNormalizeUnix(string candidate, out string normalized)
    {
        normalized = "";
        if (!candidate.StartsWith("/", StringComparison.Ordinal))
            return false;

        var segments = ReduceSegments(candidate.Split('/', StringSplitOptions.RemoveEmptyEntries));
        normalized = segments.Count == 0 ? "/" : "/" + string.Join('/', segments);
        return true;
    }

    private static bool TryNormalizeWindows(string candidate, out string normalized)
    {
        normalized = "";
        var value = candidate.Replace('/', '\\');
        string root;
        IEnumerable<string> remainder;

        if (value.Length >= 3 && char.IsAsciiLetter(value[0])
            && value[1] == ':' && value[2] == '\\')
        {
            root = $"{char.ToUpperInvariant(value[0])}:\\";
            remainder = value[3..].Split('\\', StringSplitOptions.RemoveEmptyEntries);
        }
        else if (value.StartsWith(@"\\", StringComparison.Ordinal))
        {
            var parts = value[2..].Split('\\', StringSplitOptions.RemoveEmptyEntries);
            if (parts.Length < 2 || parts[0] is "." or ".." || parts[1] is "." or "..")
                return false;
            root = $@"\\{parts[0]}\{parts[1]}";
            remainder = parts.Skip(2);
        }
        else
        {
            return false;
        }

        var segments = ReduceSegments(remainder);
        if (segments.Count == 0)
        {
            normalized = root;
        }
        else
        {
            normalized = root.EndsWith('\\')
                ? root + string.Join('\\', segments)
                : root + "\\" + string.Join('\\', segments);
        }
        return true;
    }

    private static List<string> ReduceSegments(IEnumerable<string> source)
    {
        var segments = new List<string>();
        foreach (var segment in source)
        {
            if (segment.Length == 0 || segment == ".")
                continue;
            if (segment == "..")
            {
                if (segments.Count > 0)
                    segments.RemoveAt(segments.Count - 1);
                continue;
            }
            segments.Add(segment);
        }
        return segments;
    }
}
