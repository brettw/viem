namespace Viem.Windows.Shell;

internal static class ImageLocation
{
    internal static bool IsWeb(string value) => Uri.TryCreate(value, UriKind.Absolute, out var uri) && uri.Scheme is "http" or "https";

    internal static void Validate(string value)
    {
        LinkDestination.Validate(value);
        if (value.StartsWith('#')) throw new InvalidOperationException("Enter an image file or web address.");
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
