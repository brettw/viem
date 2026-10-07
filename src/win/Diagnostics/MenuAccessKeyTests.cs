#if DEBUG
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using Windows.System;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class MenuAccessKeyTests
{
    private static void Check(bool condition, string name, string? details = null)
    { if (!condition) throw new InvalidOperationException(details == null ? name : $"{name} ({details})"); FrontendSmokeTests.UiChecks.Add(name); }

    private static string Label(MenuFlyoutItemBase item) => item switch
    {
        ToggleMenuFlyoutItem toggle => toggle.Text,
        MenuFlyoutItem command => command.Text,
        MenuFlyoutSubItem submenu => submenu.Text,
        _ => ""
    };

    private static void CheckScope(string title, IEnumerable<MenuFlyoutItemBase> items)
    {
        var commands = items.Where(item => item is not MenuFlyoutSeparator).ToArray();
        var keys = commands.Select(item => item.AccessKey).Where(key => key.Length != 0).ToArray();
        Check(keys.Distinct(StringComparer.OrdinalIgnoreCase).Count() == keys.Length
            && !keys.Any(key => keys.Any(other => key.Length < other.Length && other.StartsWith(key, StringComparison.OrdinalIgnoreCase))),
            $"{title} menu access keys do not collide within their scope");
        bool Fixed(MenuFlyoutItemBase item) => title switch
        {
            "Open Recent" => Label(item) == "Clear Menu",
            "Theme" => Label(item) is "Default" or "New theme…" or "Theme Settings…",
            "Code" => Label(item).StartsWith("Auto (", StringComparison.Ordinal),
            _ => item.Tag is not StyleKey
        };
        Check(commands.Where(Fixed).All(item => item.AccessKey.Length != 0),
            $"{title} fixed menu commands have access keys");
        foreach (var submenu in commands.OfType<MenuFlyoutSubItem>()) CheckScope(submenu.Text, submenu.Items);
    }

    internal static async Task Run(Preferences preferences)
    {
        bool originalShowMenu = preferences.ShowMenu;
        var document = new CoreDocument("alpha\r\nbeta"u8.ToArray(), format: VIEM_FORMAT_PLAIN_TEXT, fileFormat: 2);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!;
        await pane.Ready;
        int opened = 0;
        window.OpenDialogForTesting = () => { opened++; return Task.CompletedTask; };
        try
        {
            preferences.Set("windows", "showMenu", true);
            foreach (var menu in window.Menu.Items) CheckScope(menu.Title, menu.Items);
            var file = window.Menu.Items.Single(menu => menu.Title == "File");
            var open = file.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Open…");
            bool fileKeytipShown = false;
            file.AccessKeyDisplayRequested += (_, _) => fileKeytipShown = true;
            bool openKeytipShown = false;
            open.AccessKeyDisplayRequested += (_, _) => openKeytipShown = true;
            pane.FocusEditor(); await Task.Delay(150);
            ulong revision = document.State.document_revision;
            await InputRoutingTests.Key(VirtualKey.F, alt: true);
            string afterFile = $"open loaded={open.IsLoaded}, keytip={openKeytipShown}, focus={FocusManager.GetFocusedElement(pane.XamlRoot)?.GetType().Name}";
            await InputRoutingTests.Key(VirtualKey.O);
            Check(opened == 1, "native Alt+F then O invokes File Open exactly once", $"{afterFile}, opened={opened}, text={document.FormattedText()}");
            Check(openKeytipShown, "File menu displays a native keytip for Open");
            Check(window.Menu.Visibility == Visibility.Visible && preferences.ShowMenu,
                "invoking a menu command retains a persistently shown menu bar");
            Check(document.State.document_revision == revision && document.FormattedText() == "alpha\nbeta",
                "menu access keys do not enter document text or change its revision");

            pane.FocusEditor(); await Task.Delay(80);
            var endings = file.Items.OfType<MenuFlyoutSubItem>().Single(item => item.Text == "Line Endings");
            var unix = endings.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Unix (LF)");
            bool unixKeytipShown = false;
            unix.AccessKeyDisplayRequested += (_, _) => unixKeytipShown = true;
            int unixInvoked = 0;
            unix.Click += (_, _) => unixInvoked++;
            int fileInvoked = 0, endingsInvoked = 0;
            file.AccessKeyInvoked += (_, _) => fileInvoked++;
            endings.AccessKeyInvoked += (_, _) => endingsInvoked++;
            await InputRoutingTests.Key(VirtualKey.F, alt: true);
            await InputRoutingTests.Key((VirtualKey)endings.AccessKey[0]);
            string afterSubmenu = $"File invoked={fileInvoked}, submenu invoked={endingsInvoked}, unix loaded={unix.IsLoaded}, keytip={unixKeytipShown}, focus={FocusManager.GetFocusedElement(pane.XamlRoot)?.GetType().Name}";
            await InputRoutingTests.Key((VirtualKey)unix.AccessKey[0]);
            Check(document.Source(document.State.document_revision).AsSpan().SequenceEqual("alpha\nbeta"u8),
                "native access keys enter a submenu and invoke its scoped command", $"{afterSubmenu}, invoked={unixInvoked}, error={pane.LastError}");
            pane.View!.Undo();
            Check(document.Source(document.State.document_revision).AsSpan().SequenceEqual("alpha\r\nbeta"u8),
                "access-key menu edits preserve the normal undo transaction");

            pane.FocusEditor(); await Task.Delay(80);
            openKeytipShown = false;
            await InputRoutingTests.Key(VirtualKey.Menu);
            await InputRoutingTests.Key(VirtualKey.F);
            await InputRoutingTests.Key(VirtualKey.O);
            Check(opened == 2 && openKeytipShown,
                "native Alt tap then F then O displays menu keytips and invokes Open");

            pane.FocusEditor(); await Task.Delay(80);
            await InputRoutingTests.Text("i");
            await InputRoutingTests.Key(VirtualKey.O);
            Check(document.FormattedText() == "oalpha\nbeta" && opened == 2,
                "bare O still types into the editor outside menu access-key mode");
            await InputRoutingTests.Key(VirtualKey.Escape);
            pane.View.Undo();

            preferences.Set("windows", "showMenu", false);
            pane.FocusEditor(); await Task.Delay(80);
            Check(window.Menu.Visibility == Visibility.Collapsed, "hiding the menu bar collapses it before access-key activation");
            revision = document.State.document_revision;
            openKeytipShown = false;
            await InputRoutingTests.Key(VirtualKey.F, alt: true);
            Check(window.Menu.Visibility == Visibility.Visible && openKeytipShown,
                "Alt+F temporarily reveals a hidden menu bar and opens File with keytips");
            Check(!preferences.ShowMenu, "temporarily revealing the menu bar preserves the hidden preference");
            await InputRoutingTests.Key(VirtualKey.O);
            Check(opened == 3, "Alt+F then O invokes Open exactly once while the menu bar is hidden");
            Check(window.Menu.Visibility == Visibility.Collapsed,
                "invoking a command hides the temporarily revealed menu bar");

            pane.FocusEditor(); await Task.Delay(80);
            fileKeytipShown = false;
            await InputRoutingTests.Key(VirtualKey.Menu);
            Check(window.Menu.Visibility == Visibility.Visible && fileKeytipShown,
                "tapping Alt reveals a hidden menu bar with native top-level keytips");
            await InputRoutingTests.Key(VirtualKey.F);
            await InputRoutingTests.Key(VirtualKey.O);
            Check(opened == 4 && window.Menu.Visibility == Visibility.Collapsed,
                "Alt tap then F then O invokes Open and restores hidden menu visibility");

            pane.FocusEditor(); await Task.Delay(80);
            await InputRoutingTests.Key(VirtualKey.Menu);
            await InputRoutingTests.Key(VirtualKey.Escape);
            Check(window.Menu.Visibility == Visibility.Collapsed && !preferences.ShowMenu,
                "Escape dismisses temporary top-level keytips and restores hidden menu visibility");
            await InputRoutingTests.Key(VirtualKey.F, alt: true);
            await InputRoutingTests.Key(VirtualKey.Escape);
            Check(window.Menu.Visibility == Visibility.Collapsed && !preferences.ShowMenu,
                "Escape dismisses a temporarily opened File menu and restores hidden menu visibility",
                $"open loaded={open.IsLoaded}, keytips={AccessKeyManager.IsDisplayModeEnabled}, focus={FocusManager.GetFocusedElement(pane.XamlRoot)?.GetType().Name}");

            pane.FocusEditor(); await Task.Delay(80);
            await InputRoutingTests.Key(VirtualKey.F, control: true, alt: true);
            Check(window.Menu.Visibility == Visibility.Collapsed && opened == 4,
                "Ctrl+Alt input does not reveal the hidden menu bar or invoke a menu access key");
            Check(document.State.document_revision == revision && document.FormattedText() == "alpha\nbeta",
                "temporary menu activation and dismissal preserve document text and revision");

            pane.FocusEditor(); await Task.Delay(80);
            var edit = window.Menu.Items.Single(menu => menu.Title == "Edit");
            var editCommand = edit.Items.OfType<MenuFlyoutItem>().First();
            await InputRoutingTests.Key(VirtualKey.F, alt: true);
            await InputRoutingTests.Key(VirtualKey.Right);
            Check(window.Menu.Visibility == Visibility.Visible && editCommand.IsLoaded && !open.IsLoaded,
                "Right arrow switches a temporary File menu to Edit without hiding the menu bar");
            await InputRoutingTests.Key(VirtualKey.Escape);
            Check(window.Menu.Visibility == Visibility.Collapsed,
                "Escape hides a temporary menu after moving between top-level menus");
            await InputRoutingTests.Text("i");
            await InputRoutingTests.Text("q");
            Check(document.FormattedText() == "qalpha\nbeta" && opened == 4,
                "dismissing a temporary menu restores editor input without explicitly focusing the editor");
            await InputRoutingTests.Key(VirtualKey.Escape);
            pane.View.Undo();

            await InputRoutingTests.Key(VirtualKey.F, alt: true);
            await InputRoutingTests.Key((VirtualKey)endings.AccessKey[0]);
            await InputRoutingTests.Key(VirtualKey.Escape);
            Check(window.Menu.Visibility == Visibility.Visible && open.IsLoaded && !unix.IsLoaded,
                "Escape from a temporary submenu returns to its parent and keeps the menu bar visible");
            await InputRoutingTests.Key(VirtualKey.Escape);
            Check(window.Menu.Visibility == Visibility.Collapsed && !preferences.ShowMenu,
                "a second Escape dismisses the parent menu and restores the hidden preference");

            await InputRoutingTests.Key(VirtualKey.F, alt: true);
            var otherWindow = new Window { Title = "Viem menu activation test", Content = new TextBox() };
            try
            {
                otherWindow.Activate(); await Task.Delay(150);
                Check(!window.IsWindowActive && window.Menu.Visibility == Visibility.Collapsed && !preferences.ShowMenu,
                    "deactivating a window dismisses its temporary menu without changing the hidden preference");
            }
            finally
            {
                otherWindow.Close(); window.Activate(); await Task.Delay(150);
            }
            await InputRoutingTests.Text("i");
            await InputRoutingTests.Text("z");
            Check(document.FormattedText() == "zalpha\nbeta",
                "keyboard reactivation restores the editor focus held before the temporary menu");
            await InputRoutingTests.Key(VirtualKey.Escape);
            pane.View.Undo();
            openKeytipShown = false;
            string afterReactivate = $"active={window.IsWindowActive}, open loaded={open.IsLoaded}, keytips={AccessKeyManager.IsDisplayModeEnabled}, focus={FocusManager.GetFocusedElement(pane.XamlRoot)?.GetType().Name}";
            await InputRoutingTests.Key(VirtualKey.F, alt: true);
            Check(window.IsWindowActive && window.Menu.Visibility == Visibility.Visible && openKeytipShown,
                "reactivating a window allows its hidden menu to be opened again with Alt+F",
                $"before: {afterReactivate}; after: visible={window.Menu.Visibility}, open loaded={open.IsLoaded}, keytips={AccessKeyManager.IsDisplayModeEnabled}, focus={FocusManager.GetFocusedElement(pane.XamlRoot)?.GetType().Name}");
            await InputRoutingTests.Key(VirtualKey.Escape);

            pane.FocusEditor(); await Task.Delay(80);
            unixKeytipShown = false;
            await InputRoutingTests.Key(VirtualKey.F, alt: true);
            await InputRoutingTests.Key((VirtualKey)endings.AccessKey[0]);
            Check(window.Menu.Visibility == Visibility.Visible && unixKeytipShown,
                "a temporary menu remains visible while entering a submenu with keytips");
            await InputRoutingTests.Key((VirtualKey)unix.AccessKey[0]);
            Check(document.Source(document.State.document_revision).AsSpan().SequenceEqual("alpha\nbeta"u8)
                && window.Menu.Visibility == Visibility.Collapsed,
                "a hidden-menu submenu command completes its transaction and hides the menu bar");
            pane.View.Undo();
            Check(document.Source(document.State.document_revision).AsSpan().SequenceEqual("alpha\r\nbeta"u8),
                "a hidden-menu submenu edit retains exact source undo");
            Check(!preferences.ShowMenu && !new Preferences(preferences.DirectoryPath).ShowMenu,
                "temporary menu interactions never change the saved hidden-menu preference");

            preferences.Set("windows", "showMenu", true);
            pane.FocusEditor(); await Task.Delay(80);
            await InputRoutingTests.Key(VirtualKey.F, alt: true);
            await InputRoutingTests.Key(VirtualKey.Escape);
            Check(window.Menu.Visibility == Visibility.Visible && preferences.ShowMenu,
                "Escape leaves a persistently shown menu bar visible after temporary sessions");
            Check(pane.LastError == null, "menu access-key routing completes without editor errors");
        }
        finally
        {
            window.OpenDialogForTesting = null;
            preferences.Set("windows", "showMenu", originalShowMenu);
            App.Instance.Windows.Remove(window); window.Close();
        }
    }
}
#endif
