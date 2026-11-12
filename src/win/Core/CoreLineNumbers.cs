using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed record LineNumberSnapshot(ViemLineNumberSnapshotInfo Info, ViemLineNumberRegion[] Regions)
{
    internal bool Enabled => (Info.flags & VIEM_LINE_NUMBER_SNAPSHOT_ENABLED) != 0;
}

internal sealed unsafe partial class CoreView
{
    internal bool NumberEnabled { get { uint value = 0; Check(viem_core_view_number(Document.Handle, Id, &value), "Read line numbers"); return value != 0; } }
    internal void Number(bool value) { Check(viem_core_view_set_number(Document.Handle, Id, value ? 1u : 0u), "Set line numbers"); Changed?.Invoke(); }
    internal bool LineNumbersAreCurrent(ViemLineNumberSnapshotInfo expected)
    {
        var current = new ViemLineNumberSnapshotInfo();
        return viem_core_view_line_number_snapshot_info(Document.Handle, Id, &current) == VIEM_STATUS_OK
            && SameLayout(current.identity, expected.identity) && current.viewport.Equals(expected.viewport)
            && current.flags == expected.flags && current.current_source_line == expected.current_source_line;
    }
    internal LineNumberSnapshot LineNumbers(ViemLayoutSnapshotIdentity expected)
    {
        var info = new ViemLineNumberSnapshotInfo();
        Check(viem_core_view_line_number_snapshot_info(Document.Handle, Id, &info), "Read line numbers");
        if (!SameLayout(info.identity, expected)) throw new CoreException(VIEM_STATUS_STALE_REVISION, "Line number geometry changed.");
        var regions = new ViemLineNumberRegion[checked((int)info.region_count)];
        var identity = info.identity; var viewport = info.viewport; ulong count = 0;
        fixed (ViemLineNumberRegion* output = regions)
            Check(viem_core_view_copy_line_number_regions(Document.Handle, Id, &identity, &viewport, output, (ulong)regions.Length, &count), "Copy line numbers");
        if (count != (ulong)regions.Length) throw new InvalidOperationException("Incomplete line number geometry.");
        return new(info, regions);
    }
}
