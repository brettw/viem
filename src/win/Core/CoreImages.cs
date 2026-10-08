using System.Text.Json;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed record MarkdownImage(ulong Start, ulong End, string Text, string Destination, bool Editable);
internal sealed record ImageContext(ViemLogicalSelectionIdentityV1 Selection, bool CanInsert, string Text, MarkdownImage? Image);

internal sealed unsafe partial class CoreView
{
    // The core exports only the current selection and its containing image. Never
    // copy/search the full formatted document while following the caret.
    public ImageContext ImageContext()
    {
        var selection = LogicalSelection();
        using var json = JsonDocument.Parse(Copy((p, n, r) => viem_core_view_copy_image_context(Document.Handle, Id, p, n, r)));
        var root = json.RootElement;
        var identity = root.GetProperty("selection");
        if (identity.GetProperty("documentId").GetUInt64() != selection.document_id
            || identity.GetProperty("revision").GetUInt64() != selection.document_revision
            || identity.GetProperty("viewId").GetUInt64() != Id
            || identity.GetProperty("start").GetUInt64() != selection.text_start
            || identity.GetProperty("end").GetUInt64() != selection.text_end
            || identity.GetProperty("kind").GetUInt32() != selection.kind)
            throw new CoreException(VIEM_STATUS_STALE_REVISION, "The document changed. Select the image again.");
        var value = root.GetProperty("image");
        MarkdownImage? image = value.ValueKind == JsonValueKind.Null ? null : new(
            value.GetProperty("start").GetUInt64(), value.GetProperty("end").GetUInt64(),
            value.GetProperty("text").GetString() ?? "", value.GetProperty("destination").GetString() ?? "",
            value.GetProperty("editable").GetBoolean());
        return new(selection, root.GetProperty("canInsert").GetBoolean(), root.GetProperty("text").GetString() ?? "", image);
    }
    public void ValidateImageContext(ImageContext expected)
    {
        if (Id == 0 || !HasFormattingSelection) throw new InvalidOperationException("The selection changed. Select the image again.");
        var current = ImageContext();
        if (!SameSelection(current.Selection, expected.Selection) || current.Image != expected.Image)
            throw new InvalidOperationException("The document or selection changed. Select the image again.");
    }
    public void ImageResourcesChanged(ulong previousGeneration, string[] destinations)
    {
        using var arena = new NativeArena();
        var locations = destinations.Select(arena.Utf8).ToArray();
        var native = arena.Copy<ViemUtf8Slice>(locations);
        Check(viem_core_image_resources_changed(Document.Handle, Id, previousGeneration, native, (ulong)locations.Length), "Refresh image resources");
        Refresh();
    }
    public void SelectImage(ulong offset, ulong revision)
    {
        ulong documentId = Document.State.document_id;
        Apply(outcome => viem_core_view_select_image(Document.Handle, Id, documentId, revision, offset, outcome));
    }
    public void EditImage(ImageContext expected, string text, string destination, bool remove = false)
    {
        ValidateImageContext(expected);
        using var arena = new NativeArena();
        var label = arena.Utf8(text); var target = arena.Utf8(destination);
        Apply(outcome => {
            var selection = expected.Selection;
            return viem_core_view_edit_image(Document.Handle, Id, &selection, remove ? 2u : expected.Image != null ? 1u : 0u,
                expected.Image?.Start ?? 0, expected.Image?.End ?? 0, label, target, outcome);
        });
    }
}
