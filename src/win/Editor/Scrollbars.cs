using System.Numerics;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Windows.Foundation;
using Windows.UI.ViewManagement;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    // The interactive viewport still ends at the vertical scrollbar's inside
    // edge. Replay already shaped overflow behind its translucent track without
    // changing wrapping, hit testing, IME geometry or the core viewport width.
    private readonly CanvasControl scrollbarPaint = new() { IsHitTestVisible = false };
    internal double PaintViewportWidth => Canvas.ActualWidth
        + (viewport.maximum_left > 0 ? scrollbarPaint.ActualWidth : 0);
    // A standalone WinUI ScrollBar defaults to IndicatorMode.None, whose
    // template collapses the whole bar; ScrollViewer normally drives that
    // state. Show the mouse indicator whenever the content can scroll, and
    // keep it expanded unless Windows hides scroll bars automatically.
    private readonly UISettings scrollbarSettings = new();
    private bool autoHideScrollBars = true;
#if DEBUG
    internal bool? AutoHideScrollBarsForTesting { get; set; }
#endif

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
        // Windows 10 1809 has no preference; its scroll bars hide automatically.
        if (OperatingSystem.IsWindowsVersionAtLeast(10, 0, 18362))
        {
            autoHideScrollBars = scrollbarSettings.AutoHideScrollBars;
            scrollbarSettings.AutoHideScrollBarsChanged += ScrollBarPreferenceChanged;
        }
        foreach (var bar in new[] { vertical, horizontal })
        {
            bar.IndicatorMode = ScrollingIndicatorMode.None;
            bar.Loaded += (_, _) => UpdateScrollbarIndicators();
            // The template collapses an expanded bar when the pointer leaves.
            bar.PointerExited += (_, _) => DispatcherQueue.TryEnqueue(UpdateScrollbarIndicators);
        }
    }

    [System.Runtime.Versioning.SupportedOSPlatform("windows10.0.18362")]
    private void ScrollBarPreferenceChanged(UISettings sender, UISettingsAutoHideScrollBarsChangedEventArgs args)
    {
        // Raised on a background thread; read it there, apply it on the UI thread.
        bool value = sender.AutoHideScrollBars;
        DispatcherQueue.TryEnqueue(() => { autoHideScrollBars = value; UpdateScrollbarIndicators(); });
    }

    private void UpdateScrollbarIndicators()
    {
        if (disposed) return;
        bool expanded =
#if DEBUG
            !(AutoHideScrollBarsForTesting ?? autoHideScrollBars);
#else
            !autoHideScrollBars;
#endif
        foreach (var (bar, scrollable) in new[] { (vertical, viewport.maximum_top > 0), (horizontal, viewport.maximum_left > 0) })
        {
            var mode = scrollable ? ScrollingIndicatorMode.MouseIndicator : ScrollingIndicatorMode.None;
            if (bar.IndicatorMode != mode) bar.IndicatorMode = mode;
            if (bar.IsEnabled != scrollable) bar.IsEnabled = scrollable;
            if (scrollable && expanded) VisualStateManager.GoToState(bar, "ExpandedWithoutAnimation", false);
        }
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
    internal void UpdateScrollbarIndicatorsForTesting() => UpdateScrollbarIndicators();
#endif
}
