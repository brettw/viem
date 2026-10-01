#if DEBUG
using Microsoft.UI.Xaml.Controls;
using Microsoft.Graphics.Canvas;
using System.Numerics;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Rendering;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;
internal static class VariableFontTests
{
    private static void Check(bool ok, string name) { if (!ok) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
    internal static async Task Run(EditorPane pane, Preferences preferences)
    {
        var packagedFace = FontCatalog.ForFamilyChange("Flightline Code", null)!;
        using (var format = new Microsoft.Graphics.Canvas.Text.CanvasTextFormat { FontFamily = FontCatalog.RenderingFamily(packagedFace.Family, packagedFace.Weight, packagedFace.Slant, packagedFace.Stretch), FontSize = 14 })
        using (var layout = new Microsoft.Graphics.Canvas.Text.CanvasTextLayout(pane.Canvas.Device, "Packaged fonts", format, 1000, 200)) {
            FontVariations.Apply(layout, 0, 14, new() { ["wght"] = packagedFace.Weight }, packagedFace);
            var capture = new GlyphCapture(f => new GlyphFontMetadata(f)); DirectWriteGlyphCapture.Draw(layout, capture);
            var parts = capture.Extract(0, 14, 0, 0);
            Check(parts.Count > 0, "typographic font selection preserves packaged-font glyphs");
            Check(parts.First().Font.GetInformationalStrings(Microsoft.Graphics.Canvas.Text.CanvasFontInformation.PostscriptName).Values.Contains(packagedFace.Name), "typographic selection retains the packaged font collection instead of substituting a system face");
        }
        RecursiveFontChecks(pane);
        await RecursiveInspectorChecks(pane, preferences);
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
        view.EditStyleFont(style, [face.Name], face, saved);
        var wide = view.Layout();
        saved["wdth"] = 75;
        view.EditStyleFont(style, [face.Name], face, saved);
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
    internal static void RecursiveFontChecks(EditorPane pane)
    {
        var recursiveFamilies = FontCatalog.Families.Where(f => f.StartsWith("Recursive", StringComparison.OrdinalIgnoreCase)).ToArray();
        var recursiveFaces = recursiveFamilies.SelectMany(FontCatalog.Faces).ToArray();
        var face = recursiveFaces.FirstOrDefault(f => f.Source is { IsFile: true } && Path.GetFileName(f.Source.LocalPath) == "Recursive_VF_1.085.ttf")
            ?? throw new InvalidOperationException("The bundled Recursive variable font is unavailable. Families: " + string.Join(", ", recursiveFamilies) + "; Faces: " + string.Join(", ", recursiveFaces.Select(f => f.Name)));
        Check(face.Source is { IsFile: true } && Path.GetFileName(face.Source.LocalPath) == "Recursive_VF_1.085.ttf", "Recursive resolves to the bundled variable font");
        Check(!File.Exists(Path.Combine(AppContext.BaseDirectory, "Resources/fonts/recursive/recursive-static-TTFs.ttc")), "rebuilding removes the old Recursive static collection");
        Check(FontCatalog.Families.Contains(face.Family), "the bundled Recursive variable font appears in the font picker");
        var info = FontVariations.For(face);
        Check(info.Axes.Select(a => a.Tag).ToHashSet().SetEquals(["MONO", "CASL", "wght", "slnt", "CRSV"]), "Recursive exposes all five variable axes");
        var instances = info.Instances.Where(i => i.Name != "Default").ToArray();
        Check(instances.Length == 64, "Recursive exposes all 64 named instances");
        byte[] source = "Writing MMMM iii 0123"u8.ToArray();
        using var doc = new CoreDocument(source, format: VIEM_FORMAT_PLAIN_TEXT);
        using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 1000, 300);
        foreach (var instance in instances) {
            view.EditStyleFont(view.Styles().Styles.Single(s => s.Id == "Paragraph"), [face.Name, "serif"], face, instance.Values);
            var layout = view.Layout();
            var axes = view.Provider.RenderedFontAxes(layout.Clusters.First().render_run.identifier).First();
            Check(info.Axes.All(a => Math.Abs(axes.GetValueOrDefault(a.Tag, float.NaN) - instance.Values[a.Tag]) < .001f), $"Recursive {instance.Name} renders with its named-instance coordinates");
        }
        Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(source), "bundled Recursive instance selection preserves document source");
        Check(!System.Text.Encoding.UTF8.GetString(view.ExportStyleDefaults()).Contains("file:", StringComparison.OrdinalIgnoreCase), "saved Recursive styles use portable names and coordinates");
    }
    private static async Task RecursiveInspectorChecks(EditorPane pane, Preferences preferences)
    {
        var face = FontCatalog.Families.Where(f => f.StartsWith("Recursive", StringComparison.OrdinalIgnoreCase))
            .SelectMany(FontCatalog.Faces).First(f => f.Source is { IsFile: true } && Path.GetFileName(f.Source.LocalPath) == "Recursive_VF_1.085.ttf");
        var info = FontVariations.For(face);
        byte[] original = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
        using var doc = new CoreDocument("Recursive variable inspector"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 1000, 300);
        view.EditStyleFont(view.Styles().Styles.Single(s => s.Id == "Paragraph"), [face.Name], face);
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
                fractionalView.EditStyleFont(fractionalView.Styles().Styles.Single(s => s.Id == "Paragraph"), [variableFace.Name], variableFace, values);
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
