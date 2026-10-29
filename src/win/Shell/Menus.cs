using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Windowing;
using Viem.Windows.Core;
using Viem.Windows.Input;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private readonly List<(MenuFlyoutItemBase Item, Func<bool> Enabled)> validation = [];
    private readonly List<(MenuFlyoutItemBase Item, Func<bool> Enabled)> inputTargetValidation = [];
    private readonly List<(ToggleMenuFlyoutItem Item, Func<bool> Checked)> checks = [];
    private bool menusDirty = true;
    private bool styleMenuRefreshQueued;
    private bool inputTargetRefreshQueued;
    private MenuFlyoutSubItem recentMenu = null!;
    private MenuFlyoutSubItem headingMenu = null!, themeMenu = null!;
    private MenuFlyoutItem undoItem = null!, redoItem = null!;
    private ToggleMenuFlyoutItem wrapItem = null!;
    private CoreView? View => ActivePane?.View;
    private bool Rich => View != null && CoreDocument.IsMarkdown(ActivePane!.Document.State.format);
    private MenuFlyoutItem Item(string text, string accessKey, Func<Task> action, string shortcut = "", Func<bool>? enabled = null, bool inputTarget = false)
    {
        var item = new MenuFlyoutItem { Text = text, AccessKey = accessKey, KeyboardAcceleratorTextOverride = shortcut };
        item.Loaded += (_, _) => { ValidateMenus(); QueueInputTargetRefresh(); };
        item.GotFocus += (_, _) => QueueInputTargetRefresh();
        item.Click += (_, _) => Safe(action);
        if (enabled != null) (inputTarget ? inputTargetValidation : validation).Add((item, enabled));
        return item;
    }
    private MenuFlyoutItem ActionItem(string text, string accessKey, Action action, string shortcut = "", Func<bool>? enabled = null, bool inputTarget = false)
        => Item(text, accessKey, () => { action(); ActivePane?.FocusEditor(); return Task.CompletedTask; }, shortcut, enabled, inputTarget);
    private ToggleMenuFlyoutItem Toggle(string text, string accessKey, Action<bool> action, Func<bool>? state = null, bool validateOnLoad = true)
    {
        var item = new ToggleMenuFlyoutItem { Text = text, AccessKey = accessKey };
        if (validateOnLoad) item.Loaded += (_, _) => ValidateMenus();
        item.Click += (_, _) => Safe(() => { action(item.IsChecked); ActivePane?.FocusEditor(); return Task.CompletedTask; });
        if (state != null) checks.Add((item, state));
        return item;
    }
    private ToggleMenuFlyoutItem ListMenuItem(string text, string accessKey, ToolbarAction action, string shortcut = "")
    {
        var item = Toggle(text, accessKey, _ => ExecuteFormattingAction(action),
            () => InsertPresentation()?.ActionSelected(action) == true);
        item.KeyboardAcceleratorTextOverride = shortcut;
        validation.Add((item, () => FormattingActionAvailable(action)));
        return item;
    }
    private MenuBarItem Top(string title, string accessKey, params MenuFlyoutItemBase[] items)
    {
        var menu = new MenuBarItem { Title = title, AccessKey = accessKey, IsAccessKeyScope = true };
        // Validate on demand for pointer, keyboard, access-key and UI Automation
        // opening (the items' Loaded handler), rather than on every editor move.
        menu.PointerEntered += (_, _) => ValidateMenus();
        menu.GotFocus += (_, _) => ValidateMenus();
        menu.AccessKeyInvoked += (_, _) => ValidateMenus();
        foreach (var item in items) menu.Items.Add(item);
        if (items.Length != 0) TrackMenuFlyout(menu, items[0]);
        Menu.Items.Add(menu); return menu;
    }
    private static MenuFlyoutSubItem Sub(string title, string accessKey, params MenuFlyoutItemBase[] items)
    {
        var menu = new MenuFlyoutSubItem { Text = title, AccessKey = accessKey, IsAccessKeyScope = true, ExitDisplayModeOnAccessKeyInvoked = false };
        foreach (var item in items) menu.Items.Add(item);
        return menu;
    }
    private static MenuFlyoutSeparator Separator() => new();
    private void BuildMenus()
    {
        BuildDocumentModeMenu();
        recentMenu = Sub("Open Recent", "R");
        Top("File", "F",
            Item("New", "N", () => { NewWindow(null); return Task.CompletedTask; }), Item("Open…", "O", OpenDialog), recentMenu, Separator(),
            Item("Close", "C", () => ActivePane == null ? Task.CompletedTask : ClosePane(ActivePane)),
            Item("Save", "S", () => ActivePane == null ? Task.CompletedTask : Save(ActivePane), "Ctrl+S"),
            Item("Save As…", "A", () => ActivePane == null ? Task.CompletedTask : Save(ActivePane, true), "Ctrl+Shift+S"),
            Item("Export…", "E", () => ActivePane == null ? Task.CompletedTask : Export(ActivePane), enabled: () => View != null),
            Item("Duplicate", "D", () => { if (ActivePane != null) NewWindow(NewDocument(ActivePane.Document.Source(ActivePane.Document.State.document_revision), format: ActivePane.Document.State.format)); return Task.CompletedTask; }),
            Item("Revert to Last Saved…", "V", () => ActivePane == null ? Task.CompletedTask : Reload(ActivePane), enabled: () => ActivePane?.Document.FilePath != null), Separator(),
            codeLanguageMenu,
            Sub("Text Encoding", "T", new[] { (Name: "UTF-8", Key: "8"), (Name: "Latin-1", Key: "1"), (Name: "UTF-16 LE", Key: "L"), (Name: "UTF-16 BE", Key: "B") }.Select((s, i) => ActionItem(s.Name, s.Key, () => View?.SetEncoding((uint)i + 1))).ToArray()),
            Sub("Line Endings", "L", ActionItem("Unix (LF)", "U", () => View?.FileFormat(1)), ActionItem("Windows (CRLF)", "W", () => View?.FileFormat(2)), ActionItem("Classic Mac (CR)", "M", () => View?.FileFormat(3))),
            Separator(), Item("Settings…", "I", ShowSettings), Separator(), Item("Exit", "X", RequestClose));
        undoItem = ActionItem("Undo", "U", () => View?.Undo(), "Ctrl+Z", enabled: () => ActivePane != null && (ActivePane.Document.State.flags & VIEM_DOCUMENT_STATE_CAN_UNDO) != 0);
        redoItem = ActionItem("Redo", "R", () => View?.Redo(), "Ctrl+Shift+Z", enabled: () => ActivePane != null && (ActivePane.Document.State.flags & VIEM_DOCUMENT_STATE_CAN_REDO) != 0);
        Top("Edit", "E", undoItem, redoItem, Separator(),
            Item("Cut", "T", () => ActivePane?.Copy(true) ?? Task.CompletedTask, "Ctrl+X", () => ActivePane?.CanCut == true, inputTarget: true),
            Item("Copy", "C", () => ActivePane?.Copy(false) ?? Task.CompletedTask, "Ctrl+C", () => ActivePane?.CanCopy == true, inputTarget: true),
            Item("Copy Source", "O", () => ActivePane?.CopySource() ?? Task.CompletedTask, enabled: () => ActivePane?.CanCopy == true, inputTarget: true),
            Item("Paste", "P", () => ActivePane?.Paste() ?? Task.CompletedTask, "Ctrl+V"),
            Item("Paste and Match Style", "M", () => ActivePane?.Paste(true) ?? Task.CompletedTask, "Ctrl+Shift+V"),
            ActionItem("Delete", "D", () =>
            {
                if (View is not { } view) return;
                if (view.IsTextSelection) view.Key(VIEM_KEY_DELETE);
                else view.SelectionCommand("d");
            }, enabled: () => View?.HasSelection == true && ActivePane?.CanCut == true, inputTarget: true), Separator(),
            Item("Select All", "A", () => { ActivePane?.SelectAll(); return Task.CompletedTask; }),
            Sub("Select", "S", ActionItem("Word", "W", () => Select(VIEM_NATIVE_SELECT_WORD), enabled: () => View?.NativeActionAvailable(VIEM_NATIVE_SELECT_WORD) == true), ActionItem("Sentence", "S", () => Select(VIEM_NATIVE_SELECT_SENTENCE), enabled: () => View?.NativeActionAvailable(VIEM_NATIVE_SELECT_SENTENCE) == true), ActionItem("Paragraph", "P", () => Select(VIEM_NATIVE_SELECT_PARAGRAPH), enabled: () => View?.NativeActionAvailable(VIEM_NATIVE_SELECT_PARAGRAPH) == true), ActionItem("Hard Line", "L", () => Select(VIEM_NATIVE_SELECT_LINE), enabled: () => View?.NativeActionAvailable(VIEM_NATIVE_SELECT_LINE) == true), ActionItem("Visual Row", "R", () => Select(VIEM_NATIVE_SELECT_VISUAL_ROW), enabled: () => View?.NativeActionAvailable(VIEM_NATIVE_SELECT_VISUAL_ROW) == true), ActionItem("Visual Block", "B", () => Select(VIEM_NATIVE_SELECT_BLOCK), "Ctrl+Q", enabled: () => View?.NativeActionAvailable(VIEM_NATIVE_SELECT_BLOCK) == true)),
            Separator(), Sub("Find", "F", ActionItem("Find…", "F", () => Select(VIEM_NATIVE_FIND), enabled: () => View?.NativeActionAvailable(VIEM_NATIVE_FIND) == true), ActionItem("Find and Replace…", "R", () => Select(VIEM_NATIVE_FIND_REPLACE), enabled: () => View?.NativeActionAvailable(VIEM_NATIVE_FIND_REPLACE) == true), ActionItem("Find Next", "N", () => Select(VIEM_NATIVE_FIND_NEXT), enabled: () => View?.NativeActionAvailable(VIEM_NATIVE_FIND_NEXT) == true), ActionItem("Find Previous", "P", () => Select(VIEM_NATIVE_FIND_PREVIOUS), enabled: () => View?.NativeActionAvailable(VIEM_NATIVE_FIND_PREVIOUS) == true),
                ActionItem("Use Selection for Find", "U", () => View?.UseSelectionForFind(), enabled: HasFindSelection),
                ActionItem("Jump to Selection", "J", () => View?.RevealSelection(), enabled: () => View?.HasSelection == true)),
            Sub("Transformations", "N", ActionItem("Make Uppercase", "U", () => View?.SelectionCommand("U"), enabled: () => View?.HasSelection == true), ActionItem("Make Lowercase", "L", () => View?.SelectionCommand("u"), enabled: () => View?.HasSelection == true), ActionItem("Toggle Case", "T", () => View?.SelectionCommand("~"), enabled: () => View?.HasSelection == true)));
        headingMenu = Sub("Heading", "H");
        validation.Add((headingMenu, () => Rich && headingMenu.Items.Any(item => item.IsEnabled)));
        MenuBarItem? insertMenu = null;
        insertMenu = Top("Insert", "I",
            headingMenu,
            ActionItem("Code", "C", () => ExecuteFormattingAction(ToolbarAction.CharacterCode), enabled: () => FormattingActionAvailable(ToolbarAction.CharacterCode)),
            Item("Link", "L", () => { ActiveFormattingToolbar()?.ExecuteLink(); return Task.CompletedTask; }, enabled: () => {
                var toolbar = InsertPresentation();
                return toolbar?.InsertLink.IsEnabled == true;
            }),
            Item("Reference", "R", () => { ActivePane?.ShowInsertReference(); return Task.CompletedTask; }, enabled: () => {
                var toolbar = InsertPresentation();
                return toolbar?.InsertReference.IsEnabled == true;
            }), Separator(),
            ListMenuItem("Bulleted List", "B", ToolbarAction.Bullets, KeyPolicy.FormattingShortcut(NativeAction.Bullets)),
            ListMenuItem("Numbered List", "N", ToolbarAction.Numbers, KeyPolicy.FormattingShortcut(NativeAction.Numbers)),
            ListMenuItem("Task List", "A", ToolbarAction.Tasks),
            Separator(),
            ActionItem("Blockquote", "Q", () => ExecuteFormattingAction(ToolbarAction.BlockQuote), enabled: () => FormattingActionAvailable(ToolbarAction.BlockQuote)),
            ActionItem("Code Block", "O", () => ExecuteFormattingAction(ToolbarAction.CodeBlock), enabled: () => FormattingActionAvailable(ToolbarAction.CodeBlock)),
            Item("Image", "M", () => { if (ActivePane is { } pane) pane.Run(pane.ShowInsertImage); return Task.CompletedTask; }, enabled: () => {
                var toolbar = InsertPresentation();
                return toolbar?.InsertImage.IsEnabled == true;
            }),
            Item("Table", "T", () => {
                // Open after the native menu dismisses, keeping the same grid,
                // drag sizing, limits and insertion identity as the toolbar.
                var pane = ActivePane;
                DispatcherQueue.TryEnqueue(() => { if (ActivePane == pane && insertMenu != null) ActiveFormattingToolbar()?.OpenTablePicker(insertMenu); });
                return Task.CompletedTask;
            }, enabled: () => Rich && View is { } view && (view.TableContext().flags & 1) != 0));
        wrapItem = Toggle("Word wrap", "W", b => View?.Wrap(b));
        bool CanAlignIndents() => View?.Document.State.format == VIEM_FORMAT_CODE;
        var alignIndentsItem = Toggle("Align indents to previous line", "A", b => View?.AlignIndentsToPreviousLine(b),
            () => CanAlignIndents() && ActivePane?.AlignIndentsToPreviousLine == true);
        validation.Add((alignIndentsItem, CanAlignIndents));
        themeMenu = Sub("Theme", "T"); themeMenu.Loaded += (_, _) => RefreshThemeMenu();
        RefreshThemeMenu();
        Top("Format", "O", plainMode, markdownMode, codeMode, Separator(), themeMenu, StyleEditorItem(),
            ActionItem("Reload Style Sheet", "R", () => {
                preferences.ReloadCurrentTheme();
                ActivePane?.SetMessage("Reloaded theme " + preferences.ThemeDisplayName);
            }), Separator(), wrapItem, alignIndentsItem);
        var lineNumbersItem = Toggle("Show line numbers", "N", b => View?.Number(b), () => View?.NumberEnabled == true);
        validation.Add((lineNumbersItem, () => View != null));
        Top("View", "V", Toggle("Show Status Bar", "S", b => preferences.Set("appearance", "showStatusBar", b), () => preferences.ShowStatus), Toggle("Show Menu Bar", "B", b => preferences.Set("windows", "showMenu", b), () => preferences.ShowMenu), Separator(),
            Toggle("Physical Source Lines", "L", b => View?.LineMode(b ? 1u : 0u), () => View?.CurrentLineMode == 1), lineNumbersItem, Toggle("Show Invisible Characters", "I", b => View?.VisibleWhitespace(b), () => ActivePane?.WhitespaceEnabled == true),
            Separator(), ActionItem("Zoom In", "Z", () => View?.StepZoom(true), "Ctrl+="), ActionItem("Zoom Out", "O", () => View?.StepZoom(false), "Ctrl+-"), ActionItem("Actual Size", "A", () => View?.Zoom(1)),
            Separator(), ActionItem("Full Screen", "U", () => AppWindow.SetPresenter(AppWindow.Presenter.Kind == AppWindowPresenterKind.FullScreen ? AppWindowPresenterKind.Overlapped : AppWindowPresenterKind.FullScreen)));
        Top("Window", "W", ActionItem("Split Vertically", "V", () => { if (ActivePane != null) SplitPane(ActivePane, ActivePane.Document, vertical: true); }), ActionItem("Split View", "S", () => { if (ActivePane != null) SplitPane(ActivePane, ActivePane.Document); }), ActionItem("New Empty Pane", "E", () => { RequireSplitRoom(); SplitPane(ActivePane!, NewDocument()); }),
            Item("Close Other Panes", "C", () => ActivePane == null ? Task.CompletedTask : WindowCommand(ActivePane, VIEM_WINDOW_CLOSE_OTHERS, 1)), Item("Equalize Panes", "Q", () => ActivePane == null ? Task.CompletedTask : WindowCommand(ActivePane, VIEM_WINDOW_EQUALIZE_HEIGHTS, 1)),
            Separator(), ActionItem("New Window for Document", "N", () => NewWindow(ActivePane?.Document)), ActionItem("Minimize", "M", () => (AppWindow.Presenter as OverlappedPresenter)?.Minimize()));
        Top("Help", "H", Item("About Viem", "A", ShowAbout));
    }
    private async Task ShowAbout()
    {
        var content = new StackPanel { Spacing = 12, HorizontalAlignment = HorizontalAlignment.Center };
        var logo = new Image {
            Source = new BitmapImage(new Uri("ms-appx:///Assets/Viem.png")),
            Width = 128, Height = 128, Stretch = Stretch.Uniform
        };
        AutomationProperties.SetName(logo, "Viem logo");
        content.Children.Add(logo);
        content.Children.Add(new TextBlock {
            Text = "Viem", FontSize = 24, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            HorizontalAlignment = HorizontalAlignment.Center
        });
        content.Children.Add(new TextBlock { Text = "Version 0.1", HorizontalAlignment = HorizontalAlignment.Center, IsTextSelectionEnabled = true });
        content.Children.Add(new TextBlock { Text = "© 2026 Brett Wilson", IsTextSelectionEnabled = true });
        content.Children.Add(new HyperlinkButton {
            Content = "Licenses",
            NavigateUri = new Uri("https://github.com/brettw/viem/blob/main/LICENSE.md"),
            HorizontalAlignment = HorizontalAlignment.Center,
            Padding = new Thickness(0)
        });
        var dialog = new ContentDialog {
            XamlRoot = root.XamlRoot, RequestedTheme = root.RequestedTheme,
            Content = content, CloseButtonText = "OK",
            DefaultButton = ContentDialogButton.Close
        };
        AutomationProperties.SetName(dialog, "About Viem");
        await DeferExternalFileReviews(async () => { await dialog.ShowAsync(); });
    }
    private void Select(uint action) => View?.NativeAction(action);
    private bool HasFindSelection()
    {
        if (View is not { HasSelection: true } view) return false;
        if (view.Presentation.mode is VIEM_MODE_VISUAL_BLOCK or VIEM_MODE_SELECT_BLOCK or VIEM_MODE_SELECTION_BLOCK)
            return view.Selection().Segments.Any(segment => segment.text_start < segment.text_end);
        var selection = view.LogicalSelection();
        return selection.text_start < selection.text_end;
    }
    private static string HistoryCategory(uint value) => value switch { 1 => "Typing", 2 => "Style", 3 => "Line Endings", 4 => "Move Lines", 6 => "Document Format", _ => "Edit" };
    private void RefreshInputTargetMenus()
    {
        foreach (var (item, enabled) in inputTargetValidation) { try { item.IsEnabled = enabled(); } catch { item.IsEnabled = false; } }
    }
    private void QueueInputTargetRefresh()
    {
        if (closed || inputTargetRefreshQueued) return;
        inputTargetRefreshQueued = true;
        if (!DispatcherQueue.TryEnqueue(() => {
            inputTargetRefreshQueued = false;
            // Windowed popup focus does not route through its MenuBarItem.
            // Recheck the target after native focus/selection handlers settle,
            // without replacing rows or rebinding snapshot-owned actions.
            if (!closed && openMenus.Count > 0) RefreshInputTargetMenus();
        })) inputTargetRefreshQueued = false;
    }
    private void ValidateMenus()
    {
        // Native focus can pass through an intermediate control while a menu
        // opens. Recheck its edit target on each load/focus event, even when
        // the document and retained style catalogues have not changed.
        RefreshInputTargetMenus();
        if (!menusDirty) return;
        menusDirty = false;
        // The Insert menu reuses one exact toolbar presentation per validation
        // pass instead of querying every formatting capability for every row.
        insertValidationPrepared = false;
        insertValidationToolbar = null;
        using var measurement = Diagnostics.InputPerformance.Measure("menus");
        RefreshStyleMenus();
        RefreshDocumentModeMenu();
        foreach (var (item, enabled) in validation) { try { item.IsEnabled = enabled(); } catch { item.IsEnabled = false; } }
        foreach (var (item, checkedValue) in checks) { try { item.IsChecked = checkedValue(); } catch { } }
        if (View == null || undoItem == null) return;
        var state = ActivePane!.Document.State;
        undoItem.Text = "Undo " + HistoryCategory(state.undo_action_category); redoItem.Text = "Redo " + HistoryCategory(state.redo_action_category);
        try { wrapItem.IsChecked = (View.Viewport.flags & VIEM_VIEWPORT_STATE_WRAP) != 0; } catch { }
    }
    private void RefreshRecentMenu()
    {
        if (recentMenu == null) return;
        recentMenu.Items.Clear();
        foreach (string path in preferences.Recent)
        { var item = Item(Path.GetFileName(path), "", () => OpenNative(path)); ToolTipService.SetToolTip(item, path); recentMenu.Items.Add(item); }
        recentMenu.Items.Add(Separator()); recentMenu.Items.Add(ActionItem("Clear Menu", "C", preferences.ClearRecent));
    }
    private void RefreshStyleMenus()
    {
        if (View == null || headingMenu == null) return;
        // Keep tracked native rows and their original action identities intact.
        // Syntax publication or another active view can change the catalogue;
        // rebuild it after unload, before the next submenu opens.
        if (headingMenu.Items.Any(item => item.IsLoaded && item.Tag is StyleKey)) {
            menusDirty = true;
            return;
        }
        var selected = View.SelectedNamedStyles();
        var snapshot = View.Styles(selected.Identity);
        var choices = CoreView.StyleChoices(snapshot, selected, View.Document.IsReadOnly)
            .Where(choice => CoreView.HeadingLevel(choice.Key) is >= 1 and <= 6)
            .OrderBy(choice => CoreView.HeadingLevel(choice.Key)).ToArray();
        var old = headingMenu.Items.OfType<ToggleMenuFlyoutItem>().ToArray();
        if (!old.Select(item => (StyleKey)item.Tag).SequenceEqual(choices.Select(choice => choice.Key)))
        {
            headingMenu.Items.Clear();
            foreach (var choice in choices)
            {
                var item = new ToggleMenuFlyoutItem { Tag = choice.Key };
                item.Click += (_, _) => Safe(() => {
                    if (item.CommandParameter is Viem.Windows.Interop.ViemStyleSheetIdentityV1 identity)
                        View?.ChooseStyle((StyleKey)item.Tag, identity);
                    RestoreEditorFocusAfterControl(); return Task.CompletedTask;
                });
                headingMenu.Items.Add(item);
            }
        }
        var items = headingMenu.Items.Cast<ToggleMenuFlyoutItem>().ToArray();
        for (int i = 0; i < choices.Length; i++)
        {
            uint level = CoreView.HeadingLevel(choices[i].Key)!.Value;
            items[i].Text = choices[i].Name; items[i].AccessKey = level.ToString();
            items[i].IsEnabled = Rich && View.HasFormattingSelection && choices[i].Enabled;
            items[i].IsChecked = Rich && choices[i].Selected; items[i].CommandParameter = snapshot.Identity;
            items[i].KeyboardAcceleratorTextOverride = level < 6 ? $"Ctrl+{level}" : "";
        }
        TrackStyleMenuRows();
    }
    private void TrackStyleMenuRows()
    {
        foreach (var item in headingMenu.Items.Where(item => item.Tag is StyleKey)) {
            item.Unloaded -= StyleMenuRowUnloaded;
            item.Unloaded += StyleMenuRowUnloaded;
        }
    }
    private void StyleMenuRowUnloaded(object sender, RoutedEventArgs args)
    {
        // A child submenu can close while its top-level menu remains open.
        // Wait for the complete unload before replacing any retained rows.
        if (closed || !menusDirty || styleMenuRefreshQueued) return;
        styleMenuRefreshQueued = true;
        if (!DispatcherQueue.TryEnqueue(() => {
            styleMenuRefreshQueued = false;
            if (!closed && menusDirty) ValidateMenus();
        })) styleMenuRefreshQueued = false;
    }
    private static StyleWindow? styleInspector;
    private MenuFlyoutItem StyleEditorItem() => Item("Edit Styles…", "E", () => { ShowStyles(); return Task.CompletedTask; }, "F8");
    internal void ShowStyles()
    {
        if (View == null) return;
        if (styleInspector == null) { styleInspector = new StyleWindow(View, preferences); styleInspector.Closed += (_, _) => styleInspector = null; }
        else styleInspector.Retarget(View);
        styleInspector.Activate();
    }
    internal void ShowVisibleWhitespaceStyle()
    {
        if (View == null) return;
        if (styleInspector == null) { styleInspector = new StyleWindow(View, preferences, followCaret: false); styleInspector.Closed += (_, _) => styleInspector = null; }
        styleInspector.SelectVisibleWhitespace();
        styleInspector.Activate();
    }
