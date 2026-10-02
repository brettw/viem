using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;
using Windows.System;

namespace Viem.Windows.Shell;

internal sealed class TableInsertButton : Button
{
    internal Action<PointerRoutedEventArgs>? Pressed, Moved, Released, CaptureLost, Canceled;
    protected override void OnPointerPressed(PointerRoutedEventArgs e) {
        if (IsEnabled && e.GetCurrentPoint(this).Properties.IsLeftButtonPressed) { Pressed?.Invoke(e); e.Handled = true; }
        else base.OnPointerPressed(e);
    }
    protected override void OnPointerMoved(PointerRoutedEventArgs e) { Moved?.Invoke(e); if (!e.Handled) base.OnPointerMoved(e); }
    protected override void OnPointerReleased(PointerRoutedEventArgs e) { Released?.Invoke(e); if (!e.Handled) base.OnPointerReleased(e); }
    protected override void OnPointerCaptureLost(PointerRoutedEventArgs e) { CaptureLost?.Invoke(e); base.OnPointerCaptureLost(e); }
    protected override void OnPointerCanceled(PointerRoutedEventArgs e) { Canceled?.Invoke(e); base.OnPointerCanceled(e); }
}

internal sealed class TablePickerState
{
    internal int Columns = 10, Rows = 10, SelectedColumns, SelectedRows;
    internal bool HasSelection => SelectedColumns > 0 && SelectedRows > 0;
    internal void Point(int column, int row, bool growing)
    {
        if (column < 0 || row < 0) { SelectedColumns = SelectedRows = 0; return; }
        int c = Math.Min(20, column + 1), r = Math.Min(50, row + 1);
        if (growing) { Columns = Math.Max(Columns, c); Rows = Math.Max(Rows, r); }
        if (c > Columns || r > Rows) { SelectedColumns = SelectedRows = 0; return; }
        SelectedColumns = c; SelectedRows = r;
    }
    internal void Move(int dx, int dy)
    {
        SelectedColumns = Math.Clamp(Math.Max(1, SelectedColumns) + dx, 1, 20);
        SelectedRows = Math.Clamp(Math.Max(1, SelectedRows) + dy, 1, 50);
        Columns = Math.Max(Columns, SelectedColumns); Rows = Math.Max(Rows, SelectedRows);
    }
}

internal sealed class TablePickerGrid : Canvas
{
    protected override AutomationPeer OnCreateAutomationPeer() => new GridPeer(this);
    private sealed class GridPeer(TablePickerGrid owner) : FrameworkElementAutomationPeer(owner)
    {
        protected override string GetClassNameCore() => "TableDimensionsPicker";
        protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.Group;
        protected override bool IsKeyboardFocusableCore() => true;
    }
}

