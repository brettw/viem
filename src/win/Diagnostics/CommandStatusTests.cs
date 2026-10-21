#if DEBUG
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Shell;
using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class CommandStatusTests
{
    internal static async Task Run(EditorPane pane, EditorWindow window, Preferences preferences)
    {
        void Check(bool value, string name)
        { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
        CheckRejectedPhysicalAddress(pane, Check);
        await StatusFilePathTests.Run(pane, window, preferences, Check);
        var view = pane.View!;
        bool showStatus = preferences.ShowStatus;
        double height = pane.StatusControl.ActualHeight;
        ulong revision = pane.Document.State.document_revision;
        try
        {
            preferences.Set("appearance", "showStatusBar", false);
            await InputRoutingTests.Settle();
            pane.FocusEditor();
            await InputRoutingTests.Text(":");
            Check(await SelfTestRunner.Until(() => pane.PromptControl.LastDrawnText == ":"
                && pane.StatusControl.Visibility == Visibility.Visible),
                "the first colon draws in a previously hidden status bar");
            foreach (char character in "pwd")
            {
                await InputRoutingTests.Text(character.ToString());
                Check(await SelfTestRunner.Until(() => pane.PromptControl.LastDrawnText == ":" + view.Prompt().Text),
                    $"native Ex typing paints the {character} update");
            }
            var promptBounds = pane.PromptControl.TransformToVisual(pane.StatusControl).TransformBounds(
                new(0, 0, pane.PromptControl.ActualWidth, pane.PromptControl.ActualHeight));
            // WinUI snaps border margins to physical pixel edges. At 150% a
            // one-DIP top border occupies 1 1/3 DIPs in the arranged controls.
            double halfPixel = .5 / pane.XamlRoot.RasterizationScale + .01;
            Check(pane.PromptControl.ActualWidth > 0
                && pane.StatusControl.ActualHeight == height
                && Math.Abs(promptBounds.Top - pane.PromptControl.Margin.Top) <= halfPixel
                && Math.Abs(promptBounds.Bottom - (height - pane.PromptControl.Margin.Bottom)) <= halfPixel
                && pane.LocationControl.ActualWidth > 0,
                $"command entry occupies the status row and retains the location widget (prompt={pane.PromptControl.ActualWidth}x{pane.PromptControl.ActualHeight}, margins={pane.PromptControl.Margin}, status={pane.StatusControl.ActualHeight}, prior={height}, location={pane.LocationControl.ActualWidth}, error={pane.LastError})");
            await InputRoutingTests.Key(VirtualKey.Enter);
            Check(pane.CommandOutputText == Environment.CurrentDirectory && pane.OutputTextControl.Text == Environment.CurrentDirectory
                && pane.StatusControl.Visibility == Visibility.Visible, ":pwd displays its result in the status bar");
            Check(pane.OutputTextControl.IsTextSelectionEnabled && pane.StatusControl.ActualHeight == height,
                "output is selectable and preserves the status height");
            pane.Refresh(); await InputRoutingTests.Settle();
            Check(pane.CommandOutputText == Environment.CurrentDirectory, "ordinary presentation refresh preserves command output");
            pane.OutputTextControl.Focus(FocusState.Programmatic); pane.SelectAll();
            Check(pane.OutputTextControl.SelectedText == Environment.CurrentDirectory && pane.CanCopy && !pane.CanCut,
                "output selection supports copy and select-all without enabling cut");
            var edit = window.Menu.Items.Single(item => item.Title == "Edit");
            var editPeer = new MenuBarItemAutomationPeer(edit);
            editPeer.Expand(); await InputRoutingTests.Settle();
            Check(edit.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Copy").IsEnabled
                && !edit.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Cut").IsEnabled,
                $"Edit menu commands retain the read-only output selection as their target (copy={pane.CanCopy}/{edit.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Copy").IsEnabled}, cut={pane.CanCut}/{edit.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Cut").IsEnabled}, selected={pane.OutputTextControl.SelectedText}, focus={FocusManager.GetFocusedElement(pane.XamlRoot)?.GetType().Name}, active={window.IsWindowActive})");
            editPeer.Collapse(); await InputRoutingTests.Settle();
            pane.OutputTextControl.Focus(FocusState.Programmatic);
            new ButtonAutomationPeer(pane.OutputCloseControl).Invoke(); await InputRoutingTests.Settle();
            Check(pane.CommandOutputText == null && pane.StatusControl.Visibility == Visibility.Collapsed
                && FocusManager.GetFocusedElement(pane.XamlRoot) is TextBox,
                "closing focused output restores editor focus and the hidden-status preference");

            await UnicodePromptAndCaret(pane, Check);
            await LongOutputScrolling(pane, height, Check);
            Check(pane.Document.State.document_revision == revision, "prompt and output interaction never edits document text");
        }
        finally { preferences.Set("appearance", "showStatusBar", showStatus); pane.DismissCommandOutput(); }
        if (pane.LastError != null) throw new InvalidOperationException("Command status fixture has a presentation error.", pane.LastError);
    }

    private static void CheckRejectedPhysicalAddress(EditorPane pane, Action<bool, string> check)
    {
        byte[] source = "first\ncontinuation\n\nlast"u8.ToArray();
        using var document = new CoreDocument(source, format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 150, 200);
        view.LineMode(VIEM_LINE_MODE_VISUAL);
        view.GoToLine(2);
        var before = view.Presentation;
        int publishedEffects = 0;
        view.Effects += _ => publishedEffects++;
        foreach (bool sourced in new[] { false, true })
        {
            InvalidOperationException? rejected = null;
            try
            {
                if (sourced) view.ExecuteCommandLine("999p");
                else view.Ex("999p");
            }
            catch (InvalidOperationException error) { rejected = error; }
            string entry = sourced ? "Command line" : "Ex entry";
            check(rejected?.Message == "line address 999 is outside 1..=4"
                && view.Outcome.command_status == VIEM_COMMAND_STATUS_ERROR,
                $"{entry} throws the precise physical line address diagnostic");
            check(publishedEffects == 0, $"{entry} does not dispatch effects from a rejected command");
            var after = view.Presentation;
            check(document.State.document_revision == before.document_revision
                && document.Source(before.document_revision).AsSpan().SequenceEqual(source)
                && after.cursor_utf8_offset == before.cursor_utf8_offset
                && after.cursor_affinity == before.cursor_affinity,
                $"{entry} preserves source and caret after a rejected physical address");
        }
    }

    internal static async Task RunScrolled(EditorPane pane)
    {
        var view = await pane.Ready;
        await InputRoutingTests.Settle();
        pane.FocusEditor();
        view.LineMode(VIEM_LINE_MODE_VISUAL);
        view.Scroll(0, 50_000);
        await InputRoutingTests.Settle();
        var layout = view.Layout();
        ulong cursor = view.Presentation.cursor_utf8_offset;
        if (layout.Rows.Any(row => row.text_start <= cursor && cursor <= row.text_end))
            throw new InvalidOperationException("The large-document fixture did not scroll the cursor outside regional geometry.");
        if (pane.LastError != null) throw new InvalidOperationException("Scrolled status fixture has a presentation error before Ex input.", pane.LastError);
        await InputRoutingTests.Text(":pwd");
        if (pane.PromptControl.LastDrawnText != ":pwd") throw new InvalidOperationException("The scrolled document hid Ex typing.");
        await InputRoutingTests.Key(VirtualKey.Enter);
        if (pane.CommandOutputText != Environment.CurrentDirectory || pane.LastError != null)
            throw new InvalidOperationException("The scrolled document hid command output.", pane.LastError);
        FrontendSmokeTests.UiChecks.Add("Ex typing and output work with the cursor outside a large document's regional layout");
    }

    private static async Task UnicodePromptAndCaret(EditorPane pane, Action<bool, string> Check)
    {
        await InputRoutingTests.Text(":set wrap?");
        await InputRoutingTests.Key(VirtualKey.Enter);
        Check(pane.CommandOutputText == "wrap", ":set queries use status output without a modal dialog");
        await InputRoutingTests.Text(":e café 日本語");
        Check(pane.PromptControl.LastDrawnText == ":e café 日本語", "command prompt draws Unicode text");
        await InputRoutingTests.Key(VirtualKey.Back);
        Check(pane.PromptControl.LastDrawnText == ":e café 日本", "command prompt redraws native Backspace");
        await InputRoutingTests.Text(new string('x', 240));
        Check(pane.CommandOutputText == null && pane.PromptControl.ScrollOffset > 0,
            "new commands dismiss old output and long prompts scroll to the caret");
        await InputRoutingTests.Key(VirtualKey.Escape);
        Check(pane.StatusControl.Visibility == Visibility.Collapsed, "Escape restores the hidden-status preference");
    }

    private static async Task LongOutputScrolling(EditorPane pane, double height, Action<bool, string> Check)
    {
        pane.SetMessage(string.Join("\n", Enumerable.Range(1, 20).Select(i => $"line {i}: " + new string('x', 240))));
        Check(pane.CommandOutputText?.Length > 4000, "long output replaces the previous result");
        // ScrollViewer measures its extent on a later WinUI layout pass.
        await SelfTestRunner.Until(() => pane.OutputScrollControl.ScrollableHeight > 0);
        Check(pane.StatusControl.ActualHeight == height && pane.OutputScrollControl.ScrollableHeight > 0
            && pane.OutputScrollControl.ScrollableWidth > 0, "long multiline output scrolls inside one status row");
        pane.OutputScrollControl.ChangeView(100, 20, null, true);
        await SelfTestRunner.Until(() => pane.OutputScrollControl.HorizontalOffset > 0
            && pane.OutputScrollControl.VerticalOffset > 0);
        Check(pane.OutputScrollControl.HorizontalOffset > 0 && pane.OutputScrollControl.VerticalOffset > 0,
            "output scrolling exposes clipped lines and columns");
        // Expiration must not take focus from another pane/control.
        var otherControl = new Button();
        pane.Children.Add(otherControl); otherControl.Focus(FocusState.Programmatic);
        pane.ExpireCommandOutput(long.MaxValue);
        Check(ReferenceEquals(FocusManager.GetFocusedElement(pane.XamlRoot), otherControl)
            && pane.StatusControl.Visibility == Visibility.Collapsed, "expiring output preserves unrelated control focus");
        pane.Children.Remove(otherControl); pane.FocusEditor();
    }
}
#endif
