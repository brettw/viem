using System.Globalization;
using System.Collections.Concurrent;
using Microsoft.Graphics.Canvas.Text;
using Windows.UI.Text;

namespace Viem.Windows.Rendering;

internal sealed record FontFace(string Name, string Family, string StyleName, ushort Weight, FontStyle Slant, FontStretch Stretch, Uri? Source = null)
{
    public override string ToString() => StyleName;
}

/// <summary>Immutable font descriptions shared by independent UI/worker shapers.</summary>
internal static class FontCatalog
{
    // The agile, read-only font index is shared for the same process lifetime
    // as the name caches. Build it while the shell loads; never enumerate its
    // Fonts collection to resolve a family or populate the font picker.
    private static readonly Lazy<Task<CanvasFontSet>> systemFonts = new(() => Task.Run(CanvasFontSet.GetSystemFontSet));
    internal static void PrepareFonts() { _ = systemFonts.Value; _ = bundledFonts.Value; }
    private static CanvasFontSet SystemFonts => systemFonts.Value.GetAwaiter().GetResult();
    // App-local sets, like the system index, are immutable and retained for the
    // process lifetime. Creating their indices does not enumerate font faces.
    private sealed record FontSource(Uri Uri, CanvasFontSet Fonts);
    private sealed record BundledFontIndex(FontSource[] Sources, HashSet<string> Families);
    // Lazy publishes only the task. File I/O and native indexing run on its
    // worker, outside initialization/cache locks, like the system index above.
    private static readonly Lazy<Task<BundledFontIndex>> bundledFonts = new(() => Task.Run(LoadBundledFonts));
    private static BundledFontIndex BundledFonts => bundledFonts.Value.GetAwaiter().GetResult();
    private static readonly Lazy<string[]> families = new(DiscoverFamilies);
    private static readonly ConcurrentDictionary<string, bool> availableFamilies = new(StringComparer.OrdinalIgnoreCase);
    private static readonly ConcurrentDictionary<string, FontFace?> names = new(StringComparer.OrdinalIgnoreCase);
    private static readonly ConcurrentDictionary<string, FontFace[]> familyFaces = new(StringComparer.OrdinalIgnoreCase);
    internal static string[] Families => families.Value;
    internal static bool FamilyListLoaded => families.IsValueCreated;
    private static int faceDescriptionsRead;
    internal static int FaceDescriptionsRead => Volatile.Read(ref faceDescriptionsRead);
    private static BundledFontIndex LoadBundledFonts()
    {
        string directory = Path.Combine(AppContext.BaseDirectory, "Resources", "fonts");
        var result = new List<FontSource>();
        var names = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        try
        {
            if (!Directory.Exists(directory)) return new([], names);
            foreach (string file in Directory.EnumerateFiles(directory, "*", SearchOption.AllDirectories)
                .Where(f => new[] { ".ttf", ".otf", ".ttc", ".otc" }.Contains(Path.GetExtension(f), StringComparer.OrdinalIgnoreCase))
                .OrderBy(f => f, StringComparer.Ordinal))
            {
                CanvasFontSet? fonts = null;
                try
                {
                    var uri = new Uri(Path.GetFullPath(file));
                    fonts = new CanvasFontSet(uri);
                    foreach (var property in fonts.GetPropertyValues(CanvasFontPropertyIdentifier.FamilyName,
                        CultureInfo.CurrentUICulture.Name + ";en-US")) names.Add(property.Value);
                    result.Add(new(uri, fonts));
                    fonts = null; // The published index now owns this resource.
                }
                catch (Exception error) when (error is IOException or UnauthorizedAccessException or System.Runtime.InteropServices.COMException or ArgumentException)
                { System.Diagnostics.Debug.WriteLine($"Bundled font {file}: {error.Message}"); }
                finally { fonts?.Dispose(); }
            }
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException)
        { System.Diagnostics.Debug.WriteLine($"Bundled fonts: {error.Message}"); }
        return new(result.ToArray(), names);
    }
    private static string[] DiscoverFamilies()
    {
        using var startup = Diagnostics.StartupPerformance.Measure("fonts.families");
        return CanvasTextFormat.GetSystemFontFamilies()
            .Concat(BundledFonts.Families)
            .Distinct(StringComparer.OrdinalIgnoreCase).OrderBy(f => f, StringComparer.CurrentCultureIgnoreCase).ToArray();
    }
    private static string Localized(IReadOnlyDictionary<string, string> values) =>
        values.FirstOrDefault(p => string.Equals(p.Key, CultureInfo.CurrentUICulture.Name, StringComparison.OrdinalIgnoreCase)).Value
        ?? values.GetValueOrDefault("en-us") ?? values.Values.FirstOrDefault() ?? "";
    private static CanvasFontProperty Property(CanvasFontPropertyIdentifier identifier, string value) => new() { Identifier = identifier, Value = value, Locale = "" };
    private static bool IsFamily(string name)
    {
        if (availableFamilies.TryGetValue(name, out bool known)) return known;
        var property = Property(CanvasFontPropertyIdentifier.FamilyName, name);
        return availableFamilies[name] = SystemFonts.CountFontsMatchingProperty(property) > 0
            || BundledFonts.Families.Contains(name);
    }
    private static FontFace[] Discover(CanvasFontPropertyIdentifier property, string value)
    {
        using var startup = Diagnostics.StartupPerformance.Measure("fonts.matchingFaces");
        var result = new List<FontFace>();
        // Prefer packaged copies when a user also has the same face installed.
        // Filter each index first; missing-name lookup never scans every face.
        foreach (var source in BundledFonts.Sources) ReadFaces(source.Fonts, source.Uri);
        ReadFaces(SystemFonts, null);
        void ReadFaces(CanvasFontSet set, Uri? source)
        {
            using var matching = set.GetMatchingFonts([Property(property, value)]);
            foreach (var font in matching.Fonts)
            {
                using (font)
                {
                    Interlocked.Increment(ref faceDescriptionsRead);
                    if (font.Simulations != CanvasFontSimulations.None) continue;
                    string family = Localized(font.FamilyNames), style = Localized(font.FaceNames);
                    string name = Localized(font.GetInformationalStrings(CanvasFontInformation.PostscriptName));
                    if (name.Length == 0) name = Localized(font.GetInformationalStrings(CanvasFontInformation.FullName));
                    if (name.Length > 0 && family.Length > 0) result.Add(new(name, family, style, font.Weight.Weight, font.Style, font.Stretch, source));
                }
            }
        }
        return result.DistinctBy(f => f.Name, StringComparer.OrdinalIgnoreCase).OrderBy(f => f.Weight)
            .ThenBy(f => f.Slant).ThenBy(f => f.StyleName, StringComparer.CurrentCultureIgnoreCase).ThenBy(f => f.Name, StringComparer.Ordinal).ToArray();
    }
    internal static FontFace? Named(string name)
    {
        if (name.Length == 0) return null;
        if (names.TryGetValue(name, out var cached)) return cached;
        var face = Discover(CanvasFontPropertyIdentifier.PostscriptName, name).FirstOrDefault();
        // Fonts without PostScript metadata use their exact full name.
        face ??= Discover(CanvasFontPropertyIdentifier.FullName, name).FirstOrDefault(f => string.Equals(f.Name, name, StringComparison.OrdinalIgnoreCase));
        return names[name] = face;
    }
    /// Portable request tokens for the family picker's built-in "use the
    /// system font" entries. EVFontCatalog on macOS recognizes the identical
    /// strings, so a style saved with one renders consistently on both
    /// platforms; only the picker's label differs from a literal font name.
    internal const string SystemDefaultFamily = "system-ui";
    internal const string SystemMonospaceFamily = "ui-monospace";
    private const string SystemDefaultDisplayName = "System Default";
    private const string SystemMonospaceDisplayName = "System Monospace";
    internal static string DisplayFamily(string name) =>
        string.Equals(name, SystemDefaultFamily, StringComparison.OrdinalIgnoreCase) ? SystemDefaultDisplayName :
        string.Equals(name, SystemMonospaceFamily, StringComparison.OrdinalIgnoreCase) ? SystemMonospaceDisplayName :
        name.Length == 0 || IsFamily(name) ? name : Named(name)?.Family ?? name;
    /// Reverses <see cref="DisplayFamily"/> for the picker's own two entries;
    /// any other label (an installed font name typed or picked) yields null.
    internal static string? StorageFamily(string displayName) =>
        string.Equals(displayName, SystemDefaultDisplayName, StringComparison.OrdinalIgnoreCase) ? SystemDefaultFamily :
        string.Equals(displayName, SystemMonospaceDisplayName, StringComparison.OrdinalIgnoreCase) ? SystemMonospaceFamily :
        null;
    internal static FontFace[] Faces(string familyOrFace)
    {
        string family = Resolve(familyOrFace)?.Family ?? familyOrFace;
        if (family.Length == 0) return [];
        if (familyFaces.TryGetValue(family, out var cached)) return cached;
        return familyFaces[family] = Discover(CanvasFontPropertyIdentifier.FamilyName, family);
    }
    internal static FontFace? Current(string name, uint weight, uint slant)
    {
        var matching = Faces(name).Where(f => f.Weight == weight && (f.Slant != FontStyle.Normal) == (slant != 0)).ToArray();
        return matching.FirstOrDefault(f => string.Equals(f.Name, name, StringComparison.OrdinalIgnoreCase)) ?? matching.FirstOrDefault();
    }
    internal static FontFace? ForFamilyChange(string requested, FontFace? current)
    {
        var faces = Faces(requested);
        var explicitFace = faces.FirstOrDefault(f => string.Equals(f.Name, requested, StringComparison.OrdinalIgnoreCase)
            && !string.Equals(f.Family, requested, StringComparison.OrdinalIgnoreCase));
        if (explicitFace != null) return explicitFace;
        if (current != null && faces.FirstOrDefault(f => string.Equals(f.StyleName, current.StyleName, StringComparison.OrdinalIgnoreCase)) is { } corresponding) return corresponding;
        foreach (string regular in new[] { "Regular", "Normal", "Roman", "Book" })
            if (faces.FirstOrDefault(f => f.Slant == FontStyle.Normal && string.Equals(f.StyleName, regular, StringComparison.OrdinalIgnoreCase)) is { } face) return face;
        return faces.FirstOrDefault();
    }
    internal static (string Family, FontStretch Stretch)? Resolve(string requested)
    {
        if (requested.Length == 0) return null;
        string candidate = requested.ToLowerInvariant() switch {
            "monospace" or "ui-monospace" or "menlo" or "monaco" => IsFamily("Cascadia Mono") ? "Cascadia Mono" : "Consolas",
            "serif" or "times" => "Georgia",
            "sans-serif" or "system-ui" or "system" or "-apple-system" or "helvetica" or "helvetica neue" => "Segoe UI",
            _ => requested
        };
        if (IsFamily(candidate)) return (candidate, FontStretch.Normal);
        return Named(candidate) is { } face ? (face.Family, face.Stretch) : null;
    }

    // File URLs stay inside the native renderer, never in saved styles. Each
    // Flightline variant is a separate file, so choose it after weight/slant
    // overrides, including whitespace markers and style previews, are resolved.
    internal static string RenderingFamily(string family, uint weight, FontStyle slant, FontStretch stretch)
    {
        if (!BundledFonts.Families.Contains(family)) return family;
        var face = Faces(family).Where(f => f.Source != null).MinBy(f => (
            Math.Abs((int)f.Stretch - (int)stretch),
            f.Slant == slant ? 0 : f.Slant != FontStyle.Normal && slant != FontStyle.Normal ? 1 : 2,
            Math.Abs((int)f.Weight - (int)Math.Clamp(weight, 1, 999))));
        return face == null ? family : face.Source!.AbsoluteUri + "#" + face.Family;
    }
}
