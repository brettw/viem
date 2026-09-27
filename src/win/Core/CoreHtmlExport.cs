using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed unsafe partial class CoreView
{
    public ulong PrepareHtmlExport(ulong revision)
    {
        ulong export = 0;
        Check(viem_core_prepare_html_export(Document.Handle, Id, revision, &export), "Prepare HTML export");
        return export;
    }

    public static byte[] RenderHtmlExport(ulong snapshot)
    {
        // The owned snapshot never touches the live editor from the worker and
        // survives source edits or closure while its immutable bytes are copied.
        try
        {
            Check(viem_html_export_render(snapshot), "Export HTML");
            return Copy((output, capacity, required) => viem_html_export_copy_utf8(snapshot, output, capacity, required));
        }
        finally { Check(viem_html_export_release(snapshot), "Release HTML export"); }
    }
}
