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
    private MenuFlyoutItem Item(string text, Func<Task> action, string shortcut = "", Func<bool>? enabled = null)
    {
        var item = new MenuFlyoutItem { Text = text, KeyboardAcceleratorTextOverride = shortcut };
        item.Loaded += (_, _) => ValidateMenus();
        item.Click += (_, _) => Safe(action);
        if (enabled != null) validation.Add((item, enabled));
        return item;
    }
    private MenuFlyoutItem ActionItem(string text, Action action, string shortcut = "", Func<bool>? enabled = null)
        => Item(text, () => { action(); ActivePane?.FocusEditor(); return Task.CompletedTask; }, shortcut, enabled);
    private ToggleMenuFlyoutItem Toggle(string text, Action<bool> action, Func<bool>? state = null)
    {
        var item = new ToggleMenuFlyoutItem { Text = text };
        item.Loaded += (_, _) => ValidateMenus();
        item.Click += (_, _) => Safe(() => { action(item.IsChecked); ActivePane?.FocusEditor(); return Task.CompletedTask; });
        if (state != null) checks.Add((item, state));
        return item;
    }
    private MenuBarItem Top(string title, string accessKey, params MenuFlyoutItemBase[] items)
    {
        var menu = new MenuBarItem { Title = title, AccessKey = accessKey };
        // Validate on demand for pointer, keyboard, access-key and UI Automation
        // opening (the items' Loaded handler), rather than on every editor move.
        menu.PointerEntered += (_, _) => ValidateMenus();
        menu.GotFocus += (_, _) => ValidateMenus();
        menu.AccessKeyInvoked += (_, _) => ValidateMenus();
        foreach (var item in items) menu.Items.Add(item);
        Menu.Items.Add(menu); return menu;
    }
    private static MenuFlyoutSubItem Sub(string title, params MenuFlyoutItemBase[] items)
    { var menu = new MenuFlyoutSubItem { Text = title }; foreach (var i in items) menu.Items.Add(i); return menu; }
    private static MenuFlyoutSeparator Separator() => new();
    private void BuildMenus()
    {
        recentMenu = new() { Text = "Open Recent" };
        Top("File", "F",
            Item("New", () => { NewWindow(null); return Task.CompletedTask; }), Item("Open…", OpenDialog), recentMenu, Separator(),
            Item("Close", () => ActivePane == null ? Task.CompletedTask : ClosePane(ActivePane)),
            Item("Save", () => ActivePane == null ? Task.CompletedTask : Save(ActivePane), "Ctrl+S"),
            Item("Save As…", () => ActivePane == null ? Task.CompletedTask : Save(ActivePane, true), "Ctrl+Shift+S"),
            Item("Export…", () => ActivePane == null ? Task.CompletedTask : Export(ActivePane), enabled: () => View != null),
            Item("Duplicate", () => { if (ActivePane != null) NewWindow(NewDocument(ActivePane.Document.Source(ActivePane.Document.State.document_revision), format: ActivePane.Document.State.format)); return Task.CompletedTask; }),
            Item("Revert to Last Saved…", () => ActivePane == null ? Task.CompletedTask : Reload(ActivePane), enabled: () => ActivePane?.Document.FilePath != null), Separator(),
            Sub("Text Encoding", new[] { "UTF-8", "Latin-1", "UTF-16 LE", "UTF-16 BE" }.Select((s, i) => ActionItem(s, () => View?.SetEncoding((uint)i + 1))).ToArray()),
            Sub("Line Endings", ActionItem("Unix (LF)", () => View?.FileFormat(1)), ActionItem("Windows (CRLF)", () => View?.FileFormat(2)), ActionItem("Classic Mac (CR)", () => View?.FileFormat(3))),
            Separator(), Item("Settings…", ShowSettings), Separator(), Item("Exit", RequestClose));
        undoItem = ActionItem("Undo", () => View?.Undo(), "Ctrl+Z", enabled: () => ActivePane != null && (ActivePane.Document.State.flags & VIEM_DOCUMENT_STATE_CAN_UNDO) != 0);
        redoItem = ActionItem("Redo", () => View?.Redo(), "Ctrl+Shift+Z", enabled: () => ActivePane != null && (ActivePane.Document.State.flags & VIEM_DOCUMENT_STATE_CAN_REDO) != 0);
        Top("Edit", "E", undoItem, redoItem, Separator(),
            Item("Cut", () => ActivePane?.Copy(true) ?? Task.CompletedTask, "Ctrl+X", () => ActivePane?.CanCut == true),
            Item("Copy", () => ActivePane?.Copy(false) ?? Task.CompletedTask, "Ctrl+C", () => ActivePane?.CanCopy == true),
            Item("Copy Source", () => ActivePane?.CopySource() ?? Task.CompletedTask, enabled: () => ActivePane?.CanCopy == true),
            Item("Paste", () => ActivePane?.Paste() ?? Task.CompletedTask, "Ctrl+V"),
            Item("Paste and Match Style", () => ActivePane?.Paste(true) ?? Task.CompletedTask, "Ctrl+Shift+V"),
            ActionItem("Delete", () =>
            {
                if (View is not { } view) return;
                if (view.IsTextSelection) view.Key(VIEM_KEY_DELETE);
                else view.SelectionCommand("d");
            }, enabled: () => View?.HasSelection == true && ActivePane?.CanCut == true), Separator(),
            Item("Select All", () => { ActivePane?.SelectAll(); return Task.CompletedTask; }),
            Sub("Select", ActionItem("Word", () => Select("viw")), ActionItem("Sentence", () => Select("vis")), ActionItem("Paragraph", () => Select("vip")), ActionItem("Hard Line", () => Select("V")), ActionItem("Visual Block", () => { uint returnMode = View?.Presentation.mode ?? VIEM_MODE_NORMAL; View?.Key(VIEM_KEY_ESCAPE); View?.Key(VIEM_KEY_CONTROL_CHARACTER, 'q'); View?.SetSelectionOrigin(VIEM_SELECTION_ORIGIN_KEY, returnMode); }, "Ctrl+Q")),
            Separator(), Sub("Find", ActionItem("Find…", () => Select("/")), ActionItem("Find and Replace…", () => { Select(":"); View?.Text("%s/"); }), ActionItem("Find Next", () => Select("n")), ActionItem("Find Previous", () => Select("N"))),
            Sub("Transformations", ActionItem("Make Uppercase", () => View?.SelectionCommand("U"), enabled: () => View?.HasSelection == true), ActionItem("Make Lowercase", () => View?.SelectionCommand("u"), enabled: () => View?.HasSelection == true), ActionItem("Toggle Case", () => View?.SelectionCommand("~"), enabled: () => View?.HasSelection == true)));
        paragraphMenu = Sub("Paragraph", ActionItem("Bulleted List", () => View?.SetList(VIEM_LIST_STYLE_BULLET), enabled: () => Rich), ActionItem("Numbered List", () => View?.SetList(VIEM_LIST_STYLE_NUMBERED), enabled: () => Rich), ActionItem("Remove List", () => View?.SetList(VIEM_LIST_STYLE_NONE), enabled: () => Rich),
            ActionItem("Indent", () => View?.IndentList(false), enabled: () => View != null && (View.ListCapabilities() & VIEM_LIST_CAN_INDENT) != 0), ActionItem("Unindent", () => View?.IndentList(true), enabled: () => View != null && (View.ListCapabilities() & VIEM_LIST_CAN_UNINDENT) != 0), Separator());
        characterMenu = Sub("Character");
        validation.Add((paragraphMenu, () => Rich));
        validation.Add((characterMenu, () => Rich));
        themeMenu = Sub("Theme"); themeMenu.Loaded += (_, _) => RefreshThemeMenu();
        RefreshThemeMenu();
        Top("Style", "S", themeMenu, Separator(), paragraphMenu, characterMenu, Separator(), StyleEditorItem());
        wrapItem = Toggle("Word Wrap", b => View?.Wrap(b));
        BuildDocumentModeMenu();
        Top("View", "V", plainMode, markdownMode, codeMode, Separator(), Toggle("Show Status Bar", b => preferences.Set("appearance", "showStatusBar", b), () => preferences.ShowStatus), Toggle("Show Menu Bar", b => preferences.Set("windows", "showMenu", b), () => preferences.ShowMenu), Separator(), wrapItem,
            Toggle("Flow Source Paragraphs", b => View?.ParagraphFlow(b), () => View?.ParagraphFlowEnabled == true), Toggle("Physical Source Lines", b => View?.LineMode(b ? 1u : 0u), () => View?.CurrentLineMode == 1), Toggle("Show Invisible Characters", b => View?.VisibleWhitespace(b), () => ActivePane?.WhitespaceEnabled == true),
            Separator(), ActionItem("Zoom In", () => View?.StepZoom(true)), ActionItem("Zoom Out", () => View?.StepZoom(false)), ActionItem("Actual Size", () => View?.Zoom(1)),
            Separator(), ActionItem("Full Screen", () => AppWindow.SetPresenter(AppWindow.Presenter.Kind == AppWindowPresenterKind.FullScreen ? AppWindowPresenterKind.Overlapped : AppWindowPresenterKind.FullScreen)));
        Top("Window", "W", ActionItem("Split View", () => { if (ActivePane != null) AddPane(ActivePane.Document, Panes.IndexOf(ActivePane) + 1); }), ActionItem("New Empty Pane", () => AddPane(NewDocument())),
            Item("Close Other Panes", () => ActivePane == null ? Task.CompletedTask : WindowCommand(ActivePane, VIEM_WINDOW_CLOSE_OTHERS, 1)), Item("Equalize Panes", () => ActivePane == null ? Task.CompletedTask : WindowCommand(ActivePane, VIEM_WINDOW_EQUALIZE_HEIGHTS, 1)),
            Separator(), ActionItem("New Window for Document", () => NewWindow(ActivePane?.Document)), ActionItem("Minimize", () => (AppWindow.Presenter as OverlappedPresenter)?.Minimize()));
        Top("Help", "H", Item("Viem Help", () => Dialog("Viem", "A modal editor for writing.\n\nUse i to insert, Escape to return to Normal, : to enter commands, / to search, and u to undo.\n\nCtrl+C/X/V copy, cut and paste. Ctrl+Q starts Visual Block. Other vi control keys retain their meaning. Use the formatting toolbar for bold, italic and strikethrough.\n\nCtrl+W s splits the view. Ctrl+W w switches panes. :w saves; :q closes the pane.")),
            Item("About Viem", () => Dialog("Viem", "Viem for Windows\nC# / WinUI 3 · DirectWrite · Rust core\n\nWindows frontend 0.1")));
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
        { var item = Item(Path.GetFileName(path), () => OpenNative(path)); ToolTipService.SetToolTip(item, path); recentMenu.Items.Add(item); }
        recentMenu.Items.Add(Separator()); recentMenu.Items.Add(ActionItem("Clear Menu", preferences.ClearRecent));
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
    private StyleWindow? styleInspector;
    private MenuFlyoutItem StyleEditorItem() => Item("Edit Styles…", () => { ShowStyles(); return Task.CompletedTask; }, "F8");
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
        var fallback = new ToggleMenuFlyoutItem { Text = "Default", IsChecked = preferences.SelectedTheme == null };
        fallback.Click += (_, _) => Safe(() => { preferences.SelectTheme(null); return Task.CompletedTask; });
        themeMenu.Items.Add(fallback);
        themeMenu.Items.Add(Item("New theme…", () => ThemeDialogs.Create(preferences, root.XamlRoot, root.RequestedTheme)));
    }
    private SettingsWindow? settingsWindow;
    internal Task ShowSettings()
    {
        if (settingsWindow == null) { settingsWindow = new SettingsWindow(preferences); settingsWindow.Closed += (_, _) => settingsWindow = null; }
        settingsWindow.Activate(); return Task.CompletedTask;
    }
#if DEBUG
    internal SettingsWindow? SettingsInspector => settingsWindow;
#endif
}
