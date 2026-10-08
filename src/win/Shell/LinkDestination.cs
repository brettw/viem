namespace Viem.Windows.Shell;

internal enum LinkDestinationKind { Fragment, Document, Web }
internal sealed record LinkDestination(LinkDestinationKind Kind, string Value, string Fragment = "")
{
    // Resolve only an explicit click, relative to the current saved document.
    // URI handling, including escaping and launching, never invokes a shell.
    internal static LinkDestination Resolve(string destination, string? documentPath)
    {
        Validate(destination);
        if (destination.StartsWith('#')) return new(LinkDestinationKind.Fragment, "", Decode(destination[1..]));
        bool windowsPath = destination.Length >= 3 && char.IsAsciiLetter(destination[0])
            && destination[1] == ':' && destination[2] is '\\' or '/';
        if (!windowsPath && Uri.TryCreate(destination, UriKind.Absolute, out var uri))
        {
            if (uri.Scheme is "https" or "http")
            {
                if (uri.Host.Length == 0) throw new InvalidOperationException("Enter a complete web address.");
                return new(LinkDestinationKind.Web, uri.AbsoluteUri);
            }
            if (uri.Scheme != "file") throw new InvalidOperationException("Use a web address, a local document, or #heading.");
            return new(LinkDestinationKind.Document, Path.GetFullPath(uri.LocalPath), Decode(uri.Fragment.TrimStart('#')));
        }
        int hash = destination.IndexOf('#');
        string path = Decode(hash < 0 ? destination : destination[..hash]);
        string fragment = hash < 0 ? "" : Decode(destination[(hash + 1)..]);
        if (!Path.IsPathRooted(path))
        {
            if (documentPath == null) throw new InvalidOperationException("Save this document before opening a relative document link.");
            path = Path.Combine(Path.GetDirectoryName(documentPath)!, path);
        }
        return new(LinkDestinationKind.Document, Path.GetFullPath(path), fragment);
    }
    internal static void Validate(string destination)
    {
        if (destination.Length == 0 || destination.Any(char.IsControl) || Uri.UnescapeDataString(destination).Any(char.IsControl))
            throw new InvalidOperationException("Enter a link destination without control characters.");
        bool windowsPath = destination.Length >= 3 && char.IsAsciiLetter(destination[0])
            && destination[1] == ':' && destination[2] is '\\' or '/';
        if (!windowsPath && Uri.TryCreate(destination, UriKind.Absolute, out var uri)
            && uri.Scheme is not ("https" or "http" or "file"))
            throw new InvalidOperationException("Use a web address, a local document, or #heading.");
    }
    private static string Decode(string value)
    {
        string decoded = Uri.UnescapeDataString(value);
        if (decoded.Any(char.IsControl)) throw new InvalidOperationException("The link contains a control character.");
        return decoded;
    }
}
