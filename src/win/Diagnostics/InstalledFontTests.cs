#if DEBUG
using System.Buffers.Binary;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using Viem.Windows.Rendering;
using Windows.UI.Text;

namespace Viem.Windows.Diagnostics;

internal static class InstalledFontTests
{
    private static void Check(bool value, string name)
    {
        if (!value) throw new InvalidOperationException(name);
        FrontendSmokeTests.UiChecks.Add(name);
    }

    internal static void Run(CanvasDevice device)
    {
        string root = Path.Combine(AppContext.BaseDirectory, "Resources", "fonts");
        string regular = Path.Combine(root, "flightline", "FlightlineCode-Regular-VF.ttf");
        string italic = Path.Combine(root, "flightline", "FlightlineCode-Italic-VF.ttf");
        string recursive = Path.Combine(root, "recursive", "Recursive_VF_1.085.ttf");
        var metadata = FontFileMetadata.TryRead(regular)!;
        var italicMetadata = FontFileMetadata.TryRead(italic)!;
        Check(metadata is { PostScriptName: "FlightlineCode-Normal", Family: "Flightline Code" }
            && italicMetadata is { PostScriptName: "FlightlineCode-NormalItalic" },
            "bounded file metadata identifies the separate Flightline variable designs");
        Check(metadata.Matches(metadata) && !metadata.Matches(italicMetadata),
            "installed font acceptance distinguishes upright and italic metadata");
        Check(!metadata.Matches(metadata with { Fvar = [] }), "a static installed face cannot replace the bundled variable design");
        static byte[] Changed(byte[] value) { var copy = value.ToArray(); copy[^1] ^= 1; return copy; }
        Check(!metadata.Matches(metadata with { Fvar = Changed(metadata.Fvar) })
            && !metadata.Matches(metadata with { Name = Changed(metadata.Name) })
            && !metadata.Matches(metadata with { Stat = Changed(metadata.Stat) }),
            "changed axes, names, or style links retain the bundled fallback");

        // A local font set stands in for the installed index. This makes partial
        // and missing-installation coverage independent of the developer's fonts.
        using var uprightSet = new CanvasFontSet(new Uri(regular));
        using var italicSet = new CanvasFontSet(new Uri(italic));
        using var recursiveSet = new CanvasFontSet(new Uri(recursive));
        using var forced = FontCatalog.LoadFontResources(root, null);
        Check(forced.Sources.Length == 3 && forced.Installed.Length == 0,
            "forced fallback indexes every packaged font without installed discovery");
        using var partial = FontCatalog.LoadFontResources(root, uprightSet);
        Check(partial.Installed.Length == 1 && partial.Installed[0].Face is { Source: null, Slant: FontStyle.Normal }
            && partial.Sources.Select(s => Path.GetFileName(s.Uri.LocalPath)).ToHashSet().SetEquals((string[])[
                "FlightlineCode-Italic-VF.ttf", "Recursive_VF_1.085.ttf"]),
            "an installed upright design skips only its own resource and retains missing italic and Recursive files");
        using var opposite = FontCatalog.LoadFontResources(root, italicSet);
        Check(opposite.Installed.Length == 1 && opposite.Installed[0].Face is { Source: null, Slant: FontStyle.Italic }
            && opposite.Sources.Any(s => s.Uri.LocalPath == regular),
            "an installed italic design retains the missing upright resource");
        using var recursiveOnly = FontCatalog.LoadFontResources(root, recursiveSet);
        Check(recursiveOnly.Installed.Length == 1 && recursiveOnly.Installed[0].Face.PortableFamily == "Recursive"
            && recursiveOnly.Sources.Length == 2,
            "Recursive is checked independently of both Flightline designs");
        Check(FontVariations.Parse(recursiveOnly.Installed[0].Tables.Fvar, recursiveOnly.Installed[0].Tables.Name,
            recursiveOnly.Installed[0].Tables.Stat).Instances.Length == 65,
            "accepted Recursive metadata retains Default and all 64 named presets");
        using var missingSet = uprightSet.GetMatchingFonts([new CanvasFontProperty {
            Identifier = CanvasFontPropertyIdentifier.FamilyName, Value = "viem-no-such-family", Locale = "" }]);
        using var missing = FontCatalog.LoadFontResources(root, missingSet);
        Check(missing.Installed.Length == 0 && missing.Sources.Length == 3,
            "missing installed families retain every packaged design");

        // Exercise the actual renderer's choice when only one slant is installed.
        var uprightFace = partial.Installed.Single().Face;
        var italicFace = opposite.Installed.Single().Face with { Source = new Uri(italic) };
        FontFace[] mixed = [uprightFace, italicFace];
        Check(FontCatalog.ChooseRenderingFace(mixed, 437, FontStyle.Normal, FontStretch.Normal) == uprightFace
            && FontCatalog.ChooseRenderingFace(mixed, 437, FontStyle.Italic, FontStretch.Normal) == italicFace,
            "mixed installed upright and bundled italic keep the correct design for both rendering slants");

        string scratch = FrontendSmokeTests.ReportPath + ".font-metadata-test";
        try {
            File.WriteAllBytes(scratch, [0, 1]);
            Check(FontFileMetadata.TryRead(scratch) == null, "truncated font metadata safely uses native fallback");
            var header = new byte[28];
            BinaryPrimitives.WriteUInt32BigEndian(header, 0x74746366); // Collection: unsupported fast path.
            File.WriteAllBytes(scratch, header);
            Check(FontFileMetadata.TryRead(scratch) == null, "font collections remain on the native bundled path");
            BinaryPrimitives.WriteUInt32BigEndian(header, 0x00010000);
            BinaryPrimitives.WriteUInt16BigEndian(header.AsSpan(4), 1);
            BinaryPrimitives.WriteUInt32BigEndian(header.AsSpan(12), 0x6e616d65);
            BinaryPrimitives.WriteUInt32BigEndian(header.AsSpan(20), uint.MaxValue);
            BinaryPrimitives.WriteUInt32BigEndian(header.AsSpan(24), 16);
            File.WriteAllBytes(scratch, header);
            Check(FontFileMetadata.TryRead(scratch) == null, "out-of-file table offsets cannot enter the installed-font fast path");
        }
        finally { File.Delete(scratch); }
    }
}
#endif
