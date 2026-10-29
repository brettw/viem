using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Geometry;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Interop;
using Windows.Foundation;
using Windows.UI;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    // These are the same brushes used by the WinUI CheckBox template. The
    // geometry comes entirely from the current core layout, in text units.
    private Color CheckboxBrush(string key, Color fallback)
        => Application.Current.Resources.TryGetValue(key, out var value) && value is SolidColorBrush brush
            ? brush.Color : fallback;

    private ViemLayoutDecorationV1? CheckboxAt(Point point)
    {
        if (snapshot == null || View == null) return null;
        foreach (var item in snapshot.Decorations)
        {
            if ((item.flags & VIEM_LAYOUT_DECORATION_CHECKBOX) == 0
                || !OffsetRect(item.typographic_bounds, viewport).Contains(point)) continue;
            return item;
        }
        return null;
    }

    internal bool ToggleCheckboxAt(Point point)
    {
        if (snapshot == null || View == null || CheckboxAt(point) is not { } item) return false;
        if (!View.Composing)
        {
            var expected = snapshot.Info.identity;
            Run(() => View?.ToggleCheckbox(expected, item.paragraph_id));
            FocusEditor();
        }
        return true;
    }

    private void DrawCheckboxes(CanvasDrawingSession drawing)
    {
        if (snapshot == null) return;
        foreach (var item in snapshot.Decorations)
        {
            if ((item.flags & VIEM_LAYOUT_DECORATION_CHECKBOX) == 0) continue;
            var bounds = OffsetRect(item.typographic_bounds, viewport);
            if (bounds.Bottom < 0 || bounds.Top > Canvas.ActualHeight
                || bounds.Right < 0 || bounds.Left > PaintViewportWidth) continue;
            DrawCheckbox(drawing, bounds, (item.flags & VIEM_LAYOUT_DECORATION_CHECKED) != 0);
        }
    }

    private void DrawCheckbox(CanvasDrawingSession drawing, Rect bounds, bool isChecked)
    {
        // WinUI's 20-unit checkbox has a 2-unit corner radius and 1-unit
        // border. Scale both with its cap-height square; inset the stroke so
        // the complete visible control stays between cap height and baseline.
        float size = (float)bounds.Width;
        if (size <= 0) return;
        float stroke = size / 20, radius = size / 10;
        var rect = new Rect(bounds.X + stroke / 2, bounds.Y + stroke / 2,
            Math.Max(0, bounds.Width - stroke), Math.Max(0, bounds.Height - stroke));
        var fill = CheckboxBrush(isChecked ? "AccentFillColorDefaultBrush" : "ControlAltFillColorSecondaryBrush",
            isChecked ? Microsoft.UI.Colors.DodgerBlue : Microsoft.UI.Colors.Transparent);
        var border = CheckboxBrush(isChecked ? "AccentFillColorDefaultBrush" : "ControlStrongStrokeColorDefaultBrush",
            preferences.Theme.Foreground);
        drawing.FillRoundedRectangle(rect, radius, radius, fill);
        drawing.DrawRoundedRectangle(rect, radius, radius, border, stroke);
        if (!isChecked) return;
        var check = CheckboxBrush("TextOnAccentFillColorPrimaryBrush", Microsoft.UI.Colors.White);
        using var builder = new CanvasPathBuilder(drawing);
        builder.BeginFigure((float)bounds.X + size * .25f, (float)bounds.Y + size * .51f);
        builder.AddLine((float)bounds.X + size * .43f, (float)bounds.Y + size * .69f);
        builder.AddLine((float)bounds.X + size * .76f, (float)bounds.Y + size * .34f);
        builder.EndFigure(CanvasFigureLoop.Open);
        using var geometry = CanvasGeometry.CreatePath(builder);
        using var style = new CanvasStrokeStyle { StartCap = CanvasCapStyle.Round, EndCap = CanvasCapStyle.Round, LineJoin = CanvasLineJoin.Round };
        drawing.DrawGeometry(geometry, check, size * .075f, style);
    }
}
