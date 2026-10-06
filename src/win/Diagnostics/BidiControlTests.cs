#if DEBUG
using System.Numerics;
using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.UI.Dispatching;
using Viem.Windows.Core;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class BidiControlTests
{
    internal static void Run(CanvasDevice device, DispatcherQueue dispatcher, Action<bool, string> check)
    {
        var capture = new GlyphCapture(_ => throw new InvalidOperationException("A glyphless run needs no font metadata."));
        // DirectWrite keeps a character cluster map [0, 1, ...] for NO_VISUAL
        // runs while reporting glyphCount == 0. Those are not glyph indices.
        capture.DrawGlyphRun(Vector2.Zero, null!, 16, [], false, 1, null!, CanvasTextMeasuringMode.Natural,
            "", "\u200f\u200f", [0, 1], 7, CanvasGlyphOrientation.Upright);
        check(capture.Extract(7, 9, 0, 0).Count == 0 && capture.BidiLevelAt(7) == 1 && capture.BidiLevelAt(8) == 1,
            "glyphless control runs retain bidi levels without indexing or drawing nonexistent glyphs");
        check(DirectWriteProvider.VisualOrder(new uint[] { 1, 1 }).SequenceEqual(new ulong[] { 1, 0 })
            && DirectWriteProvider.VisualOrder(new uint[] { 0, 1, 2, 2, 1, 0 }).SequenceEqual(new ulong[] { 0, 4, 2, 3, 1, 5 }),
            "embedding levels resolve visual order even when control characters have identical x positions");

        string[] controls = ["\u061c", "\u200e", "\u200f", "\u202a", "\u202b", "\u202c", "\u202d", "\u202e",
            "\u2066", "\u2067", "\u2068", "\u2069"];
        foreach (uint format in new[] { VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_MARKDOWN_SOURCE, VIEM_FORMAT_MARKDOWN, VIEM_FORMAT_CODE })
        foreach (string family in new[] { "Segoe UI", "Recursive" })
        {
            byte[] original = "ab"u8.ToArray();
            using var document = new CoreDocument(original, format: format);
            using var view = new CoreView(document, device, dispatcher, 700, 200);
            view.BackgroundLayout.Enabled = false;
            byte[] styles = view.ExportStyleDefaults();
            try
            {
                view.EditStyleFont(view.Styles().Styles.Single(s => s.Id == "Paragraph"), [family], FontCatalog.ForFamilyChange(family, null));
                foreach (string control in controls)
                {
                    view.Place(1, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
                    view.Command("i"); view.Text(control + control); view.Key(VIEM_KEY_ESCAPE);
                    byte[] inserted = Encoding.UTF8.GetBytes("a" + control + control + "b");
                    var layout = view.Layout();
                    check(document.Source(document.State.document_revision).AsSpan().SequenceEqual(inserted)
                        && view.Provider.LastError == null && layout.Diagnostics.Length == 0,
                        $"RTL control U+{char.ConvertToUtf32(control, 0):X4} inserts losslessly with {family}, format {format}");
                    foreach (ulong boundary in new[] { 1UL, 1UL + (ulong)Encoding.UTF8.GetByteCount(control + control) })
                    {
                        view.Place(boundary, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, document.State.document_revision);
                        var caret = view.CaretGeometry();
                        check(float.IsFinite(caret.rect.x) && float.IsFinite(caret.rect.y) && caret.rect.height > 0,
                            "control characters retain usable caret geometry at both logical boundaries");
                    }
                    byte[] pixels = Render(view, device);
                    if (control == controls[0])
                    {
                        using var fresh = new CoreView(document, device, dispatcher, 700, 200);
                        fresh.BackgroundLayout.Enabled = false;
                        check(pixels.AsSpan().SequenceEqual(Render(fresh, device)),
                            "inserting RTL controls yields the same pixels as a fresh native layout");
                    }
                    view.Undo();
                    check(document.Source(document.State.document_revision).AsSpan().SequenceEqual(original) && !document.IsDirty,
                        "undo of RTL controls restores exact source and clean state");
                    view.Redo();
                    check(document.Source(document.State.document_revision).AsSpan().SequenceEqual(inserted), "redo preserves RTL control bytes");
                    Render(view, device);
                    view.Undo();
                }
            }
            finally { if (view.UsesGlobalStyles) view.ReplaceCodeStyles(styles); }
        }

        // Also exercise the no-visible-glyph paragraph and nested embeddings.
        foreach (string text in new[] { "\u200f", "\u200f\u200f", "A\u202bאב\u202a12\u202cגד\u202cZ", "אב \u2066abc \u2067גד\u2069 xyz\u2069 ה" })
        {
            using var document = new CoreDocument(Encoding.UTF8.GetBytes(text), format: VIEM_FORMAT_PLAIN_TEXT);
            using var view = new CoreView(document, device, dispatcher, 700, 200);
            check(view.Layout().Diagnostics.Length == 0, "control-only and nested bidi paragraphs produce valid native layout");
            Render(view, device);
        }

        byte[] largeSource = Encoding.UTF8.GetBytes(string.Concat(Enumerable.Repeat("A paragraph with words.\n", 10_000)));
        using var largeDocument = new CoreDocument(largeSource, format: VIEM_FORMAT_PLAIN_TEXT);
        using var largeView = new CoreView(largeDocument, device, dispatcher, 700, 200);
        largeView.BackgroundLayout.Enabled = false;
        long shaped = largeView.Provider.ShapedCharacters;
        largeView.Command("i"); largeView.Text("\u200f"); largeView.Key(VIEM_KEY_ESCAPE);
        check(largeView.Layout().Diagnostics.Length == 0 && shaped < 20_000 && largeView.Provider.ShapedCharacters - shaped < 2_000,
            "inserting an RTL control in a large document only reshapes bounded visible content");
        Render(largeView, device);
        largeView.Undo();
        check(largeDocument.Source(largeDocument.State.document_revision).AsSpan().SequenceEqual(largeSource),
            "undo of a large-document RTL control restores every original byte");
    }

    private static byte[] Render(CoreView view, CanvasDevice device)
    {
        var layout = view.Layout();
        using var target = new CanvasRenderTarget(device, 700, 200, 96);
        byte[]? reference = null;
        foreach (bool combine in new[] { false, true })
        {
            using (var drawing = target.CreateDrawingSession())
            {
                drawing.Clear(Microsoft.UI.Colors.White);
                using var glyphs = view.Provider.BeginDrawing(drawing, combine);
                foreach (var row in layout.Rows)
                foreach (var cluster in layout.Clusters.Where(c => c.row_index == row.row_index))
                    glyphs.Draw(cluster.render_run, new(cluster.x, row.baseline), Microsoft.UI.Colors.Black);
            }
            byte[] pixels = target.GetPixelBytes();
            if (reference != null && !reference.AsSpan().SequenceEqual(pixels))
                throw new InvalidOperationException("RTL controls differ between individual and combined glyph drawing.");
            reference = pixels;
        }
        return reference!;
    }
}
#endif
