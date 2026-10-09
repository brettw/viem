namespace Viem.Windows.Shell;

internal static class ImageLocation
{
    internal static bool IsWeb(string value) => Uri.TryCreate(value, UriKind.Absolute, out var uri) && uri.Scheme is "http" or "https";

    internal static void Validate(string value)
    {
        LinkDestination.Validate(value);
        if (value.StartsWith('#')) throw new InvalidOperationException("Enter an image file or web address.");
    }

    // A native picker returns a literal filesystem name. Encode each component
    // once so punctuation is not reinterpreted as a URL fragment or escape.
    internal static string PickedDestination(string filePath, string? documentPath)
    {
        string selected = Path.GetFullPath(filePath);
        string absolute = AbsoluteFileDestination(selected);
        if (documentPath == null) return absolute;
        string directory = Path.GetDirectoryName(Path.GetFullPath(documentPath))!;
        string root = Path.GetPathRoot(directory)!;
        string selectedRoot = Path.GetPathRoot(selected)!;
        if (!string.Equals(root, selectedRoot, StringComparison.OrdinalIgnoreCase)) return absolute;
        string[] folders = directory[root.Length..].Split(new[] { '\\', '/' }, StringSplitOptions.RemoveEmptyEntries);
        string selectedDirectory = Path.GetDirectoryName(selected)!;
        string[] selectedFolders = selectedDirectory[selectedRoot.Length..].Split(new[] { '\\', '/' }, StringSplitOptions.RemoveEmptyEntries);
        // Root-level documents need no ascent. Else a relative path must share
        // an actual directory beyond the drive or UNC share root.
        if (folders.Length != 0 && (selectedFolders.Length == 0
            || !string.Equals(folders[0], selectedFolders[0], StringComparison.OrdinalIgnoreCase))) return absolute;
        return EscapePath(Path.GetRelativePath(directory, selected).Replace('\\', '/'));
    }

    private static string EscapePath(string path) => string.Join('/', path.Split('/').Select(Uri.EscapeDataString));

    private static string AbsoluteFileDestination(string selected)
    {
        string path = selected.Replace('\\', '/');
        if (path.StartsWith("//", StringComparison.Ordinal))
        {
            int serverEnd = path.IndexOf('/', 2);
            string host = new Uri(selected, UriKind.Absolute).GetComponents(UriComponents.Host, UriFormat.UriEscaped);
            return "file://" + host + "/" + EscapePath(path[(serverEnd + 1)..]);
        }
        // Preserve the drive colon while escaping literal filename components,
        // including Unicode, before a URI parser can normalize them.
        return "file:///" + path[..2] + EscapePath(path[2..]);
    }

    // This resolution performs no resource access. Launching is reserved for an
    // explicit click on the popup's location, never for rendering an image.
    internal static Uri Resolve(string value, string? documentPath)
    {
        Validate(value);
        var target = LinkDestination.Resolve(value, documentPath);
        return new Uri(target.Value, UriKind.Absolute);
    }

    internal static string? LocalPreviewPath(string value, string? documentPath)
    {
        try
        {
            var uri = Resolve(value, documentPath);
            if (!uri.IsFile || uri.IsUnc || uri.Host.Length != 0 && uri.Host != "localhost") return null;
            string path = Path.GetFullPath(uri.LocalPath);
            if (path.StartsWith(@"\\", StringComparison.Ordinal) || path.StartsWith("//", StringComparison.Ordinal)) return null;
            var root = Path.GetPathRoot(path);
            if (root == null || new DriveInfo(root).DriveType is not (DriveType.Fixed or DriveType.Removable or DriveType.Ram)) return null;
            return path;
        }
        catch (Exception e) when (e is ArgumentException or IOException or UnauthorizedAccessException or InvalidOperationException or NotSupportedException)
        { return null; }
    }
}
