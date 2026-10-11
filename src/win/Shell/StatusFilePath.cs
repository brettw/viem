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

    /// Drop whole directories before shortening the filename at grapheme edges.
    internal static string Fit(string text, double width, Func<string, double> measure)
    {
        if (width <= 0) return "";
        if (measure(text) <= width) return text;
        string filename = text;
        for (int index = 0; index < text.Length; index++) {
            if (text[index] is not ('/' or '\\') || index + 1 == text.Length) continue;
            filename = text[(index + 1)..];
            if (measure(filename) <= width) return filename;
        }
        const string ellipsis = "...";
        if (measure(ellipsis) > width) return "";
        int[] boundaries = StringInfo.ParseCombiningCharacters(filename);
        int low = 0, high = boundaries.Length;
        while (low < high)
        {
            int middle = low + (high - low + 1) / 2;
            int end = middle == boundaries.Length ? filename.Length : boundaries[middle];
            if (measure(filename[..end] + ellipsis) <= width) low = middle;
            else high = middle - 1;
        }
        int prefixEnd = low == boundaries.Length ? filename.Length : boundaries[low];
        return filename[..prefixEnd] + ellipsis;
    }
}
