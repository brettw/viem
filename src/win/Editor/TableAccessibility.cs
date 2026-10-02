using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Windows.Foundation;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    protected override AutomationPeer OnCreateAutomationPeer() => new TableEditorPeer(this);

    /// <summary>Table nodes are views of the exact materialized core snapshot.</summary>
    private sealed class TableEditorPeer(EditorPane pane) : FrameworkElementAutomationPeer(pane)
    {
        protected override string GetClassNameCore() => nameof(EditorPane);
        protected override IList<AutomationPeer> GetChildrenCore()
        {
            var children = base.GetChildrenCore() ?? new List<AutomationPeer>();
            if (pane.View == null || pane.snapshot == null || pane.Document.State.format != VIEM_FORMAT_MARKDOWN) return children;
            foreach (var cells in pane.snapshot.TableCells.GroupBy(c => c.table_id)) {
                var first = cells.First();
                var context = pane.View.TableContextAt(first.text_start, pane.snapshot.Info.identity.document_id, pane.snapshot.Info.identity.document_revision);
                children.Add(new TablePeer(pane, context, pane.snapshot.Info.identity, cells.ToArray()));
            }
            return children;
        }
    }
    private sealed class TablePeer : FrameworkElementAutomationPeer, IGridProvider, ITableProvider, ISelectionProvider
    {
        internal readonly EditorPane Pane;
        internal readonly ViemTableContextV1 Context;
        internal readonly ViemLayoutSnapshotIdentityV1 Identity;
        private readonly CellPeer[] cells;
        internal TablePeer(EditorPane pane, ViemTableContextV1 context, ViemLayoutSnapshotIdentityV1 identity, ViemTableCellV1[] source) : base(pane)
        {
            Pane = pane; Context = context; Identity = identity;
            cells = source.Select(c => new CellPeer(this, c)).ToArray();
        }
        internal bool Current => Pane.snapshot != null && CoreView.SameLayout(Identity, Pane.snapshot.Info.identity);
        protected override object GetPatternCore(PatternInterface pattern) => pattern is PatternInterface.Grid or PatternInterface.Table or PatternInterface.Selection ? this : base.GetPatternCore(pattern);
        protected override string GetClassNameCore() => "MarkdownTable";
        protected override string GetNameCore() => $"Table, {ColumnCount} columns, {RowCount} rows including header";
        protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.Table;
        protected override IList<AutomationPeer> GetChildrenCore() => cells.Cast<AutomationPeer>().ToList();
        internal Rect ScreenBounds(ViemLayoutRectV1 rectangle)
        {
            var paneBounds = base.GetBoundingRectangleCore();
            var origin = Pane.Canvas.TransformToVisual(Pane).TransformPoint(new Point());
            var local = OffsetRect(rectangle, Pane.viewport);
            double scale = Pane.XamlRoot?.RasterizationScale ?? 1;
            return new Rect(paneBounds.X + (origin.X + local.X) * scale,
                paneBounds.Y + (origin.Y + local.Y) * scale, local.Width * scale, local.Height * scale);
        }
        protected override Rect GetBoundingRectangleCore() => cells.Length == 0 ? new Rect() : ScreenBounds(cells[0].Geometry.table_rect);

        public int RowCount => checked((int)Context.rows);
        public int ColumnCount => checked((int)Context.columns);
        public IRawElementProviderSimple GetItem(int row, int column)
        {
            if (!Current) throw new InvalidOperationException("The table changed. Refresh its accessibility snapshot.");
            var cell = cells.FirstOrDefault(c => c.Row == row && c.Column == column);
            if (cell == null) throw new InvalidOperationException("Scroll to this table cell before requesting its geometry.");
            return ProviderFromPeer(cell);
        }
        public RowOrColumnMajor RowOrColumnMajor => Microsoft.UI.Xaml.Automation.RowOrColumnMajor.RowMajor;
        public IRawElementProviderSimple[] GetColumnHeaders() => cells.Where(c => c.Row == 0).Select(ProviderFromPeer).ToArray();
        public IRawElementProviderSimple[] GetRowHeaders() => [];
        public bool CanSelectMultiple => true;
        public bool IsSelectionRequired => false;
        public IRawElementProviderSimple[] GetSelection() => cells.Where(c => c.Selected).Select(ProviderFromPeer).ToArray();
        internal IRawElementProviderSimple Provider => ProviderFromPeer(this);
    }
    private sealed class CellPeer(TablePeer table, ViemTableCellV1 cell) : FrameworkElementAutomationPeer(table.Pane), IGridItemProvider, ITableItemProvider, IInvokeProvider, IValueProvider, ISelectionItemProvider
    {
        internal ViemTableCellV1 Geometry => cell;
        public int Row => checked((int)cell.row);
        public int Column => checked((int)cell.column);
        public int RowSpan => 1;
        public int ColumnSpan => 1;
        public IRawElementProviderSimple ContainingGrid => table.Provider;
        internal bool Selected {
            get {
                var selection = table.Pane.tableSelection;
                return table.Current && selection.active != 0 && selection.table_id == cell.table_id
                    && cell.row >= Math.Min(selection.anchor_row, selection.active_row) && cell.row <= Math.Max(selection.anchor_row, selection.active_row)
                    && cell.column >= Math.Min(selection.anchor_column, selection.active_column) && cell.column <= Math.Max(selection.anchor_column, selection.active_column);
            }
        }
        protected override object GetPatternCore(PatternInterface pattern) => pattern is PatternInterface.GridItem or PatternInterface.TableItem or PatternInterface.Invoke or PatternInterface.Value or PatternInterface.SelectionItem ? this : base.GetPatternCore(pattern);
        protected override string GetClassNameCore() => "MarkdownTableCell";
        protected override string GetNameCore() => $"{(Row == 0 ? "Header" : $"Row {Row + 1}")}, column {Column + 1}{(Selected ? ", selected" : "")}";
        protected override string GetHelpTextCore() => "Invoke to edit this cell. Open the editor context menu for row, column and alignment actions.";
        protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.DataItem;
        protected override bool IsKeyboardFocusableCore() => true;
        protected override Rect GetBoundingRectangleCore() => table.ScreenBounds(cell.rect);
        public bool IsSelected => Selected;
        public IRawElementProviderSimple SelectionContainer => table.Provider;
        public void Select() => SetSelection(false);
        public void AddToSelection() => SetSelection(true);
        private void SetSelection(bool extend)
        {
            if (!table.Current || table.Pane.View == null) throw new InvalidOperationException("The table changed. Refresh its accessibility snapshot.");
            var selection = table.Pane.View.TableSelection();
            if (!extend || selection.active == 0 || selection.table_id != cell.table_id) {
                selection.anchor_row = cell.row; selection.anchor_column = cell.column;
            }
            selection.active = 1; selection.table_id = cell.table_id;
            selection.active_row = cell.row; selection.active_column = cell.column;
            table.Pane.Run(() => table.Pane.View.SelectTableCells(selection));
        }
        public void RemoveFromSelection()
        {
            if (!table.Current) throw new InvalidOperationException("The table changed. Refresh its accessibility snapshot.");
            if (!Selected || table.Pane.View == null) return;
            var selection = table.Pane.View.TableSelection();
            if (selection.anchor_row != selection.active_row || selection.anchor_column != selection.active_column)
                throw new InvalidOperationException("Choose a new rectangular selection to exclude this cell.");
            selection.active = 0;
            table.Pane.Run(() => table.Pane.View.SelectTableCells(selection));
        }
        protected override void SetFocusCore() => Invoke();
        public void Invoke()
        {
            if (!table.Current || table.Pane.View == null) throw new InvalidOperationException("The table changed. Refresh its accessibility snapshot.");
            table.Pane.Run(() => table.Pane.View.Place(cell.text_start, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, table.Identity.document_revision));
            table.Pane.FocusEditor();
        }
        public IRawElementProviderSimple[] GetColumnHeaderItems() => table.GetColumnHeaders().Skip(Column).Take(1).ToArray();
        public IRawElementProviderSimple[] GetRowHeaderItems() => [];
        public bool IsReadOnly => true;
        public string Value => table.Current ? table.Pane.Document.FormattedRange(cell.text_start, cell.text_end) : "";
        public void SetValue(string value) => throw new InvalidOperationException("Invoke the cell and use the editor text-input provider to replace its content.");
    }
}
