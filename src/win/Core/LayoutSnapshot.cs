using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed record LayoutSnapshot(ViemLayoutSnapshotInfo Info, ViemVisualRow[] Rows,
    ViemPositionedCluster[] Clusters, ViemPositionedCaret[] Carets, ViemLayoutPaintInfo Paint,
    ViemPaintStyleRun[] PaintRuns, ViemVisualSelectionRectangle[] Selection,
    ViemLayoutDecoration[] Decorations, string Diagnostics)
{
    internal byte[] DecorationLabels { get; init; } = [];
    internal ViemTableCell[] TableCells { get; init; } = [];
    internal ViemVisualSelectionSegment[] SelectionSegments { get; init; } = [];
}

internal sealed unsafe partial class CoreView
{
    private LayoutSnapshot? cachedLayout;
    internal static bool SameLayout(ViemLayoutSnapshotIdentity a, ViemLayoutSnapshotIdentity b)
        => a.view_id == b.view_id && a.document_id == b.document_id && a.document_revision == b.document_revision
        && a.layout_revision == b.layout_revision && a.configuration_generation == b.configuration_generation
        && a.measurement_environment_id == b.measurement_environment_id && a.metrics_generation == b.metrics_generation;
    public ViemLayoutSnapshotInfo LayoutInfo()
    { var info = new ViemLayoutSnapshotInfo(); Check(viem_core_view_layout_snapshot_info(Document.Handle, Id, &info), "Read layout"); return info; }
    internal Action<string>? BeforePresentationExport { get; set; }
    public LayoutSnapshot Layout()
    {
        var info = LayoutInfo(); var identity = info.identity;
        var cached = cachedLayout is { } previous && SameLayout(previous.Info.identity, identity) ? previous : null;
        var rows = cached?.Rows ?? new ViemVisualRow[checked((int)info.row_count)];
        var clusters = cached?.Clusters ?? new ViemPositionedCluster[checked((int)info.cluster_count)];
        var carets = cached?.Carets ?? new ViemPositionedCaret[checked((int)info.caret_count)];
        if (cached == null) {
            BeforePresentationExport?.Invoke("layout");
            var geometryIdentity = identity;
            fixed (ViemVisualRow* r = rows) fixed (ViemPositionedCluster* c = clusters) fixed (ViemPositionedCaret* p = carets)
                Check(viem_core_view_copy_layout_snapshot(Document.Handle, Id, &geometryIdentity, r, (ulong)rows.Length, c, (ulong)clusters.Length, p, (ulong)carets.Length, &info), "Copy layout");
        }
        bool complete = true;
        T Optional<T>(string name, T fallback, Func<T> read)
        {
            try { BeforePresentationExport?.Invoke(name); return read(); }
            catch (CoreException error) {
                complete = false;
                Document.ConfigurationWarning($"Could not display {name}. {error.Message}");
                return fallback;
            }
        }
        var defaultPaint = new ViemLayoutPaintInfo();
        defaultPaint.identity = identity; defaultPaint.flags = VIEM_LAYOUT_PAINT_DEFAULT_CANVAS;
        defaultPaint.default_paint.flags = VIEM_TEXT_PAINT_DEFAULT_FOREGROUND;
        var paintResult = Optional("paint", (Info: defaultPaint, Runs: Array.Empty<ViemPaintStyleRun>()), () => {
            if (cached != null) return (Info: cached.Paint, Runs: cached.PaintRuns);
            var paint = new ViemLayoutPaintInfo();
            Check(viem_core_view_layout_paint_info(Document.Handle, Id, &paint), "Read paint");
            if (!SameLayout(paint.identity, identity)) throw new CoreException(VIEM_STATUS_STALE_REVISION, "Paint geometry changed.");
            var runs = new ViemPaintStyleRun[checked((int)paint.paint_run_count)];
            var expected = identity;
            fixed (ViemPaintStyleRun* p = runs) Check(viem_core_view_copy_layout_paint(Document.Handle, Id, &expected, p, (ulong)runs.Length, &paint), "Copy paint");
            return (Info: paint, Runs: runs);
        });
        var selection = Optional("selection", (Segments: Array.Empty<ViemVisualSelectionSegment>(), Rectangles: Array.Empty<ViemVisualSelectionRectangle>()), () => {
            var selected = Selection();
            if (!SameLayout(selected.Info.identity.layout, identity)) throw new CoreException(VIEM_STATUS_STALE_REVISION, "Selection geometry changed.");
            return (selected.Segments, selected.Rectangles);
        });
        var furnitureResult = Optional("paragraph decorations", (Decorations: Array.Empty<ViemLayoutDecoration>(), Labels: Array.Empty<byte>()), () => {
            if (cached != null) return (Decorations: cached.Decorations, Labels: cached.DecorationLabels);
            var furniture = new ViemLayoutDecorationsInfo(); var expected = identity;
            uint status = viem_core_view_copy_layout_decorations(Document.Handle, Id, &expected, null, 0, null, 0, &furniture);
            if (status != VIEM_STATUS_BUFFER_TOO_SMALL) Check(status, "Read decorations");
            var decorations = new ViemLayoutDecoration[checked((int)furniture.decoration_count)]; var labels = new byte[checked((int)furniture.label_bytes)];
            fixed (ViemLayoutDecoration* d = decorations) fixed (byte* l = labels)
                Check(viem_core_view_copy_layout_decorations(Document.Handle, Id, &expected, d, (ulong)decorations.Length, l, (ulong)labels.Length, &furniture), "Copy decorations");
            return (Decorations: decorations, Labels: labels);
        });
        var cells = Optional("table controls", Array.Empty<ViemTableCell>(), () => cached?.TableCells ?? TableCells(identity));
        string diagnostics = cached?.Diagnostics ?? "";
        var diagnosticsIdentity = identity;
        try { diagnostics = System.Text.Encoding.UTF8.GetString(Copy((p, n, r) => {
            var expected = diagnosticsIdentity;
            return viem_core_view_copy_layout_diagnostics(Document.Handle, Id, &expected, p, n, r);
        })); } catch (CoreException) { }
        var result = new LayoutSnapshot(info, rows, clusters, carets, paintResult.Info, paintResult.Runs,
            selection.Rectangles, furnitureResult.Decorations, diagnostics) {
            TableCells = cells, SelectionSegments = selection.Segments, DecorationLabels = furnitureResult.Labels
        };
        // Retry omitted exports on the next frame; never cache an optional
        // failure for the lifetime of an otherwise valid geometry identity.
        cachedLayout = complete ? result : null;
        return result;
    }
    public (ViemVisualSelectionInfo Info, ViemVisualSelectionSegment[] Segments, ViemVisualSelectionRectangle[] Rectangles) Selection()
    {
        var info = new ViemVisualSelectionInfo(); Check(viem_core_view_visual_selection_info(Document.Handle, Id, &info), "Read selection");
        var identity = info.identity;
        var segments = new ViemVisualSelectionSegment[checked((int)info.segment_count)];
        var rectangles = new ViemVisualSelectionRectangle[checked((int)info.rectangle_count)];
        fixed (ViemVisualSelectionSegment* s = segments) fixed (ViemVisualSelectionRectangle* r = rectangles)
            Check(viem_core_view_copy_visual_selection(Document.Handle, Id, &identity, s, (ulong)segments.Length, r, (ulong)rectangles.Length, &info), "Copy selection");
        return (info, segments, rectangles);
    }
    public ViemPointerCaret PointerCaret(float x, float y, ViemLayoutSnapshotInfo snapshot)
    {
        var request = new ViemLayoutHitTestRequest(); request.identity = snapshot.identity; request.x = x; request.y = y;
        var result = new ViemPointerCaret();
        Check(viem_core_view_pointer_caret(Document.Handle, Id, &request, &result), "Preview pointer caret"); return result;
    }
    public ViemLayoutCaretGeometry CaretGeometryAt(ulong offset, uint affinity, ViemLayoutSnapshotInfo snapshot)
    {
        var request = new ViemLayoutCaretRequest(); request.identity = snapshot.identity; request.text_offset = offset; request.affinity = affinity;
        return CaretGeometry(request);
    }
    public ViemLayoutCaretGeometry CaretGeometry()
    {
        var p = Presentation;
        var request = new ViemLayoutCaretRequest(); request.identity = LayoutInfo().identity; request.text_offset = p.caret_utf8_start; request.affinity = p.cursor_affinity;
        var overlay = new ViemCompositionOverlayInfo();
        Check(viem_core_view_composition_overlay_info(Document.Handle, Id, &overlay), "Read composition caret");
        if ((overlay.flags & VIEM_COMPOSITION_OVERLAY_ACTIVE) != 0) { request.text_offset = overlay.selected_end; request.affinity = VIEM_BOUNDARY_AFFINITY_DOWNSTREAM; }
        return CaretGeometry(request);
    }
    private ViemLayoutCaretGeometry CaretGeometry(ViemLayoutCaretRequest request)
    {
        var result = new ViemLayoutCaretGeometry();
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
