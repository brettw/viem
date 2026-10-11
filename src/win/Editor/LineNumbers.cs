using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Viem.Windows.Core;
using Windows.Foundation;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    private readonly CanvasControl lineNumberCanvas = new() { Width = 0, Visibility = Visibility.Collapsed };
    private LineNumberSnapshot? lineNumbers;
    private ChromeStyle? lineNumberStyle, currentLineNumberStyle;
    private ChromeStyle? measuredNumberStyle, measuredCurrentNumberStyle;
    private ulong numberMetrics, currentNumberMetrics;
    private sealed record NumberGlyphLayout(CanvasTextLayout Layout, float FontSize);
    private readonly Dictionary<(ulong Line, bool Current, float Height), NumberGlyphLayout> numberLayouts = [];
    private float numeralWidth, currentNumeralWidth;
    private int numberDigits;
    internal double GutterWidth => lineNumberCanvas.Width;
#if DEBUG
    internal int NumeralMeasurements { get; private set; }
    internal LineNumberSnapshot? LineNumbersForTesting => lineNumbers;
    internal readonly record struct DrawnLineNumber(ulong Line, Rect Available, Rect Text, float FontSize);
    internal List<DrawnLineNumber> DrawnLineNumbers { get; } = [];
    internal void DrawLineNumbersForTesting(CanvasDrawingSession drawing) => DrawLineNumbers(drawing);
    internal static void DrawLineNumberBoxesForTesting(CanvasDrawingSession drawing, double width, double height,
        float viewportTop, Viem.Windows.Interop.ViemLineNumberRegionV1[] regions, ChromeStyle normal, ChromeStyle current)
    {
        drawing.Clear(Microsoft.UI.Colors.Transparent);
        PaintNumberBoxes(drawing, width, height, viewportTop, regions, normal, current);
    }
