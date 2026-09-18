using Microsoft.Graphics.Canvas;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Windows.UI;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    private CanvasCommandList? cachedBackground, cachedText;
    private LayoutSnapshot? drawnLayout;
    private ViemViewportStateV1 drawnViewport;
    private (double Width, double Height, float Dpi) drawnSize;
    private string? drawnWhitespaceStyle;
    private bool drawnWhitespaceEnabled;
    internal int DrawingCacheBuilds { get; private set; }

    internal void InvalidateDrawingCache()
    {
        cachedBackground?.Dispose(); cachedText?.Dispose();
        cachedBackground = cachedText = null; drawnLayout = null;
    }

    private void EnsureDrawingCache()
    {
        if (snapshot == null || View == null) return;
        var size = (Canvas.ActualWidth, Canvas.ActualHeight, Canvas.Dpi);
        string? whitespaceStyle = whitespace?.Style.GetRawText();
        if (cachedText != null && drawnLayout != null
            && CoreView.SameLayout(drawnLayout.Info.identity, snapshot.Info.identity)
            && drawnViewport.left == viewport.left && drawnViewport.top == viewport.top
            && drawnSize == size && drawnWhitespaceEnabled == WhitespaceEnabled
            && drawnWhitespaceStyle == whitespaceStyle) return;

        InvalidateDrawingCache();
        // Retain only two viewport-sized command streams. Direct2D owns the
        // referenced drawing resources; replay avoids thousands of WinRT calls
        // for unchanged glyphs during selection changes and caret blinking.
        var background = new CanvasCommandList(Canvas.Device);
        var foreground = new CanvasCommandList(Canvas.Device);
        try
        {
            using (var drawing = background.CreateDrawingSession()) DrawTextBackgrounds(drawing);
            using (var drawing = foreground.CreateDrawingSession()) DrawTextForeground(drawing);
        }
        catch { background.Dispose(); foreground.Dispose(); throw; }
        cachedBackground = background; cachedText = foreground;
        drawnLayout = snapshot; drawnViewport = viewport; drawnSize = size;
        drawnWhitespaceStyle = whitespaceStyle; drawnWhitespaceEnabled = WhitespaceEnabled;
        DrawingCacheBuilds++;
    }

    private void DrawTextBackgrounds(CanvasDrawingSession drawing)
    {
        if (snapshot == null) return;
        // Explicit character backgrounds precede selection, which must remain
        // visible even over opaque source-authored highlights.
        foreach (var cluster in snapshot.Clusters)
        {
            var paint = PaintFor(cluster.text_start);
            if ((paint.flags & VIEM_TEXT_PAINT_HAS_BACKGROUND) != 0) drawing.FillRectangle(OffsetRect(cluster.typographic_bounds, viewport), Color(paint.background));
        }
    }

    private void DrawTextForeground(CanvasDrawingSession drawing)
    {
        if (snapshot == null || View == null) return;
        var theme = preferences.Theme;
        using var glyphs = View.Provider.BeginDrawing(drawing);
        foreach (var row in snapshot.Rows)
        {
            if (row.y + row.ascent + row.descent + row.leading < viewport.top - 4 || row.y > viewport.top + Canvas.ActualHeight + 4) continue;
            for (ulong index = row.first_cluster; index < row.first_cluster + row.cluster_count; index++)
            {
                var cluster = snapshot.Clusters[checked((int)index)];
                if (cluster.ink_bounds.x + cluster.ink_bounds.width < viewport.left - 4 || cluster.ink_bounds.x > viewport.left + Canvas.ActualWidth + 4) continue;
                var paint = PaintFor(cluster.text_start);
                var bounds = OffsetRect(cluster.typographic_bounds, viewport);
                Color foreground = (paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0 ? theme.Foreground : Color(paint.foreground);
                glyphs.Draw(cluster.render_run, new(cluster.x - viewport.left, row.baseline - viewport.top), foreground);
                if ((paint.flags & VIEM_TEXT_PAINT_UNDERLINE) != 0) drawing.DrawLine((float)bounds.X, row.baseline - viewport.top + 2, (float)bounds.Right, row.baseline - viewport.top + 2, foreground);
                if ((paint.flags & VIEM_TEXT_PAINT_STRIKETHROUGH) != 0) drawing.DrawLine((float)bounds.X, row.baseline - viewport.top - row.ascent * .3f, (float)bounds.Right, row.baseline - viewport.top - row.ascent * .3f, foreground);
            }
        }
        foreach (var d in snapshot.Decorations)
        {
            var row = snapshot.Rows.FirstOrDefault(r => r.row_index == d.row_index);
            Color foreground = (d.paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0 ? theme.Foreground : Color(d.paint.foreground);
            if ((d.flags & VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER) != 0) drawing.FillRectangle(OffsetRect(d.typographic_bounds, viewport), foreground);
            else glyphs.Draw(d.render_run, new(d.x - viewport.left, row.baseline - viewport.top), foreground);
        }
        if (whitespace != null) View.Provider.DrawWhitespace(drawing, whitespace, snapshot, viewport, offset => {
            var paint = PaintFor(offset); return (paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0 ? theme.Foreground : Color(paint.foreground);
        }, theme.Foreground);
    }
}
