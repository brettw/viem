using System.Numerics;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI.Xaml;
using Windows.Foundation;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    // The interactive viewport still ends at the vertical scrollbar's inside
    // edge. Replay already shaped overflow behind its translucent track without
    // changing wrapping, hit testing, IME geometry or the core viewport width.
    private readonly CanvasControl scrollbarPaint = new() { IsHitTestVisible = false };
    internal double PaintViewportWidth => Canvas.ActualWidth
        + (viewport.maximum_left > 0 ? scrollbarPaint.ActualWidth : 0);

    private void BuildScrollbars()
    {
        Children.Add(scrollbarPaint); SetColumn(scrollbarPaint, 2);
        scrollbarPaint.Draw += (_, args) => Run(() => DrawScrollbarPaint(args.DrawingSession));
        scrollbarPaint.SizeChanged += (_, _) => Canvas.Invalidate();
        Children.Add(vertical); SetColumn(vertical, 2);
        // The horizontal bar overlays the canvas. Showing overflow must not
        // change row coverage or shorten the vertical scrollbar track.
        horizontal.VerticalAlignment = VerticalAlignment.Bottom;
        Children.Add(horizontal); SetColumn(horizontal, 1);
    }

    internal void DrawScrollbarPaint(CanvasDrawingSession drawing)
    {
        if (disposed || snapshot == null || View == null || viewport.maximum_left <= 0) {
            ClearCanvasBackground(drawing);
            return;
        }
        var transform = drawing.Transform;
        try {
            drawing.Transform = Matrix3x2.CreateTranslation(-(float)Canvas.ActualWidth, 0);
            Draw(drawing);
        } finally { drawing.Transform = transform; }
    }

#if DEBUG
    internal Rect VerticalScrollbarPaintBounds => new(Canvas.ActualWidth, 0,
        scrollbarPaint.ActualWidth, scrollbarPaint.ActualHeight);
#endif
}
