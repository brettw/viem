#if DEBUG
using System.Text;
using Microsoft.Graphics.Canvas;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class ScrollDrawingTests
{
    internal static byte[] Fixture => Encoding.UTF8.GetBytes(string.Concat(Enumerable.Repeat(
        "# Section\n\n> **office** and *café* مرحبا 👩‍💻 in a paragraph that wraps across visual rows.\n\n- A list item with more words to scroll past.\n\n", 2_000)));

    internal static async Task Run(EditorPane pane)
    {
        void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); FrontendSmokeTests.UiChecks.Add(message); }
        var view = await pane.Ready;
        view.Key(VIEM_KEY_ESCAPE);
        await BackgroundLayoutTests.Idle(view);
        using var surface = new CanvasRenderTarget(pane.Canvas.Device, (float)pane.Canvas.ActualWidth, (float)pane.Canvas.ActualHeight, pane.Canvas.Dpi);
        byte[] Draw() { using (var drawing = surface.CreateDrawingSession()) pane.Draw(drawing); return surface.GetPixelBytes(); }
        pane.InvalidateDrawingCache();
        byte[] initial = Draw();
        int builds = pane.DrawingCacheBuilds;
        var identity = view.LayoutInfo().identity;
        float top = view.Viewport.top;
        long shaped = view.Provider.ShapedCharacters;
        foreach (float delta in new[] { 16f, 32f, 48f, 32f }) view.Scroll(0, top + delta);
        byte[] scrolled = Draw();
        Check(view.Viewport.top > top && CoreView.SameLayout(identity, view.LayoutInfo().identity),
            "wheel scrolling within overscan preserves the exact layout revision");
        Check(view.Provider.ShapedCharacters == shaped && pane.DrawingCacheBuilds == builds,
            "wheel scrolling reuses both prepared geometry and native drawing commands");
        Check(!initial.AsSpan().SequenceEqual(scrolled), "replaying cached commands moves the displayed text");
        pane.InvalidateDrawingCache();
        Check(Draw().AsSpan().SequenceEqual(scrolled), "translated styled, bidi, emoji and list drawing matches a fresh recording");

        view.Format(VIEM_FORMAT_MARKDOWN_SOURCE); view.VisibleWhitespace(true);
        Draw(); builds = pane.DrawingCacheBuilds;
        view.Scroll(0, view.Viewport.top + 16);
        byte[] markers = Draw();
        Check(pane.DrawingCacheBuilds > builds, "viewport-specific whitespace markers are refreshed while scrolling");
        pane.InvalidateDrawingCache();
        Check(Draw().AsSpan().SequenceEqual(markers), "scrolling whitespace markers matches a fresh recording");
        if (pane.LastError != null) throw pane.LastError;
    }
}
#endif
