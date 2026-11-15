#if DEBUG
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class BackgroundInteractionTests
{
    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
        FrontendSmokeTests.UiChecks.Add(message);
    }

    internal static async Task Run(Preferences preferences)
    {
        string source = string.Join("\n", Enumerable.Range(0, 150).Select(i => $"Line {i}: ordinary text"));
        await using var editor = await TestEditor.Open(preferences, source, VIEM_FORMAT_PLAIN_TEXT);
        var pane = editor.Pane;
        var view = editor.View;
        await InputRoutingTests.CheckForegroundEditor(editor.Window, pane, "background interaction fixture owns editor focus");
        pane.SetDragScrollForTesting(new(80, pane.Canvas.ActualHeight + 30));
        Check(pane.DragScrollScheduledForTesting, "an out-of-bounds text drag schedules autoscroll");
        float top = view.Viewport.top;
        Check(await SelfTestRunner.Until(() => view.Viewport.top > top),
            "a stationary text drag scrolls without another pointer event");
        pane.SetDragScrollForTesting(new(80, 30));
        Check(!pane.DragScrollScheduledForTesting, "drag reentry stops autoscroll immediately");

        pane.SetDragScrollForTesting(new(80, pane.Canvas.ActualHeight + 30));
        pane.IsActive = false;
        Check(!pane.DragScrollScheduledForTesting && !pane.PointerSelectionActiveForTesting,
            "pane deactivation cancels the selection gesture before capture loss");
        pane.IsActive = true;
        Check(!pane.DragScrollScheduledForTesting, "pane reactivation does not restart an obsolete drag");

        pane.SetDragScrollForTesting(new(80, pane.Canvas.ActualHeight + 30));
        await using (var other = await TestEditor.Open(preferences, "Other window", VIEM_FORMAT_PLAIN_TEXT)) {
            Check(await SelfTestRunner.Until(() => !editor.Window.IsWindowActive),
                "a second editor window deactivates the drag owner");
            Check(!pane.DragScrollScheduledForTesting && !pane.PointerSelectionActiveForTesting,
                "window deactivation cancels autoscroll without another pointer event");
            Check(!pane.CaretBlinkScheduledForTesting, "the inactive editor window stops caret blinking");
        }
        editor.Window.Activate(); editor.Window.FocusPane(pane);
        await InputRoutingTests.CheckForegroundEditor(editor.Window, pane, "reactivated interaction fixture owns editor focus");
        int callbacks = pane.DragScrollCallbacksForTesting;
        await Task.Delay(200);
        Check(!pane.DragScrollScheduledForTesting && pane.DragScrollCallbacksForTesting == callbacks,
            "reactivation leaves a retired drag idle");
        Check(editor.Source == source, "background selection scrolling preserves source bytes");

        string table = "| Left | Middle | Right |\n| --- | --- | --- |\n"
            + string.Join("\n", Enumerable.Range(0, 100).Select(i => $"| Row {i} | Value | Text |"));
        await editor.Reset(table);
        pane = editor.Pane; view = editor.View;
        await InputRoutingTests.CheckForegroundEditor(editor.Window, pane, "table drag fixture owns editor focus");
        pane.Refresh();
        var header = view.Layout().TableCells.First(cell => cell.row == 0 && cell.column == 0);
        var viewport = view.Viewport;
        Check(pane.BeginTableColumnSelectionForTest(new(header.rect.x + header.rect.width / 2 - viewport.left,
            header.table_rect.y - 1 - viewport.top)), "a column strip starts the table drag fixture");
        view.Scroll(0, 0); pane.Refresh();
        top = view.Viewport.top;
        pane.SetDragScrollForTesting(new(header.rect.x + header.rect.width / 2, pane.Canvas.ActualHeight + 30));
        try {
            Check(await SelfTestRunner.Until(() => view.Viewport.top > top),
                "a stationary table-column drag advances the viewport");
            var selection = view.TableSelection();
            Check(selection.active != 0 && selection.anchor_column == 0 && selection.active_column == 0
                && selection.anchor_row == 0 && selection.active_row == 100,
                "table-column autoscroll preserves complete column selection instead of placing a text caret");
        }
        finally { pane.CancelPointerSelectionForTesting(); }
        callbacks = pane.DragScrollCallbacksForTesting;
        await Task.Delay(200);
        Check(!pane.DragScrollScheduledForTesting && pane.DragScrollCallbacksForTesting == callbacks,
            "ending table-column autoscroll retires its timer");
        Check(pane.LastError == null && editor.Source == table, "background table selection preserves source without native errors");
    }
}
#endif
