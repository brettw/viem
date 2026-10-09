using System.Text;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Core;

internal sealed unsafe partial class CoreView
{
    public void SetCodeBlockLanguage(ulong document, ulong revision, ulong offset, string language)
    {
        var bytes = Encoding.UTF8.GetBytes(language);
        Apply(outcome => {
            fixed (byte* data = bytes)
                return viem_core_view_set_code_block_language(Document.Handle, Id, document, revision,
                    offset, new ViemUtf8Slice { data = data, length = (ulong)bytes.Length }, outcome);
        });
    }
}
