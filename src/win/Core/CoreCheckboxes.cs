using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Core;

internal sealed unsafe partial class CoreView
{
    public void ToggleCheckbox(ViemLayoutSnapshotIdentityV1 expected, ulong paragraph) => Apply(outcome => {
        var identity = expected;
        return viem_core_view_toggle_checkbox(Document.Handle, Id, &identity, paragraph, outcome);
    });
}
