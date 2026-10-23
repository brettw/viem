using Microsoft.UI.Input;
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
    private TableHoverKind tableWidgetKind;
    private ulong tableWidgetTableId, tableWidgetRow, tableWidgetColumn;
    private Point tableWidgetViewport;
    private bool tablePopupTracking;
    private ViemTableCellV1? tableDragAnchor;
    private ViemLayoutSnapshotIdentityV1? tableDragIdentity;
    private bool tableCellDrag;
    private ViemTableSelectionV1 tableSelection;
    private (ulong Table, ulong Anchor, ulong Rows)? tableColumnGesture;
    // Whether a selection of every cell shows column rather than row controls.
    private bool tablePrefersColumns;
    private InputCursor? columnCursor;
    private bool pointerOverColumnStrip;

    private bool TableSelectionWidgetVisible => tableWidget?.Visibility == Visibility.Visible
        && tableWidgetKind is TableHoverKind.RowSelection or TableHoverKind.ColumnSelection;

    private void RefreshTableInteraction()
    {
        if (View == null || snapshot == null) return;
        tableSelection = View.TableSelection();
        if (!TableSelectionWidgetVisible) {
            if (tableWidgetIdentity is { } identity && (!CoreView.SameLayout(identity, snapshot.Info.identity) || tableWidgetViewport.X != viewport.left || tableWidgetViewport.Y != viewport.top)) HideTableWidget();
            if (tableWidgetSelection is { } expected && !CoreView.SameSelection(expected, View.LogicalSelection())) HideTableWidget();
        }
        RefreshTableSelectionWidget();
    }
    /// Selected complete rows or columns show their controls at the selection's
    /// active row or column, independent of hovering.
    private void RefreshTableSelectionWidget()
    {
        if (tablePopupTracking || View == null || snapshot == null) return;
        ViemTableContextV1 context = default;
        ViemTableCellV1? target = null;
        var kind = TableHoverKind.RowSelection;
        if (Document.State.format == VIEM_FORMAT_MARKDOWN && !gutterSelectionDragging && tableColumnGesture == null
            && tableSelection.active != 0 && tableSelection.document_revision == snapshot.Info.identity.document_revision) {
            context = View.TableContext();
            bool rows = (context.flags & VIEM_TABLE_ROW_SELECTION) != 0, columns = (context.flags & VIEM_TABLE_COLUMN_SELECTION) != 0;
            if (context.table_id == tableSelection.table_id && (rows || columns)) {
                kind = columns && (!rows || tablePrefersColumns) ? TableHoverKind.ColumnSelection : TableHoverKind.RowSelection;
                foreach (var cell in snapshot.TableCells)
                    if (cell.table_id == tableSelection.table_id
                        && (kind == TableHoverKind.RowSelection ? cell.row == tableSelection.active_row : cell.column == tableSelection.active_column)
                        && (target == null || cell.row < target.Value.row)) target = cell;
            }
        }
        if (target is not { } found) { if (TableSelectionWidgetVisible) DismissTableWidget(); return; }
        if (TableSelectionWidgetVisible && tableWidgetKind == kind && tableWidgetSelection is { } shown && CoreView.SameSelection(shown, context.selection)
            && tableWidgetRow == context.row && tableWidgetColumn == context.column
            && tableWidgetIdentity is { } identity && CoreView.SameLayout(identity, snapshot.Info.identity)
            && tableWidgetViewport.X == viewport.left && tableWidgetViewport.Y == viewport.top) return;
        ShowTableWidget(context, kind, OffsetRect(found.rect, viewport), OffsetRect(found.table_rect, viewport));
    }
    /// Hides hover controls. Controls for selected rows or columns follow the
    /// selection instead.
    private void HideTableWidget()
    {
        if (!TableSelectionWidgetVisible) DismissTableWidget();
    }
    private void DismissTableWidget()
    {
        if (tablePopupTracking) return;
        if (tableWidget != null) tableWidget.Visibility = Visibility.Collapsed;
        tableWidgetIdentity = null;
        tableWidgetSelection = null;
    }
    private Rect TableWidgetRectangle()
    {
        var origin = tableWidget!.TransformToVisual(Canvas).TransformPoint(new Point());
        return new(origin.X, origin.Y, tableWidget.Width, tableWidget.Height);
    }
    private bool TableControlOwnsPointer(Point point)
    {
        if (tableWidget?.Visibility != Visibility.Visible) return false;
        return tablePopupTracking || TableWidgetRectangle().Contains(point);
    }
    private void TableHover(Point point)
    {
        if (View == null || snapshot == null || Document.State.format != VIEM_FORMAT_MARKDOWN) { HideTableWidget(); return; }
        if (point.X < 0 || point.Y < 0 || point.X >= Canvas.ActualWidth || point.Y >= Canvas.ActualHeight) { HideTableWidget(); return; }
        Rect? retention = null;
        if (tableWidget?.Visibility == Visibility.Visible) {
            var widget = TableWidgetRectangle();
            if (widget.Contains(point) || tablePopupTracking || TableSelectionWidgetVisible) return;
            retention = TableHoverGeometry.RetentionPath(widget, tableActivation);
        }
        var visible = new Rect(0, 0, Canvas.ActualWidth, Canvas.ActualHeight);
        foreach (var cell in snapshot.TableCells) {
            var rect = OffsetRect(cell.rect, viewport); var table = OffsetRect(cell.table_rect, viewport);
            if (TableHoverGeometry.Hit(point, rect, table, visible) is not { } kind) continue;
            var activation = TableHoverGeometry.Activation(kind, rect, table, visible);
            if (retention != null && kind == tableWidgetKind && activation.Equals(tableActivation)) return;
            // A flipped toolbar's bridge crosses the opposite gutter of the same row.
            if (retention is { } rowPath && rowPath.Contains(point)
                && kind != TableHoverKind.Column && tableWidgetKind != TableHoverKind.Column
                && cell.table_id == tableWidgetTableId && cell.row == tableWidgetRow) return;
            var context = View.TableContextAt(cell.text_start, snapshot.Info.identity.document_id, snapshot.Info.identity.document_revision);
            if ((context.flags & 2) != 0) ShowTableWidget(context, kind, rect, table);
            return;
        }
        if (retention is { } path && path.Contains(point)) return;
        HideTableWidget();
    }
    private static string TableRowsName(ViemTableContextV1 context) =>
        context.row_end - context.row > 1 ? $"rows {context.row + 1}–{context.row_end}" : context.row == 0 ? "header row" : $"row {context.row + 1}";
    private static string TableColumnsName(ViemTableContextV1 context) =>
        context.column_end - context.column > 1 ? $"columns {context.column + 1}–{context.column_end}" : $"column {context.column + 1}";
    private static string TableAlignmentName(ViemTableContextV1 context) =>
        string.Concat(char.ToUpperInvariant(TableColumnsName(context)[0]).ToString(), TableColumnsName(context).AsSpan(1), " alignment");

    private void ShowTableWidget(ViemTableContextV1 context, TableHoverKind kind, Rect cell, Rect table)
    {
        if (tableWidget == null) {
            tableWidget = new Border { CornerRadius = new(5), Padding = new(3), HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Top };
            Children.Add(tableWidget);
            tableWidget.PointerExited += (_, e) => TableHover(e.GetCurrentPoint(Canvas).Position);
        }
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 1 };
        tableWidget.Child = row; tableWidget.Background = (Brush)Application.Current.Resources["SolidBackgroundFillColorBaseBrush"];
        var actions = new List<(string Name, string Icon, uint Action)>();
        string rows = TableRowsName(context), columns = TableColumnsName(context);
        bool header = (context.flags & VIEM_TABLE_HEADER) != 0;
        if (kind is TableHoverKind.Column or TableHoverKind.ColumnSelection) {
            actions.Add(($"Insert column left of {columns}", "+←", 4));
            actions.Add((TableAlignmentName(context), context.alignment == 2 ? "\uE8E3" : context.alignment == 3 ? "\uE8E2" : "\uE8E4", 7));
            actions.Add(($"Delete {columns}", "−", 6));
            actions.Add(($"Insert column right of {columns}", "+→", 5));
        } else if (kind == TableHoverKind.RowSelection) {
            if (!header) actions.Add(($"Insert row above {rows}", "+↑", 1));
            actions.Add(($"Delete {rows}", "−", 3));
            actions.Add(($"Insert row below {rows}", "+↓", 2));
        } else if (header) {
            // The header row can only gain a body row below it.
            actions.Add(("Insert row below row 1", "+↓", 2));
        } else {
            actions.Add(($"Insert row above {rows}", "+↑", 1));
            actions.Add(($"Delete {rows}", "−", 3));
            actions.Add(($"Insert row below {rows}", "+↓", 2));
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
        var placement = TableHoverGeometry.Placement(kind, cell, table, new(width, height), new(Canvas.ActualWidth, Canvas.ActualHeight));
        tableWidget.Width = width; tableWidget.Height = height; tableWidget.Margin = new(placement.X, placement.Y, 0, 0); tableWidget.Visibility = Visibility.Visible;
        tableActivation = TableHoverGeometry.Activation(kind, cell, table, new(0, 0, Canvas.ActualWidth, Canvas.ActualHeight)); tableWidgetKind = kind;
        tableWidgetTableId = context.table_id; tableWidgetRow = context.row; tableWidgetColumn = context.column;
        tableWidgetIdentity = snapshot!.Info.identity; tableWidgetViewport = new(viewport.left, viewport.top);
        tableWidgetSelection = context.selection;
    }
