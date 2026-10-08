#if DEBUG
using Microsoft.UI.Xaml.Controls;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using System.Numerics;
using System.Runtime.InteropServices;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Rendering;
using Viem.Windows.Shell;
using Windows.UI.Text;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;
internal static class VariableFontTests
{
    private static void Check(bool ok, string name) { if (!ok) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
    internal static async Task Run(EditorPane pane, Preferences preferences)
    {
        InstalledFontTests.Run(pane.Canvas.Device);
        FlightlinePortableChecks(pane);
        var packagedFace = FontCatalog.ForFamilyChange("Flightline Code", null)!;
        using (var format = new Microsoft.Graphics.Canvas.Text.CanvasTextFormat { FontFamily = FontCatalog.RenderingFamily(packagedFace.Family, packagedFace.Weight, packagedFace.Slant, packagedFace.Stretch), FontSize = 14 })
        using (var layout = new Microsoft.Graphics.Canvas.Text.CanvasTextLayout(pane.Canvas.Device, "Packaged fonts", format, 1000, 200)) {
            FontVariations.Apply(layout, 0, 14, new() { ["wght"] = packagedFace.Weight }, packagedFace);
            var capture = new GlyphCapture(f => new GlyphFontMetadata(f)); DirectWriteGlyphCapture.Draw(layout, capture);
            var parts = capture.Extract(0, 14, 0, 0);
            Check(parts.Count > 0, "typographic font selection preserves the requested design's glyphs");
            Check(FontVariations.Name(FontVariations.Table(parts.First().Font, "name"), 6, "") == packagedFace.Name, "typographic selection retains the requested original font design");
        }
        RecursiveFontChecks(pane);
        EquivalentGlyphChecks(pane.Canvas.Device, FontCatalog.Match("Recursive", "Mono Casual Light")!, "recursive/Recursive_VF_1.085.ttf");
        foreach (var design in FontCatalog.Faces("Flightline Code"))
            EquivalentGlyphChecks(pane.Canvas.Device, design, "flightline/FlightlineCode-" + (design.Slant == FontStyle.Normal ? "Regular" : "Italic") + "-VF.ttf");
        await RecursiveInspectorChecks(pane, preferences);
        FlightlineFontChecks(pane);
        await FlightlineInspectorChecks(pane, preferences);
        await FlightlineSavedInspectorChecks(pane, preferences);
        await StyleAndSettingsTests.FontChecks(pane, preferences);
        await SegoeVariable(pane, preferences, VIEM_FORMAT_MARKDOWN);
        await SegoeVariable(pane, preferences, VIEM_FORMAT_PLAIN_TEXT);
        var face = FontCatalog.Faces("Bahnschrift").FirstOrDefault() ?? throw new InvalidOperationException("Variable font regression requires the Windows Bahnschrift font.");
        var info = FontVariations.For(face);
        Check(info.Axes.Any(a => a.Tag == "wght") && info.Axes.Any(a => a.Tag == "wdth"), "native fvar discovery returns Bahnschrift weight and width");
        Check(info.Instances.Length > 1, "native fvar discovery returns named instances");
        var saved = info.Defaults; saved["wght"] = 450.25f;
        var effective = FontVariations.Effective(info, saved, 750, true, 0);
        Check(effective["wght"] == 700 && saved["wght"] == 450.25f, "relative bold clamps at font maximum without changing saved coordinates");
        var slantInfo = new FontVariationInfo([new("slnt", "Slant", -20, 0, 0, false)], [], []);
        Check(FontVariations.Effective(slantInfo, [], 400, false, 1)["slnt"] == -12 && FontVariations.Effective(slantInfo, new() { ["slnt"] = -18 }, 400, false, 1)["slnt"] == -18, "oblique targets minus twelve degrees and preserves stronger slant");
        var linked = new FontVariationInfo([new("wght", "Weight", 100, 400, 900, false)], [], new() { ["wght"] = [new(300, 550), new(400, 650)] });
        Check(FontVariations.Effective(linked, new() { ["wght"] = 300 }, 600, true, 0)["wght"] == 550 && FontVariations.Effective(linked, new() { ["wght"] = 400 }, 700, true, 0)["wght"] == 650, "each STAT weight link applies to its matching base coordinate");
        using var doc = new CoreDocument("Variable fonts MMMM"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(doc, pane.Canvas.Device, pane.Canvas.DispatcherQueue, 1000, 300);
        var style = view.Styles().Styles.Single(s => s.Id == "Paragraph");
        saved = info.Defaults; saved["wdth"] = 100;
        view.EditStyleFont(style, [face.PortableFamily], face, saved);
        var wide = view.Layout();
        saved["wdth"] = 75;
        view.EditStyleFont(style, [face.PortableFamily], face, saved);
        var narrow = view.Layout();
        Check(narrow.Clusters.Sum(c => c.advance) < wide.Clusters.Sum(c => c.advance) * .98, "DirectWrite axis values change measured and rendered glyph advances");
        Check(!CoreView.SameLayout(wide.Info.identity, narrow.Info.identity), "axis edits invalidate the shaping cache");
        var exported = System.Text.Encoding.UTF8.GetString(view.ExportStyleDefaults());
        Check(exported.Contains("font_axes") && exported.Contains("75"), "stylesheet export retains numeric axis coordinates");
        var original = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
        preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, view.ExportStyleDefaults());
        var inspector = new StyleWindow(view, preferences); inspector.Activate();
        try {
            await Task.Delay(100);
            var sliders = Descendants<Slider>(inspector.RootControl).ToArray();
            Check(sliders.Length == 2, "two variable axes occupy one inspector row");
            var slider = sliders.Single(s => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(s) == "Width");
            Check(Math.Abs(slider.Value - 75) < .001, "reopening the stylesheet restores the saved axis coordinates");
            slider.Value = 90;
            Check(inspector.FontVariantControl.SelectedItem as string == "Custom", "moving an axis selects Custom when no named instance matches");
            inspector.UndoThemeForTesting();
            slider = Descendants<Slider>(inspector.RootControl).Single(s => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(s) == "Width");
            Check(Math.Abs(slider.Value - 75) < .001, "undo restores the previous saved axis coordinate");
            inspector.RedoThemeForTesting();
            slider = Descendants<Slider>(inspector.RootControl).Single(s => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(s) == "Width");
            Check(Math.Abs(slider.Value - 90) < .001, "redo restores the edited axis coordinate");
            var preset = ((IEnumerable<object>)inspector.FontVariantControl.ItemsSource).OfType<FontInstance>().First();
            inspector.FontVariantControl.SelectedItem = preset;
            Check(Math.Abs(slider.Value - preset.Values["wdth"]) < .001, "selecting a named preset synchronizes axis sliders");
            Check(!Descendants<CheckBox>(inspector.RootControl).Any(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Declare Variant"), "font face has one inheritance checkbox and no variant checkbox");
            await Task.Delay(300);
            await WindowCapture.Save(WinRT.Interop.WindowNative.GetWindowHandle(inspector), pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".variable-font.png");
        } finally { inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, original); }
    }

    internal static void SourceChecks(FontFace face, string relativePath)
    {
        string expected = Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "Resources/fonts", relativePath));
        Check(File.Exists(expected), $"{face.Name} retains its packaged fallback file");
        if (Environment.GetEnvironmentVariable("VIEM_FORCE_BUNDLED_FONTS") == "1")
            Check(face.Source is { IsFile: true }, $"forcing bundled fonts selects a file for {face.Name}");
        string rendering = FontCatalog.RenderingFamily(face.Family, face.Weight, face.Slant, face.Stretch, face);
        if (face.Source == null)
            Check(rendering == face.Family, $"{face.Name} renders through the installed family without a file URI");
        else {
            Check(face.Source.IsFile && string.Equals(face.Source.LocalPath, expected, StringComparison.OrdinalIgnoreCase), $"{face.Name} selects its original packaged file");
            Check(rendering == face.Source.AbsoluteUri + "#" + face.Family, $"{face.Name} selects its bundled variant URI");
        }
    }