#if DEBUG
    internal StyleWindow? StyleInspector => styleInspector;
    internal void ValidateMenusForTesting() => ValidateMenus();
#endif
    private void RefreshThemeMenu()
    {
        if (themeMenu == null) return;
        preferences.EnsureCurrentThemeExists();
        themeMenu.Items.Clear();
        foreach (var file in preferences.ThemeFiles)
        {
            var item = new ToggleMenuFlyoutItem { Text = file.Name, IsChecked = preferences.SelectedThemePath == file.Path };
            item.Click += (_, _) => Safe(() => { try { preferences.SelectTheme(file.Name, file.Path); } finally { RefreshThemeMenu(); } return Task.CompletedTask; });
            themeMenu.Items.Add(item);
        }
        themeMenu.Items.Add(Separator());
        var fallback = new ToggleMenuFlyoutItem { Text = "Default", AccessKey = "D", IsChecked = preferences.SelectedTheme == null };
        fallback.Click += (_, _) => Safe(() => { preferences.SelectTheme(null); return Task.CompletedTask; });
        themeMenu.Items.Add(fallback);
        themeMenu.Items.Add(Item("New theme…", "N", () => DeferExternalFileReviews(() => ThemeDialogs.Create(preferences, root.XamlRoot, root.RequestedTheme))));
        themeMenu.Items.Add(Item("Theme Settings…", "S", ShowThemeSettings));
    }
    private SettingsWindow? settingsWindow;
    internal Task ShowSettings() => ShowSettings(selectTheme: false);
    internal Task ShowThemeSettings() => ShowSettings(selectTheme: true);
    private Task ShowSettings(bool selectTheme)
    {
        if (settingsWindow == null) { settingsWindow = new SettingsWindow(preferences) { EditVisibleWhitespaceStyle = ShowVisibleWhitespaceStyle }; settingsWindow.Closed += (_, _) => settingsWindow = null; }
        if (selectTheme) settingsWindow.SelectThemeCategory();
        settingsWindow.Activate(); return Task.CompletedTask;
    }
#if DEBUG
    internal SettingsWindow? SettingsInspector => settingsWindow;
#endif
}
