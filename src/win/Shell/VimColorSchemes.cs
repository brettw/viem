using Viem.Windows.Core;

namespace Viem.Windows.Shell;

/// Discovery of the user's Vim color schemes. Evaluation and theme mapping are
/// portable core policy; this only lists and reads files.
internal static class VimColorSchemes
{
    internal sealed record Scheme(string Name, string Path) { public override string ToString() => Name; }

    /// Vim for Windows reads `$HOME\vimfiles` (HOME defaulting to the user
    /// profile). `~\.vim` is listed too, for configurations shared with Unix.
    public static string[] Directories(string? home = null)
    {
        home ??= Environment.GetEnvironmentVariable("HOME") is { Length: > 0 } value && Path.IsPathRooted(value)
            ? value : Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        return [Path.Combine(home, "vimfiles", "colors"), Path.Combine(home, ".vim", "colors")];
    }

    /// Regular, visible `.vim` files sorted by name. A name found in an earlier
    /// directory hides later ones, as `:colorscheme` loads the first.
    public static Scheme[] Available(IEnumerable<string> directories)
    {
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var result = new List<Scheme>();
        foreach (string directory in directories) {
            string[] files;
            try { files = Directory.Exists(directory) ? Directory.GetFiles(directory) : []; }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException) { continue; }
            foreach (string file in files.OrderBy(path => path, StringComparer.Ordinal)) {
                // A "*.vim" search pattern would also match longer extensions.
                if (!Path.GetExtension(file).Equals(".vim", StringComparison.OrdinalIgnoreCase)) continue;
                string name = Path.GetFileNameWithoutExtension(file);
                if (name.Length == 0 || name.StartsWith('.') || (File.GetAttributes(file) & FileAttributes.Hidden) != 0 || !seen.Add(name)) continue;
                result.Add(new(name, file));
            }
        }
        return result.OrderBy(scheme => scheme.Name, StringComparer.OrdinalIgnoreCase).ThenBy(scheme => scheme.Name, StringComparer.Ordinal).ToArray();
    }

    public static byte[] Read(string path)
    {
        if (new FileInfo(path).Length > CoreThemes.MaximumVimColorSchemeBytes) throw new InvalidDataException("The Vim color scheme is larger than 4 MiB.");
        return File.ReadAllBytes(path);
    }

    /// A user-profile path with a leading `~`, as Windows file commands accept.
    public static string Display(string path)
    {
        string home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        return path.StartsWith(home + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase) ? "~" + path[home.Length..] : path;
    }
}
