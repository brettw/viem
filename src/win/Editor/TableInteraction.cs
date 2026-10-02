using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Windows.Foundation;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    private Border? tableWidget;
    private ViemLayoutSnapshotIdentityV1? tableWidgetIdentity;
    private ViemLogicalSelectionIdentityV1? tableWidgetSelection;
    private bool tableContextFromPointer;
    private ViemTableContextV1? tableContextTarget;
    private Rect tableActivation;
    private Point tableWidgetViewport;
    private bool tablePopupTracking;
    private ViemTableCellV1? tableDragAnchor;
    private ViemLayoutSnapshotIdentityV1? tableDragIdentity;
    private bool tableCellDrag;
    private ViemTableSelectionV1 tableSelection;

    private void RefreshTableInteraction()
    {
        if (View == null || snapshot == null) return;
        tableSelection = View.TableSelection();
        if (tableWidgetIdentity is { } identity && (!CoreView.SameLayout(identity, snapshot.Info.identity) || tableWidgetViewport.X != viewport.left || tableWidgetViewport.Y != viewport.top)) HideTableWidget();
        if (tableWidgetSelection is { } expected && !CoreView.SameSelection(expected, View.LogicalSelection())) HideTableWidget();
    }
    private void HideTableWidget()
    {
        if (tablePopupTracking) return;
        if (tableWidget != null) tableWidget.Visibility = Visibility.Collapsed;
        tableWidgetIdentity = null;
        tableWidgetSelection = null;
    }
    private void TableHover(Point point)
    {
        if (View == null || snapshot == null || Document.State.format != VIEM_FORMAT_MARKDOWN) { HideTableWidget(); return; }
        if (tableWidget?.Visibility == Visibility.Visible) {
            var w = new Rect(tableWidget.Margin.Left, tableWidget.Margin.Top, tableWidget.Width, tableWidget.Height);
            var path = new Rect(Math.Min(w.Left, tableActivation.Left) - 3, Math.Min(w.Top, tableActivation.Top) - 3,
                Math.Max(w.Right, tableActivation.Right) - Math.Min(w.Left, tableActivation.Left) + 6,
                Math.Max(w.Bottom, tableActivation.Bottom) - Math.Min(w.Top, tableActivation.Top) + 6);
            if (path.Contains(point) || tablePopupTracking) return;
        }
        foreach (var cell in snapshot.TableCells) {
            var rect = OffsetRect(cell.rect, viewport); var table = OffsetRect(cell.table_rect, viewport);
            bool column = point.Y >= table.Top - 3 && point.Y <= table.Top + 1 && point.X >= rect.Left && (point.X < rect.Right || point.X == table.Right);
            bool row = point.X >= table.Left - 20 && point.X <= table.Left + 1 && point.Y >= rect.Top && (point.Y < rect.Bottom || point.Y == table.Bottom);
            if (!column && !row) continue;
            var context = View.TableContextAt(cell.text_start, snapshot.Info.identity.document_id, snapshot.Info.identity.document_revision);
            if ((context.flags & 2) != 0) ShowTableWidget(context, column, rect, table);
            return;
        }
        HideTableWidget();
    }
    private void ShowTableWidget(ViemTableContextV1 context, bool column, Rect cell, Rect table)
    {
        if (tableWidget == null) {
            tableWidget = new Border { CornerRadius = new(5), Padding = new(3), HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Top };
            Children.Add(tableWidget);
            tableWidget.PointerExited += (_, e) => TableHover(e.GetCurrentPoint(Canvas).Position);
        }
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 1 };
        tableWidget.Child = row; tableWidget.Background = (Brush)Application.Current.Resources["SolidBackgroundFillColorBaseBrush"];
        var actions = new List<(string Name, string Icon, uint Action)>();
        if (column) {
            actions.Add(($"Insert column left of column {context.column + 1}", "+←", 4));
            actions.Add(($"Column {context.column + 1} alignment", context.alignment == 2 ? "\uE8E3" : context.alignment == 3 ? "\uE8E2" : "\uE8E4", 7));
            actions.Add(($"Delete column {context.column + 1}", "−", 6));
            actions.Add(($"Insert column right of column {context.column + 1}", "+→", 5));
        } else {
            if (context.row > 0) actions.Add(($"Insert row above row {context.row + 1}", "+↑", 1));
            actions.Add((context.row == 0 ? "Delete header row" : $"Delete row {context.row + 1}", "−", 3));
            actions.Add(($"Insert row below row {context.row + 1}", "+↓", 2));
        }
        foreach (var action in actions) {
            var button = new Button { Width = 27, Height = 24, MinWidth = 0, MinHeight = 0, Padding = new(0), AllowFocusOnInteraction = false };
            button.Content = action.Action == 7 ? new FontIcon { Glyph = action.Icon, FontSize = 13 } : new TextBlock { Text = action.Icon, FontSize = 12 };
            AutomationProperties.SetName(button, action.Name); ToolTipService.SetToolTip(button, action.Name);
            if (action.Action == 7) {
                var menu = TableAlignmentMenu(context);
                menu.Opening += (_, _) => tablePopupTracking = true;
                menu.Closed += (_, _) => { tablePopupTracking = false; RefreshTableInteraction(); };
                button.Flyout = menu;
            } else button.Click += (_, _) => PerformTableAction(context, action.Action);
            row.Children.Add(button);
        }
        double width = actions.Count * 28 + 6, height = 30;
        double x = column ? cell.X + cell.Width / 2 - width / 2 : table.X - width - 4;
        double y = column ? table.Y - height - 4 : cell.Y + cell.Height / 2 - height / 2;
        if (!column && x < 0 && table.Right + width + 4 < Canvas.ActualWidth) x = table.Right + 4;
        if (y < 0) y = cell.Bottom + 4;
        x = Math.Clamp(x, 0, Math.Max(0, Canvas.ActualWidth - width)); y = Math.Clamp(y, 0, Math.Max(0, Canvas.ActualHeight - height));
        tableWidget.Width = width; tableWidget.Height = height; tableWidget.Margin = new(x, y, 0, 0); tableWidget.Visibility = Visibility.Visible;
        tableActivation = column ? new(cell.X, table.Y - 3, cell.Width, 4) : new(table.X - 20, cell.Y, 21, cell.Height);
        tableWidgetIdentity = snapshot!.Info.identity; tableWidgetViewport = new(viewport.left, viewport.top);
        tableWidgetSelection = context.selection;
    }
    private void PerformTableAction(ViemTableContextV1 context, uint action, uint alignment = 0)
    {
        Run(() => View?.TableAction(action, context, alignment)); HideTableWidget(); FocusEditor();
    }
    private MenuFlyout TableAlignmentMenu(ViemTableContextV1 context)
    {
        var menu = new MenuFlyout();
        foreach (var (name, alignment, glyph) in new[] { ("Left", 1u, "\uE8E4"), ("Center", 2u, "\uE8E3"), ("Right", 3u, "\uE8E2") }) {
            var item = new ToggleMenuFlyoutItem { Text = name, IsChecked = Math.Max(1, context.alignment) == alignment, Icon = new FontIcon { Glyph = glyph } };
            item.Click += (_, _) => PerformTableAction(context, 7, alignment); menu.Items.Add(item);
        }
        return menu;
    }
    private void ConfigureTableContext(MenuFlyout menu)
    {
        int ordinaryItems = menu.Items.Count;
        menu.Opening += (_, _) => {
            while (menu.Items.Count > ordinaryItems) menu.Items.RemoveAt(menu.Items.Count - 1);
            bool pointer = tableContextFromPointer; tableContextFromPointer = false;
            var target = tableContextTarget; tableContextTarget = null;
            if (View == null || Document.State.format != VIEM_FORMAT_MARKDOWN) return;
            var context = pointer ? target.GetValueOrDefault() : View.TableContext(); if ((context.flags & 2) == 0) return;
            menu.Items.Add(new MenuFlyoutSeparator());
            void Add(string title, uint action) { var item = new MenuFlyoutItem { Text = title }; item.Click += (_, _) => PerformTableAction(context, action); menu.Items.Add(item); }
            if (context.row > 0) Add($"Insert row above row {context.row + 1}", 1);
            Add($"Insert row below row {context.row + 1}", 2); Add(context.row == 0 ? "Delete header row" : $"Delete row {context.row + 1}", 3);
            Add($"Insert column left of column {context.column + 1}", 4); Add($"Insert column right of column {context.column + 1}", 5); Add($"Delete column {context.column + 1}", 6);
            var alignment = new MenuFlyoutSubItem { Text = $"Column {context.column + 1} alignment" };
            var choices = TableAlignmentMenu(context); while (choices.Items.Count > 0) { var item = choices.Items[0]; choices.Items.RemoveAt(0); alignment.Items.Add(item); }
            menu.Items.Add(alignment);
        };
    }
    private void TargetTableContext(Point point)
    {
        tableContextFromPointer = true; tableContextTarget = null;
        if (View == null || snapshot == null || Document.State.format != VIEM_FORMAT_MARKDOWN) return;
        foreach (var cell in snapshot.TableCells) {
            if (!OffsetRect(cell.rect, viewport).Contains(point)) continue;
            tableContextTarget = View.TableContextAt(cell.text_start, snapshot.Info.identity.document_id, snapshot.Info.identity.document_revision);
            break;
        }
    }
    private void BeginTableDrag(Point point)
    {
        tableCellDrag = false; tableDragAnchor = null; tableDragIdentity = null;
        if (snapshot == null || Document.State.format != VIEM_FORMAT_MARKDOWN) return;
        foreach (var cell in snapshot.TableCells) if (OffsetRect(cell.rect, viewport).Contains(point)) { tableDragAnchor = cell; tableDragIdentity = snapshot.Info.identity; break; }
    }
    private bool DragTableCells(Point point)
    {
        if (View == null || snapshot == null || tableDragAnchor is not { } anchor || tableDragIdentity is not { } identity || snapshot.Info.identity.document_revision != identity.document_revision || snapshot.Info.identity.document_id != identity.document_id) return false;
        var cells = snapshot.TableCells.Where(c => c.table_id == anchor.table_id).ToArray(); if (cells.Length == 0) return false;
        double Distance(ViemTableCellV1 c) { var r = OffsetRect(c.rect, viewport); double x = Math.Max(Math.Max(r.Left - point.X, 0), point.X - r.Right), y = Math.Max(Math.Max(r.Top - point.Y, 0), point.Y - r.Bottom); return x * x + y * y; }
        var destination = cells.OrderBy(Distance).ThenByDescending(c => c.row).ThenByDescending(c => c.column).First();
        if (destination.cell_id != anchor.cell_id || anchor.text_start == anchor.text_end) tableCellDrag = true;
        if (!tableCellDrag) {
            var end = snapshot.Carets.LastOrDefault(c => c.text_offset == anchor.text_end);
            var line = snapshot.Rows.FirstOrDefault(r => r.row_index == end.row_index);
            var previous = snapshot.Carets.LastOrDefault(c => c.row_index == end.row_index && c.text_offset < anchor.text_end);
            double x = end.x - viewport.left, y = line.y - viewport.top;
            if (point.Y >= y && point.Y < y + line.line_advance && (previous.x > end.x ? point.X < x - 4 : point.X > x + 4) && OffsetRect(anchor.rect, viewport).Contains(point)) tableCellDrag = true;
        }
        if (!tableCellDrag) return false;
        var selection = Interop.Abi.New<ViemTableSelectionV1>(); selection.active = 1;
        selection.document_id = identity.document_id; selection.document_revision = identity.document_revision; selection.table_id = anchor.table_id;
        selection.anchor_row = anchor.row; selection.anchor_column = anchor.column; selection.active_row = destination.row; selection.active_column = destination.column;
        View.SelectTableCells(selection); return true;
    }
    private void DrawTableSelection(Microsoft.Graphics.Canvas.CanvasDrawingSession drawing, global::Windows.UI.Color color)
    {
        if (snapshot == null || tableSelection.active == 0 || tableSelection.document_revision != snapshot.Info.identity.document_revision) return;
        foreach (var cell in snapshot.TableCells) if (cell.table_id == tableSelection.table_id && cell.row >= Math.Min(tableSelection.anchor_row, tableSelection.active_row) && cell.row <= Math.Max(tableSelection.anchor_row, tableSelection.active_row) && cell.column >= Math.Min(tableSelection.anchor_column, tableSelection.active_column) && cell.column <= Math.Max(tableSelection.anchor_column, tableSelection.active_column)) drawing.FillRectangle(OffsetRect(cell.rect, viewport), color);
    }
}
