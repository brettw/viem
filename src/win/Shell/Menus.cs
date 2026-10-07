using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Windowing;
using Viem.Windows.Core;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private readonly List<(MenuFlyoutItemBase Item, Func<bool> Enabled)> validation = [];
    private readonly List<(ToggleMenuFlyoutItem Item, Func<bool> Checked)> checks = [];
    private bool menusDirty = true;
    private MenuFlyoutSubItem recentMenu = null!;
    private MenuFlyoutSubItem paragraphMenu = null!, characterMenu = null!, themeMenu = null!;
    private MenuFlyoutItem undoItem = null!, redoItem = null!;
    private ToggleMenuFlyoutItem wrapItem = null!;
    private CoreView? View => ActivePane?.View;
    private bool Rich => View != null && ActivePane!.Document.State.format is VIEM_FORMAT_MARKDOWN or VIEM_FORMAT_MARKDOWN_SOURCE;
    private bool CanFormatBlocks => Rich && View!.HasFormattingSelection && !View.SelectedNamedStyles().HasTable;
    private MenuFlyoutItem Item(string text, string accessKey, Func<Task> action, string shortcut = "", Func<bool>? enabled = null)
    {
        var item = new MenuFlyoutItem { Text = text, AccessKey = accessKey, KeyboardAcceleratorTextOverride = shortcut };
        item.Loaded += (_, _) => ValidateMenus();
        item.Click += (_, _) => Safe(action);
        if (enabled != null) validation.Add((item, enabled));
        return item;
    }
    private MenuFlyoutItem ActionItem(string text, string accessKey, Action action, string shortcut = "", Func<bool>? enabled = null)
        => Item(text, accessKey, () => { action(); ActivePane?.FocusEditor(); return Task.CompletedTask; }, shortcut, enabled);
    private ToggleMenuFlyoutItem Toggle(string text, string accessKey, Action<bool> action, Func<bool>? state = null)
    {
        var item = new ToggleMenuFlyoutItem { Text = text, AccessKey = accessKey };
        item.Loaded += (_, _) => ValidateMenus();
        item.Click += (_, _) => Safe(() => { action(item.IsChecked); ActivePane?.FocusEditor(); return Task.CompletedTask; });
        if (state != null) checks.Add((item, state));
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
        recentMenu = Sub("Open Recent", "R");
        Top("File", "F",
            Item("New", "N", () => { NewWindow(null); return Task.CompletedTask; }), Item("Open…", "O", OpenDialog), recentMenu, Separator(),
            Item("Close", "C", () => ActivePane == null ? Task.CompletedTask : ClosePane(ActivePane)),
            Item("Save", "S", () => ActivePane == null ? Task.CompletedTask : Save(ActivePane), "Ctrl+S"),
            Item("Save As…", "A", () => ActivePane == null ? Task.CompletedTask : Save(ActivePane, true), "Ctrl+Shift+S"),
            Item("Export…", "E", () => ActivePane == null ? Task.CompletedTask : Export(ActivePane), enabled: () => View != null),
            Item("Duplicate", "D", () => { if (ActivePane != null) NewWindow(NewDocument(ActivePane.Document.Source(ActivePane.Document.State.document_revision), format: ActivePane.Document.State.format)); return Task.CompletedTask; }),
            Item("Revert to Last Saved…", "V", () => ActivePane == null ? Task.CompletedTask : Reload(ActivePane), enabled: () => ActivePane?.Document.FilePath != null), Separator(),
            Sub("Text Encoding", "T", new[] { (Name: "UTF-8", Key: "8"), (Name: "Latin-1", Key: "1"), (Name: "UTF-16 LE", Key: "L"), (Name: "UTF-16 BE", Key: "B") }.Select((s, i) => ActionItem(s.Name, s.Key, () => View?.SetEncoding((uint)i + 1))).ToArray()),
            Sub("Line Endings", "L", ActionItem("Unix (LF)", "U", () => View?.FileFormat(1)), ActionItem("Windows (CRLF)", "W", () => View?.FileFormat(2)), ActionItem("Classic Mac (CR)", "M", () => View?.FileFormat(3))),
            Separator(), Item("Settings…", "G", ShowSettings), Separator(), Item("Exit", "X", RequestClose));
        undoItem = ActionItem("Undo", "U", () => View?.Undo(), "Ctrl+Z", enabled: () => ActivePane != null && (ActivePane.Document.State.flags & VIEM_DOCUMENT_STATE_CAN_UNDO) != 0);
        redoItem = ActionItem("Redo", "R", () => View?.Redo(), "Ctrl+Shift+Z", enabled: () => ActivePane != null && (ActivePane.Document.State.flags & VIEM_DOCUMENT_STATE_CAN_REDO) != 0);
        Top("Edit", "E", undoItem, redoItem, Separator(),
            Item("Cut", "T", () => ActivePane?.Copy(true) ?? Task.CompletedTask, "Ctrl+X", () => ActivePane?.CanCut == true),
            Item("Copy", "C", () => ActivePane?.Copy(false) ?? Task.CompletedTask, "Ctrl+C", () => ActivePane?.CanCopy == true),
            Item("Copy Source", "O", () => ActivePane?.CopySource() ?? Task.CompletedTask, enabled: () => ActivePane?.CanCopy == true),
            Item("Paste", "P", () => ActivePane?.Paste() ?? Task.CompletedTask, "Ctrl+V"),
            Item("Paste and Match Style", "M", () => ActivePane?.Paste(true) ?? Task.CompletedTask, "Ctrl+Shift+V"),
            ActionItem("Delete", "D", () =>
            {
                if (View is not { } view) return;
                if (view.IsTextSelection) view.Key(VIEM_KEY_DELETE);
                else view.SelectionCommand("d");
            }, enabled: () => View?.HasSelection == true && ActivePane?.CanCut == true), Separator(),
            Item("Select All", "A", () => { ActivePane?.SelectAll(); return Task.CompletedTask; }),
            Sub("Select", "S", ActionItem("Word", "W", () => Select("viw")), ActionItem("Sentence", "S", () => Select("vis")), ActionItem("Paragraph", "P", () => Select("vip")), ActionItem("Hard Line", "L", () => Select("V")), ActionItem("Visual Block", "B", () => { uint returnMode = View?.Presentation.mode ?? VIEM_MODE_NORMAL; View?.Key(VIEM_KEY_ESCAPE); View?.Key(VIEM_KEY_CONTROL_CHARACTER, 'q'); View?.SetSelectionOrigin(VIEM_SELECTION_ORIGIN_KEY, returnMode); }, "Ctrl+Q")),
            Separator(), Sub("Find", "F", ActionItem("Find…", "F", () => Select("/")), ActionItem("Find and Replace…", "R", () => { Select(":"); View?.Text("%s/"); }), ActionItem("Find Next", "N", () => Select("n")), ActionItem("Find Previous", "P", () => Select("N"))),
            Sub("Transformations", "N", ActionItem("Make Uppercase", "U", () => View?.SelectionCommand("U"), enabled: () => View?.HasSelection == true), ActionItem("Make Lowercase", "L", () => View?.SelectionCommand("u"), enabled: () => View?.HasSelection == true), ActionItem("Toggle Case", "T", () => View?.SelectionCommand("~"), enabled: () => View?.HasSelection == true)));
        paragraphMenu = Sub("Paragraph", "P", ActionItem("Bulleted List", "B", () => View?.SetList(VIEM_LIST_STYLE_BULLET), enabled: () => CanFormatBlocks), ActionItem("Numbered List", "N", () => View?.SetList(VIEM_LIST_STYLE_NUMBERED), enabled: () => CanFormatBlocks), ActionItem("Remove List", "R", () => View?.SetList(VIEM_LIST_STYLE_NONE), enabled: () => CanFormatBlocks),
            ActionItem("Indent", "I", () => View?.IndentList(false), enabled: () => CanFormatBlocks && (View.ListCapabilities() & VIEM_LIST_CAN_INDENT) != 0), ActionItem("Unindent", "U", () => View?.IndentList(true), enabled: () => CanFormatBlocks && (View.ListCapabilities() & VIEM_LIST_CAN_UNINDENT) != 0), Separator());
        characterMenu = Sub("Character", "C");
        validation.Add((paragraphMenu, () => Rich));
        validation.Add((characterMenu, () => Rich));
        themeMenu = Sub("Theme", "T"); themeMenu.Loaded += (_, _) => RefreshThemeMenu();
        RefreshThemeMenu();
        Top("Style", "S", themeMenu, Separator(), paragraphMenu, characterMenu, Separator(), StyleEditorItem());
        wrapItem = Toggle("Word Wrap", "W", b => View?.Wrap(b));
        BuildDocumentModeMenu();
        Top("View", "V", plainMode, markdownMode, codeMode, Separator(), Toggle("Show Status Bar", "S", b => preferences.Set("appearance", "showStatusBar", b), () => preferences.ShowStatus), Toggle("Show Menu Bar", "B", b => preferences.Set("windows", "showMenu", b), () => preferences.ShowMenu), Separator(), wrapItem,
            Toggle("Flow Source Paragraphs", "F", b => View?.ParagraphFlow(b), () => View?.ParagraphFlowEnabled == true), Toggle("Physical Source Lines", "L", b => View?.LineMode(b ? 1u : 0u), () => View?.CurrentLineMode == 1), Toggle("Show Invisible Characters", "I", b => View?.VisibleWhitespace(b), () => ActivePane?.WhitespaceEnabled == true),
            Separator(), ActionItem("Zoom In", "Z", () => View?.StepZoom(true), "Ctrl+="), ActionItem("Zoom Out", "O", () => View?.StepZoom(false), "Ctrl+-"), ActionItem("Actual Size", "A", () => View?.Zoom(1)),
            Separator(), ActionItem("Full Screen", "U", () => AppWindow.SetPresenter(AppWindow.Presenter.Kind == AppWindowPresenterKind.FullScreen ? AppWindowPresenterKind.Overlapped : AppWindowPresenterKind.FullScreen)));
        Top("Window", "W", ActionItem("Split Vertically", "V", () => { if (ActivePane != null) SplitPane(ActivePane, ActivePane.Document, vertical: true); }), ActionItem("Split View", "S", () => { if (ActivePane != null) SplitPane(ActivePane, ActivePane.Document); }), ActionItem("New Empty Pane", "E", () => { RequireSplitRoom(); SplitPane(ActivePane!, NewDocument()); }),
            Item("Close Other Panes", "C", () => ActivePane == null ? Task.CompletedTask : WindowCommand(ActivePane, VIEM_WINDOW_CLOSE_OTHERS, 1)), Item("Equalize Panes", "Q", () => ActivePane == null ? Task.CompletedTask : WindowCommand(ActivePane, VIEM_WINDOW_EQUALIZE_HEIGHTS, 1)),
            Separator(), ActionItem("New Window for Document", "N", () => NewWindow(ActivePane?.Document)), ActionItem("Minimize", "M", () => (AppWindow.Presenter as OverlappedPresenter)?.Minimize()));
        Top("Help", "H", Item("Viem Help", "H", () => Dialog("Viem", "A modal editor for writing.\n\nUse i to insert, Escape to return to Normal, : to enter commands, / to search, and u to undo.\n\nCtrl+C/X/V copy, cut and paste. Ctrl+Q starts Visual Block. Other vi control keys retain their meaning. Use the formatting toolbar for bold, italic and strikethrough.\n\nCtrl+W s splits the view. Ctrl+W w switches panes. :w saves; :q closes the pane.")),
            Item("About Viem", "A", () => Dialog("Viem", "Viem for Windows\nC# / WinUI 3 · DirectWrite · Rust core\n\nWindows frontend 0.1")));
        RefreshRecentMenu();
    }
    private void Select(string command) => View?.SelectFromCommand(command);
    private static string HistoryCategory(uint value) => value switch { 1 => "Typing", 2 => "Style", 3 => "Line Endings", 4 => "Move Lines", 6 => "Document Format", _ => "Edit" };
    private void ValidateMenus()
    {
        if (!menusDirty) return;
        menusDirty = false;
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
        if (View == null || paragraphMenu == null) return;
        var selected = View.SelectedNamedStyles();
        var snapshot = View.Styles(selected.Identity);
        var entries = CoreView.StyleChoices(snapshot, selected);
        foreach (var menu in new[] { paragraphMenu, characterMenu })
        {
            uint space = menu == paragraphMenu ? 1u : 2u;
            var choices = entries.Where(e => e.Key.Namespace == space).ToArray();
            var old = menu.Items.OfType<ToggleMenuFlyoutItem>().Where(i => i.Tag is StyleKey).ToArray();
            if (!old.Select(i => (StyleKey)i.Tag).SequenceEqual(choices.Select(c => c.Key)))
            {
                foreach (var item in old) menu.Items.Remove(item);
                foreach (var choice in choices)
                {
                    var item = new ToggleMenuFlyoutItem { Tag = choice.Key };
                    item.Click += (_, _) => Safe(() => {
                        if (item.CommandParameter is Viem.Windows.Interop.ViemStyleSheetIdentityV1 identity)
                            View?.ChooseStyle((StyleKey)item.Tag, identity);
                        formattingToolbar.RestoreEditorFocus(); return Task.CompletedTask;
                    });
                    menu.Items.Add(item);
                }
            }
            var items = menu.Items.OfType<ToggleMenuFlyoutItem>().Where(i => i.Tag is StyleKey).ToArray();
            for (int i = 0; i < choices.Length; i++)
            {
                items[i].Text = choices[i].Name; items[i].IsEnabled = View.HasFormattingSelection && choices[i].Enabled;
                items[i].IsChecked = choices[i].Selected; items[i].CommandParameter = snapshot.Identity;
                items[i].KeyboardAcceleratorTextOverride = CoreView.HeadingLevel(choices[i].Key) is { } level && level < 6 ? $"Ctrl+{level}" : "";
            }
        }
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
#if DEBUG
    internal StyleWindow? StyleInspector => styleInspector;
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
        themeMenu.Items.Add(Item("New theme…", "N", () => ThemeDialogs.Create(preferences, root.XamlRoot, root.RequestedTheme)));
        themeMenu.Items.Add(Item("Theme Settings…", "S", ShowThemeSettings));
    }
    private SettingsWindow? settingsWindow;
    internal Task ShowSettings() => ShowSettings(selectTheme: false);
    internal Task ShowThemeSettings() => ShowSettings(selectTheme: true);
    private Task ShowSettings(bool selectTheme)
    {
        if (settingsWindow == null) { settingsWindow = new SettingsWindow(preferences); settingsWindow.Closed += (_, _) => settingsWindow = null; }
        if (selectTheme) settingsWindow.SelectThemeCategory();
        settingsWindow.Activate(); return Task.CompletedTask;
    }
#if DEBUG
    internal SettingsWindow? SettingsInspector => settingsWindow;
#endif
}
