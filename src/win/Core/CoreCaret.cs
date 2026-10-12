using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed unsafe partial class CoreView
{
    public float FontEnWidth(ulong revision)
    {
        float width = 0;
        Check(viem_core_view_font_en_width(Document.Handle, Id, revision, &width), "Read typing font en width");
        return width;
    }
}
