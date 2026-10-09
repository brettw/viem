using Microsoft.UI.Dispatching;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    private bool followsCaret;
    private bool followsSyntaxStyle;
    private bool refreshAfterFollowing;
    // This snapshot is only compared with the next presentation to detect a
    // change, never resolved as a retained editing position.
    private ViemViewPresentationV1 followedPresentation;
    private DispatcherQueueTimer? caretFollowTimer;

    private void AttachView(bool followCaret)
    {
        followsCaret = followCaret;
        followsSyntaxStyle = followCaret;
        followedPresentation = documentView.Presentation;
        documentView.Changed += ViewChanged;
        documentView.FormattingContextChanged += FormattingContextChanged;
        documentView.Document.Changed += DocumentChanged;
        documentView.Document.SyntaxChanged += SyntaxChanged;
        documentView.Disposed += SourceViewClosed;
    }
    private void DetachView()
    {
        CancelCaretFollow();
        documentView.Changed -= ViewChanged;
        documentView.FormattingContextChanged -= FormattingContextChanged;
        documentView.Document.Changed -= DocumentChanged;
        documentView.Document.SyntaxChanged -= SyntaxChanged;
        documentView.Disposed -= SourceViewClosed;
    }
    private void SourceViewClosed()
    {
        DetachView();
        DismissColorPickers(commit: false);
    }
    private void CancelCaretFollow()
    {
        caretFollowTimer?.Stop();
        refreshAfterFollowing = false;
        followsSyntaxStyle = false;
    }
    private void ViewChanged()
    {
        if (closed || !followsCaret || documentView.Id == 0) return;
        var next = documentView.Presentation;
        bool changed = SelectionChanged(followedPresentation, next);
        followedPresentation = next;
        if (!changed || updating) return;
        ScheduleCaretFollow();
    }
    private void FormattingContextChanged()
    {
        if (closed || !followsCaret || documentView.Id == 0 || updating) return;
        // Pending character choices can change without moving the caret or
        // publishing source. Definition edits do not issue this notification.
        ScheduleCaretFollow();
    }
    private void ScheduleCaretFollow()
    {
        // A command that moves the caret also ends its core style edit group.
        // Drop queued colors before following the new editing context.
        DismissColorPickers(commit: false);
        followsSyntaxStyle = true;
        caretFollowTimer ??= MakeCaretFollowTimer();
        caretFollowTimer.Stop();
        caretFollowTimer.Start();
    }
    private void SyntaxChanged()
    {
        if (closed || !followsCaret || !followsSyntaxStyle || documentView.Id == 0
            || documentView.Document.State.format != VIEM_FORMAT_CODE) return;
        // Complete the caret lookup when asynchronous captures arrive, while
        // retaining explicit choices and allowing an existing timer to settle.
        caretFollowTimer ??= MakeCaretFollowTimer();
        if (!caretFollowTimer.IsRunning) caretFollowTimer.Start();
    }
    private DispatcherQueueTimer MakeCaretFollowTimer()
    {
        var timer = DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(500);
        timer.IsRepeating = false;
        timer.Tick += (_, _) => {
            if (closed || !followsCaret || documentView.Id == 0) return;
            try
            {
                bool refresh = refreshAfterFollowing;
                refreshAfterFollowing = false;
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
        if (documentView.Id == 0) return null;
#if DEBUG
        CaretStyleQueries++;
#endif
        try {
            var key = documentView.CurrentStyleEditorKey(documentView.Styles());
            return key is { } selectedKey && snapshot.Styles.Any(style => style.Key == selectedKey) ? selectedKey : null;
        }
        catch (CoreException) { return null; }
    }
    private void DocumentChanged()
    {
        if (updating || closed || documentView.Id == 0) return;
        if (sessionFamily != Preferences.StyleFamily(documentView.Document.State.format)) {
            DismissColorPickers(commit: false);
            CancelCaretFollow();
            CreateThemeSession(documentView.Document.State.format);
            followsSyntaxStyle = followsCaret;
            followedPresentation = documentView.Presentation;
            Load(followCaret: followsCaret);
            return;
        }
        if (openColorPickers.Count > 0 && !view.Styles().Identity.Equals(sheet.Identity))
            DismissColorPickers(commit: false);
        ViewChanged();
        if (caretFollowTimer?.IsRunning == true) { refreshAfterFollowing = true; return; }
        // Rebuilding focused controls can light-dismiss a native flyout. Keep
        // the inspector stable during picking and load fresh definitions on close.
        if (openColorPickers.Count > 0) { refreshAfterColorPopup = true; return; }
        Load(selected?.Key);
    }
    private static bool SelectionChanged(ViemViewPresentationV1 previous, ViemViewPresentationV1 next)
    {
        if (previous.document_id != next.document_id || previous.cursor_utf8_offset != next.cursor_utf8_offset
            || previous.cursor_affinity != next.cursor_affinity) return true;
        if (previous.mode != next.mode
            && (previous.mode is VIEM_MODE_NORMAL or VIEM_MODE_INSERT or VIEM_MODE_REPLACE)
            && (next.mode is VIEM_MODE_NORMAL or VIEM_MODE_INSERT or VIEM_MODE_REPLACE)) return true;
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
