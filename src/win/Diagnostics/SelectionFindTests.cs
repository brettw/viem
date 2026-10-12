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
        await CodeStyleMenus(preferences);
    }

    private static async Task CodeStyleMenus(Preferences preferences)
    {
        byte[] source = "// a comment\nlet value = 1;"u8.ToArray();
        var document = new CoreDocument(source, "menu-code-style.rs");
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        byte[]? defaults = null;
        StyleWindow? inspector = null;
        try
        {
            var pane = window.ActivePane!; var view = await pane.Ready;
            Check(view.UsesGlobalStyles, "Code style menu fixture uses the shared Code stylesheet");
            view.WaitForSyntax();
            _ = document.SyntaxStyleNames();
            defaults = view.ExportStyleDefaults();
            var before = view.Presentation;
            byte[] otherSource = "// another comment\nlet other = 2;"u8.ToArray();
            var otherDocument = new CoreDocument(otherSource, "other-menu-code-style.rs");
            var otherPane = window.AddPane(otherDocument); var otherView = await otherPane.Ready;
            otherView.WaitForSyntax(); window.FocusPane(pane);
            try
            {
                view.MaterializeCodeStyle("viem.menu.test.implicit");
                Check(view.Styles().Styles.Any(style => style.Name == "viem.menu.test.implicit"
                        && (style.Native.flags & VIEM_STYLE_DEFINITION_IMPLICIT) != 0)
                    && view.ExportStyleDefaults().AsSpan().SequenceEqual(defaults),
                    "opening an implicit syntax definition does not persist generated Code styles");
            }
            finally { CoreThemes.ReplaceCodeStyles(defaults); }
            var menu = window.Menu.Items.Single(item => item.Title == "Style");
            var peer = new MenuBarItemAutomationPeer(menu);
            var paragraph = menu.Items.OfType<MenuFlyoutSubItem>().Single(item => item.Text == "Paragraph");
            var character = menu.Items.OfType<MenuFlyoutSubItem>().Single(item => item.Text == "Character");
            await MenuTestHelpers.Transition(true, () => { peer.Expand(); return Task.CompletedTask; }, character);
            Check(paragraph.IsEnabled && character.IsEnabled,
                "Code views expose paragraph and character definition menus");
            Check(character.Focus(FocusState.Programmatic), "Code character styles accept native focus");
            await MenuTestHelpers.Transition(true, () => InputRoutingTests.Key(global::Windows.System.VirtualKey.Right),
                character.Items.OfType<FrameworkElement>().First());
            var comment = character.Items.OfType<ToggleMenuFlyoutItem>().Single(item => item.Text == "Comment");
            Check(comment.IsEnabled, "the Code Comment definition is selectable");
            var trackedRows = character.Items.ToArray();
            var trackedActions = trackedRows.OfType<ToggleMenuFlyoutItem>().Select(item => item.CommandParameter).ToArray();
            bool RetainsTrackedActions() => character.Items.OfType<ToggleMenuFlyoutItem>().Select(item => item.CommandParameter)
                .Zip(trackedActions, ReferenceEquals).All(same => same);
            const string addedName = "viem.menu.test.tracked";
            view.MaterializeCodeStyle(addedName);
            window.UpdateTitle(); window.ValidateMenusForTesting();
            Check(character.Items.SequenceEqual(trackedRows)
                && RetainsTrackedActions()
                && !character.Items.OfType<ToggleMenuFlyoutItem>().Any(item => item.Text == addedName),
                "a changed Code catalogue retains tracked native row and action identities until dismissal");
            await MenuTestHelpers.Transition(false, () => InputRoutingTests.Key(global::Windows.System.VirtualKey.Left), comment);
            Check(character.IsLoaded, "closing Character leaves its parent Style menu open");
            Check(character.Focus(FocusState.Programmatic), "the refreshed Code submenu accepts native focus");
            await MenuTestHelpers.Transition(true, () => InputRoutingTests.Key(global::Windows.System.VirtualKey.Right),
                character.Items.OfType<FrameworkElement>().First());
            Check(character.Items.OfType<ToggleMenuFlyoutItem>().Any(item => item.Text == addedName)
                && !character.Items.SequenceEqual(trackedRows),
                "reopening Character under the same Style menu refreshes the catalogue after tracked rows unload");
            comment = character.Items.OfType<ToggleMenuFlyoutItem>().Single(item => item.Text == "Comment");
            trackedRows = character.Items.ToArray();
            trackedActions = trackedRows.OfType<ToggleMenuFlyoutItem>().Select(item => item.CommandParameter).ToArray();
            window.FocusPane(otherPane, keepStatusFocus: true); window.ValidateMenusForTesting();
            Check(comment.IsLoaded && character.Items.SequenceEqual(trackedRows)
                && RetainsTrackedActions(),
                "switching active documents does not rebind an already-tracked Code menu action");
            await MenuTestHelpers.Transition(false, () => {
                new ToggleMenuFlyoutItemAutomationPeer(comment).Toggle(); peer.Collapse(); return Task.CompletedTask;
            }, character, comment);
            inspector = window.StyleInspector;
            Check(inspector == null && !otherDocument.IsDirty
                && otherDocument.Source(otherDocument.State.document_revision).AsSpan().SequenceEqual(otherSource),
                "a retained Code menu action rejects a changed active view without opening or editing another document");
            await MenuTestHelpers.Transition(true, () => { peer.Expand(); return Task.CompletedTask; }, character);
            Check(character.Focus(FocusState.Programmatic), "the refreshed Code submenu accepts native focus");
            await MenuTestHelpers.Transition(true, () => InputRoutingTests.Key(global::Windows.System.VirtualKey.Right),
                character.Items.OfType<FrameworkElement>().First());
            Check(!RetainsTrackedActions(),
                "reopening the Code menu binds actions to the new active document after tracked rows unload");
            comment = character.Items.OfType<ToggleMenuFlyoutItem>().Single(item => item.Text == "Comment");
            await MenuTestHelpers.Transition(false, () => {
                new ToggleMenuFlyoutItemAutomationPeer(comment).Toggle(); peer.Collapse(); return Task.CompletedTask;
            }, character, comment);
            inspector = window.StyleInspector;
            Check(inspector?.StylePicker.SelectedItem is StyleDefinition { Name: "Comment" }
                && inspector.ThemeView.UsesGlobalStyles,
                "choosing a Code character style opens that global definition in the inspector");
            Check(view.Presentation.mode == before.mode && view.Presentation.cursor_utf8_offset == before.cursor_utf8_offset
                && document.Source(document.State.document_revision).AsSpan().SequenceEqual(source) && !document.IsDirty,
                "Code style menu inspection preserves document mode, caret, source bytes and clean state");
        }
        finally {
            inspector?.Close();
            App.Instance.Windows.Remove(window); window.Close();
            if (defaults != null) CoreThemes.ReplaceCodeStyles(defaults);
        }
    }
}
#endif
