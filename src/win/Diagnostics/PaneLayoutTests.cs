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
            var positionOrigin = first.LocationToggleControl.TransformToVisual(first.StatusControl).TransformPoint(new Point());
            var empty = new Point(positionOrigin.X - 2, first.StatusControl.ActualHeight / 2);
            Check(first.StatusResizeHoverAt(empty) && !third.StatusResizeHoverAt(empty),
                "only movable status backgrounds show the resize hover cursor");
            var fileOrigin = first.FilePathControl.TransformToVisual(first.StatusControl).TransformPoint(new Point());
            Check(!first.StatusResizeHoverAt(new(fileOrigin.X + 2, fileOrigin.Y + first.FilePathControl.ActualHeight / 2))
                && !first.StatusResizeHoverAt(new(positionOrigin.X + 4, first.StatusControl.ActualHeight / 2)),
                "filename and position controls retain their usual hover cursors");
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
            await InputRoutingTests.Drag(window, first.LocationToggleControl, (global::Windows.Foundation.Point[])[new(.5, .5), new(.5, .5)], _ => { });
            Check(view.CurrentLineMode != mode, "a plain eye click toggles line mode");
            mode = view.CurrentLineMode; double start = Bars()[0];
            await InputRoutingTests.Drag(window, first.LocationToggleControl,
                (global::Windows.Foundation.Point[])[new(.5, .5), new(.5, .5 + 40 / first.LocationToggleControl.ActualHeight)], _ => { });
            Check(view.CurrentLineMode == mode && Math.Abs(Bars()[0] - start - 40) < 2,
                $"dragging the eye resizes panes and consumes its click (mode={mode}->{view.CurrentLineMode}, bar={start}->{Bars()[0]}, focusFirst={window.ActivePane == first}, status={first.StatusBarHeight}, canDrag={window.PaneStack.CanDragBar(first)})");
            window.FocusPane(third);
            var backgroundPoint = new Point(first.LocationToggleControl.TransformToVisual(first.StatusControl).TransformPoint(new Point()).X - 2,
                first.StatusControl.ActualHeight / 2);
            Check(first.StatusResizeHoverAt(backgroundPoint), "the movable status drag fixture targets empty background");
            double backgroundStart = Bars()[0];
            double backgroundX = backgroundPoint.X / first.StatusControl.ActualWidth;
            await InputRoutingTests.Drag(window, first.StatusControl,
                (global::Windows.Foundation.Point[])[new(backgroundX, .5), new(backgroundX, .5 + 40 / first.StatusControl.ActualHeight)], _ => { }, focusTarget: false);
            Check(Math.Abs(Bars()[0] - backgroundStart - 40) < 2 && window.ActivePane == third,
                $"dragging movable status background resizes without changing focus (bar={backgroundStart}->{Bars()[0]}, focusThird={window.ActivePane == third})");
            await InputRoutingTests.Drag(window, first.StatusControl,
                (global::Windows.Foundation.Point[])[new(backgroundX, .5)], _ => { }, focusTarget: false);
            Check(window.ActivePane == first, "a plain movable-status background click focuses its buffer");
            double fixedBottom = Bars()[2];
            await InputRoutingTests.Drag(window, third.StatusControl, (global::Windows.Foundation.Point[])[new(.3, .5), new(.3, -.5)], _ => { });
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
            // The portable frame reserves exactly five DIPs. WinUI rounds the
            // arranged control to a physical pixel at fractional DPI.
            double halfPixel = .5 / first.XamlRoot.RasterizationScale + .01;
            Check(window.PaneStack.SplitterWidthForTesting(splitter) == 5 && Math.Abs(splitter.ActualWidth - 5) <= halfPixel,
                $"vertical splitter occupies five DIPs rounded to native pixels (width={splitter.ActualWidth}, scale={first.XamlRoot.RasterizationScale})");
            Check(splitter.HidesTopBorder && splitter.HidesBottomBorder,
                "a full-height vertical splitter clips only its outer top and bottom borders");
            double width = first.ActualWidth;
            await InputRoutingTests.Drag(window, splitter, (global::Windows.Foundation.Point[])[new(.5, .5), new(.5 + 40 / splitter.ActualWidth, .5)], _ => { });
            Check(Math.Abs(first.ActualWidth - width - 40) < 2 && window.ActivePane == first,
                $"native vertical dragging moves the divider and preserves pane focus (width={width}->{first.ActualWidth}, right={right.ActualWidth}, focusFirst={window.ActivePane == first})");
            window.PaneStack.ResizeWidth(right, 1);
            Check(Math.Abs(right.ActualWidth - 100) < .01, "width commands cannot shrink below 100 DIPs");
            rejected = false;
            try { window.SplitPane(right, first.Document, vertical: true); } catch (InvalidOperationException e) { rejected = e.Message.Contains("No room to split"); }
            Check(rejected && window.Panes.Count == 2, "a vertical split without 205 DIPs fails before adding a view");
            await InputRoutingTests.Drag(window, right.StatusControl, (global::Windows.Foundation.Point[])[new(.3, .5), new(.3, .5)], _ => { });
            Check(window.ActivePane == right, "a plain status-bar click focuses its buffer");
            right.View!.Ex("vnew"); await window.PendingEffectsForTesting;
            Check(window.Panes.Count == 2, "vnew obeys the same admission check before creating a document");
            Check(first.Document.State.document_revision == revision, "pane operations preserve the source revision");
            view.Ex("only"); await window.PendingEffectsForTesting;
            var middle = window.SplitPane(first, first.Document); await middle.Ready;
            var bottomPane = window.SplitPane(middle, first.Document); await bottomPane.Ready;
            window.PaneStack.Equalize(); window.PaneStack.UpdateLayout();
            var bottomRight = window.SplitPane(bottomPane, first.Document, vertical: true); await bottomRight.Ready;
            window.PaneStack.UpdateLayout();
            var beneathStatus = window.PaneStack.Splitters.Single();
            Check(beneathStatus.HidesTopBorder && beneathStatus.HidesBottomBorder,
                "a vertical splitter below a full horizontal status bar omits its redundant top cap");
            var translucentCap = global::Windows.UI.Color.FromArgb(128, 0, 0, 0);
            var capPaint = ChromeStyle.Fallback(translucentCap, Microsoft.UI.Colors.Transparent) with {
                Border = new(1, 1, 1, 1),
                LeftBorder = translucentCap, TopBorder = translucentCap, RightBorder = translucentCap, BottomBorder = translucentCap
            };
            using (var target = new Microsoft.Graphics.Canvas.CanvasRenderTarget(first.Canvas.Device, 5, 20, 96))
            {
                using (var drawing = target.CreateDrawingSession()) {
                    drawing.Clear(Microsoft.UI.Colors.Transparent);
                    beneathStatus.DrawBorders(drawing, capPaint, new(5, 20));
                }
                Check(target.GetPixelBytes(2, 0, 1, 1)[3] == 0
                    && Math.Abs(target.GetPixelBytes(0, 0, 1, 1)[3] - 128) <= 1,
                    "the status junction removes only the top cap and retains the splitter side border");
            }
            var rightPanes = new List<EditorPane>();
            foreach (var row in new[] { first, middle })
            {
                var nested = window.SplitPane(row, first.Document, vertical: true); await nested.Ready;
                rightPanes.Add(nested);
            }
            rightPanes.Add(bottomRight);
            window.PaneStack.UpdateLayout();
            var nestedSplitters = window.PaneStack.Splitters.OrderBy(s => s.TransformToVisual(window.PaneStack).TransformPoint(new Point()).Y).ToArray();
            Check(nestedSplitters.Length == 3
                && nestedSplitters.Select(s => s.HidesTopBorder).SequenceEqual(new[] { true, false, false })
                && nestedSplitters.Select(s => s.HidesBottomBorder).SequenceEqual(new[] { false, false, true }),
                "nested vertical splitters show both interior borders and clip only true outer window edges");
            Check(nestedSplitters[2] == beneathStatus && !beneathStatus.HidesTopBorder,
                "a reused splitter restores its top cap when the preceding status bar is split at the junction");
            foreach (float borderWidth in new[] { 1f, 4f })
            {
                var translucent = global::Windows.UI.Color.FromArgb(128, 0, 0, 0);
                var paint = ChromeStyle.Fallback(translucent, Microsoft.UI.Colors.Transparent) with {
                    Border = new(borderWidth, borderWidth, borderWidth, borderWidth),
                    LeftBorder = translucent, TopBorder = translucent, RightBorder = translucent, BottomBorder = translucent
                };
                using var target = new Microsoft.Graphics.Canvas.CanvasRenderTarget(first.Canvas.Device, 5, 20, 96);
                using (var drawing = target.CreateDrawingSession()) {
                    drawing.Clear(Microsoft.UI.Colors.Transparent);
                    nestedSplitters[1].DrawBorders(drawing, paint, new(5, 20));
                }
                foreach (int x in new[] { 0, 2, 4 }) foreach (int y in new[] { 0, 19 })
                    Check(Math.Abs(target.GetPixelBytes(x, y, 1, 1)[3] - 128) <= 1,
                        "translucent splitter corners paint once even when side borders exceed its width");
            }
            foreach (var pane in window.Panes.Where(p => p != first && p != rightPanes[0]).ToArray())
                await window.ClosePane(pane, force: true);
            window.PaneStack.UpdateLayout();
            var survivor = window.PaneStack.Splitters.Single();
            Check(survivor == nestedSplitters[0] && survivor.HidesTopBorder && survivor.HidesBottomBorder,
                "closing neighboring rows updates border clipping on the surviving vertical splitter");
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
