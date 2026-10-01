using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed record LayoutSnapshot(ViemLayoutSnapshotInfoV1 Info, ViemVisualRowV1[] Rows,
    ViemPositionedClusterV1[] Clusters, ViemPositionedCaretV1[] Carets, ViemLayoutPaintInfoV1 Paint,
    ViemPaintStyleRunV1[] PaintRuns, ViemVisualSelectionRectangleV1[] Selection,
    ViemLayoutDecorationV1[] Decorations, string Diagnostics);

internal sealed unsafe partial class CoreView
{
    private LayoutSnapshot? cachedLayout;
    internal static bool SameLayout(ViemLayoutSnapshotIdentityV1 a, ViemLayoutSnapshotIdentityV1 b)
        => a.view_id == b.view_id && a.document_id == b.document_id && a.document_revision == b.document_revision
        && a.layout_revision == b.layout_revision && a.configuration_generation == b.configuration_generation
        && a.measurement_environment_id == b.measurement_environment_id && a.metrics_generation == b.metrics_generation;
    public ViemLayoutSnapshotInfoV1 LayoutInfo()
    { var info = New<ViemLayoutSnapshotInfoV1>(); Check(viem_core_view_layout_snapshot_info(Document.Handle, Id, &info), "Read layout"); return info; }
    public LayoutSnapshot Layout()
    {
        var info = LayoutInfo(); var identity = info.identity;
        // Geometry and paint belong to an immutable, fully identified snapshot.
        // Selection changes independently and must still be exported each turn.
        if (cachedLayout is { } cached && SameLayout(cached.Info.identity, identity))
            return cached with { Info = info, Selection = Selection().Rectangles };
        var rows = new ViemVisualRowV1[checked((int)info.row_count)];
        var clusters = new ViemPositionedClusterV1[checked((int)info.cluster_count)];
        var carets = new ViemPositionedCaretV1[checked((int)info.caret_count)];
        fixed (ViemVisualRowV1* r = rows) fixed (ViemPositionedClusterV1* c = clusters) fixed (ViemPositionedCaretV1* p = carets)
            Check(viem_core_view_copy_layout_snapshot(Document.Handle, Id, &identity, r, (ulong)rows.Length, c, (ulong)clusters.Length, p, (ulong)carets.Length, &info), "Copy layout");
        var paint = New<ViemLayoutPaintInfoV1>(); Check(viem_core_view_layout_paint_info(Document.Handle, Id, &paint), "Read paint");
        var runs = new ViemPaintStyleRunV1[checked((int)paint.paint_run_count)];
        fixed (ViemPaintStyleRunV1* p = runs) Check(viem_core_view_copy_layout_paint(Document.Handle, Id, &identity, p, (ulong)runs.Length, &paint), "Copy paint");
        var selection = Selection();
        var furniture = New<ViemLayoutDecorationsInfoV1>();
        uint s = viem_core_view_copy_layout_decorations(Document.Handle, Id, &identity, null, 0, null, 0, &furniture);
        if (s != VIEM_STATUS_BUFFER_TOO_SMALL) Check(s, "Read decorations");
        var decorations = new ViemLayoutDecorationV1[checked((int)furniture.decoration_count)]; var labels = new byte[checked((int)furniture.label_bytes)];
        fixed (ViemLayoutDecorationV1* d = decorations) fixed (byte* l = labels)
            Check(viem_core_view_copy_layout_decorations(Document.Handle, Id, &identity, d, (ulong)decorations.Length, l, (ulong)labels.Length, &furniture), "Copy decorations");
        string diagnostics = "";
        // Reading a warning cannot invalidate an otherwise complete frame.
        var diagnosticsIdentity = identity;
        try { diagnostics = System.Text.Encoding.UTF8.GetString(Copy((p, n, r) => {
            var expected = diagnosticsIdentity;
            return viem_core_view_copy_layout_diagnostics(Document.Handle, Id, &expected, p, n, r);
        })); } catch (CoreException) { }
        return cachedLayout = new(info, rows, clusters, carets, paint, runs, selection.Rectangles, decorations, diagnostics);
    }
    public (ViemVisualSelectionInfoV1 Info, ViemVisualSelectionSegmentV1[] Segments, ViemVisualSelectionRectangleV1[] Rectangles) Selection()
    {
        var info = New<ViemVisualSelectionInfoV1>(); Check(viem_core_view_visual_selection_info(Document.Handle, Id, &info), "Read selection");
        var identity = info.identity;
        var segments = new ViemVisualSelectionSegmentV1[checked((int)info.segment_count)];
        var rectangles = new ViemVisualSelectionRectangleV1[checked((int)info.rectangle_count)];
        fixed (ViemVisualSelectionSegmentV1* s = segments) fixed (ViemVisualSelectionRectangleV1* r = rectangles)
            Check(viem_core_view_copy_visual_selection(Document.Handle, Id, &identity, s, (ulong)segments.Length, r, (ulong)rectangles.Length, &info), "Copy selection");
        return (info, segments, rectangles);
    }
    public ViemLayoutCaretGeometryV1 CaretGeometry()
    {
        var p = Presentation;
        var request = New<ViemLayoutCaretRequestV1>(); request.identity = LayoutInfo().identity; request.text_offset = p.caret_utf8_start; request.affinity = p.cursor_affinity;
        var overlay = New<ViemCompositionOverlayInfoV1>();
        Check(viem_core_view_composition_overlay_info(Document.Handle, Id, &overlay), "Read composition caret");
        if ((overlay.flags & VIEM_COMPOSITION_OVERLAY_ACTIVE) != 0) { request.text_offset = overlay.selected_end; request.affinity = VIEM_BOUNDARY_AFFINITY_DOWNSTREAM; }
        var result = New<ViemLayoutCaretGeometryV1>();
        uint status = viem_core_view_caret_geometry(Document.Handle, Id, &request, &result);
        // At a document edge only the content-facing affinity may have geometry.
        // This asks for the same logical boundary in the same exact snapshot.
        if (status == VIEM_STATUS_OUTSIDE_LAYOUT_COVERAGE)
        {
            request.affinity = request.affinity == VIEM_BOUNDARY_AFFINITY_UPSTREAM ? VIEM_BOUNDARY_AFFINITY_DOWNSTREAM : VIEM_BOUNDARY_AFFINITY_UPSTREAM;
            status = viem_core_view_caret_geometry(Document.Handle, Id, &request, &result);
        }
        Check(status, "Read caret"); return result;
    }
}
