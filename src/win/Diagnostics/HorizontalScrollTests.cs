#if DEBUG
using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Xaml;
using Viem.Windows.Editor;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class HorizontalScrollTests
{
    private const string Needle = "ViemFindTarget";
    private const int MatchStart = 2_048;
    internal static byte[] Fixture => Encoding.UTF8.GetBytes(
        new string('W', MatchStart) + Needle + new string('W', 96) + "\nShort line\n");

    internal static async Task Run(EditorPane pane)
    {
        void CheckPane(string operation)
        {
            if (pane.LastError is { } error)
                throw new InvalidOperationException($"The editor reported an error after {operation}.", error);
        }
        void Step(string operation, Action action)
        {
            try { action(); }
            catch (Exception error) { throw new InvalidOperationException($"Native horizontal scrolling failed while {operation}.", error); }
            CheckPane(operation);
        }
        void Check(bool value, string message) { CheckPane(message); if (!value) throw new InvalidOperationException(message); FrontendSmokeTests.UiChecks.Add(message); }
        var view = await pane.Ready;
        CheckPane("initializing the horizontal scroll pane");
        Step("entering Normal mode", () => view.Key(VIEM_KEY_ESCAPE));
        await BackgroundLayoutTests.Idle(view);
        CheckPane("waiting for background layout");
        view.BackgroundLayout.Enabled = false;
        Step("setting view margins", () => view.Padding(17, 10, 29, 10));
        Step("disabling whitespace markers", () => view.VisibleWhitespace(false));
        using var surface = new CanvasRenderTarget(pane.Canvas.Device, (float)pane.Canvas.ActualWidth, (float)pane.Canvas.ActualHeight, pane.Canvas.Dpi);
        byte[] Draw() { using (var drawing = surface.CreateDrawingSession()) pane.Draw(drawing); CheckPane("drawing the editor"); return surface.GetPixelBytes(); }

        // Unicode wrapping intentionally leaves an unbreakable word wider than
        // the canvas. Scrollbars, caret reveal and search share that same range.
        Step("moving to the test line start", () => view.Command("gg0"));
        foreach (bool wrap in new[] { false, true })
        {
            Step($"setting wrapping {(wrap ? "on" : "off")}", () => view.Wrap(wrap));
            var overflow = view.Viewport;
            Check(overflow.maximum_left > 0 && pane.HorizontalScrollControl.Visibility == Visibility.Visible
                && pane.HorizontalScrollControl.Maximum == overflow.maximum_left,
                $"the horizontal scrollbar exposes unbreakable overflow with wrapping {(wrap ? "on" : "off")}");
            ulong cursor = view.Presentation.cursor_utf8_offset;
            Step($"scrolling the overflowing line with wrapping {(wrap ? "on" : "off")}", () => view.Scroll(overflow.maximum_left / 2, overflow.top));
            Check(view.Viewport.left > 0 && pane.HorizontalScrollControl.Value == view.Viewport.left
                && view.Presentation.cursor_utf8_offset == cursor,
                $"the horizontal scrollbar follows explicit scrolling without moving the caret with wrapping {(wrap ? "on" : "off")}");

            Step($"moving the block caret to line end with wrapping {(wrap ? "on" : "off")}", () => view.Command("gg$"));
            var caret = pane.CaretRectangle;
            Check(view.Presentation.caret_shape == VIEM_CARET_SHAPE_CELL && caret.Width > 1
                && caret.X >= -.1 && caret.Right <= pane.Canvas.ActualWidth + .1 && view.Viewport.left > 0,
                $"the complete native block caret is visible after $ with wrapping {(wrap ? "on" : "off")}");
            Step($"moving the block caret to column zero with wrapping {(wrap ? "on" : "off")}", () => view.Command("0"));
            caret = pane.CaretRectangle;
            Check(caret.X >= -.1 && caret.Right <= pane.Canvas.ActualWidth + .1 && view.Viewport.left == 0,
                $"returning to column zero reveals the native block caret with wrapping {(wrap ? "on" : "off")}");

            Step($"opening search with wrapping {(wrap ? "on" : "off")}", () => view.Command("/"));
            Step($"typing the search query with wrapping {(wrap ? "on" : "off")}", () => view.Text(Needle));
            Step($"accepting the search query with wrapping {(wrap ? "on" : "off")}", () => view.Key(VIEM_KEY_ENTER));
            var matchViewport = view.Viewport;
            var matchClusters = view.Layout().Clusters.Where(cluster =>
                cluster.text_start < (ulong)(MatchStart + Needle.Length) && (ulong)MatchStart < cluster.text_end).ToArray();
            Check(view.Presentation.cursor_utf8_offset == (ulong)MatchStart && matchClusters.Length > 0
                && matchClusters.Min(cluster => cluster.text_start) <= (ulong)MatchStart
                && matchClusters.Max(cluster => cluster.text_end) >= (ulong)(MatchStart + Needle.Length)
                && matchClusters.Min(cluster => cluster.typographic_bounds.x) >= matchViewport.left - .1f
                && matchClusters.Max(cluster => cluster.typographic_bounds.x + cluster.typographic_bounds.width)
                    <= matchViewport.left + pane.Canvas.ActualWidth + .1,
                $"the full DirectWrite search match is visible with wrapping {(wrap ? "on" : "off")}");
            double cursorX = view.CaretGeometry().rect.x;
            double matchLeft = Math.Min(cursorX, matchClusters.Min(cluster =>
                Math.Min(cluster.x, cluster.typographic_bounds.x)));
            double matchRight = Math.Max(cursorX + 2, matchClusters.Max(cluster =>
                Math.Max(cluster.x + cluster.advance, cluster.typographic_bounds.x + cluster.typographic_bounds.width)));
            double width = pane.Canvas.ActualWidth;
            double expectedLeft = Math.Clamp(
                Math.Clamp(cursorX - width / 2, matchRight - width, matchLeft), 0, matchViewport.maximum_left);
            Check(Math.Abs(matchViewport.left - expectedLeft) <= .1,
                $"search centers the native caret then shifts only enough to fit the match with wrapping {(wrap ? "on" : "off")}");
            byte[] revealed = Draw();
            pane.InvalidateDrawingCache();
            Check(Draw().AsSpan().SequenceEqual(revealed),
                $"horizontal search reveal drawing matches a fresh recording with wrapping {(wrap ? "on" : "off")}");
        }
        Step("deleting the overflowing test line", () => view.Command("ggdd"));
        Check(view.Viewport.maximum_left == 0 && pane.HorizontalScrollControl.Visibility == Visibility.Collapsed,
            "the horizontal scrollbar disappears when the wrapped overflow is removed");
        Step("undoing the test deletion", () => view.Undo());
        Check(pane.Document.Source(pane.Document.State.document_revision).AsSpan().SequenceEqual(Fixture),
            "horizontal scrolling diagnostics restore the original source");
        CheckPane("completing horizontal scrolling diagnostics");
    }
}
#endif
