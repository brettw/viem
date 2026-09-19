using System.Globalization;
using Microsoft.Graphics.Canvas.Text;
using Windows.UI.Text;

namespace Viem.Windows.Rendering;

internal sealed record FontFace(string Name, string Family, string StyleName, ushort Weight, FontStyle Slant, FontStretch Stretch)
{
    public override string ToString() => StyleName;
}

/// <summary>Installed font discovery. Only names and normalized traits cross into document state.</summary>
internal static class FontCatalog
{
    private static readonly Lazy<FontFace[]> catalog = new(Discover);
    private static readonly Lazy<Dictionary<string, FontFace>> names = new(() => catalog.Value
        .GroupBy(f => f.Name, StringComparer.OrdinalIgnoreCase).ToDictionary(g => g.Key, g => g.First(), StringComparer.OrdinalIgnoreCase));
    internal static readonly string[] Families = CanvasTextFormat.GetSystemFontFamilies()
        .Distinct(StringComparer.OrdinalIgnoreCase).OrderBy(f => f, StringComparer.CurrentCultureIgnoreCase).ToArray();
    private static readonly HashSet<string> installedFamilies = new(Families, StringComparer.OrdinalIgnoreCase);

    private static string Localized(IReadOnlyDictionary<string, string> values) =>
        values.FirstOrDefault(p => string.Equals(p.Key, CultureInfo.CurrentUICulture.Name, StringComparison.OrdinalIgnoreCase)).Value
        ?? values.GetValueOrDefault("en-us") ?? values.Values.FirstOrDefault() ?? "";
    private static FontFace[] Discover()
    {
        using var set = CanvasFontSet.GetSystemFontSet();
        var result = new List<FontFace>();
        foreach (var font in set.Fonts)
        {
            using (font)
            {
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
    internal static FontFace? Named(string name) => names.Value.GetValueOrDefault(name);
    internal static string DisplayFamily(string name) => installedFamilies.Contains(name) ? name : Named(name)?.Family ?? name;
    internal static FontFace[] Faces(string familyOrFace)
    {
        string family = Resolve(familyOrFace)?.Family ?? familyOrFace;
        return catalog.Value.Where(f => string.Equals(f.Family, family, StringComparison.OrdinalIgnoreCase)).ToArray();
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
        string candidate = requested.ToLowerInvariant() switch {
            "monospace" or "menlo" or "monaco" => installedFamilies.Contains("Cascadia Mono") ? "Cascadia Mono" : "Consolas",
            "serif" or "times" => "Georgia",
            "sans-serif" or "system-ui" or "system" or "helvetica" or "helvetica neue" => "Segoe UI",
            _ => requested
        };
        if (installedFamilies.Contains(candidate)) return (candidate, FontStretch.Normal);
        return Named(candidate) is { } face ? (face.Family, face.Stretch) : null;
    }
}