#endif

    private void BuildLineNumbers()
    {
        Children.Add(lineNumberCanvas);
        AutomationProperties.SetName(lineNumberCanvas, "Physical source line numbers");
        lineNumberCanvas.Draw += (_, args) => Run(() => DrawLineNumbers(args.DrawingSession));
        lineNumberCanvas.CreateResources += (_, _) => {
            measuredNumberStyle = measuredCurrentNumberStyle = null;
            ClearNumberLayouts();
            if (View != null) Run(Refresh);
        };
        lineNumberCanvas.PointerEntered += (_, _) => { pointerOverGutter = true; pointerOverText = false; UpdatePointerCursor(); };
        lineNumberCanvas.PointerExited += (_, _) => { pointerOverGutter = false; UpdatePointerCursor(); };
        lineNumberCanvas.PointerPressed += (_, args) => {
            if (View == null || !args.GetCurrentPoint(lineNumberCanvas).Properties.IsLeftButtonPressed) return;
            DismissLinkPopup(suppress: false); DismissImagePopup(suppress: false); DismissCommandOutput(false);
            caretHoverLocation = null; SetCaretHover(null);
            if (composing) { compositionRejected = true; ClearInput(); }
            tableCellDrag = false; tableDragAnchor = null; tableDragIdentity = null;
            FocusEditor();
            dragPoint = args.GetCurrentPoint(Canvas).Position;
            wordSelectionDragging = false;
            gutterSelectionDragging = dragging = true;
            pressedPointer = args.Pointer.PointerId;
            Run(() => View.SelectPointerRow((float)dragPoint.Y));
            if (!lineNumberCanvas.CapturePointer(args.Pointer)) { pressedPointer = null; gutterSelectionDragging = dragging = false; }
            UpdatePointerCursor(); args.Handled = true; ResetBlink();
        };
        lineNumberCanvas.PointerMoved += (_, args) => {
            if (!gutterSelectionDragging || pressedPointer != args.Pointer.PointerId || View == null) return;
            dragPoint = args.GetCurrentPoint(Canvas).Position;
            Run(() => View.SelectPointerRow((float)Math.Clamp(dragPoint.Y, 0, Math.Max(0, Canvas.ActualHeight - 0.5)), true));
            UpdatePointerCursor(); args.Handled = true;
        };
        lineNumberCanvas.PointerReleased += (_, args) => {
            if (pressedPointer != args.Pointer.PointerId) return;
            pressedPointer = null; gutterSelectionDragging = dragging = false;
            lineNumberCanvas.ReleasePointerCapture(args.Pointer); UpdatePointerCursor(); args.Handled = true;
        };
        lineNumberCanvas.PointerCaptureLost += (_, args) => {
            if (pressedPointer == args.Pointer.PointerId) { pressedPointer = null; gutterSelectionDragging = dragging = false; UpdatePointerCursor(); }
        };
        lineNumberCanvas.PointerWheelChanged += (_, args) => {
            if (View == null) return;
            var point = args.GetCurrentPoint(lineNumberCanvas);
            Run(() => { RefreshScrollGeometry(); View.Scroll(View.Viewport.left, View.Viewport.top - point.Properties.MouseWheelDelta / 120f * 60); });
            args.Handled = true;
        };
    }

    private void RefreshLineNumbers()
    {
        if (View == null || snapshot == null) return;
        lineNumbers = View.LineNumbers(snapshot.Info.identity);
        if (!lineNumbers.Enabled) { SetGutterWidth(0); return; }
        var sheet = View.Styles();
        var fallback = ChromeStyle.Fallback(preferences.Theme.Foreground, preferences.Theme.Background);
        var normal = ChromeStyle.ResolveLineNumber(sheet, current: false, fallback);
        var current = ChromeStyle.ResolveLineNumber(sheet, current: true, normal);
        lineNumberStyle = normal.Scaled(viewport.scale);
        currentLineNumberStyle = current.Scaled(viewport.scale);
        ulong metrics = View.Provider.Generation;
        bool numberChanged = measuredNumberStyle?.FontIdentity != lineNumberStyle.FontIdentity || numberMetrics != metrics;
        bool currentChanged = measuredCurrentNumberStyle?.FontIdentity != currentLineNumberStyle.FontIdentity || currentNumberMetrics != metrics;
        if (numberChanged || currentChanged) ClearNumberLayouts();
        if (numberChanged) { numeralWidth = MaxNumeralWidth(lineNumberStyle); measuredNumberStyle = lineNumberStyle; numberMetrics = metrics; }
        if (currentChanged) {
            currentNumeralWidth = lineNumberStyle.FontIdentity == currentLineNumberStyle.FontIdentity ? numeralWidth : MaxNumeralWidth(currentLineNumberStyle);
            measuredCurrentNumberStyle = currentLineNumberStyle; currentNumberMetrics = metrics;
        }
        int digits = DecimalDigits(lineNumbers.Info.source_line_count);
        if (numberDigits != digits) numberDigits = digits;
        static float Width(ChromeStyle style, float maxNumeral, int digits) => Math.Max(0,
            digits * maxNumeral + style.Margin.Horizontal + style.Padding.Horizontal + style.Border.Horizontal);
        SetGutterWidth(Math.Ceiling(Math.Max(Width(lineNumberStyle, numeralWidth, numberDigits), Width(currentLineNumberStyle, currentNumeralWidth, numberDigits))));
        lineNumberCanvas.Invalidate();
    }

    internal static int DecimalDigits(ulong lineCount)
    {
        int count = 1;
        while (lineCount >= 10) { lineCount /= 10; count++; }
        return count;
    }

    private float MaxNumeralWidth(ChromeStyle style)
    {
#if DEBUG
        NumeralMeasurements++;
#endif
        float width = 0;
        for (char digit = '0'; digit <= '9'; digit++) {
            using var layout = style.Layout(Canvas.Device, digit.ToString());
            width = Math.Max(width, (float)layout.LayoutBounds.Width);
        }
        return width;
    }

    private void SetGutterWidth(double width)
    {
        if (lineNumberCanvas.Width == width) return;
        double old = lineNumberCanvas.Width;
        window.AdjustForGutter(this, width - old);
        lineNumberCanvas.Width = width;
        lineNumberCanvas.Visibility = width > 0 ? Visibility.Visible : Visibility.Collapsed;
        InvalidateMeasure();
    }

    private void DrawLineNumbers(CanvasDrawingSession drawing)
    {
#if DEBUG
        DrawnLineNumbers.Clear();
#endif
        drawing.Clear(Microsoft.UI.Colors.Transparent);
        if (lineNumbers?.Enabled != true || lineNumberStyle == null || currentLineNumberStyle == null || View == null
            || !View.LineNumbersAreCurrent(lineNumbers.Info)) return;
        PaintNumberBoxes(drawing, GutterWidth, lineNumberCanvas.ActualHeight, lineNumbers.Info.viewport.y,
            lineNumbers.Regions, lineNumberStyle, currentLineNumberStyle);
        foreach (var region in lineNumbers.Regions) {
            var style = IsCurrentNumber(region) ? currentLineNumberStyle : lineNumberStyle;
            // Typography uses the exact full physical band, independently of the
            // paint-only pixel partition used for backgrounds and borders.
            var bounds = NumberBox(region, style, GutterWidth, lineNumbers.Info.viewport.y);
            if (bounds.Height <= 0 || bounds.Width <= 0) continue;
            double availableHeight = Math.Max(0, bounds.Height - style.Border.Vertical - style.Padding.Vertical);
            double availableWidth = Math.Max(0, bounds.Width - style.Border.Horizontal - style.Padding.Horizontal);
            if (availableHeight <= 0 || availableWidth <= 0) continue;
            var glyph = NumberLayout(region.source_line, IsCurrentNumber(region), style, (float)availableHeight);
            var layout = glyph.Layout;
            double textWidth = layout.LayoutBounds.Width;
            double left = bounds.X + style.Border.Left + style.Padding.Left;
            uint alignment = style.BaseDirection == Viem.Windows.Interop.Native.VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT
                ? style.Alignment switch { 1 => 2u, 2 => 1u, _ => style.Alignment } : style.Alignment;
            left += alignment switch { 2 => Math.Max(0, availableWidth - textWidth), 3 => Math.Max(0, (availableWidth - textWidth) / 2), _ => 0 };
            double y = bounds.Y + style.Border.Top + style.Padding.Top + (availableHeight - layout.LayoutBounds.Height) / 2;
            using var clip = drawing.CreateLayer(1, NumberPaintBounds(bounds, drawing.Dpi, GutterWidth, lineNumberCanvas.ActualHeight));
            using var textClip = drawing.CreateLayer(1, new Rect(bounds.X + style.Border.Left + style.Padding.Left,
                bounds.Y + style.Border.Top + style.Padding.Top, availableWidth, availableHeight));
            drawing.DrawTextLayout(layout, (float)(left - layout.LayoutBounds.X), (float)(y - layout.LayoutBounds.Y), style.Foreground);
#if DEBUG
            DrawnLineNumbers.Add(new(region.source_line,
                new(bounds.X + style.Border.Left + style.Padding.Left, bounds.Y + style.Border.Top + style.Padding.Top, availableWidth, availableHeight),
                new(left, y, textWidth, layout.LayoutBounds.Height), glyph.FontSize));
#endif
        }
    }

    private static void PaintNumberBoxes(CanvasDrawingSession drawing, double width, double height, float viewportTop,
        IReadOnlyList<Viem.Windows.Interop.ViemLineNumberRegionV1> regions, ChromeStyle normal, ChromeStyle current)
    {
        // Fill the ordinary column through the gaps, then each line box once.
        // Partition on one device-pixel grid before subtraction, clipping, or
        // filling. Independently antialiased adjacent edges lose coverage and
        // leave seams, particularly when scrolling at fractional DPI or zoom.
        var rawBaseBox = new Rect(normal.Margin.Left, 0, Math.Max(0, width - normal.Margin.Horizontal), height);
        var baseBox = NumberPaintBounds(rawBaseBox, drawing.Dpi, width, height);
        var gaps = new List<Rect> { baseBox };
        var boxes = new List<(ChromeStyle Style, Rect Raw, Rect Paint)>(regions.Count);
        foreach (var region in regions) {
            var style = IsCurrentNumber(region) ? current : normal;
            var raw = NumberBox(region, style, width, viewportTop);
            var hole = NumberPaintBounds(raw, drawing.Dpi, width, height);
            boxes.Add((style, raw, hole));
            for (int index = gaps.Count - 1; index >= 0; index--) {
                var area = gaps[index];
                double left = Math.Max(area.Left, hole.Left), right = Math.Min(area.Right, hole.Right);
                double top = Math.Max(area.Top, hole.Top), bottom = Math.Min(area.Bottom, hole.Bottom);
                if (right <= left || bottom <= top) continue;
                gaps.RemoveAt(index);
                if (top > area.Top) gaps.Add(new(area.Left, area.Top, area.Width, top - area.Top));
                if (bottom < area.Bottom) gaps.Add(new(area.Left, bottom, area.Width, area.Bottom - bottom));
                if (left > area.Left) gaps.Add(new(area.Left, top, left - area.Left, bottom - top));
                if (right < area.Right) gaps.Add(new(right, top, area.Right - right, bottom - top));
            }
        }
        var antialiasing = drawing.Antialiasing;
        drawing.Antialiasing = CanvasAntialiasing.Aliased;
        try {
            var gapStyle = normal with { Border = normal.Border with { Top = 0, Bottom = 0 } };
            foreach (var gap in gaps) {
                if (gap.Width <= 0 || gap.Height <= 0) continue;
                using var gapClip = drawing.CreateLayer(1, gap);
                PaintNumberBox(drawing, rawBaseBox, baseBox, gapStyle);
            }
            foreach (var box in boxes) {
                if (box.Paint.Width <= 0 || box.Paint.Height <= 0) continue;
                PaintNumberBox(drawing, box.Raw, box.Paint, box.Style);
            }
        } finally { drawing.Antialiasing = antialiasing; }
    }

    private static Rect NumberBox(Viem.Windows.Interop.ViemLineNumberRegionV1 region, ChromeStyle style, double width, float viewportTop)
    {
        double top = (double)region.top - viewportTop + style.Margin.Top;
        double bottom = (double)region.bottom - viewportTop - style.Margin.Bottom;
        return new(style.Margin.Left, top, Math.Max(0, width - style.Margin.Horizontal), Math.Max(0, bottom - top));
    }

    private static bool IsCurrentNumber(Viem.Windows.Interop.ViemLineNumberRegionV1 region)
        => (region.flags & Viem.Windows.Interop.Native.VIEM_LINE_NUMBER_REGION_CURRENT) != 0;

    private static double NumberPaintEdge(double value, float dpi)
        => Math.Round(value * dpi / 96, MidpointRounding.AwayFromZero) * 96 / dpi;

    internal static Rect NumberPaintBounds(Rect raw, float dpi, double? canvasWidth = null, double? canvasHeight = null)
    {
        double left = NumberPaintEdge(raw.Left, dpi), top = NumberPaintEdge(raw.Top, dpi);
        if (raw.Width <= 0 || raw.Height <= 0) return new(left, top, 0, 0);
        double right = NumberPaintEdge(raw.Right, dpi), bottom = NumberPaintEdge(raw.Bottom, dpi);
        // The control may end partway through its last physical pixel. Include
        // that pixel at the outer canvas edge, while authored margin edges keep
        // the ordinary nearest-pixel policy used by the interior partition.
        if (canvasWidth == raw.Right) right = Math.Ceiling(raw.Right * dpi / 96) * 96 / dpi;
        if (canvasHeight == raw.Bottom) bottom = Math.Ceiling(raw.Bottom * dpi / 96) * 96 / dpi;
        return new(left, top, Math.Max(0, right - left), Math.Max(0, bottom - top));
    }

    private static void PaintNumberBox(CanvasDrawingSession drawing, Rect raw, Rect bounds, ChromeStyle style)
    {
        if (style.Background.A > 0) drawing.FillRectangle(bounds, style.Background);
        double left = style.Border.Left > 0 ? Math.Clamp(NumberPaintEdge(raw.Left + style.Border.Left, drawing.Dpi), bounds.Left, bounds.Right) : bounds.Left;
        double right = style.Border.Right > 0 ? Math.Clamp(NumberPaintEdge(raw.Right - style.Border.Right, drawing.Dpi), left, bounds.Right) : bounds.Right;
        double top = style.Border.Top > 0 ? Math.Clamp(NumberPaintEdge(raw.Top + style.Border.Top, drawing.Dpi), bounds.Top, bounds.Bottom) : bounds.Top;
        double bottom = style.Border.Bottom > 0 ? Math.Clamp(NumberPaintEdge(raw.Bottom - style.Border.Bottom, drawing.Dpi), top, bounds.Bottom) : bounds.Bottom;
        // Vertical edges own corners. The horizontal center strips complete one
        // partition, even when a short box cannot fit all declared border widths.
        if (style.Border.Left > 0 && left > bounds.Left)
            drawing.FillRectangle(new Rect(bounds.Left, bounds.Top, left - bounds.Left, bounds.Height), style.LeftBorder);
        if (style.Border.Right > 0 && right < bounds.Right)
            drawing.FillRectangle(new Rect(right, bounds.Top, bounds.Right - right, bounds.Height), style.RightBorder);
        if (style.Border.Top > 0 && top > bounds.Top && right > left)
            drawing.FillRectangle(new Rect(left, bounds.Top, right - left, top - bounds.Top), style.TopBorder);
        if (style.Border.Bottom > 0 && bottom < bounds.Bottom && right > left)
            drawing.FillRectangle(new Rect(left, bottom, right - left, bounds.Bottom - bottom), style.BottomBorder);
    }

    private NumberGlyphLayout NumberLayout(ulong line, bool current, ChromeStyle style, float height)
    {
        var key = (line, current, height);
        if (numberLayouts.TryGetValue(key, out var retained)) return retained;
        if (numberLayouts.Count >= 512) ClearNumberLayouts();
        string text = line.ToString(System.Globalization.CultureInfo.InvariantCulture);
        var layout = style.Layout(lineNumberCanvas.Device, text);
        float fontSize = style.Size;
        for (int attempt = 0; attempt < 4 && layout.LayoutBounds.Height > height + .001; attempt++) {
            fontSize = Math.Max(.001f, fontSize * height / Math.Max(.001f, (float)layout.LayoutBounds.Height) * .999f);
            layout.Dispose(); layout = style.Layout(lineNumberCanvas.Device, text, fontSize);
        }
        var glyph = new NumberGlyphLayout(layout, fontSize);
        numberLayouts.Add(key, glyph);
        return glyph;
    }

    private void ClearNumberLayouts()
    {
        foreach (var glyph in numberLayouts.Values) glyph.Layout.Dispose();
        numberLayouts.Clear();
    }
}
