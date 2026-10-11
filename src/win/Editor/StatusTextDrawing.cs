using System.Globalization;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Windows.Foundation;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    // Keep native labels for accessibility, sizing and hit targets. Their text
    // paints through the same shaper as the command line and document gutter.
    private readonly CanvasControl statusTextCanvas = new() { IsHitTestVisible = false };
    private sealed record StatusTextShape(string Text, ChromeStyle Style, CanvasTextLayout Layout);
    private readonly Dictionary<TextBlock, StatusTextShape> statusTextShapes = [];
    private readonly Dictionary<string, double> statusTextWidths = [];

    private void BuildStatusText()
    {
        status.Children.Add(statusTextCanvas); SetColumnSpan(statusTextCanvas, 2);
        statusTextCanvas.Draw += (_, args) => Run(() => DrawStatusText(args.DrawingSession));
        statusTextCanvas.CreateResources += (_, _) => { ClearStatusText(); statusTextCanvas.Invalidate(); };
        foreach (var label in new[] { mode, message, filePath, location })
            label.SizeChanged += (_, _) => statusTextCanvas.Invalidate();
        normalStatus.SizeChanged += (_, _) => statusTextCanvas.Invalidate();
    }

    private void DrawStatusText(CanvasDrawingSession drawing)
    {
        drawing.Clear(Microsoft.UI.Colors.Transparent);
        var style = statusStyle ?? preferences.StatusStyles.Status;
        double top = style.Border.Top + style.Padding.Top;
        double height = Math.Max(0, status.ActualHeight - top - StatusBottomBorder - style.Padding.Bottom);
        if (height <= 0) return;
#if DEBUG
        DrawnStatusTexts.Clear();
#endif
        foreach (var label in new[] { mode, message, filePath, location }) {
            bool visible = true;
            for (DependencyObject? control = label; control != null && control != status; control = VisualTreeHelper.GetParent(control))
                if (control is UIElement { Visibility: Visibility.Collapsed }) { visible = false; break; }
            if (!visible || label.ActualWidth <= 0 || label.Text.Length == 0) continue;
            var origin = label.TransformToVisual(status).TransformPoint(new Point());
            string text = label == message ? TrimStatusRight(label.Text, label.ActualWidth, MeasureStatusText) : label.Text;
            if (text.Length == 0) continue;
            if (!statusTextShapes.TryGetValue(label, out var shape) || shape.Text != text || shape.Style.FontIdentity != style.FontIdentity) {
                shape?.Layout.Dispose();
                shape = new(text, style, style.Layout(statusTextCanvas.Device, text));
                statusTextShapes[label] = shape;
            }
            var bounds = shape.Layout.LayoutBounds;
            double y = top + (height - bounds.Height) / 2;
            using var clip = drawing.CreateLayer(1, new Rect(origin.X, top, label.ActualWidth, height));
            drawing.DrawTextLayout(shape.Layout, (float)(origin.X - bounds.X), (float)(y - bounds.Y), style.Foreground);
#if DEBUG
            DrawnStatusTexts.Add(new(label.Text, text, style, new(origin.X, y, bounds.Width, bounds.Height)));
#endif
        }
    }

    private double MeasureStatusText(string text)
    {
        if (statusTextWidths.TryGetValue(text, out double width)) return width;
        if (statusTextWidths.Count >= 256) statusTextWidths.Clear();
        using var layout = (statusStyle ?? preferences.StatusStyles.Status).Layout(CanvasDevice.GetSharedDevice(), text);
        width = layout.LayoutBounds.Width; statusTextWidths[text] = width;
        return width;
    }

    private void ClearStatusText()
    {
        foreach (var shape in statusTextShapes.Values) shape.Layout.Dispose();
        statusTextShapes.Clear(); statusTextWidths.Clear();
    }

    internal static string TrimStatusRight(string text, double width, Func<string, double> measure)
    {
        if (width <= 0) return "";
        if (measure(text) <= width) return text;
        const string ellipsis = "…";
        if (measure(ellipsis) > width) return "";
        int[] boundaries = StringInfo.ParseCombiningCharacters(text);
        int low = 0, high = boundaries.Length;
        while (low < high) {
            int middle = low + (high - low + 1) / 2;
            int end = middle == boundaries.Length ? text.Length : boundaries[middle];
            if (measure(text[..end] + ellipsis) <= width) low = middle; else high = middle - 1;
        }
        int prefixEnd = low == boundaries.Length ? text.Length : boundaries[low];
        return low == 0 ? ellipsis : text[..prefixEnd] + ellipsis;
    }

#if DEBUG
    internal readonly record struct DrawnStatusText(string Original, string Text, ChromeStyle Style, Rect Bounds);
    internal List<DrawnStatusText> DrawnStatusTexts { get; } = [];
    internal void DrawStatusTextForTesting(CanvasDrawingSession drawing) => DrawStatusText(drawing);
    internal CanvasTextLayout? StatusTextLayoutForTesting(TextBlock label) => statusTextShapes.GetValueOrDefault(label)?.Layout;
#endif
}
