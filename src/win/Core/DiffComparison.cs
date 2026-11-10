using Viem.Windows.Interop;
using static Viem.Windows.Interop.Abi;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Core;

internal sealed unsafe partial class CoreDocument
{
    internal byte[]? SavedSource()
    {
        uint available = 0;
        ulong size = 0;
        uint status = viem_core_copy_saved_source(Handle, null, 0, &size, &available);
        if (status != VIEM_STATUS_BUFFER_TOO_SMALL) Check(status, "Read saved version");
        if (available == 0) return null;
        byte[] bytes = new byte[checked((int)size)];
        fixed (byte* p = bytes) Check(viem_core_copy_saved_source(Handle, p, size, &size, &available), "Copy saved version");
        return bytes;
    }

    internal void SetModifiable(bool enabled) =>
        Check(viem_core_set_modifiable(Handle, enabled ? 1u : 0u), "Set diff edit protection");

    private int deletedComparisons;
    internal void ProtectDeletedSide(bool protect)
    {
        if (protect && deletedComparisons == 0) SetModifiable(false);
        if (!protect && deletedComparisons == 1) SetModifiable(true);
        deletedComparisons += protect ? 1 : -1;
    }

    // The projection is shared even when two windows compare different bases.
    private int diffViews;
    private DocumentModeState? priorDiffMode;
    internal bool SwitchingDiffProjection { get; private set; }

    private void SetDiffMode(CoreView view, uint mode, string language, DocumentModeState expected)
    {
        SwitchingDiffProjection = true;
        try { view.SetDocumentMode(mode, language, expected); }
        finally { SwitchingDiffProjection = false; }
    }
    internal void EnterDiff(CoreView view)
    {
        if (diffViews == 0) {
            var mode = DocumentModes.Read(this);
            if (mode.Format != "code") {
                SetDiffMode(view, VIEM_DOCUMENT_MODE_CODE, mode.Format == "markdown" ? "markdown" : "", mode);
                priorDiffMode = mode;
            }
        }
        ++diffViews;
    }

    // Reload replaces all views together and opens the new core in the old
    // one's format. The last diff window still restores the pre-diff format.
    internal void InheritDiffMode(CoreDocument previous)
    {
        diffViews = previous.diffViews;
        priorDiffMode = previous.priorDiffMode;
    }
    internal void LeaveDiff(CoreView view, bool closingDocument = false)
    {
        if (--diffViews != 0) return;
        var prior = priorDiffMode;
        priorDiffMode = null;
        if (!closingDocument && prior != null && State.format == VIEM_FORMAT_CODE)
            SetDiffMode(view, prior.Format == "markdown" ? VIEM_DOCUMENT_MODE_MARKDOWN : VIEM_DOCUMENT_MODE_PLAIN_TEXT,
                "", DocumentModes.Read(this));
    }
}

internal sealed unsafe class DiffComparison : IDisposable
{
    private readonly CoreView basis;
    private CoreView live;
    private ulong handle;
    internal (ulong Added, ulong Removed) LineTotals { get; private set; }
    internal DiffComparison(CoreView basis, CoreView live, bool aligned, bool collapsed)
    {
        this.basis = basis;
        this.live = live;
        ulong session = 0;
        uint flags = (aligned ? VIEM_DIFF_SESSION_ALIGNED : 0) | (collapsed ? VIEM_DIFF_SESSION_COLLAPSED : 0);
        Check(viem_diff_session_create(basis.Document.Handle, live.Document.Handle, flags, &session), "Compare documents");
        handle = session;
    }
    internal void ReplaceLiveView(CoreView replacement) => live = replacement;
    internal void Sync()
    {
        using var timing = Diagnostics.InputPerformance.Measure("diff.sync");
        uint result = 0;
        Check(viem_diff_session_sync(handle, basis.Document.Handle, basis.Id, live.Document.Handle, live.Id, &result), "Update comparison");
        var info = New<ViemDiffSessionInfoV1>();
        Check(viem_diff_session_info(handle, &info), "Read comparison totals");
        LineTotals = (info.added, info.removed);
    }
    internal void Align(bool aligned)
    {
        Check(viem_diff_session_set_aligned(handle, aligned ? 1u : 0u), "Align sides");
        Sync();
    }
    internal bool Navigate(bool next)
    {
        var target = New<ViemDiffNavigationV1>();
        Check(viem_diff_session_navigate(handle, basis.Document.Handle, basis.Id, live.Document.Handle, live.Id,
            next ? VIEM_DIFF_NAVIGATE_NEXT : VIEM_DIFF_NAVIGATE_PREVIOUS, &target), "Go to change");
        return (target.flags & VIEM_DIFF_NAVIGATION_FOUND) != 0;
    }
    internal void SetCollapsed(bool collapsed) =>
        Check(viem_diff_session_set_collapsed(handle, collapsed ? 1u : 0u), "Collapse diff context");

    internal bool OpenCollapsed(bool fromLive, ulong line)
    {
        uint opened = 0;
        Check(viem_diff_session_open_collapsed(handle, fromLive ? 1u : 0u, line, &opened), "Open collapsed context");
        return opened != 0;
    }

    internal bool HasOpenedContext(bool fromLive)
    {
        var view = fromLive ? live : basis;
        uint opened = 0;
        Check(viem_diff_view_opened_context(view.Document.Handle, view.Id, &opened), "Check for opened diff context");
        return opened != 0;
    }
    internal uint Mirror(bool fromLive)
    {
        using var timing = Diagnostics.InputPerformance.Measure("diff.mirror");
        uint changed = 0;
        Check(viem_diff_session_sync_scroll(handle, basis.Document.Handle, basis.Id, live.Document.Handle, live.Id,
            fromLive ? 1u : 0u, &changed), "Align vertical scroll range and position");
        return changed;
    }
    internal (ViemDiffScrollbarMarkV1[] Marks, float Extent, ViemRgbaV1 Added, ViemRgbaV1 Removed) Marks(bool newSide)
    {
        using var timing = Diagnostics.InputPerformance.Measure(newSide ? "diff.marks.live" : "diff.marks.base");
        var view = newSide ? live : basis;
        ulong count = 0;
        float extent = 0;
        uint side = newSide ? 1u : 0u;
        uint status = viem_diff_session_scrollbar_marks(handle, view.Document.Handle, view.Id, side, null, 0, &count, &extent);
        if (status != VIEM_STATUS_BUFFER_TOO_SMALL) Check(status, "Measure diff marks");
        var marks = new ViemDiffScrollbarMarkV1[checked((int)count)];
        fixed (ViemDiffScrollbarMarkV1* p = marks)
            Check(viem_diff_session_scrollbar_marks(handle, view.Document.Handle, view.Id, side, p, count, &count, &extent), "Read diff marks");
        ViemRgbaV1 added, removed;
        Check(viem_diff_line_colors(&added, &removed), "Read diff colors");
        return (marks, extent, added, removed);
    }
    // A retiring view will discard its own presentation. Clearing it first
    // would synchronously lay out the departing file without diff styles.
    internal void RetireWithViews()
    {
        if (handle == 0) return;
        viem_diff_session_destroy(handle);
        handle = 0;
    }
    public void Dispose()
    {
        if (handle == 0) return;
        try { Check(viem_diff_clear_view(live.Document.Handle, live.Id), "Clear comparison"); }
        finally { RetireWithViews(); }
    }
}
