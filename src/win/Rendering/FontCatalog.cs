using System.Globalization;
using System.Collections.Concurrent;
using Microsoft.Graphics.Canvas.Text;
using Windows.UI.Text;

namespace Viem.Windows.Rendering;

internal sealed record FontFace(string Name, string Family, string StyleName, ushort Weight, FontStyle Slant, FontStretch Stretch)
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
    internal static void PrepareSystemFonts() { _ = systemFonts.Value; }
    private static CanvasFontSet SystemFonts => systemFonts.Value.GetAwaiter().GetResult();
    private static readonly Lazy<string[]> families = new(DiscoverFamilies);
    private static readonly ConcurrentDictionary<string, bool> installedFamilies = new(StringComparer.OrdinalIgnoreCase);
    private static readonly ConcurrentDictionary<string, FontFace?> names = new(StringComparer.OrdinalIgnoreCase);
    private static readonly ConcurrentDictionary<string, FontFace[]> familyFaces = new(StringComparer.OrdinalIgnoreCase);
    internal static string[] Families => families.Value;
    internal static bool FamilyListLoaded => families.IsValueCreated;
    private static int faceDescriptionsRead;
    internal static int FaceDescriptionsRead => Volatile.Read(ref faceDescriptionsRead);
    private static string[] DiscoverFamilies()
    {
        using var startup = Diagnostics.StartupPerformance.Measure("fonts.families");
        return CanvasTextFormat.GetSystemFontFamilies().Distinct(StringComparer.OrdinalIgnoreCase).OrderBy(f => f, StringComparer.CurrentCultureIgnoreCase).ToArray();
    }
    private static string Localized(IReadOnlyDictionary<string, string> values) =>
        values.FirstOrDefault(p => string.Equals(p.Key, CultureInfo.CurrentUICulture.Name, StringComparison.OrdinalIgnoreCase)).Value
        ?? values.GetValueOrDefault("en-us") ?? values.Values.FirstOrDefault() ?? "";
    private static CanvasFontProperty Property(CanvasFontPropertyIdentifier identifier, string value) => new() { Identifier = identifier, Value = value, Locale = "" };
    private static bool IsFamily(string name)
    {
        if (installedFamilies.TryGetValue(name, out bool known)) return known;
        var set = SystemFonts;
        return installedFamilies[name] = set.CountFontsMatchingProperty(Property(CanvasFontPropertyIdentifier.FamilyName, name)) > 0;
    }
    private static FontFace[] Discover(CanvasFontPropertyIdentifier property, string value)
    {
        using var startup = Diagnostics.StartupPerformance.Measure("fonts.matchingFaces");
        var set = SystemFonts;
        using var matching = set.GetMatchingFonts([Property(property, value)]);
        var result = new List<FontFace>();
        // Never materialize every system face to resolve a document font.
        // DirectWrite filters its indexed metadata before creating face objects.
        foreach (var font in matching.Fonts)
        {
            using (font)
            {
                Interlocked.Increment(ref faceDescriptionsRead);
                if (font.Simulations != CanvasFontSimulations.None) continue;
                string family = Localized(font.FamilyNames), style = Localized(font.FaceNames);
                string name = Localized(font.GetInformationalStrings(CanvasFontInformation.PostscriptName));
                if (name.Length == 0) name = Localized(font.GetInformationalStrings(CanvasFontInformation.FullName));
                if (name.Length > 0 && family.Length > 0) result.Add(new(name, family, style, font.Weight.Weight, font.Style, font.Stretch));
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
    internal static string DisplayFamily(string name) => name.Length == 0 || IsFamily(name) ? name : Named(name)?.Family ?? name;
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
            "monospace" or "menlo" or "monaco" => IsFamily("Cascadia Mono") ? "Cascadia Mono" : "Consolas",
            "serif" or "times" => "Georgia",
            "sans-serif" or "system-ui" or "system" or "-apple-system" or "helvetica" or "helvetica neue" => "Segoe UI",
            _ => requested
        };
        if (IsFamily(candidate)) return (candidate, FontStretch.Normal);
        return Named(candidate) is { } face ? (face.Family, face.Stretch) : null;
    }
}
