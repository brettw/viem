using System.Text.Json;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

// The native dropdown binds Text through ICustomPropertyProvider under NativeAOT.
[WinRT.GeneratedBindableCustomProperty([nameof(LinkHeading.Text)], [])]
internal sealed partial record LinkHeading(string Text, string Destination, byte Level, ulong Offset)
{
    // Editable ComboBox uses the item's string value independently of its
    // dropdown template. Keep that value a destination throughout tracking.
    public override string ToString() => Destination;
}

internal sealed unsafe partial class CoreView
{
    public (IReadOnlyList<LinkHeading> Headings, bool Truncated) LinkHeadings(LinkContext expected)
    {
        ValidateLinkContext(expected);
        var selection = expected.Selection;
        using var json = JsonDocument.Parse(Copy((p, n, r) => viem_core_copy_link_headings(Document.Handle,
            selection.document_id, selection.document_revision, p, n, r)));
        return (json.RootElement.GetProperty("headings").EnumerateArray().Select(h => new LinkHeading(
            h.GetProperty("text").GetString() ?? "", h.GetProperty("destination").GetString() ?? "",
            h.GetProperty("level").GetByte(), h.GetProperty("offset").GetUInt64())).ToArray(),
            json.RootElement.GetProperty("truncated").GetBoolean());
    }
}
