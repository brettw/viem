using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Input;
using Viem.Windows.Editor;
using Viem.Windows.Interop;
using Windows.Foundation;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

/// The window's native views follow the shared, bounded split tree.
internal sealed unsafe partial class PaneStackPanel : Panel, IDisposable
{
    private ulong handle;
    private readonly Dictionary<EditorPane, ulong> ids = [];
    private readonly Dictionary<ulong, VerticalSplitter> splitters = [];
    private ViemPaneFrame[] frames = [];
    private Preferences? preferences;
    internal Size MinimumSize { get; private set; } = new(100, 0);
    internal IReadOnlyList<EditorPane> OrderedPanes => frames.Where(f => f.kind == 0).Select(f => ids.First(p => p.Value == f.id).Key).ToArray();
    internal IReadOnlyList<VerticalSplitter> Splitters => splitters.Values.ToArray();
#if DEBUG
    internal double SplitterWidthForTesting(VerticalSplitter splitter)
    {
        ulong id = splitters.Single(entry => ReferenceEquals(entry.Value, splitter)).Key;
        return frames.Single(frame => frame.kind == 1 && frame.id == id).width;
    }
#endif
    internal PaneStackPanel() { ulong id = 0; Check(viem_pane_layout_create(&id)); handle = id; }
    internal void Configure(Preferences value) { preferences = value; value.Changed += ApplyTheme; }
    internal static Brush SeparatorBrush(Preferences value) => new SolidColorBrush(value.Theme.StatusForeground) { Opacity = .25 };
    private void ApplyTheme() { foreach (var splitter in splitters.Values) splitter.ApplyTheme(preferences!); InvalidateMeasure(); }
    private static void Check(uint status)
    {
        if (status == VIEM_STATUS_POLICY_REQUIRED) throw new InvalidOperationException("No room to split the current view.");
        if (status == VIEM_STATUS_UNSUPPORTED_OPERATION) throw new InvalidOperationException("Cannot rotate or exchange a row or column containing nested splits.");
        Abi.Check(status, "Arrange views");
    }
    internal void Rebuild(IReadOnlyList<EditorPane> panes, int? splitIndex = null, bool vertical = false, bool splitBefore = false)
    {
        foreach (var pane in ids.Keys.Where(p => !panes.Contains(p)).ToArray()) RemovePane(pane);
        foreach (var pane in panes.Where(p => !ids.ContainsKey(p)))
        {
            ulong id = 1;
            if (ids.Count > 0)
            {
                var source = splitIndex is { } i ? panes[i] : panes.Last(p => ids.ContainsKey(p));
                Check(viem_pane_layout_split(handle, ids[source], vertical ? 1u : 0u, pane.StatusBarHeight, splitBefore ? 0u : 1u, &id));
            }
            ids.Add(pane, id); Children.Add(pane);
        }
        RefreshFrames(); InvalidateMeasure();
    }
    internal void ReplacePane(EditorPane old, EditorPane next)
    {
        ulong id = ids[old]; ids.Remove(old); ids.Add(next, id);
        Children.Remove(old); Children.Add(next); InvalidateMeasure();
    }
    internal void RemovePane(EditorPane pane)
    {
        if (!ids.Remove(pane, out ulong id)) return;
        Check(viem_pane_layout_remove(handle, id)); Children.Remove(pane); RefreshFrames();
    }
    internal void RequireSplitRoom(EditorPane pane, double statusHeight, bool vertical)
    { UpdateLayout(); Check(viem_pane_layout_can_split(handle, ids[pane], vertical ? 1u : 0u, statusHeight)); }
    internal bool CanDragBar(EditorPane pane) => ids.TryGetValue(pane, out ulong id) && frames.Any(f => f.id == id && f.kind == 0 && (f.flags & VIEM_PANE_FRAME_STATUS_DRAGGABLE) != 0);
    private void Fit(Size size)
    {
        if (ids.Count == 0) return;
        foreach (var pane in ids.Keys) pane.SetStatusBottomEdge(!CanDragBar(pane));
        var chrome = ids.Select(p => new ViemPaneChrome { id = p.Value, status_height = p.Key.StatusBarHeight }).ToArray();
        fixed (ViemPaneChrome* input = chrome) Check(viem_pane_layout_update(handle, Math.Max(0, size.Width), Math.Max(0, size.Height), input, (ulong)chrome.Length));
        RefreshFrames();
    }
    private void RefreshFrames()
    {
        var next = new ViemPaneFrame[511]; ViemPaneSnapshot snapshot = default;
        fixed (ViemPaneFrame* output = next) Check(viem_pane_layout_copy(handle, output, (ulong)next.Length, &snapshot));
        frames = next.Take((int)snapshot.count).ToArray(); MinimumSize = new(snapshot.minimum_width, snapshot.minimum_height);
        var live = frames.Where(f => f.kind == 1).Select(f => f.id).ToHashSet();
        foreach (ulong id in splitters.Keys.Where(id => !live.Contains(id)).ToArray())
        { var splitter = splitters[id]; Children.Remove(splitter); splitter.Dispose(); splitters.Remove(id); }
        foreach (ulong id in live.Where(id => !splitters.ContainsKey(id)))
        {
            var splitter = new VerticalSplitter(this, delta => DragSplitter(id, delta));
            if (preferences != null) splitter.ApplyTheme(preferences);
            splitters.Add(id, splitter); Children.Add(splitter);
        }
    }
    internal EditorPane Action(EditorPane pane, uint operation, ulong count = 0, double value = 0, uint flags = 0)
    {
        UpdateLayout(); var point = pane.WindowFocusPoint; var origin = pane.TransformToVisual(this).TransformPoint(new Point()); ulong focus = 0;
        Check(viem_pane_layout_action(handle, ids[pane], operation, count, value, flags, origin.X + point.X, origin.Y + point.Y, &focus));
        RefreshFrames(); InvalidateMeasure(); UpdateLayout();
        ulong focusedId = focus;
        return ids.First(p => p.Value == focusedId).Key;
    }
    internal void Equalize(uint axis = 0) { if (ids.Count > 0) Action(ids.Keys.First(), 6, axis); }
    internal void DragBar(EditorPane pane, double delta)
    { if (!ids.TryGetValue(pane, out ulong id) || !double.IsFinite(delta)) return; Check(viem_pane_layout_drag(handle, id, 0, delta)); RefreshFrames(); InvalidateMeasure(); UpdateLayout(); }
    private void DragSplitter(ulong id, double delta)
    { Check(viem_pane_layout_drag(handle, id, 1, delta)); RefreshFrames(); InvalidateMeasure(); UpdateLayout(); }
    internal void ResizePane(EditorPane pane, double textHeight) => Action(pane, 5, 0, textHeight);
    internal void ResizeWidth(EditorPane pane, double width) => Action(pane, 5, 1, width);
    protected override Size MeasureOverride(Size availableSize)
    {
        var size = new Size(double.IsFinite(availableSize.Width) ? availableSize.Width : ActualWidth, double.IsFinite(availableSize.Height) ? availableSize.Height : ActualHeight);
        Fit(size);
        foreach (var frame in frames) Control(frame).Measure(new(frame.width, frame.height));
        return size;
    }
    private UIElement Control(ViemPaneFrame frame) => frame.kind == 1 ? splitters[frame.id] : ids.First(p => p.Value == frame.id).Key;
    protected override Size ArrangeOverride(Size finalSize)
    {
        Fit(finalSize);
        foreach (var frame in frames)
        {
            var control = Control(frame);
            if (control is VerticalSplitter splitter) splitter.UpdateBorderVisibility(frame, finalSize.Height);
            control.Arrange(new Rect(frame.x, frame.y, frame.width, frame.height));
            control.Clip = new RectangleGeometry { Rect = new Rect(0, 0, frame.width, frame.height) };
        }
        return finalSize;
    }
    public void Dispose()
    {
        if (preferences != null) preferences.Changed -= ApplyTheme;
        foreach (var splitter in splitters.Values) splitter.Dispose(); splitters.Clear();
        if (handle != 0) { Check(viem_pane_layout_destroy(handle)); handle = 0; }
    }
}

