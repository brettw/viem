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
        Composing = true;
        try { Apply(o => { var r = New<ViemCompositionBeginV1>(); r.document_revision = p.document_revision; r.replacement_start = p.cursor_utf8_offset; r.replacement_end = p.cursor_utf8_offset;
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
