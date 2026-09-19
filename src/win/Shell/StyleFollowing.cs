using Microsoft.UI.Dispatching;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    private bool followsCaret;
    private bool refreshAfterFollowing;
    // This snapshot is only compared with the next presentation to detect a
    // change, never resolved as a retained editing position.
    private ViemViewPresentationV1 followedPresentation;
    private DispatcherQueueTimer? caretFollowTimer;

    private void AttachView(bool followCaret)
    {
        followsCaret = followCaret;
        followedPresentation = view.Presentation;
        view.Changed += ViewChanged;
        view.Document.Changed += DocumentChanged;
        view.Disposed += Close;
    }
    private void DetachView()
    {
        CancelCaretFollow();
        view.Changed -= ViewChanged;
        view.Document.Changed -= DocumentChanged;
        view.Disposed -= Close;
    }
    private void CancelCaretFollow()
    {
        caretFollowTimer?.Stop();
        refreshAfterFollowing = false;
    }
    private void ViewChanged()
    {
        if (closed || !followsCaret || view.Id == 0) return;
        var next = view.Presentation;
        bool changed = SelectionChanged(followedPresentation, next);
        followedPresentation = next;
        if (!changed) return;
        caretFollowTimer ??= MakeCaretFollowTimer();
        caretFollowTimer.Stop();
        caretFollowTimer.Start();
    }
    private DispatcherQueueTimer MakeCaretFollowTimer()
    {
        var timer = DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(500);
        timer.IsRepeating = false;
        timer.Tick += (_, _) => {
            if (closed || !followsCaret || view.Id == 0) return;
            try
            {
                bool refresh = refreshAfterFollowing;
                refreshAfterFollowing = false;
                if (!CommitPendingName()) return;
                var latest = view.Styles();
                var key = CurrentCaretStyle(latest);
                var fallback = latest.Styles.FirstOrDefault(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0)?.Key;
                if (refresh || (key ?? fallback) != selected.Key) Load(key, snapshot: latest);
            }
            catch (Exception exception) { error.Text = exception.Message; error.Visibility = Microsoft.UI.Xaml.Visibility.Visible; }
        };
        return timer;
    }
    private StyleKey? CurrentCaretStyle(StyleSheet snapshot)
    {
#if DEBUG
        CaretStyleQueries++;
#endif
        try { return view.CurrentStyleEditorKey(snapshot); }
        catch (CoreException) { return null; }
    }
    private void DocumentChanged()
    {
        if (updating || closed) return;
        ViewChanged();
        if (caretFollowTimer?.IsRunning == true) { refreshAfterFollowing = true; return; }
        // Rebuilding focused controls can light-dismiss a native flyout. Keep
        // the inspector stable during picking and load fresh definitions on close.
        if (openColorPickers.Count > 0) { refreshAfterColorPopup = true; return; }
        Load(selected?.Key);
    }
    private bool CommitPendingName()
    {
        if (selected == null || name.IsReadOnly || name.Text == selected.Name) return true;
        Try(() => view.EditStyleString(selected, VIEM_STYLE_EDIT_SET_DISPLAY_NAME, 0, name.Text));
        return error.Visibility != Microsoft.UI.Xaml.Visibility.Visible;
    }
    private static bool SelectionChanged(ViemViewPresentationV1 previous, ViemViewPresentationV1 next)
    {
        if (previous.document_id != next.document_id || previous.cursor_utf8_offset != next.cursor_utf8_offset
            || previous.cursor_affinity != next.cursor_affinity) return true;
        const uint flags = VIEM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR | VIEM_VIEW_PRESENTATION_VISUAL_ANCHOR_AFFINITY_EXACT | VIEM_VIEW_PRESENTATION_HAS_VISUAL_BLOCK;
        if ((previous.flags & flags) != (next.flags & flags)) return true;
        if ((next.flags & VIEM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR) != 0)
        {
            if (previous.visual_anchor_utf8_offset != next.visual_anchor_utf8_offset) return true;
            if ((next.flags & VIEM_VIEW_PRESENTATION_VISUAL_ANCHOR_AFFINITY_EXACT) != 0 && previous.visual_anchor_affinity != next.visual_anchor_affinity) return true;
            if (previous.mode != VIEM_MODE_COMMAND_LINE && next.mode != VIEM_MODE_COMMAND_LINE && previous.mode != next.mode) return true;
        }
        return (next.flags & VIEM_VIEW_PRESENTATION_HAS_VISUAL_BLOCK) != 0
            && (previous.visual_block_left_x != next.visual_block_left_x || previous.visual_block_right_x != next.visual_block_right_x);
    }
#if DEBUG
    internal int CaretStyleQueries { get; private set; }
    internal int StyleLoads { get; private set; }
    internal bool CaretFollowScheduled => caretFollowTimer?.IsRunning == true;
#endif
}
