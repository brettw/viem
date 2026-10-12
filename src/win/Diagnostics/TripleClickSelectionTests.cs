#if DEBUG
using Viem.Windows.Editor;
using Windows.Foundation;
using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class TripleClickSelectionTests
{
    internal static async Task Run(EditorPane pane)
    {
        void Check(bool value, string name)
        { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
        var view = await pane.Ready;
        var window = App.Instance.Windows.Single(w => w.Panes.Contains(pane));
        uint lineMode = view.CurrentLineMode;
        const string line = "first words continue across several wrapped rows of text ";
        string source = string.Concat(Enumerable.Repeat(line, 24)) + "\nlast";
        view.Key(VIEM_KEY_ESCAPE); view.Ex("set autoselect keymodel= selectmode=");
        view.Command("i"); view.Text(source); view.Key(VIEM_KEY_ESCAPE);
        view.Wrap(true); view.Command("gg");
        try {
            window.Activate(); pane.FocusEditor();
            foreach (uint policy in new[] { VIEM_LINE_MODE_VISUAL, VIEM_LINE_MODE_PHYSICAL_SOURCE })
            foreach (var (command, returnMode) in new[] { ("", VIEM_MODE_NORMAL), ("i", VIEM_MODE_INSERT), ("R", VIEM_MODE_REPLACE) }) {
                view.Key(VIEM_KEY_ESCAPE); view.Command("gg"); view.LineMode(policy); view.Command(command);
                var rows = view.Layout().Rows;
                var first = rows[0];
                Check(first.text_end < (ulong)(source.Length - 5), "triple-click fixture spans multiple visual rows");
                var cluster = view.Layout().Clusters.First(c => c.text_start == first.text_start);
                var point = new Point((cluster.x + cluster.advance * .1 - view.Viewport.left) / pane.Canvas.ActualWidth,
                    (first.baseline - view.Viewport.top - 3) / pane.Canvas.ActualHeight);
                await Task.Delay(TimeSpan.FromMilliseconds(InputRoutingTests.GetDoubleClickTime() + 50));
                await InputRoutingTests.Drag(window, pane.Canvas, new[] { point }, _ => { }, clickCount: 3);
                var selected = view.LogicalSelection();
                ulong expectedEnd = policy == VIEM_LINE_MODE_VISUAL ? first.text_end : (ulong)(source.Length - 4);
                Check(selected.text_start == 0 && selected.text_end == expectedEnd
                    && view.Presentation.mode == VIEM_MODE_SELECTION_LINE,
                    $"native triple-click from {returnMode} selects the active {policy} line domain");
                await InputRoutingTests.Key(VirtualKey.Right);
                Check(view.Presentation.mode == returnMode && !view.HasSelection,
                    $"collapsing triple-click selection restores originating mode {returnMode}");
                Check(pane.Document.FormattedText() == source, "triple-click and selection collapse preserve exact text");
            }
            view.Key(VIEM_KEY_ESCAPE); view.Command("gg"); view.LineMode(VIEM_LINE_MODE_PHYSICAL_SOURCE);
            var firstRow = view.Layout().Rows[0];
            var firstCluster = view.Layout().Clusters.First(c => c.text_start == 0);
            var fraction = new Point((firstCluster.x + firstCluster.advance * .1 - view.Viewport.left) / pane.Canvas.ActualWidth,
                (firstRow.baseline - view.Viewport.top - 3) / pane.Canvas.ActualHeight);
            await Task.Delay(TimeSpan.FromMilliseconds(InputRoutingTests.GetDoubleClickTime() + 50));
            await InputRoutingTests.Drag(window, pane.Canvas, new[] { fraction }, _ => { }, clickCount: 3);
            await InputRoutingTests.Key(VirtualKey.Delete);
            Check(pane.Document.FormattedText() == "last", "native Delete consumes the complete triple-clicked physical line");
            view.Undo();
            Check(pane.Document.FormattedText() == source, "undo restores the triple-clicked line deletion exactly");
            if (pane.LastError is { } error) throw error;
        } finally {
            view.Key(VIEM_KEY_ESCAPE); view.Ex("%d"); view.LineMode(lineMode); pane.FocusEditor();
        }
    }
}
#endif
