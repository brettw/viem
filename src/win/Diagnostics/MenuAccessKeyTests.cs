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
    {
        if (!condition) throw new InvalidOperationException(details == null ? name : $"{name} ({details})");
        FrontendSmokeTests.UiChecks.Add(name);
    }

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
            "Code language" => Label(item).StartsWith("Auto (", StringComparison.Ordinal) || item is MenuFlyoutSubItem,
            "Obscure languages" => false,
            _ => item.Tag is not (StyleKey or CodeSyntaxStyle)
        };
        Check(commands.Where(Fixed).All(item => item.AccessKey.Length != 0),
            $"{title} fixed menu commands have access keys");
        foreach (var submenu in commands.OfType<MenuFlyoutSubItem>()) CheckScope(submenu.Text, submenu.Items);
    }

    /// A temporary menu hides once a command or dismissal has finished,
    /// after its flyout's closing animation; wait for that, boundedly.
    private static async Task WaitForHiddenMenu(EditorWindow window)
    {
        for (int i = 0; i < 40 && window.Menu.Visibility != Visibility.Collapsed; i++) await Task.Delay(25);
    }

    internal static async Task Run(Preferences preferences)
    {
        bool originalShowMenu = preferences.ShowMenu;
        var document = new CoreDocument("alpha\r\nbeta"u8.ToArray(), format: VIEM_FORMAT_PLAIN_TEXT, fileFormat: 2);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window);
        window.Activate();
        var pane = window.ActivePane!;
        await pane.Ready;
        int opened = 0;
        window.OpenDialogForTesting = () => {
            opened++;
            return Task.CompletedTask;
        };
        try
        {
            preferences.Set("windows", "showMenu", true);
            Check(window.Menu.Items.Select(menu => menu.AccessKey).Distinct(StringComparer.OrdinalIgnoreCase).Count() == window.Menu.Items.Count,
                "top-level menu access keys are unique");
            foreach (var menu in window.Menu.Items) CheckScope(menu.Title, menu.Items);
            var file = window.Menu.Items.Single(menu => menu.Title == "File");
            var format = window.Menu.Items.Single(menu => menu.Title == "Format");
            var plain = (ToggleMenuFlyoutItem)format.Items[0];
            var code = file.Items.OfType<MenuFlyoutSubItem>().Single(item => item.Text == "Code language");
            var obscure = (MenuFlyoutSubItem)code.Items[1];
            pane.FocusEditor();
            await InputRoutingTests.Settle();
            await InputRoutingTests.Key((VirtualKey)format.AccessKey[0], alt: true);
            Check(plain.IsLoaded && code.Items.Count == 3 && obscure.Items.Count == 0,
                "native Alt+O opens Format without constructing Code language controls");
            await InputRoutingTests.Key(VirtualKey.Escape);
            var open = file.Items.OfType<MenuFlyoutItem>().Single(item => item.Text == "Open…");
            bool openKeytipShown = false;
            open.AccessKeyDisplayRequested += (_, _) => openKeytipShown = true;
            pane.FocusEditor();
            await InputRoutingTests.Settle();
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

            pane.FocusEditor();
            await InputRoutingTests.Settle();
            await InputRoutingTests.Text("i");
            await InputRoutingTests.Key(VirtualKey.O);
            Check(document.FormattedText() == "oalpha\nbeta" && opened == 1,
                "bare O still types into the editor outside menu access-key mode");
            await InputRoutingTests.Key(VirtualKey.Escape);
            pane.View!.Undo();

            preferences.Set("windows", "showMenu", false);
            pane.FocusEditor();
            await InputRoutingTests.Settle();
            Check(window.Menu.Visibility == Visibility.Collapsed, "hiding the menu bar collapses it before access-key activation");
            revision = document.State.document_revision;
            openKeytipShown = false;
            await InputRoutingTests.Key(VirtualKey.F, alt: true);
            Check(window.Menu.Visibility == Visibility.Visible && openKeytipShown,
                "Alt+F temporarily reveals a hidden menu bar and opens File with keytips");
            await InputRoutingTests.Key(VirtualKey.O);
            await WaitForHiddenMenu(window);
            Check(opened == 2 && window.Menu.Visibility == Visibility.Collapsed,
                "Alt+F then O invokes Open once and hides the temporarily revealed menu bar");
            await InputRoutingTests.Key(VirtualKey.F, alt: true);
            await InputRoutingTests.Key(VirtualKey.Escape);
            await WaitForHiddenMenu(window);
            Check(window.Menu.Visibility == Visibility.Collapsed && !preferences.ShowMenu,
                "Escape dismisses a temporarily opened File menu and restores hidden menu visibility");
            pane.FocusEditor();
            await InputRoutingTests.Settle();
            await InputRoutingTests.Key(VirtualKey.F, control: true, alt: true);
            Check(window.Menu.Visibility == Visibility.Collapsed && opened == 2,
                "Ctrl+Alt input does not reveal the hidden menu bar or invoke a menu access key");
            Check(document.State.document_revision == revision && document.FormattedText() == "alpha\nbeta"
                && !new Preferences(preferences.DirectoryPath).ShowMenu,
                "temporary menu activation preserves document text and the saved hidden-menu preference");

            preferences.Set("windows", "showMenu", true);
            pane.FocusEditor();
            await InputRoutingTests.Settle();
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
            App.Instance.Windows.Remove(window);
            window.Close();
        }
    }
}
#endif
