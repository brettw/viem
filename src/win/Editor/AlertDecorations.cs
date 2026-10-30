using System.Numerics;
using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Geometry;
using Microsoft.Graphics.Canvas.Text;
using Windows.Foundation;
using Windows.UI;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

/// <summary>GitHub alert furniture: an outlined icon and a semibold title in
/// the alert's text color. Icons use the macOS EVAlertDrawing 16-unit grid,
/// scaled to the core's square bounds.</summary>
internal sealed partial class EditorPane
{
    internal static uint AlertKind(uint flags) => (flags & VIEM_LAYOUT_DECORATION_ALERT_KIND_MASK) >> (int)VIEM_LAYOUT_DECORATION_ALERT_KIND_SHIFT;

    private void DrawAlerts(CanvasDrawingSession drawing)
    {
        if (snapshot == null) return;
        foreach (var item in snapshot.Decorations)
        {
            if ((item.flags & (VIEM_LAYOUT_DECORATION_ALERT_ICON | VIEM_LAYOUT_DECORATION_ALERT_TITLE)) == 0) continue;
            var bounds = OffsetRect(item.typographic_bounds, viewport);
            if (bounds.Bottom < 0 || bounds.Top > Canvas.ActualHeight || bounds.Right < 0 || bounds.Left > PaintViewportWidth) continue;
            var foreground = (item.paint.flags & VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0 ? preferences.Theme.Foreground : Color(item.paint.foreground);
            if ((item.flags & VIEM_LAYOUT_DECORATION_ALERT_ICON) != 0) { DrawAlertIcon(drawing, AlertKind(item.flags), bounds, foreground); continue; }
            // Shaped title clusters draw with the cached text at their baseline.
            if ((item.flags & VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN) != 0) continue;
            var label = Encoding.UTF8.GetString(snapshot.DecorationLabels, checked((int)item.label_byte_start), checked((int)item.label_byte_length));
            using var format = new CanvasTextFormat { FontFamily = "Segoe UI", FontSize = item.font_size, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
                HorizontalAlignment = CanvasHorizontalAlignment.Left, VerticalAlignment = CanvasVerticalAlignment.Center, WordWrapping = CanvasWordWrapping.NoWrap };
            drawing.DrawText(label, new Rect(bounds.X, bounds.Y, Math.Max(bounds.Width, item.font_size * 8), bounds.Height), foreground, format);
        }
    }

    internal static void DrawAlertIcon(CanvasDrawingSession drawing, uint kind, Rect bounds, Color color)
    {
        if (bounds.Width <= 0 || bounds.Height <= 0) return;
        float unit = (float)bounds.Width / 16;
        Vector2 P(double x, double y) => new((float)(bounds.X + x * unit), (float)(bounds.Y + y * unit));
        using var builder = new CanvasPathBuilder(drawing);
        (Vector2 Start, Vector2 End) stroke = (P(8, 7.5), P(8, 11.5));
        Vector2 dot = P(8, 4.75);
        bool exclamation = true;
        switch (kind)
        {
            case VIEM_ALERT_TIP:
                builder.BeginFigure(P(5.75, 11));
                builder.AddCubicBezier(P(5.75, 9.6), P(3.5, 8.8), P(3.5, 6.25));
                builder.AddCubicBezier(P(3.5, 3.75), P(5.5, 1.75), P(8, 1.75));
                builder.AddCubicBezier(P(10.5, 1.75), P(12.5, 3.75), P(12.5, 6.25));
                builder.AddCubicBezier(P(12.5, 8.8), P(10.25, 9.6), P(10.25, 11));
                builder.EndFigure(CanvasFigureLoop.Closed);
                Line(P(6, 13.25), P(10, 13.25)); Line(P(6.75, 15), P(9.25, 15));
                exclamation = false;
                break;
            case VIEM_ALERT_IMPORTANT:
                builder.BeginFigure(P(3.25, 1.75));
                builder.AddLine(P(12.75, 1.75)); builder.AddQuadraticBezier(P(14.25, 1.75), P(14.25, 3.25));
                builder.AddLine(P(14.25, 9.75)); builder.AddQuadraticBezier(P(14.25, 11.25), P(12.75, 11.25));
                builder.AddLine(P(7.75, 11.25)); builder.AddLine(P(4.75, 14.25)); builder.AddLine(P(4.75, 11.25));
                builder.AddLine(P(3.25, 11.25)); builder.AddQuadraticBezier(P(1.75, 11.25), P(1.75, 9.75));
                builder.AddLine(P(1.75, 3.25)); builder.AddQuadraticBezier(P(1.75, 1.75), P(3.25, 1.75));
                builder.EndFigure(CanvasFigureLoop.Closed);
                stroke = (P(8, 4.25), P(8, 6.75)); dot = P(8, 8.75);
                break;
            case VIEM_ALERT_WARNING:
                builder.BeginFigure(P(8, 1.75)); builder.AddLine(P(14.75, 14)); builder.AddLine(P(1.25, 14));
                builder.EndFigure(CanvasFigureLoop.Closed);
                stroke = (P(8, 6.25), P(8, 9.25)); dot = P(8, 11.5);
                break;
            case VIEM_ALERT_CAUTION:
                var points = new[] { (5.25, 1.75), (10.75, 1.75), (14.25, 5.25), (14.25, 10.75), (10.75, 14.25), (5.25, 14.25), (1.75, 10.75), (1.75, 5.25) };
                builder.BeginFigure(P(points[0].Item1, points[0].Item2));
                foreach (var (x, y) in points.Skip(1)) builder.AddLine(P(x, y));
                builder.EndFigure(CanvasFigureLoop.Closed);
                stroke = (P(8, 4.75), P(8, 8.25)); dot = P(8, 10.75);
                break;
            default:
            {
                using var circle = CanvasGeometry.CreateEllipse(drawing, P(8, 8), 6.5f * unit, 6.5f * unit);
                builder.AddGeometry(circle);
                break;
            }
        }
        if (exclamation) Line(stroke.Start, stroke.End);
        using var style = new CanvasStrokeStyle { StartCap = CanvasCapStyle.Round, EndCap = CanvasCapStyle.Round, LineJoin = CanvasLineJoin.Round };
        using var geometry = CanvasGeometry.CreatePath(builder);
        drawing.DrawGeometry(geometry, color, 1.5f * unit, style);
        if (exclamation) drawing.FillCircle(dot, unit, color);

        void Line(Vector2 start, Vector2 end) { builder.BeginFigure(start); builder.AddLine(end); builder.EndFigure(CanvasFigureLoop.Open); }
    }
}
