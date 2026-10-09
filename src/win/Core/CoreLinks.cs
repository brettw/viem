using System.Text.Json;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed record MarkdownLink(ulong Start, ulong End, string Text, string Destination, bool Editable);
internal sealed record LinkContext(ViemLogicalSelectionIdentityV1 Selection, bool CanInsert, bool Linked, bool CanRemoveSelection, bool CanExitLink, string Text, MarkdownLink? Link);

internal sealed unsafe partial class CoreView
{
    // The core exports only the current selection and its containing link. Never
    // copy/search the full formatted document while following the caret.
    public LinkContext LinkContext()
    {
        var selection = LogicalSelection();
        using var json = JsonDocument.Parse(Copy((p, n, r) => viem_core_view_copy_link_context(Document.Handle, Id, p, n, r)));
        var root = json.RootElement;
        var identity = root.GetProperty("selection");
        if (identity.GetProperty("documentId").GetUInt64() != selection.document_id
            || identity.GetProperty("revision").GetUInt64() != selection.document_revision
            || identity.GetProperty("viewId").GetUInt64() != Id
            || identity.GetProperty("start").GetUInt64() != selection.text_start
            || identity.GetProperty("end").GetUInt64() != selection.text_end
            || identity.GetProperty("kind").GetUInt32() != selection.kind)
            throw new CoreException(VIEM_STATUS_STALE_REVISION, "The document changed. Select the link again.");
        var value = root.GetProperty("link");
        MarkdownLink? link = value.ValueKind == JsonValueKind.Null ? null : new(
            value.GetProperty("start").GetUInt64(), value.GetProperty("end").GetUInt64(),
            value.GetProperty("text").GetString() ?? "", value.GetProperty("destination").GetString() ?? "",
            value.GetProperty("editable").GetBoolean());
        return new(selection, root.GetProperty("canInsert").GetBoolean(), root.GetProperty("linked").GetBoolean(),
            root.GetProperty("canRemoveSelection").GetBoolean(), root.GetProperty("canExitLink").GetBoolean(),
            root.GetProperty("text").GetString() ?? "", link);
    }
    public void ValidateLinkContext(LinkContext expected)
    {
        if (Id == 0 || !HasFormattingSelection) throw new InvalidOperationException("The selection changed. Select the link again.");
        var current = LinkContext();
        if (!SameSelection(current.Selection, expected.Selection) || current.Link != expected.Link
            || current.Linked != expected.Linked || current.CanExitLink != expected.CanExitLink || current.CanRemoveSelection != expected.CanRemoveSelection)
            throw new InvalidOperationException("The document or selection changed. Select the link again.");
    }
    public void ToggleLink(LinkContext expected)
    {
        ValidateLinkContext(expected);
        Apply(outcome => {
            var selection = expected.Selection;
            return viem_core_view_edit_link(Document.Handle, Id, &selection, HasSelection ? 4u : 3u,
                expected.Link?.Start ?? 0, expected.Link?.End ?? 0, default, default, outcome);
        });
        FormattingContextChanged?.Invoke();
    }
    public void EditLink(LinkContext expected, string text, string destination, bool remove = false, bool insertOnly = false)
    {
        ValidateLinkContext(expected);
        using var arena = new NativeArena();
        var label = arena.Utf8(text); var target = arena.Utf8(destination);
        Apply(outcome => {
            var selection = expected.Selection;
            return viem_core_view_edit_link(Document.Handle, Id, &selection, remove ? 2u : !insertOnly && expected.Link != null ? 1u : 0u,
                expected.Link?.Start ?? 0, expected.Link?.End ?? 0, label, target, outcome);
        });
    }
    public void GoToLinkFragment(string fragment)
    {
        var state = Document.State;
        using var arena = new NativeArena(); var text = arena.Utf8(fragment);
        ulong offset = 0; byte found = 0;
        Check(viem_core_find_link_fragment(Document.Handle, state.document_id, state.document_revision, text, &offset, &found), "Find link destination");
        if (found == 0) throw new InvalidOperationException("This heading was not found in the document.");
        Place(offset, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, state.document_revision);
    }
}
