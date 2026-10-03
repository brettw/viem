#if DEBUG
using System.Text;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Shell;
using Windows.Foundation;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class TableInteractionTests
{
    private static void Check(bool condition, string name)
    { if (!condition) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    private static void Geometry()
    {
        var viewport = new Rect(0, 0, 600, 400);
        var table = new Rect(100, 100, 200, 90);
        var first = new Rect(100, 100, 100, 30);
        var second = new Rect(200, 100, 100, 30);
        var body = new Rect(100, 130, 100, 30);
        var last = new Rect(100, 160, 100, 30);
        Check(TableHoverGeometry.Hit(new(310, 145), body, table, viewport) == TableHoverKind.RightRow
            && TableHoverGeometry.Hit(new(90, 145), body, table, viewport) == TableHoverKind.LeftRow,
            "row hover mirrors the twenty-DIP gutter on both table sides");
        Check(TableHoverGeometry.Hit(new(320, 145), body, table, viewport) == TableHoverKind.RightRow
            && TableHoverGeometry.Hit(new(320.1, 145), body, table, viewport) == null,
            "right row hover ends at the twenty-DIP gutter boundary");
        Check(TableHoverGeometry.Hit(new(300, 100), first, table, viewport) == null
            && TableHoverGeometry.Hit(new(300, 100), second, table, viewport) == TableHoverKind.Column,
            "top-right corner targets the final column before the right row gutter");
        Check(TableHoverGeometry.Hit(new(310, 130), first, table, viewport) == null
            && TableHoverGeometry.Hit(new(310, 130), body, table, viewport) == TableHoverKind.RightRow
            && TableHoverGeometry.Hit(new(310, 190), first, table, viewport) == null
            && TableHoverGeometry.Hit(new(310, 190), last, table, viewport) == TableHoverKind.RightRow,
            "right row hover gives shared boundaries to the following row and the final edge to the last row");
        var clipped = new Rect(100, 100, 700, 90);
        Check(TableHoverGeometry.Hit(new(599, 145), body, clipped, viewport) == null
            && TableHoverGeometry.Hit(new(810, 145), body, clipped, viewport) == null,
            "offscreen table edges do not become viewport-edge row hotspots");
        var widget = new Size(90, 30);
        var right = TableHoverGeometry.Placement(TableHoverKind.RightRow, body, table, widget, new(600, 400));
        var left = TableHoverGeometry.Placement(TableHoverKind.LeftRow, body, table, widget, new(600, 400));
        Check(right.Left == table.Right + 4 && left.Right == table.Left - 4,
            "row toolbar first chooses the side that activated it");
        var nearRight = new Rect(350, 100, 200, 90);
        var flipped = TableHoverGeometry.Placement(TableHoverKind.RightRow, body, nearRight, widget, new(600, 400));
        Check(flipped.Right == nearRight.Left - 4,
            "right row toolbar flips left when that side fits without covering text");
        var wide = new Rect(0, 100, 590, 90);
        var clamped = TableHoverGeometry.Placement(TableHoverKind.RightRow, body, wide, widget, new(600, 400));
        Check(clamped.Left >= 0 && clamped.Right <= 600,
            "row toolbar remains inside the viewport when neither outside position fits");
        Check(TableHoverGeometry.RetentionPath(right, TableHoverGeometry.Activation(TableHoverKind.RightRow, body, table))
            .Contains(new(302, 145)), "right row toolbar retains the bridge from its activation strip");
    }

    internal static async Task Run(Preferences preferences)
    {
        Geometry();
        const string source = "| Head | Other |\n| --- | --- |\n| First | One |\n| Second | Two |";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        try {
            var pane = window.ActivePane!;
            var view = await pane.Ready;
            await Task.Delay(150);
            pane.Refresh();
            var layout = view.Layout();
            var viewport = view.Viewport;
            var selection = view.LogicalSelection();
            Button[] Buttons() => ((StackPanel)pane.VisibleTableWidgetForTest!.Child).Children.OfType<Button>().ToArray();
            string[] Names() => Buttons().Select(AutomationProperties.GetName).ToArray();
            Point Hover(ulong row, bool right) {
                var cell = layout.TableCells.First(cell => cell.row == row);
                return new(right ? cell.table_rect.x + cell.table_rect.width - viewport.left
                    : cell.table_rect.x - viewport.left, cell.rect.y + cell.rect.height / 2 - viewport.top);
            }
            pane.HoverTableForTest(Hover(0, true));
            var firstBody = layout.TableCells.First(cell => cell.row == 1);
            pane.HoverTableForTest(new(firstBody.table_rect.x + firstBody.table_rect.width - viewport.left,
                firstBody.rect.y - viewport.top));
            Check(Names().Contains("Delete row 2"),
                "moving along the right gutter retargets at the next row boundary outside the actual toolbar");
            pane.HoverTableForTest(new(pane.Canvas.ActualWidth - 1, pane.Canvas.ActualHeight - 1));
            pane.HoverTableForTest(Hover(1, false));
            var flippedWidget = pane.VisibleTableWidgetForTest!;
            var flippedButton = Buttons()[0];
            Check(flippedWidget.Margin.Left >= Hover(1, true).X,
                "the narrow left gutter places its row toolbar on the right");
            pane.HoverTableForTest(Hover(1, true));
            pane.HoverTableForTest(new(flippedWidget.Margin.Left - 1,
                flippedWidget.Margin.Top + flippedWidget.Height / 2));
            pane.HoverTableForTest(new(flippedWidget.Margin.Left + flippedWidget.Width / 2,
                flippedWidget.Margin.Top + flippedWidget.Height / 2));
            Check(ReferenceEquals(flippedButton, Buttons()[0]),
                "crossing the opposite gutter toward a flipped row toolbar preserves its buttons and target");
            for (ulong row = 0; row < 3; row++) {
                pane.HoverTableForTest(new(pane.Canvas.ActualWidth - 1, pane.Canvas.ActualHeight - 1));
                pane.HoverTableForTest(Hover(row, false));
                var expected = Names();
                pane.HoverTableForTest(Hover(row, true));
                Check(Names().SequenceEqual(expected) && Names().Contains(row == 0 ? "Delete header row" : $"Delete row {row + 1}"),
                    $"both row gutters expose identical actions for row {row + 1}");
                Check(row != 0 || !Names().Any(name => name.StartsWith("Insert row above", StringComparison.Ordinal)),
                    "header right row toolbar omits Insert row above");
            }
            pane.HoverTableForTest(Hover(1, true));
            var widget = pane.VisibleTableWidgetForTest!;
            var buttons = Buttons();
            pane.HoverTableForTest(new(widget.Margin.Left + widget.Width / 2, widget.Margin.Top + widget.Height / 2));
            Check(ReferenceEquals(buttons[0], Buttons()[0]), "moving from right gutter into the toolbar preserves its exact target");
            Check(CoreView.SameSelection(selection, view.LogicalSelection())
                && source == Encoding.UTF8.GetString(document.Source(document.State.document_revision)),
                "right row hover leaves caret selection and source unchanged");
            new ButtonAutomationPeer(Buttons().Single(button => AutomationProperties.GetName(button) == "Delete row 2")).Invoke();
            await Task.Delay(80);
            Check(!document.FormattedText().Contains("First", StringComparison.Ordinal)
                && document.FormattedText().Contains("Second", StringComparison.Ordinal),
                "right row toolbar action deletes its targeted row without retargeting to the caret");
            view.SetMarkdownSource(true); pane.Refresh();
            pane.HoverTableForTest(Hover(1, true));
            Check(pane.VisibleTableWidgetForTest == null, "Source view does not show right or left row widgets");
            if (pane.LastError != null) throw pane.LastError;
        }
        finally { App.Instance.Windows.Remove(window); window.Close(); }
    }
}
#endif
