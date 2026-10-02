#if DEBUG
using System.Diagnostics;
using System.Numerics;
using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Dispatching;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class BackgroundLayoutTests
{
    internal static async Task Idle(CoreView view)
    {
        var start = Stopwatch.StartNew();
        while (!view.BackgroundLayout.IsIdle && start.Elapsed < TimeSpan.FromSeconds(15)) await Task.Delay(10);
        if (view.BackgroundLayout.LastError != null) throw new InvalidOperationException(view.BackgroundLayout.LastError);
        if (!view.BackgroundLayout.IsIdle) throw new InvalidOperationException("Background layout did not become idle.");
    }

    internal static async Task Run(CanvasDevice device, DispatcherQueue dispatcher)
    {
        void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); FrontendSmokeTests.UiChecks.Add(message); }
        byte[] tableSource = Encoding.UTF8.GetBytes("| A | B |\n| --- | --- |\n" + string.Join("\n",
            Enumerable.Range(0, 350).Select(i => $"| row {i} | {(i == 349 ? new string('W', 120) : "small")} |")));
        using (var tableDoc = new CoreDocument(tableSource, format: VIEM_FORMAT_MARKDOWN, markdownFormattedView: true))
        using (var tableView = new CoreView(tableDoc, device, dispatcher, 600, 400))
        {
            tableView.BackgroundLayout.Enabled = false;
            tableView.TableWidthRefinement.Enabled = true;
            tableView.TableWidthRefinement.Update();
            var tableCaret = tableView.Presentation.cursor_utf8_offset;
            var timer = Stopwatch.StartNew();
            while (tableView.TableWidthRefinement.Started == 0 && timer.Elapsed < TimeSpan.FromSeconds(5)) await Task.Delay(1);
            tableView.Scroll(0, 6);
            var tableTop = tableView.Viewport.top;
            timer.Restart();
            while (!tableView.TableWidthRefinement.IsIdle && timer.Elapsed < TimeSpan.FromSeconds(15)) await Task.Delay(10);
            Check(tableView.TableWidthRefinement.IsIdle && tableView.TableWidthRefinement.LastError == null && tableView.TableWidthRefinement.Installed > 0,
                "table width discovery resumes after a small scroll and completes on the idle worker");
            Check(tableView.Presentation.cursor_utf8_offset == tableCaret && tableView.Viewport.top == tableTop
                && tableDoc.Source(tableDoc.State.document_revision).AsSpan().SequenceEqual(tableSource),
                "table width refinement preserves source, caret and viewport");
            int tableStarted = tableView.TableWidthRefinement.Started;
            tableView.Refresh(); await Task.Delay(50);
            Check(tableView.TableWidthRefinement.Started == tableStarted, "completed table discovery does not poll while idle");
            tableView.Resize(610, 410);
            tableView.Dispose();
            timer.Restart();
            while (!tableView.TableWidthRefinement.IsIdle && timer.Elapsed < TimeSpan.FromSeconds(15)) await Task.Delay(10);
            Check(tableView.TableWidthRefinement.IsIdle, "closing a table view cancels refinement without waiting on its worker");
        }

        int uiThread = Environment.CurrentManagedThreadId;
        byte[] source = Encoding.UTF8.GetBytes(string.Concat(Enumerable.Range(0, 10_000).Select(i =>
            $"Paragraph {i}: **office** and *words* مرحبا 👩‍💻 that wrap into multiple visual rows.\n\n")));
        using var doc = new CoreDocument(source, format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(doc, device, dispatcher, 600, 400);
        var original = view.Viewport;
        ulong caret = view.Presentation.cursor_utf8_offset;
        await Idle(view);
        Check(view.Provider.BackgroundShapedCharacters > 0 && view.Provider.BackgroundShapingThread != uiThread,
            "large Markdown pre-layout shapes on an independent worker");
        Check(view.Provider.BackgroundShapedCharacters < 30_000 && view.BackgroundLayout.Started <= 32,
            "background work stops after a bounded band, not the whole file");
        Check(view.Viewport.top == original.top && view.Viewport.layout_revision == original.layout_revision && view.Presentation.cursor_utf8_offset == caret,
            "background cache installation preserves viewport, caret and visible layout");
        int started = view.BackgroundLayout.Started;
        view.Command("l"); await Task.Delay(100);
        Check(view.BackgroundLayout.Started == started, "caret-only movement does not schedule idle pre-layout");
        long before = view.Provider.ShapedCharacters;
        for (int page = 0; page < 3; page++) view.Key(VIEM_KEY_PAGE_DOWN);
        Check(view.Provider.ShapedCharacters == before, "three prepared Markdown pages need no foreground shaping");
        int drawCalls = 0;
        byte[] Pixels(CoreView rendered, bool combine = true)
        {
            using var target = new CanvasRenderTarget(device, 600, 400, 96);
            var snapshot = rendered.Layout();
            float top = rendered.Viewport.top;
            using (var drawing = target.CreateDrawingSession())
            {
                drawing.Clear(Microsoft.UI.Colors.White);
                using var batch = rendered.Provider.BeginDrawing(drawing, combine);
                foreach (var row in snapshot.Rows)
                foreach (var cluster in snapshot.Clusters.Where(c => c.row_index == row.row_index))
                    batch.Draw(cluster.render_run, new Vector2(cluster.x, row.baseline - top), Microsoft.UI.Colors.Black);
                batch.Flush(); drawCalls = batch.DrawCalls;
            }
            return target.GetPixelBytes();
        }
        byte[] batched = Pixels(view); int combinedCalls = drawCalls;
        Check(batched.AsSpan().SequenceEqual(Pixels(view, false)), "combined glyph runs match individual styled, bidi and emoji drawing pixel for pixel");
        Check(combinedCalls * 2 < drawCalls, "glyph batching removes most per-cluster native drawing calls");
        using (var cold = new CoreView(doc, device, dispatcher, 600, 400))
        {
            cold.BackgroundLayout.Enabled = false;
            cold.Command("l");
            for (int page = 0; page < 3; page++) cold.Key(VIEM_KEY_PAGE_DOWN);
            Check(Pixels(view).AsSpan().SequenceEqual(Pixels(cold)), "worker-prepared styled, bidi and emoji glyphs match foreground pixels");
        }
        // Changes race deliberately with queued or running chunks. Installation
        // must validate the new width, font generation and source revision.
        view.Resize(300, 240); view.Command("ggi"); view.Text("Changed "); view.Key(VIEM_KEY_ESCAPE);
        view.Provider.InvalidateMetrics(); view.Refresh();
        await Idle(view);
        view.Key(VIEM_KEY_PAGE_DOWN);
        Check(view.Layout().Info.viewport_width == 300 && doc.FormattedText().StartsWith("Changed "),
            "pre-layout rejects obsolete width, metrics and document revisions");
        Check(!doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(source), "only the explicit edit changes source");
        view.Resize(310, 250);
        await Task.Delay(1);
        view.Dispose();
        await Idle(view);
        Check(view.Provider.LiveResourceCount == 0, "closing with queued or active pre-layout releases all native resources");

        // Distinct source lines share a small glyph alphabet. Scrolling must
        // shape new paragraphs without repeating the native ink query per line.
        using var repeated = new CoreDocument(Encoding.UTF8.GetBytes(string.Concat(
            Enumerable.Range(0, 10_000).Select(i => $"Line {i}: repeated letters abcdef ghijkl mnopqr stuvwx yz.\n"))));
        using var inkView = new CoreView(repeated, device, dispatcher, 600, 400);
        inkView.BackgroundLayout.Enabled = false;
        long queries = inkView.Provider.GlyphBoundsQueries, characters = inkView.Provider.ShapedCharacters;
        for (int page = 0; page < 12; page++) inkView.Key(VIEM_KEY_PAGE_DOWN);
        Check(inkView.Provider.GlyphBoundsQueries - queries < (inkView.Provider.ShapedCharacters - characters) / 10,
            "large-file paging reuses glyph ink bounds across distinct source lines");
        Check(inkView.Provider.CachedGlyphBounds is > 0 and <= 1024, "native glyph ink cache remains size bounded");
        var pixels = Pixels(inkView);
        queries = inkView.Provider.GlyphBoundsQueries;
        inkView.Provider.InvalidateMetrics(); inkView.Resize(600, 400);
        Check(inkView.Provider.GlyphBoundsQueries > queries && pixels.AsSpan().SequenceEqual(Pixels(inkView)),
            "metrics invalidation recomputes native ink bounds and preserves rendering");
        queries = inkView.Provider.GlyphBoundsQueries;
        inkView.Provider.ResetDevice(device); inkView.Resize(600, 400);
        Check(inkView.Provider.GlyphBoundsQueries > queries, "device replacement invalidates native ink bounds");
        queries = inkView.Provider.GlyphBoundsQueries;
        inkView.Zoom(1.5f);
        Check(inkView.Provider.GlyphBoundsQueries > queries, "different glyph sizes cannot reuse old native ink bounds");
        inkView.Dispose();
        Check(inkView.Provider.CachedGlyphBounds == 0, "disposing a view releases its cached native font faces and ink bounds");
    }
}
#endif
