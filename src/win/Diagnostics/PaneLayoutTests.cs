#if DEBUG
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Interop;
using Viem.Windows.Shell;
using Windows.Foundation;
using Windows.System;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Diagnostics;

internal static class PaneLayoutTests
{
    private static void Check(bool condition, string message)
    {
        if (!condition) throw new InvalidOperationException(message);
        FrontendSmokeTests.UiChecks.Add(message);
    }
    internal static async Task Run(Preferences preferences)
    {
        bool showStatus = preferences.ShowStatus;
        preferences.Set("appearance", "showStatusBar", true);
        var window = new EditorWindow(preferences);
        App.Instance.Windows.Add(window); window.Activate();
        try
        {
            var first = window.ActivePane!; await first.Ready; await Task.Delay(150);
            var second = window.SplitPane(first, first.Document); await second.Ready;
            window.PaneStack.Equalize(); window.PaneStack.UpdateLayout();
            var third = window.SplitPane(second, first.Document); await third.Ready;
            window.PaneStack.Equalize(); window.PaneStack.UpdateLayout(); await Task.Delay(80);
            var view = first.View!;
            var baseStyle = view.Styles().Styles.Single(s => s.Namespace == VIEM_STYLE_NAMESPACE_BLOCK && s.Id == "Paragraph");
            view.EditStyle(baseStyle, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_SIZE, CoreView.Number(20));
            var spacing = New<ViemStyleEditValueV1>(); spacing.kind = VIEM_STYLE_VALUE_LINE_SPACING;
            spacing.enum_value = VIEM_STYLE_LINE_SPACING_MULTIPLIER; spacing.number = 1.5f;
            view.EditStyle(baseStyle, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING, spacing);
            Check(Math.Abs(view.DefaultLineHeight - 30) < .001, "pane units use Base Paragraph size times spacing");
            view.Zoom(2);
            Check(Math.Abs(view.DefaultLineHeight - 60) < .001, "pane units include view zoom");
            view.Zoom(1);
            view.Ex("resize 4"); await window.PendingEffectsForTesting; window.PaneStack.UpdateLayout();
            Check(Math.Abs(first.ActualHeight - first.StatusBarHeight - 120) < 1, "counted pane height excludes its status bar");
            window.PaneStack.Equalize(); window.PaneStack.UpdateLayout();
            double[] Bars() => window.Panes.Select(p => p.StatusControl.TransformToVisual(window.PaneStack).TransformPoint(new Point()).Y).ToArray();
            var initial = Bars();
            window.PaneStack.DragBar(first, 100_000);
            var bottom = Bars();
            Check(Math.Abs(bottom[0] + first.StatusBarHeight - bottom[1]) < .01
                && Math.Abs(bottom[1] + second.StatusBarHeight - bottom[2]) < .01,
                "drag collects touching bars at the bottom edge");
            window.PaneStack.DragBar(first, 100);
            window.PaneStack.DragBar(first, -12);
            var reverse = Bars();
            Check(Math.Abs(reverse[0] - bottom[0] + 12) < .01 && reverse[1] == bottom[1],
                "reversing after a blocked drag releases previously pushed bars");
            window.PaneStack.DragBar(second, -100_000);
            var top = Bars();
            Check(Math.Abs(top[0]) < .01 && Math.Abs(top[0] + first.StatusBarHeight - top[1]) < .01,
                "drag collects bars at the top edge");
            window.PaneStack.DragBar(second, 12);
            Check(Bars()[0] == top[0] && Math.Abs(Bars()[1] - top[1] - 12) < .01,
                "reversing at the top moves only the grabbed bar");
            var beforeBottom = Bars(); window.PaneStack.DragBar(third, -100);
            Check(Bars().SequenceEqual(beforeBottom), "the bottom status bar cannot move");
            bool rejected = false;
            try { window.SplitPane(first, first.Document); } catch (InvalidOperationException e) { rejected = e.Message.Contains("No room to split"); }
            Check(rejected && window.Panes.Count == 3, "splitting a collapsed editor fails without adding a pane");
            window.PaneStack.Equalize(); window.PaneStack.UpdateLayout();
            uint mode = view.CurrentLineMode;
            await InputRoutingTests.Drag(window, first.LocationToggleControl, [new(.5, .5), new(.5, .5)], _ => { });
            Check(view.CurrentLineMode != mode, "a plain eye click toggles line mode");
            mode = view.CurrentLineMode; double start = Bars()[0];
            await InputRoutingTests.Drag(window, first.LocationToggleControl,
                [new(.5, .5), new(.5, .5 + 40 / first.LocationToggleControl.ActualHeight)], _ => { });
            Check(view.CurrentLineMode == mode && Math.Abs(Bars()[0] - start - 40) < 2,
                "dragging the eye resizes panes and consumes its click");
            double fixedBottom = Bars()[2];
            await InputRoutingTests.Drag(window, third.StatusControl, [new(.3, .5), new(.3, -.5)], _ => { });
            Check(Bars()[2] == fixedBottom, "native pointer dragging cannot move the bottom status bar");
            view.Ex("only"); await window.PendingEffectsForTesting;
            window.FocusPane(first);
            ulong revision = first.Document.State.document_revision;
            view.Ex("vs"); await window.PendingEffectsForTesting;
            var right = window.ActivePane!; await right.Ready; window.PaneStack.UpdateLayout();
            Check(window.Panes.Count == 2 && right.Document == first.Document && window.PaneStack.MinimumSize.Width == 205,
                "vertical Ex split shares a buffer and enforces a 100-DIP minimum per pane");
            view.Ex("vertical 2resize 24"); await window.PendingEffectsForTesting; window.PaneStack.UpdateLayout();
            Check(Math.Abs(right.ActualWidth - Math.Max(100, right.View!.DefaultColumnWidth * 24)) < 1,
                "indexed vertical resize uses the target pane's default font and zoom");
            window.FocusPane(first);
            var splitter = window.PaneStack.Splitters.Single();
            Check(Math.Abs(splitter.ActualWidth - 5) < .01, "vertical splitter occupies five DIPs");
            double width = first.ActualWidth;
            await InputRoutingTests.Drag(window, splitter, [new(.5, .5), new(.5 + 40 / splitter.ActualWidth, .5)], _ => { });
            Check(Math.Abs(first.ActualWidth - width - 40) < 2 && window.ActivePane == first,
                "native vertical dragging moves the divider and preserves pane focus");
            window.PaneStack.ResizeWidth(right, 1);
            Check(Math.Abs(right.ActualWidth - 100) < .01, "width commands cannot shrink below 100 DIPs");
            rejected = false;
            try { window.SplitPane(right, first.Document, vertical: true); } catch (InvalidOperationException e) { rejected = e.Message.Contains("No room to split"); }
            Check(rejected && window.Panes.Count == 2, "a vertical split without 205 DIPs fails before adding a view");
            await InputRoutingTests.Drag(window, right.StatusControl, [new(.3, .5), new(.3, .5)], _ => { });
            Check(window.ActivePane == right, "a plain status-bar click focuses its buffer");
            right.View!.Ex("vnew"); await window.PendingEffectsForTesting;
            Check(window.Panes.Count == 2, "vnew obeys the same admission check before creating a document");
            Check(first.Document.State.document_revision == revision, "pane operations preserve the source revision");
        }
        finally
        {
            foreach (var pane in window.Panes.ToArray()) await window.ClosePane(pane, force: true);
            App.Instance.Windows.Remove(window);
            preferences.Set("appearance", "showStatusBar", showStatus);
        }
    }
}
#endif
