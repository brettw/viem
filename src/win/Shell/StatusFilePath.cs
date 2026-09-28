using System.Globalization;

namespace Viem.Windows.Shell;

internal static class StatusFilePath
{
    internal static string? Full(string? file) => file == null ? null : Path.GetFullPath(file);

    internal static string? Relative(string? file, string directory)
    {
        if (Full(file) is not { } target) return null;
        string relative = Path.GetRelativePath(directory, target);
        // Different drives/UNC shares have no path relative to this directory.
        return Path.IsPathRooted(relative) ? null : relative;
    }

    internal static string Display(string? file, string directory)
    {
        if (Full(file) is not { } target) return "Untitled";
        string cwd = Path.GetFullPath(directory);
        if (Relative(target, cwd) is not { } relative) return target;
        string root = Path.GetPathRoot(cwd)!;
        char[] separators = [Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar];
        int depth = cwd[root.Length..].Split(separators, StringSplitOptions.RemoveEmptyEntries).Length;
        int ascent = relative.Split(separators, StringSplitOptions.RemoveEmptyEntries).TakeWhile(part => part == "..").Count();
        return ascent >= depth ? target : relative;
    }

    /// Fit the suffix without splitting a surrogate pair or grapheme cluster.
    internal static string TrimLeft(string text, double width, Func<string, double> measure)
    {
        if (width <= 0) return "";
        if (measure(text) <= width) return text;
        const string ellipsis = "…";
        if (measure(ellipsis) > width) return "";
        int[] boundaries = StringInfo.ParseCombiningCharacters(text);
        int low = 0, high = boundaries.Length;
        while (low < high)
        {
            int middle = low + (high - low) / 2;
            if (measure(ellipsis + text[boundaries[middle]..]) <= width) high = middle;
            else low = middle + 1;
        }
        return low == boundaries.Length ? ellipsis : ellipsis + text[boundaries[low]..];
    }
}