internal sealed partial class DragCursorGrid : Grid, IDisposable
{
    private readonly InputSystemCursor cursor = InputSystemCursor.Create(InputSystemCursorShape.SizeNorthSouth);
    private bool dragging, hovering;
    internal void SetDragging(bool value) { dragging = value; UpdateCursor(); }
    internal void SetResizeHover(bool value) { hovering = value; UpdateCursor(); }
    private void UpdateCursor() => ProtectedCursor = dragging || hovering ? cursor : null;
    public void Dispose() { dragging = hovering = false; ReleasePointerCaptures(); ProtectedCursor = null; cursor.Dispose(); }
}

internal sealed partial class VerticalSplitter : Grid, IDisposable
{
    private readonly InputSystemCursor cursor = InputSystemCursor.Create(InputSystemCursorShape.SizeWestEast);
    private uint? pointer;
    private Point press, last;
    private bool dragging;
    private readonly Microsoft.Graphics.Canvas.UI.Xaml.CanvasControl decoration = new() { IsHitTestVisible = false };
    private Viem.Windows.Core.ChromeStyle? style;
    internal bool HidesTopBorder { get; private set; } = true;
    internal bool HidesBottomBorder { get; private set; } = true;
    internal VerticalSplitter(UIElement root, Action<double> move)
    {
        ProtectedCursor = cursor; BorderThickness = new(0);
        Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        Children.Add(decoration);
        decoration.Draw += (_, args) => {
            args.DrawingSession.Clear(Microsoft.UI.Colors.Transparent);
            if (style != null) DrawBorders(args.DrawingSession, style, new(decoration.ActualWidth, decoration.ActualHeight));
        };
        AutomationProperties.SetName(this, "Resize views horizontally");
        PointerPressed += (_, e) => {
            var p = e.GetCurrentPoint(root);
            if (!p.Properties.IsLeftButtonPressed || pointer != null) return;
            // A divider is narrower than a normal drag. Capture the press so
            // its first movement outside the strip still reaches this owner;
            // the threshold below continues to distinguish clicks from drags.
            bool captured = CapturePointer(e.Pointer);
            if (!captured) return;
            pointer = e.Pointer.PointerId; press = last = p.Position;
        };
        PointerMoved += (_, e) => {
            if (pointer != e.Pointer.PointerId) return;
            var p = e.GetCurrentPoint(root);
            if (!p.Properties.IsLeftButtonPressed) return;
            if (!dragging) { if (Math.Abs(p.Position.X - press.X) < 4 && Math.Abs(p.Position.Y - press.Y) < 4) return; dragging = true; }
            double delta = p.Position.X - last.X; last = p.Position; move(delta); e.Handled = true;
        };
        PointerReleased += (_, e) => {
            if (pointer != e.Pointer.PointerId) return;
            bool dragged = dragging; pointer = null; dragging = false;
            ReleasePointerCapture(e.Pointer); if (dragged) e.Handled = true;
        };
        PointerCaptureLost += (_, _) => { pointer = null; dragging = false; };
    }
    internal void UpdateBorderVisibility(ViemPaneFrame frame, double containerHeight)
    {
        bool top = frame.y <= .000001 || (frame.flags & VIEM_PANE_FRAME_SUPPRESS_SPLITTER_TOP_BORDER) != 0;
        bool bottom = frame.y + frame.height >= containerHeight - .000001;
        if (top == HidesTopBorder && bottom == HidesBottomBorder) return;
        HidesTopBorder = top; HidesBottomBorder = bottom; decoration.Invalidate();
    }
    internal void DrawBorders(Microsoft.Graphics.Canvas.CanvasDrawingSession drawing, Viem.Windows.Core.ChromeStyle paint, Size size)
    {
        if (paint.Background.A > 0) drawing.FillRectangle(new Rect(0, 0, size.Width, size.Height), paint.Background);
        // Vertical strips own corners; every border pixel is painted once.
        double left = Math.Min(paint.Border.Left, size.Width), right = Math.Min(paint.Border.Right, Math.Max(0, size.Width - left));
        if (left > 0) drawing.FillRectangle(new Rect(0, 0, left, size.Height), paint.LeftBorder);
        if (right > 0) drawing.FillRectangle(new Rect(size.Width - right, 0, right, size.Height), paint.RightBorder);
        double top = HidesTopBorder ? 0 : Math.Min(paint.Border.Top, size.Height);
        double bottom = HidesBottomBorder ? 0 : Math.Min(paint.Border.Bottom, Math.Max(0, size.Height - top));
        double middle = Math.Max(0, size.Width - left - right);
        if (top > 0 && middle > 0) drawing.FillRectangle(new Rect(left, 0, middle, top), paint.TopBorder);
        if (bottom > 0 && middle > 0) drawing.FillRectangle(new Rect(left, size.Height - bottom, middle, bottom), paint.BottomBorder);
    }
    internal void ApplyTheme(Preferences preferences) { style = preferences.StatusStyles.Status; decoration.Invalidate(); }
    public void Dispose() { pointer = null; dragging = false; ReleasePointerCaptures(); ProtectedCursor = null; cursor.Dispose(); decoration.RemoveFromVisualTree(); }
}
