using System.Globalization;
using System.Collections.Concurrent;
using Microsoft.Graphics.Canvas.Text;
using Windows.UI.Text;

namespace Viem.Windows.Rendering;

internal sealed record FontFace(string Name, string Family, string StyleName, ushort Weight, FontStyle Slant, FontStretch Stretch, Uri? Source = null, string? TypographicFamily = null, string? TypographicStyle = null, string? NativeName = null)
{
    public string PortableFamily => string.IsNullOrEmpty(TypographicFamily) ? Family : TypographicFamily;
    public string PortableStyle => string.IsNullOrEmpty(TypographicStyle) ? StyleName : TypographicStyle;
    // NativeName is populated only when a packaged variable design has been
    // normalized, whether its selected resource is installed or app-local.
    internal bool PreferredDesign => Source != null || NativeName != null;
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
    internal sealed record FontSource(Uri Uri, CanvasFontSet Fonts);
    internal sealed record InstalledFontDesign(Uri PackagedSource, FontFace Face, FontFileMetadata Tables,
        HashSet<string> Families, HashSet<string> PreferredFamilies);
    internal sealed record FontResourceIndex(FontSource[] Sources, InstalledFontDesign[] Installed, HashSet<string> Families) : IDisposable
    {
        public void Dispose() { foreach (var source in Sources) source.Fonts.Dispose(); }
    }
    // Lazy publishes only the task. File I/O and native indexing run on its
    // worker, outside initialization/cache locks, like the system index above.
    private static readonly Lazy<Task<FontResourceIndex>> bundledFonts = new(() => Task.Run(LoadBundledFonts));
    private static FontResourceIndex BundledFonts => bundledFonts.Value.GetAwaiter().GetResult();
    private static readonly Lazy<string[]> families = new(DiscoverFamilies);
    private static readonly ConcurrentDictionary<string, bool> availableFamilies = new(StringComparer.OrdinalIgnoreCase);
    private static readonly ConcurrentDictionary<string, FontFace[]> familyFaces = new(StringComparer.OrdinalIgnoreCase);
    internal static string[] Families => families.Value;
    internal static bool FamilyListLoaded => families.IsValueCreated;
    private static int faceDescriptionsRead;
    internal static int FaceDescriptionsRead => Volatile.Read(ref faceDescriptionsRead);
    private static int installedCandidatesChecked;
    internal static int InstalledCandidatesChecked => Volatile.Read(ref installedCandidatesChecked);
    internal static int BundledFileCount => BundledFonts.Sources.Length;
    internal static int InstalledDesignCount => BundledFonts.Installed.Length;
    private static FontResourceIndex LoadBundledFonts()
    {
        string directory = Path.Combine(AppContext.BaseDirectory, "Resources", "fonts");
        return LoadFontResources(directory, Environment.GetEnvironmentVariable("VIEM_FORCE_BUNDLED_FONTS") == "1" ? null : SystemFonts);
    }
    internal static FontResourceIndex LoadFontResources(string directory, CanvasFontSet? installedFonts)
    {
        using var startup = Diagnostics.StartupPerformance.Measure("fonts.resources");
        var result = new List<FontSource>();
        var installed = new List<InstalledFontDesign>();
        var names = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        try
        {
            if (!Directory.Exists(directory)) return new([], [], names);
            foreach (string file in Directory.EnumerateFiles(directory, "*", SearchOption.AllDirectories)
                .Where(f => new[] { ".ttf", ".otf", ".ttc", ".otc" }.Contains(Path.GetExtension(f), StringComparer.OrdinalIgnoreCase))
                .OrderBy(f => f, StringComparer.Ordinal))
            {
                CanvasFontSet? fonts = null;
                try
                {
                    var uri = new Uri(Path.GetFullPath(file));
                    if (installedFonts != null && FontFileMetadata.TryRead(file) is { } metadata
                        && FindInstalledDesign(uri, metadata, installedFonts) is { } design) {
                        installed.Add(design);
                        names.UnionWith(design.Families);
                        continue;
                    }
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
        return new(result.ToArray(), installed.ToArray(), names);
    }
    private static InstalledFontDesign? FindInstalledDesign(Uri packagedSource, FontFileMetadata metadata, CanvasFontSet installedFonts)
    {
        try {
            // A variable slant design can match italic as well as upright
            // system faces, so reject conflicts in either slant for it.
            bool variableSlant = FontVariations.Parse(metadata.Fvar, metadata.Name, metadata.Stat).Axes
                .Any(a => a.Tag is "ital" or "slnt");
            int checkedCount = 0;
            foreach (var identifier in new[] { CanvasFontPropertyIdentifier.PreferredFamilyName, CanvasFontPropertyIdentifier.FamilyName }) {
                // Query only this font's family; never enumerate the system set.
                using var matching = installedFonts.GetMatchingFonts([Property(identifier, metadata.Family)]);
                var fonts = matching.Fonts;
                try {
                    if (fonts.Count > 256 - checkedCount) return null;
                    FontFace? selected = null;
                    var incompatibleSlants = new HashSet<FontStyle>();
                    foreach (var font in fonts) {
                        checkedCount++;
                        Interlocked.Increment(ref installedCandidatesChecked);
                        if (font.Simulations != CanvasFontSimulations.None) continue;
                        byte[] names = FontVariations.Table(font, "name");
                        bool same = metadata.Name.AsSpan().SequenceEqual(names)
                            && metadata.Matches(new(FontVariations.Table(font, "fvar"), names, FontVariations.Table(font, "STAT")));
                        if (!same) { incompatibleSlants.Add(font.Style); continue; }
                        if (selected != null) continue;
                        string family = Localized(font.FamilyNames), style = Localized(font.FaceNames);
                        string nativeName = English(font.GetInformationalStrings(CanvasFontInformation.PostscriptName));
                        if (family.Length == 0 || nativeName.Length == 0) continue;
                        selected = VariableDesign(new(nativeName, family, style, font.Weight.Weight, font.Style, font.Stretch), metadata);
                    }
                    // A full system collection could otherwise prefer an old
                    // static copy at a named weight. Keep the isolated bundled
                    // design whenever a competing face is not equivalent.
                    if (selected == null) continue;
                    if (variableSlant ? incompatibleSlants.Count != 0
                        : incompatibleSlants.Any(s => (s != FontStyle.Normal) == (selected.Slant != FontStyle.Normal))) return null;
                    var aliases = matching.GetPropertyValues(CanvasFontPropertyIdentifier.FamilyName,
                        CultureInfo.CurrentUICulture.Name + ";en-US").Select(p => p.Value).ToHashSet(StringComparer.OrdinalIgnoreCase);
                    var preferred = matching.GetPropertyValues(CanvasFontPropertyIdentifier.PreferredFamilyName,
                        CultureInfo.CurrentUICulture.Name + ";en-US").Select(p => p.Value).ToHashSet(StringComparer.OrdinalIgnoreCase);
                    aliases.Add(selected.Family); preferred.Add(selected.PortableFamily);
                    return new(packagedSource, selected, metadata, aliases, preferred);
                }
                finally { foreach (var font in fonts) font.Dispose(); }
            }
        }
        catch (Exception error) when (error is System.Runtime.InteropServices.COMException or ArgumentException or IndexOutOfRangeException or OverflowException)
        { System.Diagnostics.Debug.WriteLine($"Installed font {metadata.PostScriptName}: {error.Message}"); }
        return null;
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
    private static string English(IReadOnlyDictionary<string, string> values) =>
        values.FirstOrDefault(p => string.Equals(p.Key, "en-US", StringComparison.OrdinalIgnoreCase)).Value
        ?? values.Values.FirstOrDefault() ?? "";
    private static string PreferredEnglish(IReadOnlyDictionary<string, string> preferred, IReadOnlyDictionary<string, string> fallback) =>
        preferred.Count == 0 ? English(fallback) : English(preferred);
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
        // Preferred packaged designs may use a verified installed resource.
        // Filter each index first; missing-name lookup never scans every face.
        foreach (var installed in BundledFonts.Installed) {
            var names = property == CanvasFontPropertyIdentifier.FamilyName ? installed.Families : installed.PreferredFamilies;
            if (names.Contains(value)) result.Add(property == CanvasFontPropertyIdentifier.FamilyName
                ? installed.Face with { Family = value } : installed.Face);
        }
        foreach (var source in BundledFonts.Sources) ReadFaces(source.Fonts, source.Uri);
        ReadFaces(SystemFonts, null);
        void ReadFaces(CanvasFontSet set, Uri? source)
        {
            using var matching = set.GetMatchingFonts([Property(property, value)]);
            foreach (var font in matching.Fonts)
            {
                using (font)
                {
                    if (font.Simulations != CanvasFontSimulations.None) continue;
                    // Accepted designs and native bundled files already cover
                    // this filtered family's slant; do not expand their named
                    // instances into duplicate face descriptions.
                    string portableFamily = PreferredEnglish(font.GetInformationalStrings(CanvasFontInformation.PreferredFamilyNames), font.FamilyNames);
                    if (source == null && result.Any(f => f.NativeName != null && f.Slant == font.Style
                        && string.Equals(f.PortableFamily, portableFamily, StringComparison.OrdinalIgnoreCase))) continue;
                    Interlocked.Increment(ref faceDescriptionsRead);
                    string family = Localized(font.FamilyNames), style = Localized(font.FaceNames);
                    string name = English(font.GetInformationalStrings(CanvasFontInformation.PostscriptName));
                    if (name.Length == 0) name = English(font.GetInformationalStrings(CanvasFontInformation.FullName));
                    if (name.Length > 0 && family.Length > 0) {
                        string portableStyle = PreferredEnglish(font.GetInformationalStrings(CanvasFontInformation.PreferredSubfamilyNames), font.FaceNames);
                        // System faces retain their native named-instance catalogue;
                        // normalize only app-local variable files into their designs.
                        var face = new FontFace(name, family, style, font.Weight.Weight, font.Style, font.Stretch, source, portableFamily, portableStyle);
                        result.Add(source == null ? face : BundledVariableDesign(face, font));
                    }
                }
            }
        }
        return result.DistinctBy(f => f.Name, StringComparer.OrdinalIgnoreCase).OrderBy(f => f.Weight)
            .ThenBy(f => f.Slant).ThenBy(f => f.StyleName, StringComparer.CurrentCultureIgnoreCase).ThenBy(f => f.Name, StringComparer.Ordinal).ToArray();
    }
    private static FontFace BundledVariableDesign(FontFace face, CanvasFontFace font)
    {
        try {
            byte[] fvar = FontVariations.Table(font, "fvar");
            if (fvar.Length == 0) return face;
            byte[] names = FontVariations.Table(font, "name");
            return VariableDesign(face, new(fvar, names, []));
        }
        catch (Exception error) when (error is System.Runtime.InteropServices.COMException or ArgumentException or IndexOutOfRangeException or OverflowException)
        { System.Diagnostics.Debug.WriteLine($"Font design {face.Name}: {error.Message}"); return face; }
    }
    private static FontFace VariableDesign(FontFace face, FontFileMetadata tables)
    {
        var info = FontVariations.Parse(tables.Fvar, tables.Name, []);
        if (info.Axes.Length == 0) return face;
        // DirectWrite's WWS projections synthesize names and weights from
        // STAT. Separate upright/italic files can receive the same name.
        // Keep original design identity and portable names separately from
        // the native lookup, and offer fvar instances in the picker.
        byte[] names = tables.Name;
        string style = FontVariations.Name(names, 17, FontVariations.Name(names, 2, face.PortableStyle));
        var weight = info.Axes.FirstOrDefault(a => a.Tag == "wght");
        return face with {
            Name = FontVariations.Name(names, 6, face.Name), NativeName = face.Name,
            TypographicFamily = FontVariations.Name(names, 16, FontVariations.Name(names, 1, face.PortableFamily)),
            StyleName = style, TypographicStyle = style,
            Weight = weight == null ? face.Weight : (ushort)Math.Clamp(Math.Round(weight.Default), 1, 1000),
        };
    }
    internal static CanvasFontSet VariationFontSet(FontFace face) => face.Source == null ? SystemFonts
        : BundledFonts.Sources.Single(s => s.Uri == face.Source).Fonts;
    internal static (byte[] Fvar, byte[] Name, byte[] Stat) VariationTables(FontFace face)
    {
        if (face.Source == null && face.NativeName != null
            && BundledFonts.Installed.FirstOrDefault(d => d.Face.Name == face.Name) is { } installed)
            return (installed.Tables.Fvar, installed.Tables.Name, installed.Tables.Stat);
        var sets = face.Source == null ? new[] { SystemFonts } : BundledFonts.Sources.Where(s => s.Uri == face.Source).Select(s => s.Fonts).ToArray();
        foreach (var set in sets) {
            using var matching = set.GetMatchingFonts([Property(CanvasFontPropertyIdentifier.PostscriptName, face.NativeName ?? face.Name)]);
            foreach (var font in matching.Fonts) using (font) {
                byte[] fvar = FontVariations.Table(font, "fvar");
                if (fvar.Length != 0) return (fvar, FontVariations.Table(font, "name"), FontVariations.Table(font, "STAT"));
            }
        }
        return ([], [], []);
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
        name;
    /// Reverses <see cref="DisplayFamily"/> for the picker's own two entries;
    /// any other label (an installed font name typed or picked) yields null.
    internal static string? StorageFamily(string displayName) =>
        string.Equals(displayName, SystemDefaultDisplayName, StringComparison.OrdinalIgnoreCase) ? SystemDefaultFamily :
        string.Equals(displayName, SystemMonospaceDisplayName, StringComparison.OrdinalIgnoreCase) ? SystemMonospaceFamily :
        null;
    internal static FontFace[] Faces(string requestedFamily)
    {
        if (requestedFamily.Length == 0) return [];
        var direct = familyFaces.GetOrAdd(requestedFamily, DiscoverFamily);
        if (direct.Length != 0) return direct;
        string family = Resolve(requestedFamily)?.Family ?? requestedFamily;
        return familyFaces[requestedFamily] = string.Equals(family, requestedFamily, StringComparison.OrdinalIgnoreCase)
            ? [] : Faces(family);
    }
    private static FontFace[] DiscoverFamily(string requested)
    {
        var direct = Discover(CanvasFontPropertyIdentifier.FamilyName, requested)
            .Concat(Discover(CanvasFontPropertyIdentifier.PreferredFamilyName, requested))
            .DistinctBy(f => f.Name, StringComparer.OrdinalIgnoreCase).OrderBy(f => f.Weight)
            .ThenBy(f => f.Slant).ThenBy(f => f.StyleName, StringComparer.CurrentCultureIgnoreCase)
            .ThenBy(f => f.Name, StringComparer.Ordinal).ToArray();
        // Match the Mac catalogue: installed static copies must not hide the
        // packaged variable design or duplicate its presets. Retain installed
        // alternatives when that slant has no packaged variable resource.
        var bundledDesigns = direct.Where(f => f.PreferredDesign && FontVariations.For(f).Axes.Length != 0)
            .Select(f => (f.PortableFamily.ToUpperInvariant(), f.Slant)).ToHashSet();
        return direct.Where(f => f.PreferredDesign || !bundledDesigns.Contains((f.PortableFamily.ToUpperInvariant(), f.Slant))).ToArray();
    }
    internal static FontFace? Match(string family, string subfamily)
    {
        if (subfamily.Length == 0) return null;
        var faces = Faces(family);
        FontFace? Find(IEnumerable<FontFace> candidates)
        {
            var exact = candidates.FirstOrDefault(f => string.Equals(f.PortableStyle, subfamily, StringComparison.OrdinalIgnoreCase))
                ?? candidates.FirstOrDefault(f => string.Equals(f.StyleName, subfamily, StringComparison.OrdinalIgnoreCase));
            // DirectWrite need not enumerate named variable instances as faces.
            return exact ?? candidates.FirstOrDefault(f => FontVariations.For(f).Instances.Any(i =>
                string.Equals(i.Name, subfamily, StringComparison.OrdinalIgnoreCase)));
        }
        // A packaged variable instance wins over an older installed static face.
        return Find(faces.Where(f => f.PreferredDesign)) ?? Find(faces.Where(f => !f.PreferredDesign));
    }
    internal static Dictionary<string, float> NamedCoordinates(FontFace? face, string subfamily)
    {
        var instance = FontVariations.For(face).Instances.FirstOrDefault(i =>
            string.Equals(i.Name, subfamily, StringComparison.OrdinalIgnoreCase));
        // A named design supplies defaults, but base weight is independent.
        return instance?.Values.Where(p => p.Key != "wght").ToDictionary(p => p.Key, p => p.Value) ?? [];
    }
    internal static FontFace? Current(string name, uint weight, uint slant)
    {
        var matching = Faces(name).Where(f => (f.Slant != FontStyle.Normal) == (slant != 0)).ToArray();
        return matching.FirstOrDefault(f => f.Weight == weight)
            ?? matching.FirstOrDefault(f => FontVariations.For(f).Axes.Any(a => a.Tag == "wght"));
    }
    internal static FontFace? ForFamilyChange(string requested, FontFace? current)
    {
        var faces = Faces(requested);
        if (current != null && faces.FirstOrDefault(f => string.Equals(f.StyleName, current.StyleName, StringComparison.OrdinalIgnoreCase)) is { } corresponding) return corresponding;
        foreach (string regular in new[] { "Regular", "Normal", "Roman", "Book" })
            if (faces.FirstOrDefault(f => f.Slant == FontStyle.Normal && string.Equals(f.StyleName, regular, StringComparison.OrdinalIgnoreCase)) is { } face) return face;
        return faces.FirstOrDefault();
    }
    internal static (string Family, FontStretch Stretch)? Resolve(string requested, string subfamily = "")
    {
        if (requested.Length == 0) return null;
        if (subfamily.Length != 0 && Match(requested, subfamily) is { } selected) return (selected.Family, selected.Stretch);
        string candidate = requested.ToLowerInvariant() switch {
            "monospace" or "ui-monospace" or "menlo" or "monaco" => IsFamily("Cascadia Mono") ? "Cascadia Mono" : "Consolas",
            "serif" or "times" => "Georgia",
            "sans-serif" or "system-ui" or "system" or "-apple-system" or "helvetica" or "helvetica neue" => "Segoe UI",
            _ => requested
        };
        if (IsFamily(candidate)) return (candidate, FontStretch.Normal);
        var preferred = familyFaces.GetOrAdd(candidate, DiscoverFamily);
        return preferred.FirstOrDefault() is { } face ? (face.Family, face.Stretch) : null;
    }

    // File URLs stay inside the native renderer, never in saved styles.
    // Flightline's upright and italic designs are separate variable files;
    // choose the source after weight/slant overrides are resolved, including
    // whitespace markers and style previews.
    internal static string RenderingFamily(string family, uint weight, FontStyle slant, FontStretch stretch, FontFace? selected = null)
    {
        if (!BundledFonts.Families.Contains(family)) return family;
        var face = selected ?? RenderingFace(family, weight, slant, stretch);
        return face?.Source == null ? family : face.Source.AbsoluteUri + "#" + face.Family;
    }
    internal static FontFace? RenderingFace(string family, uint weight, FontStyle slant, FontStretch stretch, bool discoverSystem = false)
    {
        // Ordinary system-family rendering is handled by DirectWrite. Only
        // explicit face/axis requests need native variation-table discovery.
        if (!discoverSystem && !BundledFonts.Families.Contains(family)) return null;
        var faces = Faces(family);
        var available = BundledFonts.Families.Contains(family) ? faces.Where(f => f.PreferredDesign) : faces;
        return ChooseRenderingFace(available, weight, slant, stretch);
    }
    internal static FontFace? ChooseRenderingFace(IEnumerable<FontFace> available, uint weight, FontStyle slant, FontStretch stretch)
    {
        return available.MinBy(f => (
            Math.Abs((int)f.Stretch - (int)stretch),
            f.Slant == slant ? 0 : f.Slant != FontStyle.Normal && slant != FontStyle.Normal ? 1 : 2,
            Math.Abs((int)f.Weight - (int)Math.Clamp(weight, 1, 999))));
    }
}
