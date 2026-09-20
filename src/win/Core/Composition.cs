using System.Text;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed unsafe partial class CoreView
{
    public bool Composing { get; private set; }
    public void BeginComposition()
    {
        var p = Presentation;
        ulong start = p.cursor_utf8_offset, end = start;
        if (IsTextSelectionMode(p.mode))
        {
            var selection = LogicalSelection();
            if (selection.kind == VIEM_LOGICAL_SELECTION_KIND_BLOCK)
                throw new InvalidOperationException("IME composition requires a character or line selection.");
            start = selection.text_start; end = selection.text_end;
        }
        Composing = true;
        try { Apply(o => { var r = New<ViemCompositionBeginV1>(); r.document_revision = p.document_revision; r.replacement_start = start; r.replacement_end = end;
            return viem_core_view_composition_begin(Document.Handle, Id, &r, o); }); }
        catch { Composing = false; throw; }
    }
    public void UpdateComposition(string text, int selectionStart, int selectionLength)
    {
        if (!Composing) return;
        byte[] bytes = Encoding.UTF8.GetBytes(text);
        Apply(o => { var r = New<ViemCompositionUpdateV1>(); r.document_revision = Document.State.document_revision;
            r.selected_start = (ulong)Encoding.UTF8.GetByteCount(text.AsSpan(0, Math.Clamp(selectionStart, 0, text.Length)));
            r.selected_end = (ulong)Encoding.UTF8.GetByteCount(text.AsSpan(0, Math.Clamp(selectionStart + selectionLength, 0, text.Length)));
            fixed (byte* p = bytes) { r.marked_text = new() { data = p, length = (ulong)bytes.Length }; return viem_core_view_composition_update(Document.Handle, Id, &r, o); } });
    }
    public void CommitComposition(string text)
    {
        if (!Composing) { Text(text); return; }
        byte[] bytes = Encoding.UTF8.GetBytes(text);
        Apply(o => { var r = New<ViemCompositionCommitV1>(); r.document_revision = Document.State.document_revision;
            fixed (byte* p = bytes) { r.committed_text = new() { data = p, length = (ulong)bytes.Length }; return viem_core_view_composition_commit(Document.Handle, Id, &r, o); } });
        Composing = false;
    }
    public void CancelComposition()
    {
        if (!Composing) return;
        Apply(o => { var r = New<ViemCompositionCancelV1>(); r.document_revision = Document.State.document_revision; return viem_core_view_composition_cancel(Document.Handle, Id, &r, o); });
        Composing = false;
    }
}