/// <summary>Transient native popup state; document edits stay in the core.</summary>
internal sealed class TablePicker
{
    private const double Cell = 21;
    private readonly TableInsertButton button;
    private readonly Popup popup = new() { IsLightDismissEnabled = true };
    private readonly TextBlock title = new() { Text = "Insert Table", FontSize = 14, FontWeight = global::Windows.UI.Text.FontWeights.SemiBold };
    private readonly TextBlock detail = new() { FontSize = 10, TextWrapping = TextWrapping.Wrap, Margin = new(0, 4, 0, 8) };
    private readonly ScrollViewer scroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollMode = ScrollMode.Enabled, VerticalScrollMode = ScrollMode.Enabled };
    private readonly TablePickerGrid grid = new() { IsTabStop = true, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
    private readonly StackPanel content = new() { Padding = new(12) };
    private readonly List<Rectangle> cells = [];
    private readonly Action restoreFocus;
    private Action<uint, uint>? accept;
    private Point press, lastPoint;
    private bool toolbarPress, dragging, releasingCapture;
    private UIElement? captured;
    private Microsoft.UI.Xaml.Input.Pointer? pointer;
    private double availableWidth, availableHeight;
    private readonly Microsoft.UI.Dispatching.DispatcherQueueTimer timer;
    internal TablePickerState State { get; private set; } = new();
    internal bool IsOpen => popup.IsOpen;
    internal event Action? Closed;

    internal TablePicker(TableInsertButton button, Action restoreFocus)
    {
        this.button = button; this.restoreFocus = restoreFocus;
        content.Children.Add(title); content.Children.Add(detail); content.Children.Add(scroll); scroll.Content = grid;
        popup.Child = new Border { Child = content, Background = (Brush)Application.Current.Resources["SolidBackgroundFillColorBaseBrush"], BorderBrush = (Brush)Application.Current.Resources["SurfaceStrokeColorDefaultBrush"], BorderThickness = new(1), CornerRadius = new(8) };
        popup.Closed += (_, _) => { timer.Stop(); ReleaseCapture(); accept = null; State = new(); restoreFocus(); Closed?.Invoke(); };
        button.Pressed = e => Start?.Invoke(e);
        button.Moved = e => Track(button, e); button.Released = e => Release(button, e);
        button.CaptureLost = e => CaptureLost(button, e);
        button.Canceled = _ => Close();
        button.Click += (_, _) => Start?.Invoke(null);
        grid.PointerPressed += (_, e) => {
            if (!e.GetCurrentPoint(grid).Properties.IsLeftButtonPressed) return;
            toolbarPress = false; dragging = true;
            if (!Capture(grid, e)) { Close(); return; }
            TrackPoint(e, true); e.Handled = true;
        };
        grid.PointerMoved += Track; grid.PointerReleased += Release; grid.PointerCaptureLost += CaptureLost;
        grid.PointerCanceled += (_, _) => Close();
        grid.KeyDown += (_, e) => {
            switch (e.Key) {
                case VirtualKey.Escape: Close(); break;
                case VirtualKey.Enter: Commit(); break;
                case VirtualKey.Left: State.Move(-1, 0); Update(true); break;
                case VirtualKey.Right: State.Move(1, 0); Update(true); break;
                case VirtualKey.Up: State.Move(0, -1); Update(true); break;
                case VirtualKey.Down: State.Move(0, 1); Update(true); break;
                default: return;
            }
            e.Handled = true;
        };
        AutomationProperties.SetName(grid, "Table dimensions");
        AutomationProperties.SetHelpText(grid, "Use arrow keys to choose columns and body rows. Press Enter to insert or Escape to cancel.");
        AutomationProperties.SetLiveSetting(grid, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        timer = button.DispatcherQueue.CreateTimer(); timer.Interval = TimeSpan.FromMilliseconds(50);
        timer.Tick += (_, _) => Autoscroll();
    }
    internal Action<PointerRoutedEventArgs?>? Start;
    internal void Open(PointerRoutedEventArgs? e, Action<uint, uint> accept)
    {
        if (popup.IsOpen) { Close(); return; }
        this.accept = accept; State = new(); cells.Clear(); grid.Children.Clear();
        if (e == null) State.Move(0, 0);
        popup.XamlRoot = button.XamlRoot;
        var at = button.TransformToVisual(null).TransformPoint(new(0, button.ActualHeight + 4));
        var size = button.XamlRoot.Size;
        at.X = Math.Clamp(at.X, 4, Math.Max(4, size.Width - 240));
        if (at.Y + 278 > size.Height) at.Y = Math.Max(4, at.Y - button.ActualHeight - 282);
        popup.HorizontalOffset = at.X; popup.VerticalOffset = at.Y;
        availableWidth = Math.Max(100, size.Width - at.X - 4); availableHeight = Math.Max(100, size.Height - at.Y - 4);
        toolbarPress = e != null; dragging = false;
        Update(); popup.IsOpen = true; grid.Focus(FocusState.Programmatic);
        if (e != null) {
            press = e.GetCurrentPoint(button.XamlRoot.Content).Position;
            if (!Capture(button, e)) { Close(); return; }
        }
        timer.Start();
    }
    internal void Close() { if (popup.IsOpen) popup.IsOpen = false; }
    private void Commit()
    {
        var result = State; var completion = accept;
        Close(); if (result.HasSelection) completion?.Invoke((uint)result.SelectedColumns, (uint)result.SelectedRows);
    }
    private bool Capture(UIElement element, PointerRoutedEventArgs e) {
        if (!element.CapturePointer(e.Pointer)) return false;
        captured = element; pointer = e.Pointer; return true;
    }
    private void ReleaseCapture()
    {
        releasingCapture = true;
        if (pointer != null) captured?.ReleasePointerCapture(pointer);
        pointer = null; captured = null; dragging = false; releasingCapture = false;
    }
    private void CaptureLost(object sender, PointerRoutedEventArgs e) { if (!releasingCapture && popup.IsOpen) Close(); }
    private void Track(object sender, PointerRoutedEventArgs e)
    {
        if (!popup.IsOpen) return;
        if (toolbarPress && !dragging) {
            var p = e.GetCurrentPoint(button.XamlRoot.Content).Position;
            if (Math.Abs(p.X - press.X) >= 4 || Math.Abs(p.Y - press.Y) >= 4) dragging = true;
        }
        TrackPoint(e, dragging); if (dragging) e.Handled = true;
    }
    private void TrackPoint(PointerRoutedEventArgs e, bool grow)
    {
        lastPoint = e.GetCurrentPoint(scroll).Position;
        PointAt(lastPoint, grow);
    }
    private void PointAt(Point point, bool grow)
    {
        // Popup chrome, including the area above/left, never denotes a cell.
        if (point.X < 0 || point.Y < 0) State.Point(-1, -1, grow);
        else State.Point((int)Math.Floor((point.X + scroll.HorizontalOffset) / Cell), (int)Math.Floor((point.Y + scroll.VerticalOffset) / Cell), grow);
        Update();
    }
    private void Release(object sender, PointerRoutedEventArgs e)
    {
        if (!popup.IsOpen) return;
        if (dragging) { TrackPoint(e, true); ReleaseCapture(); Commit(); }
        else if (toolbarPress) { toolbarPress = false; ReleaseCapture(); }
        e.Handled = true;
    }
    private void Update(bool reveal = false)
    {
        grid.Width = State.Columns * Cell; grid.Height = State.Rows * Cell;
        double width = Math.Min(availableWidth, Math.Max(234, grid.Width + 24));
        double height = Math.Min(availableHeight, grid.Height + 68);
        content.Width = width; scroll.Width = width - 24; scroll.Height = Math.Max(20, height - 68);
        var accent = (Brush)Application.Current.Resources["AccentFillColorDefaultBrush"];
        var selectedFill = new SolidColorBrush(((SolidColorBrush)accent).Color) { Opacity = 0.18 };
        var normal = (Brush)Application.Current.Resources["ControlStrokeColorDefaultBrush"];
        int count = State.Columns * State.Rows;
        while (cells.Count < count) { var cell = new Rectangle { Width = Cell - 4, Height = Cell - 4, StrokeThickness = 1, IsHitTestVisible = false }; cells.Add(cell); grid.Children.Add(cell); }
        for (int i = 0; i < cells.Count; i++) {
            int row = i / State.Columns, col = i % State.Columns;
            var cell = cells[i]; Canvas.SetLeft(cell, col * Cell + 2); Canvas.SetTop(cell, row * Cell + 2);
            bool selected = col < State.SelectedColumns && row < State.SelectedRows;
            cell.Stroke = selected ? accent : normal;
            cell.Fill = selected ? selectedFill : null;
        }
        title.Text = State.HasSelection ? $"{State.SelectedColumns} × {State.SelectedRows} Table" : "Insert Table";
        detail.Text = State.HasSelection ? $"{State.SelectedColumns} {(State.SelectedColumns == 1 ? "column" : "columns")}, {State.SelectedRows} body {(State.SelectedRows == 1 ? "row" : "rows")} + header" : "Choose columns and body rows; a header is added";
        if (AutomationProperties.GetName(grid) != detail.Text) {
            AutomationProperties.SetName(grid, detail.Text);
            FrameworkElementAutomationPeer.FromElement(grid)?.RaiseAutomationEvent(AutomationEvents.LiveRegionChanged);
        }
        if (reveal) scroll.ChangeView(Math.Max(0, State.SelectedColumns * Cell - scroll.Width), Math.Max(0, State.SelectedRows * Cell - scroll.Height), null, true);
    }
    private void Autoscroll()
    {
        if (!dragging || !popup.IsOpen) return;
        double dx = lastPoint.X > scroll.Width - 12 ? 14 : lastPoint.X < 12 ? -14 : 0;
        double dy = lastPoint.Y > scroll.Height - 12 ? 14 : lastPoint.Y < 12 ? -14 : 0;
        if (dx == 0 && dy == 0) return;
        scroll.ChangeView(Math.Clamp(scroll.HorizontalOffset + dx, 0, Math.Max(0, grid.Width - scroll.Width)), Math.Clamp(scroll.VerticalOffset + dy, 0, Math.Max(0, grid.Height - scroll.Height)), null, true);
        PointAt(lastPoint, true);
    }
}
