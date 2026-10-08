using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;
using Windows.UI.Core;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private bool temporaryMenu, pendingMenuAlt, menuDismissQueued, menuFlyoutOpened, restoreMenuFocusOnActivation;
    private readonly HashSet<MenuBarItem> openMenus = [];
    private WeakReference<Control>? menuReturnFocus;

    private static bool MenuKeyDown(VirtualKey key) =>
        (InputKeyboardSource.GetKeyStateForCurrentThread(key) & CoreVirtualKeyStates.Down) != 0;
    private static bool IsAltKey(VirtualKey key) => key is VirtualKey.Menu or VirtualKey.LeftMenu or VirtualKey.RightMenu;
    private static bool OtherMenuModifierDown() => MenuKeyDown(VirtualKey.Control) || MenuKeyDown(VirtualKey.Shift)
        || MenuKeyDown(VirtualKey.LeftWindows) || MenuKeyDown(VirtualKey.RightWindows);

    private void ConfigureMenuAccessKeys()
    {
        root.PreviewKeyDown += RevealMenuForAlt;
        root.PreviewKeyUp += CompleteMenuAlt;
        root.PointerPressed += (_, _) => { pendingMenuAlt = false; QueueMenuDismiss(); };
        root.LostFocus += (_, _) => QueueMenuDismiss();
        AccessKeyManager.IsDisplayModeEnabledChanged += MenuAccessModeChanged;
    }

    private void TrackMenuFlyout(MenuBarItem menu, MenuFlyoutItemBase firstItem)
    {
        // MenuBarItem does not expose its flyout. Its stable first row is loaded
        // for the lifetime of the native popup, including nested menu navigation.
        firstItem.Loaded += (_, _) => { openMenus.Add(menu); pendingMenuAlt = false; menuFlyoutOpened = true; };
        firstItem.Unloaded += (_, _) => { openMenus.Remove(menu); QueueMenuDismiss(); };
    }

    private void RevealMenuForAlt(object sender, KeyRoutedEventArgs e)
    {
        if (IsAltKey(e.Key))
        {
            if (preferences.ShowMenu || temporaryMenu || OtherMenuModifierDown()) return;
            if (FocusManager.GetFocusedElement(root.XamlRoot) is Control focus) menuReturnFocus = new(focus);
            temporaryMenu = pendingMenuAlt = true;
            menuFlyoutOpened = false;
            UpdateMenuVisibility();
            // Register and arrange the native access-key owners before the next
            // key, which may open a flyout immediately rather than waiting a frame.
            Menu.UpdateLayout();
            return;
        }
        if (!temporaryMenu) return;
        pendingMenuAlt = false;
        if (OtherMenuModifierDown() || MenuKeyDown(VirtualKey.Menu)
            && !Menu.Items.Any(item => item.AccessKey.Length == 1 && item.AccessKey[0] == (char)e.Key))
        {
            HideTemporaryMenu(restoreFocus: true);
            return;
        }
        QueueMenuDismiss();
    }

    private void CompleteMenuAlt(object sender, KeyRoutedEventArgs e)
    {
        if (!IsAltKey(e.Key) || !pendingMenuAlt) return;
        pendingMenuAlt = false;
        // Access keys run before routed preview events. When the bar started
        // collapsed, WinUI could not record the initial bare Alt-down itself.
        if (temporaryMenu && !OtherMenuModifierDown() && !AccessKeyManager.IsDisplayModeEnabled)
            AccessKeyManager.EnterDisplayMode(root.XamlRoot);
        QueueMenuDismiss();
    }

    private void MenuAccessModeChanged(object? sender, object args) => QueueMenuDismiss();

    private void QueueMenuDismiss()
    {
        if (closed || !temporaryMenu || menuDismissQueued) return;
        menuDismissQueued = true;
        DispatcherQueue.TryEnqueue(() => {
            menuDismissQueued = false;
            // Wait until a command, native dismissal, or switch between top-level
            // menus has finished. Keytip dismissal alone does not close a flyout.
            if (closed || !temporaryMenu || pendingMenuAlt || IsMenuFlyoutOpen()
                || !menuFlyoutOpened && AccessKeyManager.IsDisplayModeEnabled) return;
            HideTemporaryMenu(restoreFocus: true);
        });
    }

    private bool IsMenuFlyoutOpen() => openMenus.Count != 0 || Menu.Items.Any(menu =>
        // A new flyout can be opening before its first row has loaded when the
        // keyboard switches menus. The native automation state covers that gap.
        FrameworkElementAutomationPeer.CreatePeerForElement(menu) is MenuBarItemAutomationPeer
            { ExpandCollapseState: ExpandCollapseState.Expanded });

    private void HideTemporaryMenu(bool restoreFocus = false, bool deactivating = false)
    {
        if (!temporaryMenu) return;
        // Exiting native access-key mode can unload the popup and move focus
        // to the title bar. Capture menu ownership before that transition so
        // deactivation retains the original editor focus for keyboard return.
        var focused = root.XamlRoot == null ? null : FocusManager.GetFocusedElement(root.XamlRoot) as DependencyObject;
        bool focusInMenu = false;
        for (var element = focused; element != null; element = VisualTreeHelper.GetParent(element))
            if (element == Menu || element is MenuFlyoutPresenter && openMenus.Count != 0)
            { focusInMenu = true; break; }
        // WinUI can close the flyout before reporting window deactivation.
        // A live keyboard session still owns its saved return target in that
        // case; pointer reactivation below deliberately never restores it.
        bool returnAfterActivation = deactivating && (focusInMenu
            || menuFlyoutOpened && menuReturnFocus?.TryGetTarget(out _) == true);
        temporaryMenu = pendingMenuAlt = false;
        if (IsWindowActive && AccessKeyManager.IsDisplayModeEnabled) AccessKeyManager.ExitDisplayMode();
        UpdateMenuVisibility();
        if (restoreFocus && IsWindowActive && focusInMenu)
            RestoreMenuFocus();
        // Focusing during deactivation would reactivate this window. Retain the
        // target for keyboard activation; mouse activation must follow its click.
        restoreMenuFocusOnActivation = returnAfterActivation;
        if (!restoreMenuFocusOnActivation) menuReturnFocus = null;
    }

    private void RestoreMenuFocus()
    {
        if (menuReturnFocus?.TryGetTarget(out var previous) == true && previous.IsLoaded && previous.XamlRoot == root.XamlRoot)
            previous.Focus(FocusState.Keyboard);
        else ActivePane?.FocusEditor();
    }

    private void RestoreMenuFocusAfterActivation(WindowActivationState state)
    {
        if (!restoreMenuFocusOnActivation) return;
        restoreMenuFocusOnActivation = false;
        if (state == WindowActivationState.CodeActivated) RestoreMenuFocus();
        menuReturnFocus = null;
    }

    private void UpdateMenuVisibility()
    {
        if (preferences.ShowMenu) { temporaryMenu = pendingMenuAlt = restoreMenuFocusOnActivation = false; menuReturnFocus = null; }
        Menu.Visibility = preferences.ShowMenu || temporaryMenu ? Visibility.Visible : Visibility.Collapsed;
        menuToggle.IsChecked = preferences.ShowMenu;
    }

    private void ReleaseMenuAccessKeys()
    {
        AccessKeyManager.IsDisplayModeEnabledChanged -= MenuAccessModeChanged;
        menuReturnFocus = null;
        openMenus.Clear();
    }
#if DEBUG
    internal string MenuFocusForTesting => $"active={IsWindowActive}, temporary={temporaryMenu}, restore={restoreMenuFocusOnActivation}, return={(menuReturnFocus?.TryGetTarget(out var target) == true ? target.GetType().Name : "none")}, open={openMenus.Count}, focus={(root.XamlRoot == null ? null : FocusManager.GetFocusedElement(root.XamlRoot)?.GetType().Name)}, keytips={AccessKeyManager.IsDisplayModeEnabled}";
#endif
}
