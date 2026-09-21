#if DEBUG
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Editor;
using Viem.Windows.Input;
using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class SelectionInputTests
{
    private static void Check(bool value, string name)
    { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
    internal static async Task Run(EditorPane pane)
    {
        var view = pane.View!;
        var window = App.Instance.Windows.Single(w => w.Panes.Contains(pane));
        const string original = "alpha beta\nsecond line\nthird";
        async Task Start(uint offset = 0)
        {
            await InputRoutingTests.Key(VirtualKey.Escape);
            view.Place(offset, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, pane.Document.State.document_revision);
            pane.FocusEditor();
        }
        async Task SelectTwo()
        {
            await Start();
            await InputRoutingTests.Key(VirtualKey.Right, shift: true);
            await InputRoutingTests.Key(VirtualKey.Right, shift: true);
        }
        try
        {
            view.Ex("set autoselect keymodel= selectmode=");
            pane.FocusEditor();
            await InputRoutingTests.Text("ialpha beta"); await InputRoutingTests.Key(VirtualKey.Enter);
            await InputRoutingTests.Text("second line"); await InputRoutingTests.Key(VirtualKey.Enter);
            await InputRoutingTests.Text("third");
            await Start();
            await SelectTwo();
            var selection = view.LogicalSelection();
            Check(view.Presentation.mode == VIEM_MODE_SELECTION_CHARACTER && selection.text_start == 0 && selection.text_end == 2,
                "native Shift+Right selects exactly the traversed characters");
            Check(pane.ModeControl.Text == "SELECTION" && view.Presentation.caret_shape == VIEM_CARET_SHAPE_BOUNDARY
                && pane.CanCopy && pane.CanCut && view.Selection().Rectangles.Length > 0,
                "Select mode has a native status label, thin caret, selection paint and clipboard actions");
            await InputRoutingTests.Text("Q👩‍💻");
            Check(pane.Document.FormattedText() == "Q👩‍💻pha beta\nsecond line\nthird" && view.Presentation.mode == VIEM_MODE_INSERT,
                "native Unicode text replaces Select mode as one committed text input");
            await InputRoutingTests.Key(VirtualKey.Z, control: true);
            Check(pane.Document.FormattedText() == original, "one native undo restores selection replacement and subsequent text");

            await Start(); await InputRoutingTests.Key(VirtualKey.Right, control: true, shift: true);
            selection = view.LogicalSelection();
            Check(selection.text_start == 0 && selection.text_end == 6, "native Ctrl+Shift+Right extends by a whole word");
            await InputRoutingTests.Key(VirtualKey.Left);
            Check(!view.HasSelection, "unshifted navigation ends selection when keymodel includes stopsel");
            await Start(3); await InputRoutingTests.Key(VirtualKey.Home, shift: true);
            selection = view.LogicalSelection();
            Check(selection.text_start == 0 && selection.text_end == 3, "native Shift+Home extends to the line start");
            await Start(3); await InputRoutingTests.Key(VirtualKey.End, shift: true);
            selection = view.LogicalSelection();
            Check(selection.text_start == 3 && selection.text_end == 10, "native Shift+End extends through the line end");
            await Start(12); await InputRoutingTests.Key(VirtualKey.Home, control: true, shift: true);
            selection = view.LogicalSelection();
            Check(selection.text_start == 0 && selection.text_end == 12, "native Ctrl+Shift+Home selects to the document start");
            await Start(3); await InputRoutingTests.Key(VirtualKey.End, control: true, shift: true);
            selection = view.LogicalSelection();
            Check(selection.text_start == 3 && selection.text_end == (ulong)original.Length, "native Ctrl+Shift+End selects through EOF");
            await Start(); await InputRoutingTests.Key(VirtualKey.Down, shift: true);
            Check(view.IsTextSelection && view.LogicalSelection().text_end > 10, "native Shift+Down extends to the next row");
            await Start(); await InputRoutingTests.Text("i");
            await InputRoutingTests.Key(VirtualKey.Right, shift: true); await InputRoutingTests.Key(VirtualKey.Left);
            Check(view.Presentation.mode == VIEM_MODE_INSERT, "ending a native selection returns to its originating Insert mode");
            await InputRoutingTests.Text("Z"); await InputRoutingTests.Key(VirtualKey.Z, control: true);
            Check(pane.Document.FormattedText() == original, "typing after a collapsed Insert selection retains normal undo grouping");

            await SelectTwo(); await InputRoutingTests.Key(VirtualKey.C, control: true);
            var nativeCopy = await ClipboardFormats.Read();
            Check(nativeCopy.Text == "al" && nativeCopy.Fragment.Length > 0 && pane.Document.FormattedText() == original
                && view.IsTextSelection && view.LogicalSelection().text_start == 0 && view.LogicalSelection().text_end == 2,
                "native Copy writes plain and private system clipboard formats while retaining the exact Select range");
            await Start(); view.Command("\"*yy");
            var linewiseCopy = await ClipboardFormats.Read();
            Check(linewiseCopy.Text == "alpha beta\n", "linewise Vim yank exposes its terminating newline on the Windows clipboard");
            view.Command("p");
            Check(pane.Document.FormattedText() == "alpha beta\nalpha beta\nsecond line\nthird",
                "bare put after a Windows clipboard yank retains the unnamed linewise register");
            view.Undo(); await Start();
            view.ClipboardText = linewiseCopy.Text; view.ClipboardFragment = linewiseCopy.Fragment; view.ClipboardGeneration++;
            view.Command("j\"*p");
            Check(pane.Document.FormattedText() == "alpha beta\nsecond line\nalpha beta\nthird",
                "Windows clipboard round trip retains linewise Vim put semantics");
            view.Undo();
            await SelectTwo(); await InputRoutingTests.Key(VirtualKey.X, control: true);
            Check(pane.Document.FormattedText() == "pha beta\nsecond line\nthird", "native Cut deletes a Select selection");
            view.Undo(); await SelectTwo(); ClipboardFormats.Write("PASTE", "");
            await InputRoutingTests.Key(VirtualKey.V, control: true);
            Check(pane.Document.FormattedText() == "PASTEpha beta\nsecond line\nthird", "native Paste replaces a Select selection");
            view.Undo(); await SelectTwo();
            var edit = window.Menu.Items.Single(item => item.Title == "Edit");
            var peer = new MenuBarItemAutomationPeer(edit); peer.Expand(); await Task.Delay(60);
            var delete = edit.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Delete");
            Check(delete.IsEnabled, "Edit Delete is enabled for Select mode");
            new MenuFlyoutItemAutomationPeer(delete).Invoke(); await Task.Delay(80);
            Check(pane.Document.FormattedText() == "pha beta\nsecond line\nthird" && view.Presentation.mode == VIEM_MODE_INSERT,
                "Edit Delete removes Select text and enters Insert like the physical Delete key");
            pane.FocusEditor(); await InputRoutingTests.Text("Q");
            Check(pane.Document.FormattedText() == "Qpha beta\nsecond line\nthird", "typing continues after Edit Delete");
            view.Undo();
            Check(pane.Document.FormattedText() == original, "one undo restores Edit Delete and subsequent typing");

            await Start(); await InputRoutingTests.Text("vl");
            peer.Expand(); await Task.Delay(60);
            new MenuFlyoutItemAutomationPeer(delete).Invoke(); await Task.Delay(80);
            Check(pane.Document.FormattedText() == "pha beta\nsecond line\nthird" && view.Presentation.mode == VIEM_MODE_NORMAL,
                "Edit Delete retains Visual mode's delete command behavior");
            view.Undo();

            await Start(); window.Activate();
            var layout = view.Layout(); var viewport = view.Viewport;
            var first = layout.Clusters.First(c => c.text_start == 0); var last = layout.Clusters.First(c => c.text_start == 4);
            double y = (layout.Rows.First().baseline - viewport.top - 3) / pane.Canvas.ActualHeight;
            var points = new[] {
                new global::Windows.Foundation.Point((first.x + first.advance * .1 - viewport.left) / pane.Canvas.ActualWidth, y),
                new global::Windows.Foundation.Point((last.x + last.advance * .9 - viewport.left) / pane.Canvas.ActualWidth, y)
            };
            await InputRoutingTests.Drag(window, pane.Canvas, points, _ => { });
            Check(view.IsTextSelection && pane.CanCopy && view.Selection().Rectangles.Length > 0,
                "native pointer dragging uses selectmode=mouse and paints an actionable selection");

            view.Ex("set noautoselect keymodel=startsel selectmode=key");
            await SelectTwo(); await InputRoutingTests.Key(VirtualKey.Right);
            Check(view.IsTextSelection, "omitting stopsel keeps unshifted navigation inside the selection");
            view.Ex("set keymodel= selectmode=");
            await Start(); await InputRoutingTests.Key(VirtualKey.Right, shift: true);
            Check(!view.HasSelection, "empty keymodel preserves Vim navigation without starting a selection");
            view.Ex("set keymodel=startsel,stopsel selectmode=");
            await SelectTwo(); Check(view.IsVisual && !view.IsTextSelection, "empty selectmode starts Visual mode from Shift navigation");
            view.Ex("set selectmode=cmd"); await Start(); await InputRoutingTests.Text("v");
            Check(view.IsTextSelection, "selectmode=cmd makes the v command enter Select mode");
            await InputRoutingTests.Key(VirtualKey.G, control: true);
            Check(view.IsVisual, "Ctrl+G switches Select mode back to Visual mode");
            await InputRoutingTests.Key(VirtualKey.G, control: true);
            Check(view.IsTextSelection, "Ctrl+G switches Visual mode back to Select mode");
            await Start(); await InputRoutingTests.Text("V");
            Check(view.Presentation.mode == VIEM_MODE_SELECT_LINE && pane.ModeControl.Text == "S-LINE" && pane.CanCopy,
                "Select Line mode exposes its selection and status to the Windows frontend");
            var lineSelection = view.LogicalSelection(); await InputRoutingTests.Key(VirtualKey.C, control: true);
            Check(view.Presentation.mode == VIEM_MODE_SELECT_LINE && view.LogicalSelection().Equals(lineSelection),
                "native Copy retains Select Line extent and mode");
            await Start(); await InputRoutingTests.Key(VirtualKey.Q, control: true);
            Check(view.Presentation.mode == VIEM_MODE_SELECT_BLOCK && pane.ModeControl.Text == "S-BLOCK" && pane.CanCopy,
                "Select Block mode keeps the Windows Ctrl+Q entry and clipboard availability");
            var blockSelection = view.LogicalSelection(); await InputRoutingTests.Key(VirtualKey.C, control: true);
            Check(view.Presentation.mode == VIEM_MODE_SELECT_BLOCK && view.LogicalSelection().Equals(blockSelection),
                "native Copy retains the exact Select Block geometry and mode");
            bool compositionRejected = false;
            try { view.BeginComposition(); } catch (InvalidOperationException) { compositionRejected = true; }
            Check(compositionRejected && !view.Composing && pane.Document.FormattedText() == original,
                "unsupported rectangular IME composition cannot delete the selection's bounding text");
            Check(pane.Document.FormattedText() == original && pane.LastError == null, "selection options and native navigation preserve source bytes");
        }
        finally
        {
            view.Key(VIEM_KEY_ESCAPE); view.Ex("set autoselect& keymodel& selectmode&"); view.Ex("%d"); pane.FocusEditor();
        }
    }
}
#endif
