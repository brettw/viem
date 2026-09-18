#if DEBUG
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
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
        async Task Screenshot(string name) => await WindowCapture.Save(window.Hwnd, pane.Canvas.Device, FrontendSmokeTests.ReportPath + name + ".png");
        var view = pane.View!;
        bool showStatus = preferences.ShowStatus;
        double height = pane.StatusControl.ActualHeight;
        ulong revision = pane.Document.State.document_revision;
        try
        {
            preferences.Set("appearance", "showStatusBar", false);
            await Task.Delay(80);
            pane.FocusEditor();
            await InputRoutingTests.Text(":");
            Check(pane.PromptControl.LastDrawnText == ":" && pane.StatusControl.Visibility == Visibility.Visible,
                "the first colon draws in a previously hidden status bar");
            foreach (char character in "pwd")
            {
                await InputRoutingTests.Text(character.ToString());
                Check(pane.PromptControl.LastDrawnText == ":" + view.Prompt().Text, $"native Ex typing paints the {character} update");
            }
            Check(pane.PromptControl.ActualWidth > 0 && pane.PromptControl.ActualHeight == height
                && pane.LocationControl.ActualWidth > 0, "command entry occupies the status row and retains the location widget");
            await Screenshot(".command-pwd");
            await InputRoutingTests.Key(VirtualKey.Enter);
            Check(pane.CommandOutputText == Environment.CurrentDirectory && pane.OutputTextControl.Text == Environment.CurrentDirectory
                && pane.StatusControl.Visibility == Visibility.Visible, ":pwd displays its result in the status bar");
            Check(pane.OutputTextControl.IsTextSelectionEnabled && pane.StatusControl.ActualHeight == height,
                "output is selectable and preserves the status height");
            pane.Refresh(); await Task.Delay(50);
            await Screenshot(".command-output");
            Check(pane.CommandOutputText == Environment.CurrentDirectory, "ordinary presentation refresh preserves command output");
            pane.OutputTextControl.Focus(FocusState.Programmatic); pane.SelectAll();
            Check(pane.OutputTextControl.SelectedText == Environment.CurrentDirectory && pane.CanCopy && !pane.CanCut,
                "output selection supports copy and select-all without enabling cut");
            var edit = window.Menu.Items.Single(item => item.Title == "Edit");
            var editPeer = new MenuBarItemAutomationPeer(edit);
            editPeer.Expand(); await Task.Delay(100);
            Check(edit.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Copy").IsEnabled
                && !edit.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Cut").IsEnabled,
                "Edit menu commands retain the read-only output selection as their target");
            editPeer.Collapse(); await Task.Delay(80);
            pane.OutputTextControl.Focus(FocusState.Programmatic);
            new ButtonAutomationPeer(pane.OutputCloseControl).Invoke(); await Task.Delay(80);
            Check(pane.CommandOutputText == null && pane.StatusControl.Visibility == Visibility.Collapsed
                && FocusManager.GetFocusedElement(pane.XamlRoot) is TextBox,
                "closing focused output restores editor focus and the hidden-status preference");

            await InputRoutingTests.Text(":set wrap?");
            await InputRoutingTests.Key(VirtualKey.Enter);
            Check(pane.CommandOutputText == "wrap=1", ":set queries use status output without a modal dialog");
            await InputRoutingTests.Text(":e café 日本語");
            Check(pane.PromptControl.LastDrawnText == ":e café 日本語", "command prompt draws Unicode text");
            await InputRoutingTests.Key(VirtualKey.Back);
            Check(pane.PromptControl.LastDrawnText == ":e café 日本", "command prompt redraws native Backspace");
            await InputRoutingTests.Text(new string('x', 240));
            Check(pane.CommandOutputText == null && pane.PromptControl.ScrollOffset > 0,
                "new commands dismiss old output and long prompts scroll to the caret");
            await InputRoutingTests.Key(VirtualKey.Escape);
            Check(pane.StatusControl.Visibility == Visibility.Collapsed, "Escape restores the hidden-status preference");

            pane.SetMessage(string.Join("\n", Enumerable.Range(1, 20).Select(i => $"line {i}: " + new string('x', 240))));
            Check(pane.CommandOutputText?.Length > 4000, "long output replaces the previous result");
            // ScrollViewer measures its extent on a later WinUI layout pass.
            for (int i = 0; i < 20 && pane.OutputScrollControl.ScrollableHeight == 0; i++) await Task.Delay(100);
            await Screenshot(".command-output-long");
            Check(pane.StatusControl.ActualHeight == height && pane.OutputScrollControl.ScrollableHeight > 0
                && pane.OutputScrollControl.ScrollableWidth > 0, "long multiline output scrolls inside one status row");
            pane.OutputScrollControl.ChangeView(100, 20, null, true); await Task.Delay(80);
            Check(pane.OutputScrollControl.HorizontalOffset > 0 && pane.OutputScrollControl.VerticalOffset > 0,
                "output scrolling exposes clipped lines and columns");
            // Expiration must not take focus from another pane/control.
            var otherControl = new Button();
            pane.Children.Add(otherControl); otherControl.Focus(FocusState.Programmatic);
            pane.ExpireCommandOutput(long.MaxValue);
            Check(ReferenceEquals(FocusManager.GetFocusedElement(pane.XamlRoot), otherControl)
                && pane.StatusControl.Visibility == Visibility.Collapsed, "expiring output preserves unrelated control focus");
            pane.Children.Remove(otherControl); pane.FocusEditor();
            Check(pane.Document.State.document_revision == revision, "prompt and output interaction never edits document text");
        }
        finally { preferences.Set("appearance", "showStatusBar", showStatus); pane.DismissCommandOutput(); }
        if (pane.LastError != null) throw pane.LastError;
    }

    internal static async Task RunScrolled(EditorPane pane)
    {
        var view = await pane.Ready;
        await Task.Delay(100);
        pane.FocusEditor();
        view.LineMode(VIEM_LINE_MODE_VISUAL);
        view.Scroll(0, 50_000);
        await Task.Delay(100);
        var layout = view.Layout();
        ulong cursor = view.Presentation.cursor_utf8_offset;
        if (layout.Rows.Any(row => row.text_start <= cursor && cursor <= row.text_end))
            throw new InvalidOperationException("The large-document fixture did not scroll the cursor outside regional geometry.");
        if (pane.LastError != null) throw pane.LastError;
        await InputRoutingTests.Text(":pwd");
        if (pane.PromptControl.LastDrawnText != ":pwd") throw new InvalidOperationException("The scrolled document hid Ex typing.");
        await InputRoutingTests.Key(VirtualKey.Enter);
        if (pane.CommandOutputText != Environment.CurrentDirectory || pane.LastError != null)
            throw new InvalidOperationException("The scrolled document hid command output.", pane.LastError);
        FrontendSmokeTests.UiChecks.Add("Ex typing and output work with the cursor outside a large document's regional layout");
    }
}
#endif
