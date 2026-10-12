#if DEBUG
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Xaml;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Windows.Foundation;

namespace Viem.Windows.Diagnostics;

internal static class ScrollbarOverlayTests
{
    internal static async Task Run(EditorPane pane)
    {
        void Check(bool value, string name)
        { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
        var view = await pane.Ready;
        double width = pane.Canvas.ActualWidth, height = pane.Canvas.ActualHeight;
        var identity = view.LayoutInfo().identity;
        long shaped = view.Provider.ShapedCharacters;
        try {
            foreach (var visibility in new[] { Visibility.Collapsed, Visibility.Visible }) {
                pane.HorizontalScrollControl.Visibility = visibility;
                pane.UpdateLayout();
                var track = pane.VerticalScrollControl;
                var trackTop = track.TransformToVisual(pane.Canvas).TransformPoint(new Point());
                var horizontalTop = pane.HorizontalScrollControl.TransformToVisual(pane.Canvas).TransformPoint(new Point());
                Check(pane.Canvas.ActualWidth == width && pane.Canvas.ActualHeight == height
                    && Math.Abs(trackTop.Y) < .1 && Math.Abs(track.ActualHeight - height) < .1,
                    $"{visibility} horizontal scrollbar keeps the full editor and vertical track height");
                if (visibility == Visibility.Visible)
                    Check(Math.Abs(horizontalTop.Y + pane.HorizontalScrollControl.ActualHeight - height) < .1,
                        "the native horizontal scrollbar overlays the bottom of the document canvas");
            }
        } finally { pane.Refresh(); }
        Check(CoreView.SameLayout(identity, view.LayoutInfo().identity) && view.Provider.ShapedCharacters == shaped,
            "changing horizontal scrollbar visibility does not reshape or resize the document");

        var band = pane.VerticalScrollbarPaintBounds;
        Check(band.Width > 0 && pane.PaintViewportWidth == width + band.Width
            && view.LayoutInfo().viewport_width == (float)width,
            "overflow paint reaches beneath the vertical scrollbar without changing the core viewport");
        using var target = new CanvasRenderTarget(pane.Canvas.Device, (float)pane.PaintViewportWidth,
            (float)height, pane.Canvas.Dpi);
        byte[] Draw() { using (var drawing = target.CreateDrawingSession()) pane.Draw(drawing); return target.GetPixelBytes(); }
        byte[] cached = Draw();
        int pixelsWide = (int)target.SizeInPixels.Width, pixelsHigh = (int)target.SizeInPixels.Height;
        int firstX = Math.Min(pixelsWide - 1, (int)Math.Ceiling((band.X + 1) * pane.Canvas.Dpi / 96));
        bool ink = false;
        for (int y = 1; y < pixelsHigh && !ink; y++)
            for (int x = firstX; x < pixelsWide; x++) {
                int offset = (y * pixelsWide + x) * 4;
                if (Math.Abs(cached[offset] - cached[0]) + Math.Abs(cached[offset + 1] - cached[1])
                    + Math.Abs(cached[offset + 2] - cached[2]) > 30) { ink = true; break; }
            }
        Check(ink, "DirectWrite overflow ink remains visible beneath the vertical scrollbar track");
        pane.InvalidateDrawingCache();
        Check(Draw().AsSpan().SequenceEqual(cached),
            "overflow behind the scrollbar has identical cached and freshly recorded pixels");
        using var strip = new CanvasRenderTarget(pane.Canvas.Device, (float)band.Width, (float)band.Height, pane.Canvas.Dpi);
        using (var drawing = strip.CreateDrawingSession()) pane.DrawScrollbarPaint(drawing);
        byte[] stripPixels = strip.GetPixelBytes();
        bool stripInk = false;
        for (int offset = 4; offset < stripPixels.Length; offset += 4)
            if (Math.Abs(stripPixels[offset] - stripPixels[0]) + Math.Abs(stripPixels[offset + 1] - stripPixels[1])
                + Math.Abs(stripPixels[offset + 2] - stripPixels[2]) > 30) { stripInk = true; break; }
        Check(stripInk, "the scrollbar canvas callback translates overflow ink into its native strip");
        Check(view.Provider.ShapedCharacters == shaped,
            "painting the scrollbar strip reuses materialized glyphs without wider shaping");
    }
}
#endif
