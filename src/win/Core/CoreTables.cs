using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed unsafe partial class CoreView
{
    public static bool SameSelection(ViemLogicalSelectionIdentityV1 left, ViemLogicalSelectionIdentityV1 right)
    {
        if (left.document_id != right.document_id || left.document_revision != right.document_revision
            || left.view_id != right.view_id || left.text_start != right.text_start || left.text_end != right.text_end
            || left.kind != right.kind) return false;
        for (int i = 0; i < 32; i++) if (left.state_identity[i] != right.state_identity[i]) return false;
        return true;
    }
    public ViemTableCellV1[] TableCells(ViemLayoutSnapshotIdentityV1 identity)
    {
        ulong required = 0;
        uint status = viem_core_view_copy_table_cells(Document.Handle, Id, &identity, null, 0, &required);
        if (status != VIEM_STATUS_BUFFER_TOO_SMALL) Check(status, "Read table geometry");
        var result = new ViemTableCellV1[checked((int)required)];
        fixed (ViemTableCellV1* p = result) Check(viem_core_view_copy_table_cells(Document.Handle, Id, &identity, p, required, &required), "Copy table geometry");
        return result;
    }
    public ViemTableSelectionV1 TableSelection()
    {
        var selection = New<ViemTableSelectionV1>();
        Check(viem_core_view_table_selection(Document.Handle, Id, &selection), "Read cell selection");
        return selection;
    }
    public string TableSelectionText(ViemTableSelectionV1 selection)
    {
        ulong required = 0;
        uint status = viem_core_view_copy_table_selection_text(Document.Handle, Id, &selection, null, 0, &required);
        if (status != VIEM_STATUS_BUFFER_TOO_SMALL) Check(status, "Read selected table text");
        var bytes = new byte[checked((int)required)];
        fixed (byte* p = bytes) Check(viem_core_view_copy_table_selection_text(Document.Handle, Id, &selection, p, required, &required), "Copy selected table text");
        return System.Text.Encoding.UTF8.GetString(bytes);
    }
    public ViemFormattedUtf8RangeV1[] TableSelectionRanges(ViemTableSelectionV1 selection)
    {
        ulong required = 0;
        uint status = viem_core_view_copy_table_selection_ranges(Document.Handle, Id, &selection, null, 0, &required);
        if (status != VIEM_STATUS_BUFFER_TOO_SMALL) Check(status, "Read selected table ranges");
        var ranges = new ViemFormattedUtf8RangeV1[checked((int)required)];
        fixed (ViemFormattedUtf8RangeV1* p = ranges) Check(viem_core_view_copy_table_selection_ranges(Document.Handle, Id, &selection, p, required, &required), "Copy selected table ranges");
        return ranges;
    }
    public void SelectTableCells(ViemTableSelectionV1 selection) => Apply(outcome => {
        var request = selection; return viem_core_view_select_table_cells(Document.Handle, Id, &request, outcome);
    });
    public ViemTableContextV1 TableContext()
    {
        var result = New<ViemTableContextV1>();
        Check(viem_core_view_table_context(Document.Handle, Id, &result), "Read table actions");
        return result;
    }
    public ViemTableContextV1 TableContextAt(ulong offset, ulong documentId, ulong revision)
    {
        var result = New<ViemTableContextV1>();
        Check(viem_core_view_table_context_at(Document.Handle, Id, documentId, revision, offset, &result), "Read table cell");
        return result;
    }
    public void InsertTable(uint columns, uint bodyRows, ViemLogicalSelectionIdentityV1 selection) => Apply(outcome => {
        var request = New<ViemInsertTableV1>(); request.expected_selection = selection;
        request.columns = columns; request.body_rows = bodyRows;
        return viem_core_view_insert_table(Document.Handle, Id, &request, outcome);
    });
    public void TableAction(uint action, ViemTableContextV1 expected, uint alignment = 0) => Apply(outcome => {
        var request = New<ViemTableActionV1>(); request.action = action; request.expected = expected; request.alignment = alignment;
        return viem_core_view_table_action(Document.Handle, Id, &request, outcome);
    });
}