    private sealed record FontShapeRun(string Name, Dictionary<string, float> Axes, CanvasGlyph[] Glyphs, float Size, uint Bidi, Vector2 Offset);
    private sealed record FontShape(double Width, double Height, FontShapeRun[] Runs, byte[] Pixels);

    internal static void EquivalentGlyphChecks(CanvasDevice device, FontFace selected, string relativePath)
    {
        var source = new Uri(Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "Resources/fonts", relativePath)));
        using var bundled = new CanvasFontSet(source);
        var info = FontVariations.For(selected);
        var coordinates = selected.PortableFamily == "Flightline Code"
            ? new[] { 250f, 400f, 437f, 700f }.Select(weight => new Dictionary<string, float> { ["wght"] = weight })
            : info.Instances.Where(i => i.Name != "Default").Where((_, index) => index % 10 == 0).Select(i => i.Values).Append(info.Defaults);
        foreach (var axes in coordinates) {
            var expected = Shape(selected, axes, bundled);
            var actual = Shape(selected, axes, null);
            Check(expected.Width == actual.Width && expected.Height == actual.Height && expected.Runs.Length > 0
                && expected.Runs.All(run => run.Name == selected.Name)
                && expected.Runs.Length == actual.Runs.Length && expected.Runs.Zip(actual.Runs).All(pair =>
                    pair.First.Name == pair.Second.Name && pair.First.Size == pair.Second.Size && pair.First.Bidi == pair.Second.Bidi
                    && pair.First.Offset == pair.Second.Offset && pair.First.Glyphs.SequenceEqual(pair.Second.Glyphs)
                    && pair.First.Axes.Count == pair.Second.Axes.Count
                    && pair.First.Axes.All(axis => pair.Second.Axes.TryGetValue(axis.Key, out float value) && value == axis.Value)),
                $"{selected.Name} selected and bundled sources produce identical native glyphs, axes, offsets and bounds ({string.Join(", ", axes.Select(a => $"{a.Key}={a.Value}"))})");
            Check(expected.Pixels.Any(value => value != 255) && expected.Pixels.AsSpan().SequenceEqual(actual.Pixels), $"{selected.Name} selected and bundled sources draw identical nonempty pixels");
        }
        FontShape Shape(FontFace face, Dictionary<string, float> axes, CanvasFontSet? reference)
        {
            const string text = "Writing MMMM iii 0123 — AV fi fl";
            using var format = new CanvasTextFormat {
                FontFamily = reference == null ? FontCatalog.RenderingFamily(face.Family, face.Weight, face.Slant, face.Stretch, face) : source.AbsoluteUri + "#" + face.Family,
                FontSize = 16, FontWeight = new FontWeight { Weight = (ushort)Math.Clamp(axes.GetValueOrDefault("wght", 400), 1, 999) },
                FontStyle = face.Slant, FontStretch = face.Stretch,
            };
            using var layout = new CanvasTextLayout(device, text, format, 2000, 300);
            if (reference != null) UseReferenceCollection(layout, reference, face.PortableFamily, text.Length);
            FontVariations.Apply(layout, 0, text.Length, axes, reference == null ? face : null);
            var fonts = new List<CanvasFontFace>();
            var capture = new GlyphCapture(font => { fonts.Add(font); return new GlyphFontMetadata(font); });
            try {
                DirectWriteGlyphCapture.Draw(layout, capture);
                var parts = capture.Extract(0, text.Length, 0, 0);
                using var surface = new CanvasRenderTarget(device, 800, 100, 96);
                using (var drawing = surface.CreateDrawingSession()) {
                    drawing.Clear(Microsoft.UI.Colors.White);
                    using var brush = new Microsoft.Graphics.Canvas.Brushes.CanvasSolidColorBrush(drawing, Microsoft.UI.Colors.Black);
                    foreach (var part in parts)
                        drawing.DrawGlyphRun(part.Offset + new Vector2(8, 8), part.Font, part.Size, part.Glyphs, false, part.BidiLevel, brush);
                }
                return new(layout.LayoutBounds.Width, layout.LayoutBounds.Height, parts.Select(part => new FontShapeRun(
                    FontVariations.Name(FontVariations.Table(part.Font, "name"), 6, ""), FontVariations.NativeAxes(part.Font),
                    part.Glyphs, part.Size, part.BidiLevel, part.Offset)).ToArray(), surface.GetPixelBytes());
            } finally { foreach (var font in fonts.Distinct()) font.Dispose(); }
        }
    }

    [DllImport("dwrite.dll")] private static extern int DWriteCreateFactory(uint kind, in Guid id, out nint factory);
    [StructLayout(LayoutKind.Sequential)] private struct TextRange { public uint Start, Length; }
    private static unsafe void UseReferenceCollection(CanvasTextLayout layout, CanvasFontSet fonts, string family, int length)
    {
        // Build the reference from the packaged resource even when production
        // skipped it. Keep this collection local to the test and its layout.
        Guid id = new("F3744D80-21F7-42EB-B35D-995BC72FC223");
        nint factory = 0, set = 0, collection = 0, native = 0;
        try {
            Marshal.ThrowExceptionForHR(DWriteCreateFactory(0, in id, out factory));
            set = FontVariations.NativeResource(fonts, new("53585141-D9F8-4095-8321-D73CF6BD116B"));
            Check(set != 0, "the packaged reference exposes its native font set");
            Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, nint, uint, nint*, int>)(*(nint**)factory)[52])(factory, set, 0, &collection));
            native = FontVariations.NativeResource(layout, new("05A9BF42-223F-4441-B5FB-8263685F55E9"));
            Check(native != 0, "the packaged reference exposes its native text layout");
            var range = new TextRange { Length = (uint)length };
            Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, nint, TextRange, int>)(*(nint**)native)[30])(native, collection, range));
            fixed (char* name = family)
                Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, char*, TextRange, int>)(*(nint**)native)[31])(native, name, range));
        } finally {
            if (native != 0) Marshal.Release(native);
            if (collection != 0) Marshal.Release(collection);
            if (set != 0) Marshal.Release(set);
            if (factory != 0) Marshal.Release(factory);
            GC.KeepAlive(layout); GC.KeepAlive(fonts);
        }
    }

    internal static void FlightlineFontChecks(EditorPane pane)
    {
        var faces = FontCatalog.Faces("Flightline Code");
        string[] files = ["FlightlineCode-Regular-VF.ttf", "FlightlineCode-Italic-VF.ttf"];
        Check(Directory.EnumerateFiles(Path.Combine(AppContext.BaseDirectory, "Resources/fonts/flightline"), "*.ttf")
            .Select(Path.GetFileName).ToHashSet().SetEquals(files), "Flightline packaging contains two variable fonts and no static files");
        byte[] source = "Writing MMMM iii 0123"u8.ToArray();
        using var doc = new CoreDocument(source, format: VIEM_FORMAT_PLAIN_TEXT);
        using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 1000, 300);
        foreach (string file in files) {
            bool italic = file.Contains("Italic", StringComparison.Ordinal);
            var face = faces.Single(f => f.Name == (italic ? "FlightlineCode-NormalItalic" : "FlightlineCode-Normal"));
            SourceChecks(face, "flightline/" + file);
            var info = FontVariations.For(face);
            Check(info.Axes.Length == 1 && info.Axes[0] is { Tag: "wght", Minimum: 200, Default: 400, Maximum: 700 }, "Flightline exposes its 200–700 weight axis");
            var instances = info.Instances.Where(i => i.Name != "Default").ToArray();
            Check(instances.Length == 6, "each Flightline design exposes six named presets");
            foreach (float weight in instances.Select(i => i.Values["wght"]).Append(437).Distinct()) {
                view.EditStyleFont(view.Styles().Styles.Single(s => s.Id == "Paragraph"), [face.PortableFamily, "serif"], face, new() { ["wght"] = weight });
                var layout = view.Layout();
                var run = layout.Clusters.First().render_run.identifier;
                Check(Math.Abs(view.Provider.RenderedFontAxes(run).First()["wght"] - weight) < .001f, $"Flightline {file} renders weight {weight}");
                Check(view.Provider.RenderedFontNames(run).Any(n => n.StartsWith("FlightlineCode-Normal", StringComparison.Ordinal) && n.Contains("Italic", StringComparison.Ordinal) == italic), "Flightline selects the designed upright or italic outlines");
            }
        }
        var upright = faces.Single(f => f.Slant == FontStyle.Normal);
        var paragraph = view.Styles().Styles.Single(s => s.Id == "Paragraph");
        view.EditStyleFont(paragraph, [upright.PortableFamily], upright, new() { ["wght"] = 437 });
        void Emphasis(bool bold, uint slant, float weight) {
            var style = view.Styles().Styles.Single(s => s.Id == "Paragraph");
            view.EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_BOLD, CoreView.Enum(VIEM_STYLE_VALUE_BOOLEAN, bold ? 1u : 0u));
            view.EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_SLANT, CoreView.Enum(VIEM_STYLE_VALUE_FONT_SLANT, slant));
            var run = view.Layout().Clusters.First().render_run.identifier;
            Check(Math.Abs(view.Provider.RenderedFontAxes(run).First()["wght"] - weight) < .001f, "Flightline emphasis uses clamped effective weight and restores its saved base weight");
            Check(view.Provider.RenderedFontNames(run).Any(n => n.StartsWith("FlightlineCode-Normal", StringComparison.Ordinal) && n.Contains("Italic", StringComparison.Ordinal) == (slant != 0)), "Flightline emphasis switches the actual variable font collection between upright and italic");
        }
        Emphasis(true, 0, 700); Emphasis(true, 1, 700); Emphasis(false, 1, 437); Emphasis(false, 0, 437);
        Check(FontCatalog.Resolve("FlightlineCode-does-not-exist") == null, "unknown Flightline names remain unavailable");
        Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(source), "Flightline font changes preserve document source");
        Check(!System.Text.Encoding.UTF8.GetString(view.ExportStyleDefaults()).Contains("file:", StringComparison.OrdinalIgnoreCase), "Flightline styles retain portable font requests");
    }

    private static async Task FlightlineInspectorChecks(EditorPane pane, Preferences preferences)
    {
        byte[] original = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
        byte[] source = "Flightline variable controls"u8.ToArray();
        using var doc = new CoreDocument(source, format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 1000, 300);
        var upright = FontCatalog.Faces("Flightline Code").Single(f => f.Slant == FontStyle.Normal);
        view.EditStyleFont(view.Styles().Styles.Single(s => s.Id == "Paragraph"), [upright.PortableFamily], upright);
        preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, view.ExportStyleDefaults());
        var inspector = new StyleWindow(view, preferences); inspector.Activate();
        try {
            await Task.Delay(100);
            foreach (bool italic in new[] { false, true }) {
                inspector.FontVariantControl.SelectedItem = FontCatalog.Faces("Flightline Code")
                    .Single(f => (f.Slant != FontStyle.Normal) == italic);
                var slider = Descendants<Slider>(inspector.RootControl).Single();
                Check(slider.Minimum == 200 && slider.Maximum == 700 && slider.StepFrequency == 1, "Flightline's inspector weight control covers the design range in integer steps");
                var presets = ((IEnumerable<object>)inspector.FontVariantControl.ItemsSource).OfType<FontInstance>().ToArray();
                Check(presets.Length == 7, "Flightline's picker exposes Default and all six named presets");
                foreach (var preset in presets) {
                    inspector.FontVariantControl.SelectedItem = preset;
                    Check(Math.Abs(slider.Value - preset.Values["wght"]) < .001, "Flightline preset selection synchronizes its weight slider");
                }
                slider.Value = 437;
                Check(inspector.FontVariantControl.SelectedItem as string == "Custom", "intermediate Flightline weights display Custom");
                Check(inspector.Error.Length == 0, "Flightline controls apply without inspector errors");
                inspector.UndoThemeForTesting();
                slider = Descendants<Slider>(inspector.RootControl).Single();
                Check(Math.Abs(slider.Value - presets.Last().Values["wght"]) < .001, "Flightline slider undo restores the previous preset weight");
                inspector.RedoThemeForTesting();
                slider = Descendants<Slider>(inspector.RootControl).Single();
                Check(slider.Value == 437, "Flightline slider redo restores the custom weight");
                using var renderedDoc = new CoreDocument(source, format: VIEM_FORMAT_MARKDOWN);
                renderedDoc.InitializeStyleDefaults(inspector.ThemeView.ExportStyleDefaults());
                using var renderedView = new CoreView(renderedDoc, pane.Canvas.Device, pane.DispatcherQueue, 1000, 300);
                var run = renderedView.Layout().Clusters.First().render_run.identifier;
                Check(Math.Abs(renderedView.Provider.RenderedFontAxes(run).First()["wght"] - 437) < .001f, "saved Flightline inspector edits reach native shaping");
                Check(renderedView.Provider.RenderedFontNames(run).Any(n => n.StartsWith("FlightlineCode-Normal", StringComparison.Ordinal) && n.Contains("Italic", StringComparison.Ordinal) == italic), "Flightline inspector retains its separate italic design");
            }
            Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(source), "Flightline inspector leaves Markdown source unchanged");
        } finally { inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, original); }
    }

    private static byte[] FlightlineStyles(string subfamily, float weight) => System.Text.Encoding.UTF8.GetBytes($$$"""
        {"version":1,"block_styles":[{"id":"Paragraph","name":"Base Paragraph","role":"Paragraph",
          "based_on":null,"next_paragraph_style":null,"block":{},"character":{
            "font_families":["Flightline Code","serif"],"font_face":"{{{subfamily}}}",
            "font_axes":{"wght":{{{weight}}}},"weight":{{{weight}}},"slant":"Upright"}}]}
        """);

    private static void FlightlinePortableChecks(EditorPane pane)
    {
        // Resolve before opening the picker: discovery must not depend on which
        // path populated the catalogue first, nor on installed static copies.
        Check(FontCatalog.Resolve("Flightline Code", "Normal")?.Family == "Flightline Code", "portable Flightline family resolves before picker discovery");
        var faces = FontCatalog.Faces("Flightline Code");
        Check(faces.Length == 2 && faces.Select(f => f.Name).ToHashSet().SetEquals(["FlightlineCode-Normal", "FlightlineCode-NormalItalic"]), "Flightline retains both original variable designs without duplicate installed static faces");
        var normal = FontCatalog.Match("Flightline Code", "Normal");
        var italic = FontCatalog.Match("Flightline Code", "Normal Italic");
        Check(normal is { Slant: FontStyle.Normal, Weight: 400 }
            && italic is { Slant: FontStyle.Italic, Weight: 400 }, "portable Flightline Normal and Normal Italic identify different designs at the default weight");
        Check(FontCatalog.Current("Flightline Code", 250, 0) == normal, "a custom variable weight without a subfamily retains the upright catalogue and sliders");
        foreach (var face in faces) {
            var info = FontVariations.For(face);
            bool slanted = face.Slant != FontStyle.Normal;
            var requests = info.Instances.Where(i => i.Name != "Default").Select(i => (i.Name, Weight: i.Values["wght"]))
                .Append((face.PortableStyle, 250f));
            foreach (var (subfamily, weight) in requests) {
                Check(FontCatalog.Match("Flightline Code", subfamily) == face, $"portable Flightline {subfamily} resolves its own design");
                byte[] source = "Saved font request"u8.ToArray();
                using var doc = new CoreDocument(source, format: VIEM_FORMAT_PLAIN_TEXT);
                Check(doc.InitializeStyleDefaults(FlightlineStyles(subfamily, weight)).Length == 0, "saved Flightline family, subfamily and weight load without diagnostics");
                using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 1000, 300);
                byte[] saved = view.ExportStyleDefaults();
                var run = view.Layout().Clusters.First().render_run.identifier;
                Check(view.Provider.RenderedFontNames(run).Contains(slanted ? "FlightlineCode-NormalItalic" : "FlightlineCode-Normal")
                    && Math.Abs(view.Provider.RenderedFontAxes(run).First()["wght"] - weight) < .001f,
                    $"saved Flightline {subfamily} renders its design and weight {weight} without a picker edit");
                Check(saved.AsSpan().SequenceEqual(view.ExportStyleDefaults()) && source.AsSpan().SequenceEqual(doc.Source(doc.State.document_revision)), "resolving a saved Flightline request changes neither source nor style declarations");
            }
        }
        // Exercise the shipped theme itself without pinning its numeric values.
        using var theme = System.Text.Json.JsonDocument.Parse(CoreThemes.Defaults());
        var markdown = theme.RootElement.GetProperty("styles").GetProperty("markdown");
        using var midnightDoc = new CoreDocument("`Midnight code`"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        midnightDoc.InitializeStyleDefaults(System.Text.Encoding.UTF8.GetBytes(markdown.GetRawText()));
        using var midnight = new CoreView(midnightDoc, pane.Canvas.Device, pane.DispatcherQueue, 1000, 300);
        midnight.SetMarkdownSource(false);
        var midnightRun = midnight.Layout().Clusters.First().render_run.identifier;
        Check(midnight.Provider.RenderedFontNames(midnightRun).Contains("FlightlineCode-Normal"), "Midnight inline Code renders the upright Flightline design");
    }

    private static async Task FlightlineSavedInspectorChecks(EditorPane pane, Preferences preferences)
    {
        byte[] original = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
        try {
            foreach (string subfamily in new[] { "Normal", "Normal Italic", "Light", "Light Italic", "" }) {
                using var doc = new CoreDocument("Saved Flightline inspector"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
                byte[] styles = FlightlineStyles(subfamily, 250);
                doc.InitializeStyleDefaults(styles);
                using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 1000, 300);
                preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, styles);
                byte[] before = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
                var inspector = new StyleWindow(view, preferences); inspector.Activate();
                try {
                    await Task.Delay(50);
                    var slider = Descendants<Slider>(inspector.RootControl).Single();
                    bool italic = subfamily.Contains("Italic", StringComparison.Ordinal);
                    var presets = inspector.FontVariantControl.Items.OfType<FontInstance>().ToArray();
                    Check(slider.Value == 250 && inspector.FontVariantControl.SelectedItem as string == "Custom"
                        && presets.Length == 7 && presets.Where(p => p.Name != "Default").All(p => p.Name.Contains("Italic", StringComparison.Ordinal) == italic),
                        "opening a saved Flightline style populates matching presets and the custom weight without changing design");
                    inspector.RefreshForTesting();
                    Check(before.AsSpan().SequenceEqual(preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN)), "opening and refreshing Flightline controls do not rewrite the saved request");
                    inspector.FontVariantControl.SelectedItem = presets.Single(p => p.Name == (italic ? "Light Italic" : "Light"));
                    Check(slider.Value == 350 && inspector.FontVariantControl.SelectedItem is FontInstance { Name: "Light" or "Light Italic" }, "a saved Flightline style selects its matching Light preset consistently");
                    inspector.UndoThemeForTesting();
                    Check(Descendants<Slider>(inspector.RootControl).Single().Value == 250 && inspector.FontVariantControl.SelectedItem as string == "Custom", "undo restores the saved custom Flightline weight and picker state");
                } finally { inspector.Close(); }
            }
        } finally { preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, original); }
    }

    internal static void RecursiveFontChecks(EditorPane pane)
    {
        var recursiveFamilies = FontCatalog.Families.Where(f => f.StartsWith("Recursive", StringComparison.OrdinalIgnoreCase)).ToArray();
        var recursiveFaces = FontCatalog.Faces("Recursive");
        var face = recursiveFaces.SingleOrDefault(f => f.Name == "Recursive-SansLinearLight")
            ?? throw new InvalidOperationException("The Recursive variable design is unavailable. Families: " + string.Join(", ", recursiveFamilies) + "; Faces: " + string.Join(", ", recursiveFaces.Select(f => f.Name)));
        SourceChecks(face, "recursive/Recursive_VF_1.085.ttf");
        Check(!File.Exists(Path.Combine(AppContext.BaseDirectory, "Resources/fonts/recursive/recursive-static-TTFs.ttc")), "rebuilding removes the old Recursive static collection");
        Check(FontCatalog.Families.Contains(face.Family), "the Recursive variable font appears in the font picker");
        var info = FontVariations.For(face);
        Check(info.Axes.Select(a => a.Tag).ToHashSet().SetEquals(["MONO", "CASL", "wght", "slnt", "CRSV"]), "Recursive exposes all five variable axes");
        var instances = info.Instances.Where(i => i.Name != "Default").ToArray();
        Check(instances.Length == 64, "Recursive exposes all 64 named instances");
        byte[] source = "Writing MMMM iii 0123"u8.ToArray();
        using var doc = new CoreDocument(source, format: VIEM_FORMAT_PLAIN_TEXT);
        // This is the same family/subfamily spelling saved by the Mac picker.
        Check(doc.InitializeStyleDefaults("""
            {"version":1,"block_styles":[{"id":"Paragraph","name":"Base Paragraph","role":"Paragraph",
              "based_on":null,"next_paragraph_style":null,"block":{},"character":{
                "font_families":["Recursive","serif"],"font_face":"Mono Casual Light","weight":300}}]}
            """u8.ToArray()).Length == 0, "portable Mac font names load without diagnostics");
        using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 1000, 300);
        var portable = view.Layout();
        var portableAxes = view.Provider.RenderedFontAxes(portable.Clusters.First().render_run.identifier).First();
        Check(portableAxes["MONO"] == 1 && portableAxes["CASL"] == 1 && portableAxes["wght"] == 300,
            "portable Recursive family and named instance resolve to the intended Windows outlines without explicit axes");
        foreach (var instance in instances) {
            view.EditStyleFont(view.Styles().Styles.Single(s => s.Id == "Paragraph"), [face.PortableFamily, "serif"], face, instance.Values);
            var layout = view.Layout();
            var axes = view.Provider.RenderedFontAxes(layout.Clusters.First().render_run.identifier).First();
            Check(info.Axes.All(a => Math.Abs(axes.GetValueOrDefault(a.Tag, float.NaN) - instance.Values[a.Tag]) < .001f), $"Recursive {instance.Name} renders with its named-instance coordinates");
        }
        Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(source), "Recursive instance selection preserves document source");
        var saved = view.Styles();
        Check(saved.StringList(saved.Styles.Single(s => s.Id == "Paragraph").Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)).SequenceEqual(new[] { "Recursive", "serif" }),
            "Windows saves the same portable family and fallback order as Mac");
        Check(!System.Text.Encoding.UTF8.GetString(view.ExportStyleDefaults()).Contains("file:", StringComparison.OrdinalIgnoreCase), "saved Recursive styles use portable names and coordinates");
    }
    private static async Task RecursiveInspectorChecks(EditorPane pane, Preferences preferences)
    {
        var face = FontCatalog.Faces("Recursive").Single(f => f.Name == "Recursive-SansLinearLight");
        var info = FontVariations.For(face);
        byte[] original = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
        using var doc = new CoreDocument("Recursive variable inspector"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 1000, 300);
        view.EditStyleFont(view.Styles().Styles.Single(s => s.Id == "Paragraph"), [face.PortableFamily], face);
        preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, view.ExportStyleDefaults());
        var inspector = new StyleWindow(view, preferences); inspector.Activate();
        try {
            await Task.Delay(100);
            var sliders = Descendants<Slider>(inspector.RootControl).ToArray();
            Check(sliders.Length == 5, "Recursive exposes five sliders in the style inspector");
            var rowCounts = sliders.GroupBy(s => Microsoft.UI.Xaml.Media.VisualTreeHelper.GetParent(Microsoft.UI.Xaml.Media.VisualTreeHelper.GetParent(s)))
                .Select(g => g.Count()).OrderDescending().ToArray();
            Check(rowCounts.SequenceEqual([2, 2, 1]), "Recursive's five axes occupy two full rows and one half row");
            var preset = ((IEnumerable<object>)inspector.FontVariantControl.ItemsSource).OfType<FontInstance>()
                .First(i => i.Values["MONO"] == 1 && i.Values["CASL"] == 1 && i.Values["slnt"] == -15);
            inspector.FontVariantControl.SelectedItem = preset;
            Check(info.Axes.All(a => Math.Abs(sliders.Single(s => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(s) == a.Name).Value - preset.Values[a.Tag]) < .001f), "Recursive named instances synchronize all five sliders");
            Check(inspector.Error.Length == 0, "Recursive variable-font controls apply without inspector errors");
        } finally { inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, original); }
    }
    private static async Task SegoeVariable(EditorPane pane, Preferences preferences, uint format)
    {
        byte[] original = preferences.ThemeStyleDefaults(format);
        byte[] source = "Variable fonts remain visible. **Bold** and *italic*.\n\nAnother paragraph with مرحبا and 👩‍💻."u8.ToArray();
        using var doc = new CoreDocument(source, format: format);
        var editor = new EditorWindow(preferences, doc);
        App.Instance.Windows.Add(editor); editor.Activate();
        var rendered = editor.ActivePane!;
        var view = await rendered.Ready;
        var inspector = new StyleWindow(view, preferences); inspector.Activate();
        try {
            await Task.Delay(200);
            view.Place(23, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, doc.State.document_revision);
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == "Paragraph");
            inspector.FontFamilyControl.SelectedItem = "Segoe UI Variable Text";
            Check(inspector.Error.Length == 0 && rendered.LastError == null, $"selecting Segoe UI Variable Text keeps the live document healthy (inspector={inspector.Error}, editor={rendered.LastError})");
            _ = view.Layout();
            using var specimen = new BlockStylePreview(VIEM_STYLE_ROLE_PARAGRAPH, pane.Canvas.Device, pane.DispatcherQueue);
            byte[] Pixels(CoreView target) {
                using var surface = new CanvasRenderTarget(pane.Canvas.Device, 1000, 300, 96);
                var snapshot = target.Layout();
                using (var drawing = surface.CreateDrawingSession()) {
                    drawing.Clear(Microsoft.UI.Colors.White);
                    using var batch = target.Provider.BeginDrawing(drawing);
                    foreach (var row in snapshot.Rows)
                        foreach (var c in snapshot.Clusters.Where(c => c.row_index == row.row_index))
                            batch.Draw(c.render_run, new Vector2(c.x, row.baseline - target.Viewport.top), Microsoft.UI.Colors.Black);
                }
                return surface.GetPixelBytes();
            }
            void PanePixels() {
                using var surface = new CanvasRenderTarget(pane.Canvas.Device, (float)rendered.Canvas.ActualWidth, (float)rendered.Canvas.ActualHeight, rendered.Canvas.Dpi);
                using (var drawing = surface.CreateDrawingSession()) rendered.Draw(drawing);
                byte[] bytes = surface.GetPixelBytes();
                var bg = preferences.Theme.Background;
                int ink = 0;
                for (int i = 0; i < bytes.Length; i += 4)
                    if (Math.Abs(bytes[i] - bg.B) + Math.Abs(bytes[i + 1] - bg.G) + Math.Abs(bytes[i + 2] - bg.R) > 80) ink++;
                Check(ink > 150, "the live editor pane draws variable-font text after slider changes");
            }
            void VisiblePixels(byte[] pixels, string name) => Check(pixels.Count(b => b < 245) > 150, name);
            var sliders = Descendants<Slider>(inspector.RootControl).ToArray();
            Check(sliders.Length > 0, "Segoe UI Variable Text exposes variable axis controls");
            Check(sliders.All(s => s.StepFrequency == 1), "variable-axis sliders use integer steps");
            Dictionary<string, float> FirstAxes() => view.Provider.RenderedFontAxes(view.Layout().Clusters.First().render_run.identifier).First(a => a.ContainsKey("opsz"));
            byte[] PreviewPixels() {
                var sheet = inspector.ThemeView.Styles();
                specimen.Update(sheet, sheet.Styles.Single(s => s.Id == "Paragraph"), Microsoft.UI.Colors.Black);
                using var preview = new CanvasRenderTarget(pane.Canvas.Device, 560, 200, 96);
                using (var drawing = preview.CreateDrawingSession()) { drawing.Clear(Microsoft.UI.Colors.White); specimen.Draw(drawing, 560, 200, Microsoft.UI.Colors.Black); }
                return preview.GetPixelBytes();
            }
            var opticalName = FontVariations.For(FontCatalog.Faces("Segoe UI Variable Text").First()).Axes.Single(a => a.Tag == "opsz").Name;
            var opticalSlider = sliders.Single(s => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(s) == opticalName);
            opticalSlider.Value = opticalSlider.Minimum;
            await Task.Delay(30);
            byte[] smallOptical = Pixels(view), smallPreview = PreviewPixels();
            Check(FirstAxes()["opsz"] == opticalSlider.Minimum, "rendered font face retains the minimum explicit optical size");
            opticalSlider.Value = opticalSlider.Maximum;
            await Task.Delay(30);
            Check(FirstAxes()["opsz"] == opticalSlider.Maximum, "rendered font face retains the maximum explicit optical size");
            Check(!smallOptical.AsSpan().SequenceEqual(Pixels(view)), "explicit optical size changes Segoe UI Variable glyphs at a fixed rendering size");
            Check(!smallPreview.AsSpan().SequenceEqual(PreviewPixels()), "optical-size changes update the actual shaped style preview");
            var variableFace = FontCatalog.Faces("Segoe UI Variable Text").First();
            foreach (float size in new[] { 14f, 28f }) {
                using var autoFormat = new Microsoft.Graphics.Canvas.Text.CanvasTextFormat { FontFamily = variableFace.Family, FontSize = size };
                const string text = "Automatic optical size";
                using var autoLayout = new Microsoft.Graphics.Canvas.Text.CanvasTextLayout(pane.Canvas.Device, text, autoFormat, 1000, 200);
                FontVariations.Apply(autoLayout, 0, text.Length, new() { ["wght"] = 400 }, variableFace);
                var capture = new GlyphCapture(f => new GlyphFontMetadata(f));
                DirectWriteGlyphCapture.Draw(autoLayout, capture);
                var axes = FontVariations.NativeAxes(capture.Extract(0, text.Length, 0, 0).First().Font);
                Check(Math.Abs(axes["opsz"] - size * .75f) < .001f, "unspecified optical size follows the font size in points");
            }
            foreach (var slider in sliders) {
                foreach (double value in new[] { slider.Minimum, slider.Minimum + (slider.Maximum - slider.Minimum) * .29137, (slider.Minimum + slider.Maximum) / 2, slider.Maximum }) {
                    slider.Value = value;
                    await Task.Delay(30);
                    Check(inspector.Error.Length == 0 && rendered.LastError == null, $"Segoe UI Variable Text axis {Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(slider)}={value} updates without stale layout errors (inspector={inspector.Error}, editor={rendered.LastError})");
                    PanePixels();
                    VisiblePixels(Pixels(view), "Segoe UI Variable Text draws visible native document glyphs");
                    using var fresh = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, view.Layout().Info.viewport_width, view.Layout().Info.viewport_height);
                    fresh.Padding(preferences.Margin("top"), preferences.Margin("left"), preferences.Margin("bottom"), preferences.Margin("right"));
                    VisiblePixels(Pixels(fresh), "fresh Segoe UI Variable Text glyphs are visible");
                    Check(Pixels(view).AsSpan().SequenceEqual(Pixels(fresh)), "cached Segoe UI Variable Text faces render identically to freshly shaped faces");
                    var sheet = inspector.ThemeView.Styles();
                    specimen.Update(sheet, sheet.Styles.Single(s => s.Id == "Paragraph"), Microsoft.UI.Colors.Black);
                    using var preview = new CanvasRenderTarget(pane.Canvas.Device, 560, 200, 96);
                    using (var drawing = preview.CreateDrawingSession()) { drawing.Clear(Microsoft.UI.Colors.White); specimen.Draw(drawing, 560, 200, Microsoft.UI.Colors.Black); }
                    var specimenLayout = specimen.Layout(560, 200);
                    var selectedRow = specimenLayout.Rows.First(r => r.text_start > 0);
                    VisiblePixels(preview.GetPixelBytes(0, (int)Math.Floor(selectedRow.y), 560, (int)Math.Ceiling(selectedRow.ascent + selectedRow.descent)), "the selected variable-font preview paragraph remains visible after slider changes");
                    var layout = view.Layout();
                    Check(layout.Clusters.Length > 0 && layout.Clusters.Any(c => c.advance > 0), "Segoe UI Variable Text edits retain visible document glyphs");
                }
            }
            var weightSlider = sliders.First();
            weightSlider.Value = 350.26;
            PanePixels();
            Check(weightSlider.Value == 350 && FirstAxes()["wght"] == 350, "fractional slider input commits and renders an integer weight");
            VisiblePixels(Pixels(view), "Segoe UI Variable Text rounded weight remains visible without mouse input");
            using (var fractionalDoc = new CoreDocument(source, format: format))
            using (var fractionalView = new CoreView(fractionalDoc, pane.Canvas.Device, pane.DispatcherQueue, 1000, 300)) {
                var values = FontVariations.For(variableFace).Defaults; values["wght"] = 350.26f;
                fractionalView.EditStyleFont(fractionalView.Styles().Styles.Single(s => s.Id == "Paragraph"), [variableFace.PortableFamily], variableFace, values);
                VisiblePixels(Pixels(fractionalView), "fractional coordinates from stylesheets remain renderable");
                var axes = fractionalView.Provider.RenderedFontAxes(fractionalView.Layout().Clusters.First().render_run.identifier).First();
                Check(Math.Abs(axes["wght"] - 350.26f) < .001f, "stylesheet coordinates retain precision independently of slider steps");
            }
            if (Environment.GetEnvironmentVariable("VIEM_TEST_POINTER_INPUT") == "1") {
                inspector.AppWindow.Show(true); inspector.Activate();
                await Task.Delay(200);
                foreach (var slider in sliders) {
                    var points = Enumerable.Range(0, 20).Select(i => new global::Windows.Foundation.Point(.15 + i * .03, .5)).ToArray();
                    await InputRoutingTests.Drag(inspector, slider, points, step => {
                        Check(inspector.Error.Length == 0 && rendered.LastError == null, $"native variable-axis dragging keeps the document healthy ({rendered.LastError})");
                        PanePixels();
                        VisiblePixels(Pixels(view), $"native variable-axis dragging keeps glyphs visible at step {step}, value {slider.Value}");
                    });
                }
            }
            inspector.UndoThemeForTesting();
            PanePixels();
            inspector.RedoThemeForTesting();
            PanePixels();
            Check(inspector.Error.Length == 0 && rendered.LastError == null, "undo and redo retain healthy variable-font document rendering");
            if (format == VIEM_FORMAT_MARKDOWN) {
                using var longDoc = new CoreDocument(System.Text.Encoding.UTF8.GetBytes(string.Concat(Enumerable.Repeat("Variable fonts **bold** and مرحبا 👩‍💻 remain visible.\n\n", 2000))), format: format);
                longDoc.InitializeStyleDefaults(inspector.ThemeView.ExportStyleDefaults());
                using var background = new CoreView(longDoc, pane.Canvas.Device, pane.DispatcherQueue, 800, 400);
                await BackgroundLayoutTests.Idle(background);
                Check(background.Provider.BackgroundShapedCharacters > 0, "variable-font glyph capture supports background shaping");
                background.Key(VIEM_KEY_PAGE_DOWN);
                VisiblePixels(Pixels(background), "worker-prepared variable-font glyphs remain visible after scrolling");
            }
            Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(source) && !doc.IsDirty, "Segoe UI Variable Text theme edits preserve document bytes and clean state");
        } finally { inspector.Close(); preferences.SaveThemeStyles(format, original); editor.Close(); App.Instance.Windows.Remove(editor); }
    }
    private static IEnumerable<T> Descendants<T>(Microsoft.UI.Xaml.DependencyObject root) where T : Microsoft.UI.Xaml.DependencyObject
    {
        if (root is T typed) yield return typed;
        for (int i = 0; i < Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChildrenCount(root); i++)
            foreach (var child in Descendants<T>(Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChild(root, i))) yield return child;
    }
}
#endif
