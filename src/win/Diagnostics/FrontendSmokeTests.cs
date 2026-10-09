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
    { try { if (ChildProcess is { HasExited: false }) ChildProcess.Kill(); } catch { } if (ReportPath != null) File.WriteAllText(ReportPath, JsonSerializer.Serialize(new NativeTestFailure(false, error.ToString()), NativeTestJsonContext.Default.NativeTestFailure)); }
    internal static void WriteSuccess(List<string> checks) => File.WriteAllText(ReportPath!,
        JsonSerializer.Serialize(new NativeTestSuccess(true, checks.Count, checks, !System.Runtime.CompilerServices.RuntimeFeature.IsDynamicCodeSupported), NativeTestJsonContext.Default.NativeTestSuccess));
    public static async Task FileChecks(CanvasDevice device, Microsoft.UI.Dispatching.DispatcherQueue dispatcher, string profile)
    {
        void Check(bool value, string name) { if (!value) throw new InvalidOperationException(name); UiChecks.Add(name); }
        string test = Path.Combine(profile, "files"); Directory.CreateDirectory(test);
        string path = Path.Combine(test, "recovery.md");
        byte[] disk = "# original\r\n"u8.ToArray(); File.WriteAllBytes(path, disk);
        using (var doc = new CoreDocument("exact source"u8.ToArray()))
        using (var view = new CoreView(doc, device, dispatcher, 500, 200))
        {
            doc.ConfigureEditingDefaults("{\"tabstop\":0}"u8.ToArray(), "{}"u8.ToArray(), 0, "{}"u8.ToArray());
            Check(doc.ConfigurationDiagnostics.Contains("indentation"), "invalid optional settings warn without losing the document");
            bool strictRejected = false;
            try { doc.ConfigureDefaults("{\"tabstop\":0}"u8.ToArray(), "{}"u8.ToArray(), 0, "{}"u8.ToArray()); }
            catch { strictRejected = true; }
            Check(strictRejected, "optional recovery does not bypass strict settings validation");
            view.Command("i"); view.Provider.FailShapingBatchesForTest = 1;
            view.Text("X");
            Check(doc.FormattedText() == "Xexact source" && view.Layout().Diagnostics.Contains("default system font"),
                "native shaping failure recovers a usable frame and inserts text exactly once");
            view.Key(VIEM_KEY_ESCAPE); view.Command("u");
            Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual("exact source"u8), "undo after font recovery restores exact source");
            view.Provider.FailShapingBatchesForTest = 2;
            bool declined = false;
            try { view.Provider.InvalidateMetrics(); view.Resize(420, 200); } catch { declined = true; }
            Check(declined, "an unavailable native device has a finite recovery budget");
            view.Resize(500, 200);
            Check(view.Layout().Rows.Length > 0, "editing resumes when native shaping becomes available");
        }
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
        await RecoveryCleanupTests.Run(device, dispatcher, profile, Check);
        string config = Path.Combine(test, "config.json");
        File.WriteAllText(config, "{\"version\":1,\"future\":42,\"editing\":{\"future\":true,\"indentation\":{\"tabstop\":4,\"future\":1}}}");
        var preferences = new Preferences(test); preferences.Set("editing", "indentation", new System.Text.Json.Nodes.JsonObject { ["shiftwidth"] = 3 });
        using (var json = JsonDocument.Parse(File.ReadAllText(config)))
            Check(json.RootElement.GetProperty("future").GetInt32() == 42 && json.RootElement.GetProperty("editing").GetProperty("indentation").GetProperty("future").GetInt32() == 1, "settings preserve unknown nested keys");
        byte[] before = File.ReadAllBytes(config); bool rejected = false;
        try { preferences.Set("editing", "indentation", new System.Text.Json.Nodes.JsonObject { ["tabstop"] = 0 }); } catch { rejected = true; }
        Check(rejected && File.ReadAllBytes(config).AsSpan().SequenceEqual(before), "invalid preferences leave prior file untouched");
        int settingsChanges = 0, recentChanges = 0;
        preferences.Changed += () => settingsChanges++;
        preferences.RecentChanged += () => recentChanges++;
        preferences.Remember(path);
        Check(settingsChanges == 0 && recentChanges == 1 && new Preferences(test).Recent.SequenceEqual(new[] { path }),
            "remembering an open file persists the recent menu without refreshing document settings");
        var external = System.Text.Json.Nodes.JsonNode.Parse(File.ReadAllText(config))!;
        external["appearance"] = new System.Text.Json.Nodes.JsonObject { ["showStatusBar"] = false };
        File.WriteAllText(config, external.ToJsonString());
        preferences.Remember(path);
        Check(settingsChanges == 1 && !preferences.ShowStatus,
            "recent-file updates still merge and notify externally changed editor settings");
        preferences.ClearRecent();
        Check(settingsChanges == 1 && recentChanges == 2 && new Preferences(test).Recent.Length == 0,
            "clearing recent files updates only the recent menu");
        File.WriteAllText(config, "{\"version\":99}"); preferences = new Preferences(test);
        Check(preferences.Error != null && File.ReadAllText(config) == "{\"version\":99}", "unsupported config version is reported and preserved");
    }
    public static void Run(CanvasDevice device, Microsoft.UI.Dispatching.DispatcherQueue dispatcher)
    {
        var checks = new List<string>(UiChecks);
        void Check(bool condition, string name) { if (!condition) throw new InvalidOperationException(name); checks.Add(name); }
        Check(CoreDocument.FormatForPath("document.md") == VIEM_FORMAT_MARKDOWN_SOURCE
            && CoreDocument.FormatForPath("document.markdown") == VIEM_FORMAT_MARKDOWN_SOURCE
            && CoreDocument.FormatForPath("document.html") == VIEM_FORMAT_CODE
            && CoreDocument.FormatForPath("document.htm") == VIEM_FORMAT_CODE
            && CoreDocument.FormatForPath("document.XHTML") == VIEM_FORMAT_CODE,
            "Markdown paths default to source presentation and HTML paths use Code");
        GlobalSelectionOptionTests.Run(device, dispatcher, Check);
        BidiControlTests.Run(device, dispatcher, Check);
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
            var beforeCopy = view.Presentation; var beforeSegments = view.Selection().Segments.Select(segment => (segment.text_start, segment.text_end)).ToArray();
            view.CopyOrCut(false); Check(copied.Contains("Hello alpha"), "clipboard write effect and ownership");
            Check(view.Presentation.mode == beforeCopy.mode && view.Presentation.cursor_utf8_offset == beforeCopy.cursor_utf8_offset
                && view.Selection().Segments.Select(segment => (segment.text_start, segment.text_end)).SequenceEqual(beforeSegments), "native Copy preserves selection and caret");
            copied = ""; view.Key(VIEM_KEY_ESCAPE); view.Command("gg\"*yy");
            Check(copied == "Hello alpha beta\n", "Windows maps Vim's * register to the system clipboard");
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
        Scenario("alpha beta", VIEM_FORMAT_PLAIN_TEXT, (doc, view) => {
            view.Ex("set keymodel=startsel,stopsel selectmode=mouse,key");
            view.Key(VIEM_KEY_RIGHT, modifiers: VIEM_KEY_MODIFIER_SHIFT); view.Key(VIEM_KEY_RIGHT, modifiers: VIEM_KEY_MODIFIER_SHIFT);
            byte[] before = doc.Source(doc.State.document_revision);
            view.BeginComposition(); view.UpdateComposition("日本", 2, 0);
            Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before), "Select IME marked text stays outside authoritative source");
            view.CancelComposition();
            Check(view.IsTextSelection && view.LogicalSelection().text_end == 2 && doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before),
                "cancelling Select IME restores the exact selected text and Select mode");
            view.BeginComposition(); view.UpdateComposition("日本", 2, 0); view.CommitComposition("日本");
            Check(doc.FormattedText() == "日本pha beta", "Select IME commits over the selected range rather than at its active caret");
            view.Undo(); Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before), "one undo restores a Select IME replacement");
        });
        Scenario("alpha beta", VIEM_FORMAT_MARKDOWN, (doc, view) => {
            view.Ex("set keymodel=startsel,stopsel selectmode=mouse,key,cmd");
            byte[] before = doc.Source(doc.State.document_revision);
            view.SelectFromCommand("viw");
            Check(view.IsTextSelection && view.LogicalSelection().text_start == 0 && view.LogicalSelection().text_end == 5
                && doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before),
                "native text-object selection remains a command sequence when selectmode includes cmd");
            view.ToggleSemantic(VIEM_SEMANTIC_STYLE_STRONG);
            Check(view.SemanticStyle(VIEM_SEMANTIC_STYLE_STRONG).state == VIEM_SEMANTIC_STYLE_STATE_ON, "formatting applies to Select mode without replacing its text");
            view.Undo(); Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before), "Select formatting retains one-step undo");
            view.Ex("set noautoselect selectmode=mouse"); view.SelectFromCommand("viw", VIEM_SELECTION_ORIGIN_MOUSE);
            Check(view.IsTextSelection, "native double-click selection uses the mouse option");
            view.SelectFromCommand("viw", VIEM_SELECTION_ORIGIN_KEY);
            Check(view.IsVisual && !view.IsTextSelection, "native menu selection independently uses the key option");
        });
        Scenario("before chosen after", VIEM_FORMAT_PLAIN_TEXT, (doc, view) => {
            view.Place(7, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, doc.State.document_revision);
            view.Command("i"); view.SelectFromCommand("viw");
            Check(view.Presentation.mode == VIEM_MODE_SELECTION_CHARACTER
                && view.LogicalSelection().text_start == 7 && view.LogicalSelection().text_end == 13,
                "native menu selection preserves the insertion boundary before entering its command sequence");
            view.Key(VIEM_KEY_RIGHT);
            Check(view.Presentation.mode == VIEM_MODE_INSERT && view.Presentation.cursor_utf8_offset == 13,
                "collapsing native menu selection resumes its original Insert mode");
        });
        Scenario("alpha\nbeta", 1, (doc, view) => {
            view.Command(":"); view.Text("echo 日本語"); var prompt = view.Prompt();
            view.EditPrompt(prompt, 5, 14, "word"); Check(view.Prompt().Text == "echo word", "UTF-8 command-line selection replacement");
            view.Key(VIEM_KEY_ESCAPE); view.Command("i"); view.Text("x"); view.Key(VIEM_KEY_ESCAPE);
            var before = doc.Source(doc.State.document_revision);
            doc.ConfigureDefaults("{\"tabstop\":4}"u8.ToArray(), "{}"u8.ToArray(), 72, "[]"u8.ToArray());
            Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before), "application defaults preserve source");
        });
        Scenario("# Heading\n\nHello **world** and مرحبا.\n", 2, (doc, view) => {
            var source = doc.Source(doc.State.document_revision); var layout = view.Layout();
            Check(layout.Clusters.Length > 0 && layout.Rows.Length > 1, "formatted DirectWrite layout");
            Check(layout.Clusters.Any(c => (c.bidi_level & 1) != 0), "DirectWrite bidi levels");
            Check(layout.PaintRuns.Length > 0 || view.Styles().Styles.Length > 1, "stylesheet export");
            view.Zoom(1.25f); Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(source), "zoom is presentation-only");
            view.SetMarkdownSource(true); Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(source), "source-view switch preserves source bytes");
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
        Scenario(string.Concat(Enumerable.Range(0, 2_000).Select(i => $"Paragraph {i}: **office** and *words* in a long paragraph that wraps across several visual rows.\n\n")), 2, (doc, view) => {
            var source = doc.Source(doc.State.document_revision);
            Check(view.Provider.ShapedCharacters < 20_000, "large Markdown opening shapes only a viewport band");
            long metadata = view.Provider.FontMetadataReads, shaped = view.Provider.ShapedCharacters;
            for (int page = 0; page < 30; page++) view.Key(VIEM_KEY_PAGE_DOWN);
            Check(view.Provider.FontMetadataReads - metadata < (view.Provider.ShapedCharacters - shaped) / 10,
                "paging reads native font metadata per run, not per character");
            Check(view.Layout().Info.coverage_hard_line_start > 0 && view.Layout().Info.coverage_hard_line_end < 2_000,
                "Markdown paging keeps geometry regional");
            long beforeReturn = view.Provider.ShapedCharacters;
            for (int page = 0; page < 5; page++) view.Key(VIEM_KEY_PAGE_UP);
            Check(view.Provider.ShapedCharacters == beforeReturn, "nearby Markdown pages reuse retained paragraph geometry");
            Check(view.Provider.GlyphBoundsHits > 0, "large Markdown reuses exact repeated glyph-ink queries within shaping requests");
            long beforeBounds = view.Provider.GlyphBoundsQueries;
            view.Provider.InvalidateMetrics();
            view.Key(VIEM_KEY_PAGE_DOWN);
            Check(view.Provider.ShapedCharacters > beforeReturn, "paging refreshes cached geometry after font metrics change");
            Check(view.Provider.GlyphBoundsQueries > beforeBounds, "glyph-ink queries are recomputed after font metrics change");
            Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(source), "Markdown paging preserves source bytes");
        });
        Scenario("\talpha  \n", 1, (doc, view) => {
            var before = doc.Source(doc.State.document_revision);
            doc.ConfigureDefaults("{}"u8.ToArray(), "{\"visibleWhitespace\":{\"style\":{\"size\":180,\"foreground\":{\"red\":1,\"green\":0,\"blue\":0,\"alpha\":1},\"underline\":true}}}"u8.ToArray(), 80, "[]"u8.ToArray());
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
        foreach (var key in new[] { VirtualKey.Left, VirtualKey.Right, VirtualKey.Up, VirtualKey.Down, VirtualKey.Home, VirtualKey.End, VirtualKey.PageUp, VirtualKey.PageDown })
        {
            var route = KeyPolicy.Route(key, false, true, false);
            Check(route.Kind != 0 && route.Modifiers == VIEM_KEY_MODIFIER_SHIFT, $"Shift+{key} reaches the core as selection navigation");
        }
        var wordLeft = KeyPolicy.Route(VirtualKey.Left, true, true, false);
        var wordRight = KeyPolicy.Route(VirtualKey.Right, true, true, false);
        Check(wordLeft.Kind == VIEM_KEY_WORD_LEFT && wordRight.Kind == VIEM_KEY_WORD_RIGHT
            && wordLeft.Modifiers == (VIEM_KEY_MODIFIER_SHIFT | VIEM_KEY_MODIFIER_CONTROL) && wordRight.Modifiers == wordLeft.Modifiers,
            "Ctrl+Shift word navigation preserves both movement and selection");
        Check(KeyPolicy.Route(VirtualKey.Home, true, true, false).Kind == VIEM_KEY_DOCUMENT_START
            && KeyPolicy.Route(VirtualKey.End, true, true, false).Kind == VIEM_KEY_DOCUMENT_END
            && KeyPolicy.Route(VirtualKey.End, true, true, false).Modifiers == (VIEM_KEY_MODIFIER_SHIFT | VIEM_KEY_MODIFIER_CONTROL),
            "Ctrl+Shift Home/End retain document-edge navigation and Shift");
        Check(KeyPolicy.Route(VirtualKey.A, false, true, false).Kind == 0
            && KeyPolicy.Route(VirtualKey.A, true, true, true).Action == NativeAction.None,
            "shifted characters and AltGr text remain native text input");
        Check(KeyPolicy.Route(VirtualKey.Number6, true, false, false).Codepoint == '^', "preserve vi Ctrl+6/Ctrl+^");
        WriteSuccess(checks);
    }
}
#endif