#if DEBUG
    internal void HoverTableForTest(Point point) => TableHover(point);
    internal Border? VisibleTableWidgetForTest => tableWidget?.Visibility == Visibility.Visible ? tableWidget : null;
#endif

    private void PerformTableAction(ViemTableContextV1 context, uint action, uint alignment = 0)
    {
        Run(() => View?.TableAction(action, context, alignment)); HideTableWidget(); FocusEditor();
    }
    private MenuFlyout TableAlignmentMenu(ViemTableContextV1 context)
    {
        var menu = new MenuFlyout();
        foreach (var (name, alignment, glyph) in new[] { ("Left", 1u, "\uE8E4"), ("Center", 2u, "\uE8E3"), ("Right", 3u, "\uE8E2") }) {
            // Columns with different alignments check no choice.
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
            var selected = View.TableContext();
            var context = pointer ? target.GetValueOrDefault() : selected; if ((context.flags & 2) == 0) return;
            // Clicking inside selected complete rows or columns targets them.
            if (pointer && (selected.flags & (VIEM_TABLE_ROW_SELECTION | VIEM_TABLE_COLUMN_SELECTION)) != 0 && selected.table_id == context.table_id
                && context.row >= selected.row && context.row < selected.row_end && context.column >= selected.column && context.column < selected.column_end) context = selected;
            bool rowGroup = (context.flags & VIEM_TABLE_ROW_SELECTION) != 0, columnGroup = (context.flags & VIEM_TABLE_COLUMN_SELECTION) != 0;
            string rows = TableRowsName(context), columns = TableColumnsName(context);
            menu.Items.Add(new MenuFlyoutSeparator());
            void Add(string title, uint action) { var item = new MenuFlyoutItem { Text = title }; item.Click += (_, _) => PerformTableAction(context, action); menu.Items.Add(item); }
            if (rowGroup || !columnGroup) {
                if (context.row > 0) Add($"Insert row above {rows}", 1);
                Add($"Insert row below {rows}", 2);
                // The header row alone has no delete; a selected group clears it.
                if (rowGroup || context.row > 0) Add($"Delete {rows}", 3);
            }
            if (rowGroup && !columnGroup) return;
            Add($"Insert column left of {columns}", 4); Add($"Insert column right of {columns}", 5); Add($"Delete {columns}", 6);
            var alignment = new MenuFlyoutSubItem { Text = TableAlignmentName(context) };
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
    /// The table cell whose column owns the strip along the table's top edge.
    private ViemTableCellV1? TableColumnStripCell(Point point)
    {
        if (snapshot == null || Document.State.format != VIEM_FORMAT_MARKDOWN) return null;
        if (tableWidget?.Visibility == Visibility.Visible && TableWidgetRectangle().Contains(point)) return null;
        var visible = new Rect(0, 0, Canvas.ActualWidth, Canvas.ActualHeight);
        foreach (var cell in snapshot.TableCells)
            if (TableHoverGeometry.Hit(point, OffsetRect(cell.rect, viewport), OffsetRect(cell.table_rect, viewport), visible) == TableHoverKind.Column) return cell;
        return null;
    }
    /// Begins selecting complete columns, like the row-selection margin does
    /// for rows. Returns false when the point is not in a column strip.
    private bool BeginTableColumnSelection(Point point)
    {
        if (View == null || snapshot == null || TableColumnStripCell(point) is not { } cell) return false;
        var context = View.TableContextAt(cell.text_start, snapshot.Info.identity.document_id, snapshot.Info.identity.document_revision);
        if ((context.flags & VIEM_TABLE_IN_TABLE) == 0 || context.rows == 0) return false;
        tableCellDrag = false; tableDragAnchor = null; tableDragIdentity = null;
        DismissTableWidget();
        tableColumnGesture = (cell.table_id, cell.column, context.rows);
        tablePrefersColumns = true;
        SelectTableColumns(cell.column);
        return true;
    }
    private void DragTableColumns(Point point)
    {
        if (snapshot == null || tableColumnGesture is not { } gesture) return;
        ViemTableCellV1? nearest = null; double best = double.MaxValue;
        foreach (var cell in snapshot.TableCells) {
            if (cell.table_id != gesture.Table) continue;
            var rect = OffsetRect(cell.rect, viewport);
            double distance = Math.Max(Math.Max(rect.Left - point.X, 0), point.X - rect.Right);
            if (distance < best || distance == best && nearest is { } current && cell.column > current.column) { best = distance; nearest = cell; }
        }
        if (nearest is { } found) SelectTableColumns(found.column);
    }
    private void SelectTableColumns(ulong column)
    {
        if (View == null || snapshot == null || tableColumnGesture is not { } gesture) return;
        // Like row selection, the caret ends the selection in the drag's
        // direction: at the end of the last column's final cell, or at the
        // start of the first column's header cell.
        bool forward = column >= gesture.Anchor;
        ulong anchorRow = forward ? 0 : gesture.Rows - 1, activeRow = forward ? gesture.Rows - 1 : 0;
        var current = View.TableSelection();
        if (current.active != 0 && current.table_id == gesture.Table && current.anchor_row == anchorRow
            && current.anchor_column == gesture.Anchor && current.active_row == activeRow && current.active_column == column) return;
        var selection = Interop.Abi.New<ViemTableSelectionV1>(); selection.active = 1;
        selection.document_id = snapshot.Info.identity.document_id; selection.document_revision = snapshot.Info.identity.document_revision;
        selection.table_id = gesture.Table; selection.anchor_row = anchorRow; selection.anchor_column = gesture.Anchor;
        selection.active_row = activeRow; selection.active_column = column;
        View.SelectTableCells(selection);
    }
    private void EndTableColumnSelection()
    {
        tableColumnGesture = null;
        try { RefreshTableInteraction(); }
        catch (CoreException error) { PresentationWarning("table controls", error); }
    }
#if DEBUG
    internal bool BeginTableColumnSelectionForTest(Point point) => BeginTableColumnSelection(point);
    internal void DragTableColumnsForTest(Point point) => DragTableColumns(point);
    internal void EndTableColumnSelectionForTest() => EndTableColumnSelection();
    internal void RefreshTableInteractionForTest() => RefreshTableInteraction();
    internal bool ColumnStripForTest(Point point) => TableColumnStripCell(point) != null;
#endif

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
