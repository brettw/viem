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
    private float drawnTop, drawnBottom;
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
            && drawnViewport.left == viewport.left
            && (drawnViewport.top == viewport.top || (!WhitespaceEnabled
                && viewport.top >= drawnTop && viewport.top + Canvas.ActualHeight <= drawnBottom))
            && drawnSize == size && drawnWhitespaceEnabled == WhitespaceEnabled
            && drawnWhitespaceStyle == whitespaceStyle) return;

        InvalidateDrawingCache();
        // Retain two bounded command streams, including one screen of vertical
        // overscan in the direction of travel. Translate their replay instead of
        // reissuing thousands of WinRT glyph calls for every wheel tick.
        // Whitespace markers are viewport-specific, so retain their old path.
        bool backwards = viewport.top < drawnViewport.top;
        drawnTop = viewport.top - (!WhitespaceEnabled && backwards ? (float)Canvas.ActualHeight : 0);
        drawnBottom = viewport.top + (!WhitespaceEnabled && !backwards ? 2 : 1) * (float)Canvas.ActualHeight;
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
        // Shared edges of tiled box slices must not be antialiased separately.
        var antialiasing = drawing.Antialiasing;
        drawing.Antialiasing = CanvasAntialiasing.Aliased;
        try {
            foreach (var decoration in snapshot.Decorations)
            {
                if ((decoration.flags & (VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND | VIEM_LAYOUT_DECORATION_BLOCK_BORDER | VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER)) == 0) continue;
                if (decoration.typographic_bounds.y + decoration.typographic_bounds.height < drawnTop - 4
                    || decoration.typographic_bounds.y > drawnBottom + 4) continue;
                drawing.FillRectangle(OffsetRect(decoration.typographic_bounds, viewport), (decoration.paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0 ? preferences.Theme.Foreground : Color(decoration.paint.foreground));
            }
        } finally { drawing.Antialiasing = antialiasing; }
        // Explicit character backgrounds precede selection, which must remain
        // visible even over opaque source-authored highlights.
        foreach (var cluster in snapshot.Clusters)
        {
            if (cluster.typographic_bounds.y + cluster.typographic_bounds.height < drawnTop - 4
                || cluster.typographic_bounds.y > drawnBottom + 4) continue;
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
            if (row.y + row.ascent + row.descent + row.leading < drawnTop - 4 || row.y > drawnBottom + 4) continue;
            for (ulong index = row.first_cluster; index < row.first_cluster + row.cluster_count; index++)
            {
                var cluster = snapshot.Clusters[checked((int)index)];
                if (View.Provider.IsInlineImage(cluster.render_run)) { glyphs.Flush(); continue; }
                if (cluster.ink_bounds.x + cluster.ink_bounds.width < viewport.left - 4 || cluster.ink_bounds.x > viewport.left + Canvas.ActualWidth + 4) continue;
                var paint = PaintFor(cluster.text_start);
                var bounds = OffsetRect(cluster.typographic_bounds, viewport);
                Color foreground = (paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0 ? theme.Foreground : Color(paint.foreground);
                glyphs.Draw(cluster.render_run, new(cluster.x - viewport.left, row.baseline - viewport.top), foreground);
                if ((paint.flags & (VIEM_TEXT_PAINT_UNDERLINE | VIEM_TEXT_PAINT_STRIKETHROUGH)) != 0) glyphs.Flush();
                if ((paint.flags & VIEM_TEXT_PAINT_UNDERLINE) != 0) drawing.DrawLine((float)bounds.X, row.baseline - viewport.top + 2, (float)bounds.Right, row.baseline - viewport.top + 2, foreground);
                if ((paint.flags & VIEM_TEXT_PAINT_STRIKETHROUGH) != 0) drawing.DrawLine((float)bounds.X, row.baseline - viewport.top - row.ascent * .3f, (float)bounds.Right, row.baseline - viewport.top - row.ascent * .3f, foreground);
            }
        }
        glyphs.Flush();
        foreach (var d in snapshot.Decorations)
        {
            var row = snapshot.Rows.FirstOrDefault(r => r.row_index == d.row_index);
            // A quote border can span several rows even when its owner row is
            // outside the recording band; cull against its actual bounds.
            if (d.typographic_bounds.y + d.typographic_bounds.height < drawnTop - 4
                || d.typographic_bounds.y > drawnBottom + 4) continue;
            Color foreground = (d.paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0 ? theme.Foreground : Color(d.paint.foreground);
            if ((d.flags & (VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND | VIEM_LAYOUT_DECORATION_BLOCK_BORDER | VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER)) != 0) continue;
            glyphs.Draw(d.render_run, new(d.x - viewport.left, row.baseline - viewport.top), foreground);
        }
        glyphs.Flush();
        if (whitespace != null) View.Provider.DrawWhitespace(drawing, whitespace, snapshot, viewport, offset => {
            var paint = PaintFor(offset); return (paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0 ? theme.Foreground : Color(paint.foreground);
        }, theme.Foreground);
    }
}
