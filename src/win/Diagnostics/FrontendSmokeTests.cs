#if DEBUG
using System.Text;
using System.Text.Json;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Dispatching;
using Viem.Windows.Core;
using Viem.Windows.Input;
using Viem.Windows.Interop;
using Viem.Windows.Shell;
using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

/// <summary>Integration checks run on the real WinUI/DirectWrite UI thread.</summary>
internal static class FrontendSmokeTests
{
    internal static readonly List<string> UiChecks = [];
    internal static bool Started;
    internal static System.Diagnostics.Process? ChildProcess;
    internal static string? ReportPath { get; } = FindReportPath();
    private static string? FindReportPath()
    { var args = Environment.GetCommandLineArgs(); int i = Array.IndexOf(args, "--self-test"); return i >= 0 && i + 1 < args.Length ? Path.GetFullPath(args[i + 1]) : null; }
    public static void WriteFailure(Exception error)
    { try { if (ChildProcess is { HasExited: false }) ChildProcess.Kill(); } catch { } if (ReportPath != null) File.WriteAllText(ReportPath, JsonSerializer.Serialize(new { passed = false, error = error.ToString() }, new JsonSerializerOptions { WriteIndented = true })); }
    public static async Task FileChecks(CanvasDevice device, Microsoft.UI.Dispatching.DispatcherQueue dispatcher, string profile)
    {
        void Check(bool value, string name) { if (!value) throw new InvalidOperationException(name); UiChecks.Add(name); }
        string test = Path.Combine(profile, "files"); Directory.CreateDirectory(test);
        string path = Path.Combine(test, "recovery.md");
        byte[] disk = "# original\r\n"u8.ToArray(); File.WriteAllBytes(path, disk);
        using (var doc = new CoreDocument(disk, path))
        using (var view = new CoreView(doc, device, dispatcher, 500, 200))
        {
            Exception? failure = null;
            using var owner = DocumentRecovery.Claim(doc, path, profile, dispatcher, e => failure = e);
            using var other = DocumentRecovery.Claim(doc, path, profile, dispatcher, e => failure = e);
            Check(owner.Slot != other.Slot, "recovery slots are claimed exclusively");
            view.Command("i"); view.Text("Unsaved "); view.Key(VIEM_KEY_ESCAPE);
            owner.Write(RecoverySnapshot.Capture(doc)); Check(await owner.Pending, "recovery snapshot writes atomically");
            Check(File.ReadAllBytes(path).AsSpan().SequenceEqual(disk), "recovery never autosaves over source");
            var snapshot = DocumentRecovery.Read(owner.Slot)!.snapshot!;
            using var reopened = new CoreDocument(snapshot.source, format: snapshot.Format, encoding: snapshot.encoding, fileFormat: snapshot.fileFormat);
            reopened.MarkRecovered(); Check(reopened.IsDirty && reopened.FormattedText().StartsWith("Unsaved"), "recovery restores interpretation and dirty state");
            Check(failure == null, "recovery reports no background failures");
            other.Dispose(); Check(File.Exists(owner.Slot) && !File.Exists(other.Slot), "recovery cleanup removes only owned slot");
        }
        Check(DocumentRecovery.Candidates(path, profile).Length == 0, "document close cleans recovery slots");
        string config = Path.Combine(test, "config.json");
        File.WriteAllText(config, "{\"version\":1,\"future\":42,\"editing\":{\"future\":true,\"indentation\":{\"tabstop\":4,\"future\":1}}}");
        var preferences = new Preferences(test); preferences.Set("editing", "indentation", new { shiftwidth = 3 });
        using (var json = JsonDocument.Parse(File.ReadAllText(config)))
            Check(json.RootElement.GetProperty("future").GetInt32() == 42 && json.RootElement.GetProperty("editing").GetProperty("indentation").GetProperty("future").GetInt32() == 1, "settings preserve unknown nested keys");
        byte[] before = File.ReadAllBytes(config); bool rejected = false;
        try { preferences.Set("editing", "indentation", new { tabstop = 0 }); } catch { rejected = true; }
        Check(rejected && File.ReadAllBytes(config).AsSpan().SequenceEqual(before), "invalid preferences leave prior file untouched");
        File.WriteAllText(config, "{\"version\":99}"); preferences = new Preferences(test);
        Check(preferences.Error != null && File.ReadAllText(config) == "{\"version\":99}", "unsupported config version is reported and preserved");
    }
    public static void Run(CanvasDevice device, Microsoft.UI.Dispatching.DispatcherQueue dispatcher)
    {
        var checks = new List<string>(UiChecks);
        void Check(bool condition, string name) { if (!condition) throw new InvalidOperationException(name); checks.Add(name); }
        void Scenario(string text, uint format, Action<CoreDocument, CoreView> run)
        { using var doc = new CoreDocument(Encoding.UTF8.GetBytes(text), format: format); using var view = new CoreView(doc, device, dispatcher, 700, 400); run(doc, view); }
        Scenario("alpha beta\nsecond line\n", 1, (doc, view) => {
            Check(doc.FormattedText() == "alpha beta\nsecond line\n", "C ABI open and snapshot identity");
            view.Command("w"); Check(view.Presentation.cursor_utf8_offset == 6, "vi word motion");
            view.Command("de"); Check(doc.FormattedText() == "alpha \nsecond line\n", "vi operator motion: " + doc.FormattedText());
            view.Undo(); Check(doc.FormattedText() == "alpha beta\nsecond line\n", "core undo");
            view.Redo(); view.Undo();
            view.Key(VIEM_KEY_ESCAPE); view.Command("ggi"); view.Text("Hello "); view.Key(VIEM_KEY_ESCAPE);
            Check(doc.FormattedText().StartsWith("Hello alpha"), "committed text through core");
            view.SelectAll(); string copied = ""; view.Effects += e => { if (e.Clipboard.Length > 0) copied = e.Clipboard[0].Text; };
            view.CopyOrCut(false); Check(copied.Contains("Hello alpha"), "clipboard write effect and ownership");
            view.Key(VIEM_KEY_ESCAPE); view.Command("G"); view.ClipboardText = " pasted "; view.Paste(true);
            Check(doc.FormattedText().Contains(" pasted "), "clipboard host-context paste");
            view.Key(VIEM_KEY_ESCAPE); view.Ex("set nowrap"); Check((view.Viewport.flags & VIEM_VIEWPORT_STATE_WRAP) == 0, "Ex options affect view");
            view.Ex("%s/alpha/ALPHA/g"); Check(doc.FormattedText().Contains("ALPHA"), "Ex substitute");
            using var other = new CoreView(doc, device, dispatcher, 350, 250); other.Refresh();
            Check(other.Document.FormattedText() == doc.FormattedText(), "split views share document state");
        });
        Scenario("a👩‍💻e\u0301z", 1, (doc, view) => {
            view.Command("lx"); Check(doc.FormattedText() == "ae\u0301z", "grapheme-safe emoji deletion");
            view.Undo(); view.Command("ggi"); var before = doc.Source(doc.State.document_revision);
            view.BeginComposition(); view.UpdateComposition("日本", 2, 0);
            Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before), "IME marked text is not source");
            view.CancelComposition(); Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before), "IME cancellation preserves source");
            view.BeginComposition(); view.UpdateComposition("日本", 2, 0); view.CommitComposition("日本"); view.Key(VIEM_KEY_ESCAPE);
            Check(doc.FormattedText().StartsWith("日本"), "IME commit"); view.Undo(); Check(doc.FormattedText() == "a👩‍💻e\u0301z", "IME undo grouping");
        });
        Scenario("abc", 1, (doc, view) => {
            view.Command("i"); view.Key(VIEM_KEY_CONTROL_CHARACTER, 'q'); view.ClipboardText = "paste"; view.Paste(true);
            Check(doc.FormattedText() == "pasteabc", "Windows paste overrides literal-next input");
        });
        Scenario("alpha\nbeta", 1, (doc, view) => {
            view.Command(":"); view.Text("echo 日本語"); var prompt = view.Prompt();
            view.EditPrompt(prompt, 5, 14, "word"); Check(view.Prompt().Text == "echo word", "UTF-8 command-line selection replacement");
            view.Key(VIEM_KEY_ESCAPE); view.Command("i"); view.Text("x"); view.Key(VIEM_KEY_ESCAPE);
            var before = doc.Source(doc.State.document_revision);
            doc.ConfigureDefaults("{\"tabstop\":4}"u8.ToArray(), "{}"u8.ToArray(), 72, "", "[]"u8.ToArray());
            Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before), "application defaults preserve source");
        });
        Scenario("# Heading\n\nHello **world** and مرحبا.\n", 2, (doc, view) => {
            var source = doc.Source(doc.State.document_revision); var layout = view.Layout();
            Check(layout.Clusters.Length > 0 && layout.Rows.Length > 1, "formatted DirectWrite layout");
            Check(layout.Clusters.Any(c => (c.bidi_level & 1) != 0), "DirectWrite bidi levels");
            Check(layout.PaintRuns.Length > 0 || view.Styles().Styles.Length > 1, "stylesheet export");
            view.Zoom(1.25f); Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(source), "zoom is presentation-only");
            view.Format(VIEM_FORMAT_MARKDOWN_SOURCE); Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(source), "source-view switch preserves source bytes");
        });
        Scenario("# Heading\n\nHello **office** and مرحبا 👩‍💻.\n", 2, (doc, view) => {
            var layout = view.Layout();
            using var target = new CanvasRenderTarget(device, 700, 400, 96);
            byte[] Render(bool batched)
            {
                using (var drawing = target.CreateDrawingSession())
                {
                    drawing.Clear(Microsoft.UI.Colors.White);
                    using var batch = view.Provider.BeginDrawing(drawing);
                    int index = 0;
                    foreach (var row in layout.Rows)
                    foreach (var cluster in layout.Clusters.Where(c => c.row_index == row.row_index))
                    {
                        var color = (index++ % 3) switch { 0 => Microsoft.UI.Colors.Red, 1 => Microsoft.UI.Colors.Blue, _ => Microsoft.UI.Colors.Black };
                        var baseline = new System.Numerics.Vector2(cluster.x, row.baseline);
                        if (batched) batch.Draw(cluster.render_run, baseline, color);
                        else view.Provider.Draw(drawing, cluster.render_run, baseline, color);
                    }
                }
                return target.GetPixelBytes();
            }
            Check(Render(false).AsSpan().SequenceEqual(Render(true)), "reused glyph brushes preserve colored, styled, bidi and emoji pixels");
        });
        Scenario("<p>Text</p>", 3, (doc, view) => {
            view.Command("i"); view.ToggleSemantic(VIEM_SEMANTIC_STYLE_STRONG); view.Text("Bold "); view.Key(VIEM_KEY_ESCAPE);
            Check(doc.FormattedText().Contains("Bold Text"), "HTML typing style");
            using (var reopened = new CoreDocument(doc.Source(doc.State.document_revision), format: VIEM_FORMAT_HTML))
            using (var reopenedView = new CoreView(reopened, device, dispatcher, 700, 400))
            { reopenedView.Command("ggviw"); Check(reopenedView.SemanticStyle(VIEM_SEMANTIC_STYLE_STRONG).state == VIEM_SEMANTIC_STYLE_STATE_ON, "HTML style survives reopening"); }
            view.Undo(); Check(doc.FormattedText() == "Text", "formatted undo");
            string id = view.CreateStyle(2, "Test Character"); var style = view.Styles().Styles.Single(s => s.Id == id);
            view.EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT, CoreView.Enum(VIEM_STYLE_VALUE_UNSIGNED, 650));
            Check(view.Styles().Styles.Single(s => s.Id == id).Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value == 650, "typed font weight declaration");
            view.Command("ggviw"); view.AssignStyle(2, id);
            string fragment = Encoding.UTF8.GetString(view.ClipboardJson(0, 4));
            Check(ClipboardFormats.Html(fragment).Contains("font-weight:650"), "HTML clipboard preserves character style");
        });
        Scenario("<p style='text-align:center;line-height:1.5;margin-inline-start:12pt'>one<br>two</p><p>three</p>", 3, (doc, view) => {
            string text = doc.FormattedText(); string fragment = Encoding.UTF8.GetString(view.ClipboardJson(0, (ulong)Encoding.UTF8.GetByteCount(text)));
            string html = ClipboardFormats.Html(fragment);
            using var imported = new CoreDocument(Encoding.UTF8.GetBytes(html), format: VIEM_FORMAT_HTML);
            Check(imported.FormattedText() == text, "HTML clipboard preserves paragraph and hard-break boundaries");
            Check(html.Contains("text-align:center") && html.Contains("line-height:1.5") && html.Contains("margin-inline-start:12pt"), "HTML clipboard preserves paragraph declarations");
            using var importedView = new CoreView(imported, device, dispatcher, 700, 400);
            using var actual = JsonDocument.Parse(importedView.ClipboardJson(0, (ulong)Encoding.UTF8.GetByteCount(text))); using var expected = JsonDocument.Parse(fragment);
            Check(actual.RootElement.GetProperty("character_runs")[0].GetProperty("size").GetSingle() == expected.RootElement.GetProperty("character_runs")[0].GetProperty("size").GetSingle(), "HTML clipboard preserves point sizes");
        });
        Scenario("\talpha  \n", 1, (doc, view) => {
            var before = doc.Source(doc.State.document_revision);
            doc.ConfigureDefaults("{}"u8.ToArray(), "{\"visibleWhitespace\":{\"style\":{\"size\":180,\"foreground\":{\"red\":1,\"green\":0,\"blue\":0,\"alpha\":1},\"underline\":true}}}"u8.ToArray(), 80, "", "[]"u8.ToArray());
            view.Refresh(); var layout = view.Layout(); var markers = view.Whitespace(layout.Info);
            Check(markers.Markers.Length > 0 && markers.Style.GetProperty("size").GetSingle() == 180, "whitespace exports portable style declarations");
            using var target = new CanvasRenderTarget(device, 700, 400, 96);
            using (var drawing = target.CreateDrawingSession()) { drawing.Clear(Microsoft.UI.Colors.White); view.Provider.DrawWhitespace(drawing, markers, layout, view.Viewport, _ => Microsoft.UI.Colors.Black, Microsoft.UI.Colors.Black); }
            var pixels = target.GetPixelBytes();
            Check(Enumerable.Range(0, pixels.Length / 4).Any(i => pixels[4 * i + 2] > 200 && pixels[4 * i + 1] < 100 && pixels[4 * i] < 100), "oversized whitespace marker ink fits and uses configured foreground");
            Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before), "whitespace drawing leaves source unchanged");
        });
        Scenario(string.Concat(Enumerable.Repeat("A long line with office ligatures and AV kerning.\n", 25_000)), 1, (doc, view) => {
            long initial = view.Provider.ShapedCharacters;
            Check(initial < 100_000, "large document initial layout is bounded");
            var beforeDrag = view.Layout();
            view.Place(40f, 40f); view.Place(140f, 40f, true);
            var afterDrag = view.Layout();
            Check(ReferenceEquals(beforeDrag.Clusters, afterDrag.Clusters) && ReferenceEquals(beforeDrag.PaintRuns, afterDrag.PaintRuns) && afterDrag.Selection.Length > 0, "large-document drag reuses immutable geometry and paint while refreshing selection");
            view.Key(VIEM_KEY_ESCAPE);
            view.Resize(650, 350);
            Check(!ReferenceEquals(beforeDrag.Clusters, view.Layout().Clusters), "resize invalidates the frontend layout export cache");
            Check(view.Provider.ShapedCharacters - initial < 20_000, "resize reuses shaping cache");
            view.Command("i"); view.Text("x"); view.Key(VIEM_KEY_ESCAPE);
            Check(view.Provider.ShapedCharacters - initial < 40_000, "local edit does not reshape whole document");
            var beforeZoomLayout = view.Layout();
            Check(!ReferenceEquals(afterDrag.Clusters, beforeZoomLayout.Clusters), "editing invalidates the frontend layout export cache");
            long beforeZoom = view.Provider.ShapedCharacters; view.Zoom(1.5f);
            Check(!ReferenceEquals(beforeZoomLayout.Clusters, view.Layout().Clusters), "zoom invalidates the frontend layout export cache");
            Check(view.Provider.ShapedCharacters > beforeZoom, "scale invalidates affected metrics");
            var layout = view.Layout(); Check(layout.Info.coverage_hard_line_end < 25_000, "viewport export remains regional");
            view.Scroll(0, 100_000); Check(view.Viewport.top > 0, "large document scrolling");
        });
        foreach (var key in new[] { VirtualKey.A, VirtualKey.B, VirtualKey.D, VirtualKey.E, VirtualKey.F, VirtualKey.H, VirtualKey.I, VirtualKey.J, VirtualKey.K, VirtualKey.L, VirtualKey.M, VirtualKey.N, VirtualKey.O, VirtualKey.P, VirtualKey.Q, VirtualKey.R, VirtualKey.T, VirtualKey.U, VirtualKey.W, VirtualKey.Y })
            Check(KeyPolicy.Route(key, true, false, false).Kind == VIEM_KEY_CONTROL_CHARACTER, $"preserve Ctrl+{key}");
        Check(KeyPolicy.Route(VirtualKey.C, true, false, false, true).Action == NativeAction.Copy, "Ctrl+C overrides literal/vi bindings");
        Check(KeyPolicy.Route(VirtualKey.X, true, false, false).Action == NativeAction.Cut, "Ctrl+X override");
        Check(KeyPolicy.Route(VirtualKey.V, true, false, false).Action == NativeAction.Paste, "Ctrl+V override");
        Check(KeyPolicy.Route(VirtualKey.V, true, false, true).Action == NativeAction.None, "AltGr remains text input");
        Check(KeyPolicy.Route(VirtualKey.Z, true, false, true).Action == NativeAction.None
            && KeyPolicy.Route(VirtualKey.Z, true, true, true).Action == NativeAction.None, "AltGr+Z does not invoke history");
        Check(KeyPolicy.Route(VirtualKey.Z, true, true, false, true).Kind == VIEM_KEY_CONTROL_CHARACTER, "literal-next preserves Ctrl+Shift+Z as a control character");
        Check(KeyPolicy.Route(VirtualKey.F5, false, true, false).Modifiers == VIEM_KEY_MODIFIER_SHIFT, "function-key modifiers preserved");
        Check(KeyPolicy.Route(VirtualKey.Number6, true, false, false).Codepoint == '^', "preserve vi Ctrl+6/Ctrl+^");
        File.WriteAllText(ReportPath!, JsonSerializer.Serialize(new { passed = true, count = checks.Count, checks }, new JsonSerializerOptions { WriteIndented = true }));
    }
}
#endif
