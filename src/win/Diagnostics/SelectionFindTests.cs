#if DEBUG
using System.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class SelectionFindTests
{
    private static void Check(bool value, string name)
    { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }

    internal static async Task Run(Preferences preferences)
    {
        byte[] source = Encoding.UTF8.GetBytes("a.c abc a.c\n" + string.Concat(Enumerable.Repeat("wrapped word ", 160))
            + "\n" + string.Join("\n", Enumerable.Repeat("additional row", 100)));
        var document = new CoreDocument(source, format: VIEM_FORMAT_PLAIN_TEXT);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        try
        {
            var pane = window.ActivePane!; var view = await pane.Ready;
            view.BackgroundLayout.Enabled = false;
            var edit = window.Menu.Items.Single(menu => menu.Title == "Edit");
            var peer = new MenuBarItemAutomationPeer(edit);
            var find = edit.Items.OfType<MenuFlyoutSubItem>().Single(menu => menu.Text == "Find");
            var select = edit.Items.OfType<MenuFlyoutSubItem>().Single(menu => menu.Text == "Select");
            FrameworkElement? openItem = null;
            MenuFlyoutItem FindItem(string label) => find.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == label);
            async Task Open(MenuFlyoutSubItem submenu)
            {
                await MenuTestHelpers.Transition(true, () => { peer.Expand(); return Task.CompletedTask; }, submenu);
                Check(submenu.Focus(FocusState.Programmatic), "selection menu accepts native focus");
                openItem = submenu.Items.OfType<FrameworkElement>().First();
                await MenuTestHelpers.Transition(true, () => InputRoutingTests.Key(global::Windows.System.VirtualKey.Right), openItem);
            }
            Task Close(Action? action = null) => MenuTestHelpers.Transition(false, () => {
                action?.Invoke(); peer.Collapse(); return Task.CompletedTask;
            }, openItem == null ? [find] : [find, openItem]);
            async Task Invoke(MenuFlyoutSubItem submenu, MenuFlyoutItem item)
            {
                await Open(submenu);
                Check(item.IsEnabled, item.Text + " is enabled for the current selection");
                await Close(() => new MenuFlyoutItemAutomationPeer(item).Invoke());
                Check(pane.LastError == null, item.Text + " completes without a native routing error");
            }

            await Open(find);
            Check(!FindItem("Use Selection for Find").IsEnabled && !FindItem("Jump to Selection").IsEnabled,
                "selection Find actions are disabled without a selection");
            await Close();
            var empty = window.AddPane(new CoreDocument([], format: VIEM_FORMAT_PLAIN_TEXT));
            var emptyView = await empty.Ready;
            emptyView.Command("v");
            await Open(find);
            Check(emptyView.HasSelection && !FindItem("Use Selection for Find").IsEnabled
                && FindItem("Jump to Selection").IsEnabled,
                "an empty Visual selection can be revealed but cannot become a Find pattern");
            await Close();
            await window.ClosePane(empty, true);
            view.Command("v2l");
            var selected = view.LogicalSelection();
            await Invoke(find, FindItem("Use Selection for Find"));
            Check(view.HasSelection && view.LogicalSelection().text_start == selected.text_start
                && view.LogicalSelection().text_end == selected.text_end && view.Presentation.cursor_utf8_offset == 2,
                "Use Selection for Find retains the exact selection and caret");
            var stale = view.Selection().Info.identity;
            view.Command("h");
            bool rejected = false;
            try { view.UseSelectionForFind(stale); }
            catch (CoreException error) when (error.Status == VIEM_STATUS_STALE_REVISION) { rejected = true; }
            Check(rejected && view.Presentation.cursor_utf8_offset == 1,
                "a changed selection cannot replace the accepted find pattern through a stale ABI identity");
            view.Key(VIEM_KEY_ESCAPE); view.Command("0n");
            Check(view.Presentation.cursor_utf8_offset == 8,
                "native selected search treats punctuation literally and preserves the accepted pattern after rejection");
            await InputRoutingTests.Settle();
            for (int i = 0; i < 100 && (view.Presentation.flags & VIEM_VIEW_PRESENTATION_HAS_SEARCH_COUNT) == 0; i++)
                await Task.Delay(10);
            pane.Refresh();
            Check(view.Presentation.search_match_current == 2 && view.Presentation.search_match_total == 2
                && pane.LocationControl.Text.EndsWith("[2/2]"),
                "shared search counts cross the ABI and appear in the native location status");

            view.Command("ggv2l"); view.Scroll(0, 600);
            Check(view.Viewport.top > 100, "the selected text can be scrolled outside the viewport");
            await Invoke(find, FindItem("Jump to Selection"));
            Check(view.Viewport.top < 100 && view.HasSelection && view.Presentation.cursor_utf8_offset == 2,
                "Jump to Selection reveals the retained active endpoint without changing the selection");

            view.Key(VIEM_KEY_ESCAPE); view.Wrap(true); view.Command("j0");
            var row = view.Layout().Rows.First(value => value.text_start == 12);
            Check(row.text_end < row.hard_line_end, "Visual Row fixture wraps within one physical line");
            await Invoke(select, select.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Visual Row"));
            var rowSelection = view.LogicalSelection();
            Check(rowSelection.text_start == row.text_start && rowSelection.text_end == row.text_end,
                "Select Visual Row selects only the displayed row on a wrapped physical line");
            Check(document.Source(document.State.document_revision).AsSpan().SequenceEqual(source) && !document.IsDirty,
                "native find, reveal and visual-row selection preserve exact source bytes and clean state");
        }
        finally { App.Instance.Windows.Remove(window); window.Close(); }
    }
}
#endif
