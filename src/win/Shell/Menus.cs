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
    private MenuBarItem paragraphMenu = null!, characterMenu = null!;
    private MenuFlyoutItem undoItem = null!, redoItem = null!;
    private ToggleMenuFlyoutItem wrapItem = null!, boldItem = null!, italicItem = null!;
    private CoreView? View => ActivePane?.View;
    private bool Rich => View != null && ActivePane!.Document.State.format is VIEM_FORMAT_MARKDOWN or VIEM_FORMAT_HTML or VIEM_FORMAT_RTF;
    private bool DirectCharacter => View != null && ActivePane!.Document.State.format is VIEM_FORMAT_HTML or VIEM_FORMAT_RTF && View.LogicalSelection().kind is VIEM_LOGICAL_SELECTION_KIND_CHARACTER or VIEM_LOGICAL_SELECTION_KIND_LINE;
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
        MenuFlyoutSubItem Formats(string title, bool convert, uint[] formats) => Sub(title, formats.Select(f => ActionItem(CoreDocument.FormatName(f), () => View?.Format(f, convert), enabled: () => View != null && ActivePane!.Document.State.format != f)).ToArray());
        Top("File", "F",
            Item("New", () => { NewWindow(null); return Task.CompletedTask; }), Item("Open…", OpenDialog), recentMenu, Separator(),
            Item("Close", () => ActivePane == null ? Task.CompletedTask : ClosePane(ActivePane)),
            Item("Save", () => ActivePane == null ? Task.CompletedTask : Save(ActivePane), "Ctrl+S"),
            Item("Save As…", () => ActivePane == null ? Task.CompletedTask : Save(ActivePane, true), "Ctrl+Shift+S"),
            Item("Duplicate", () => { if (ActivePane != null) NewWindow(NewDocument(ActivePane.Document.Source(ActivePane.Document.State.document_revision), format: ActivePane.Document.State.format)); return Task.CompletedTask; }),
            Item("Revert to Last Saved…", () => ActivePane == null ? Task.CompletedTask : Reload(ActivePane), enabled: () => ActivePane?.Document.FilePath != null), Separator(),
            Formats("Convert to", true, [1, 2, 3]), Formats("Reinterpret as", false, [1, 7, 2, 5, 3, 6, 4]),
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
            ActionItem("Delete", () => { if (View?.IsVisual == true) View.Command("d"); }, enabled: () => View?.IsVisual == true && ActivePane?.CanCut == true), Separator(),
            Item("Select All", () => { ActivePane?.SelectAll(); return Task.CompletedTask; }),
            Sub("Select", ActionItem("Word", () => Select("viw")), ActionItem("Sentence", () => Select("vis")), ActionItem("Paragraph", () => Select("vip")), ActionItem("Hard Line", () => Select("V")), ActionItem("Visual Block", () => { View?.Key(VIEM_KEY_ESCAPE); View?.Key(VIEM_KEY_CONTROL_CHARACTER, 'q'); }, "Ctrl+Q")),
            Separator(), Sub("Find", ActionItem("Find…", () => Select("/")), ActionItem("Find and Replace…", () => { Select(":"); View?.Text("%s/"); }), ActionItem("Find Next", () => Select("n")), ActionItem("Find Previous", () => Select("N"))),
            Sub("Transformations", ActionItem("Make Uppercase", () => View?.Command("U"), enabled: () => View?.IsVisual == true), ActionItem("Make Lowercase", () => View?.Command("u"), enabled: () => View?.IsVisual == true), ActionItem("Toggle Case", () => View?.Command("~"), enabled: () => View?.IsVisual == true)));
        boldItem = Toggle("Bold", _ => View?.ToggleSemantic(VIEM_SEMANTIC_STYLE_STRONG));
        italicItem = Toggle("Italic", _ => View?.ToggleSemantic(VIEM_SEMANTIC_STYLE_EMPHASIS));
        validation.Add((boldItem, () => CanSemantic(VIEM_SEMANTIC_STYLE_STRONG))); validation.Add((italicItem, () => CanSemantic(VIEM_SEMANTIC_STYLE_EMPHASIS)));
        Top("Format", "O", Item("Font…", ShowFontDialog, enabled: () => DirectCharacter), boldItem, italicItem,
            ActionItem("Underline", () => ToggleDecoration(VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE), enabled: () => DirectCharacter),
            ActionItem("Strikethrough", () => ToggleDecoration(VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH), enabled: () => DirectCharacter),
            Item("Text Color…", () => ShowColor(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND), enabled: () => DirectCharacter), Item("Highlight Color…", () => ShowColor(VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND), enabled: () => DirectCharacter),
            Separator(), Sub("Style", StyleEditorItem(), ActionItem("Save as Default Style", SaveStyleDefaults), ActionItem("Reload Code Style Sheet", LoadCodeStyles)),
            Separator(), Sub("Paragraph", Sub("Alignment", ParagraphAction("Start", VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT, VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT, 1), ParagraphAction("Center", VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT, VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT, 3), ParagraphAction("End", VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT, VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT, 2)),
                Sub("Writing Direction", ParagraphAction("Automatic", VIEM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION, VIEM_STYLE_VALUE_WRITING_DIRECTION, 0), ParagraphAction("Left to Right", VIEM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION, VIEM_STYLE_VALUE_WRITING_DIRECTION, 1), ParagraphAction("Right to Left", VIEM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION, VIEM_STYLE_VALUE_WRITING_DIRECTION, 2)),
                Sub("Line Spacing", Spacing("Normal", 1, 0), Spacing("Single", 2, 1), Spacing("1.5 Lines", 2, 1.5f), Spacing("Double", 2, 2))));
        paragraphMenu = Top("Paragraph", "P", ActionItem("Bulleted List", () => View?.SetList(VIEM_LIST_STYLE_BULLET), enabled: () => Rich), ActionItem("Numbered List", () => View?.SetList(VIEM_LIST_STYLE_NUMBERED), enabled: () => Rich), ActionItem("Remove List", () => View?.SetList(VIEM_LIST_STYLE_NONE), enabled: () => Rich),
            ActionItem("Indent", () => View?.IndentList(false), enabled: () => View != null && (View.ListCapabilities() & VIEM_LIST_CAN_INDENT) != 0), ActionItem("Unindent", () => View?.IndentList(true), enabled: () => View != null && (View.ListCapabilities() & VIEM_LIST_CAN_UNINDENT) != 0), Separator());
        for (uint level = 0; level <= 6; level++) { uint l = level; paragraphMenu.Items.Add(ActionItem(l == 0 ? "Base Paragraph" : $"Heading {l}", () => View?.SetParagraph(l), l < 6 ? $"Ctrl+{l}" : "", () => Rich)); }
        paragraphMenu.Items.Add(Separator()); paragraphMenu.Items.Add(StyleEditorItem());
        characterMenu = Top("Character", "C", ActionItem("Default Paragraph", () => View?.AssignStyle(2, ""), enabled: () => Rich), Separator(), StyleEditorItem());
        wrapItem = Toggle("Word Wrap", b => View?.Wrap(b));
        Top("View", "V", Toggle("Show Status Bar", b => preferences.Set("appearance", "showStatusBar", b), () => preferences.ShowStatus), Toggle("Show Menu Bar", b => preferences.Set("windows", "showMenu", b), () => preferences.ShowMenu), Separator(), wrapItem,
            Toggle("Flow Source Paragraphs", b => View?.ParagraphFlow(b), () => View?.ParagraphFlowEnabled == true), Toggle("Physical Source Lines", b => View?.LineMode(b ? 1u : 0u), () => View?.CurrentLineMode == 1), Toggle("Show Invisible Characters", b => View?.VisibleWhitespace(b), () => ActivePane?.WhitespaceEnabled == true),
            Separator(), ActionItem("Zoom In", () => View?.StepZoom(true)), ActionItem("Zoom Out", () => View?.StepZoom(false)), ActionItem("Actual Size", () => View?.Zoom(1)),
            Separator(), ActionItem("Full Screen", () => AppWindow.SetPresenter(AppWindow.Presenter.Kind == AppWindowPresenterKind.FullScreen ? AppWindowPresenterKind.Overlapped : AppWindowPresenterKind.FullScreen)));
        Top("Window", "W", ActionItem("Split View", () => { if (ActivePane != null) AddPane(ActivePane.Document, Panes.IndexOf(ActivePane) + 1); }), ActionItem("New Empty Pane", () => AddPane(NewDocument())),
            Item("Close Other Panes", () => ActivePane == null ? Task.CompletedTask : WindowCommand(ActivePane, VIEM_WINDOW_CLOSE_OTHERS, 1)), Item("Equalize Panes", () => ActivePane == null ? Task.CompletedTask : WindowCommand(ActivePane, VIEM_WINDOW_EQUALIZE_HEIGHTS, 1)),
            Separator(), ActionItem("New Window for Document", () => NewWindow(ActivePane?.Document)), ActionItem("Minimize", () => (AppWindow.Presenter as OverlappedPresenter)?.Minimize()));
        Top("Help", "H", Item("Viem Help", () => Dialog("Viem", "A modal editor for writing.\n\nUse i to insert, Escape to return to Normal, : to enter commands, / to search, and u to undo.\n\nCtrl+C/X/V copy, cut and paste. Ctrl+Q starts Visual Block. Other vi control keys retain their meaning. Use the menus for formatting shortcuts that conflict with vi.\n\nCtrl+W s splits the view. Ctrl+W w switches panes. :w saves; :q closes the pane.")),
            Item("About Viem", () => Dialog("Viem", "Viem for Windows\nC# / WinUI 3 · DirectWrite · Rust core\n\nWindows frontend 0.1")));
        RefreshRecentMenu();
    }
    private void Select(string command) { View?.Key(VIEM_KEY_ESCAPE); View?.Command(command); }
    private bool CanSemantic(uint style) { if (View == null) return false; var p = View.SemanticStyle(style); return (p.flags & (VIEM_SEMANTIC_STYLE_CAN_SET | VIEM_SEMANTIC_STYLE_CAN_CLEAR)) != 0; }
    private void ToggleDecoration(uint property) => View?.DirectStyle(property, CoreView.Enum(VIEM_STYLE_VALUE_BOOLEAN, View.DecorationState(property) == 1 ? 0u : 1u));
    private MenuFlyoutItem ParagraphAction(string text, uint property, uint kind, uint value) => ActionItem(text, () => View?.DirectStyle(property, CoreView.Enum(kind, value)), enabled: () => Rich);
    private MenuFlyoutItem Spacing(string name, uint kind, float amount) => ActionItem(name, () => SetSpacing(kind, amount), enabled: () => Rich);
    private static string HistoryCategory(uint value) => value switch { 1 => "Typing", 2 => "Style", 3 => "Line Endings", 4 => "Move Lines", 6 => "Document Format", _ => "Edit" };
    private void ValidateMenus()
    {
        if (!menusDirty) return;
        menusDirty = false;
        using var measurement = Diagnostics.InputPerformance.Measure("menus");
        foreach (var (item, enabled) in validation) { try { item.IsEnabled = enabled(); } catch { item.IsEnabled = false; } }
        foreach (var (item, checkedValue) in checks) { try { item.IsChecked = checkedValue(); } catch { } }
        if (View == null || undoItem == null) return;
        var state = ActivePane!.Document.State;
        undoItem.Text = "Undo " + HistoryCategory(state.undo_action_category); redoItem.Text = "Redo " + HistoryCategory(state.redo_action_category);
        try { wrapItem.IsChecked = (View.Viewport.flags & VIEM_VIEWPORT_STATE_WRAP) != 0; boldItem.IsChecked = View.SemanticStyle(1).state == 1; italicItem.IsChecked = View.SemanticStyle(2).state == 1; } catch { }
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
        // Dynamic definitions are inserted only when the backing sheet changes,
        // never while native menu tracking is in progress.
        foreach (var menu in new[] { paragraphMenu, characterMenu })
            foreach (var old in menu.Items.Where(i => i.Tag is string).ToArray()) { menu.Items.Remove(old); validation.RemoveAll(v => v.Item == old); }
        foreach (var style in View.Styles().Styles.Where(s => (s.Native.flags & (VIEM_STYLE_DEFINITION_INTERNAL | VIEM_STYLE_DEFINITION_INTERNAL_LIST)) == 0 && s.Has(VIEM_STYLE_CAPABILITY_ASSIGN)))
        {
            var menu = style.Namespace == 1 ? paragraphMenu : characterMenu;
            if (menu.Items.OfType<MenuFlyoutItem>().Any(i => (i.Tag as string) == style.Id)) continue;
            var item = ActionItem(style.Name, () => View?.AssignStyle(style.Namespace, style.Id), enabled: () => Rich); item.Tag = style.Id; menu.Items.Insert(menu.Items.Count - 2, item);
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
    private void SaveStyleDefaults()
    {
        if (View == null) return;
        string name = View.UsesGlobalStyles ? "code_style.json" : CoreDocument.FormatName(ActivePane!.Document.State.format).Replace(" Source", "").ToLowerInvariant() + "_style.json";
        Preferences.AtomicWrite(Path.Combine(preferences.DirectoryPath, name), View.ExportStyleDefaults()); ActivePane?.SetMessage("Saved " + name);
    }
    private unsafe void LoadCodeStyles()
    {
        string path = Path.Combine(preferences.DirectoryPath, "code_style.json");
        if (!File.Exists(path)) return;
        byte[] bytes = File.ReadAllBytes(path); fixed (byte* p = bytes) Viem.Windows.Interop.Abi.Check(viem_code_replace_style_json(p, (ulong)bytes.Length), "Load Code styles");
        foreach (var doc in App.Instance.Windows.SelectMany(w => w.Panes).Select(p => p.Document).Distinct()) doc.NotifyChanged();
    }
    private unsafe void SetSpacing(uint kind, float amount) { var v = CoreView.Enum(VIEM_STYLE_VALUE_LINE_SPACING, kind); v.number = amount; View?.DirectStyle(VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING, v); }
    private async Task ShowFontDialog()
    {
        if (View == null) return;
        var view = View;
        var current = view.Typography();
        string chosen = current.Family.Length == 0 ? "Segoe UI" : current.Family;
        var family = new ComboBox { IsEditable = true, Width = 300, ItemsSource = FontCatalog.Families, Text = FontCatalog.DisplayFamily(chosen), Header = "Font family" };
        family.SelectedItem = FontCatalog.Families.FirstOrDefault(f => string.Equals(f, family.Text, StringComparison.OrdinalIgnoreCase));
        var variant = new ComboBox { Header = "Variant", Width = 300, PlaceholderText = "Custom / mixed", ItemsSource = FontCatalog.Faces(chosen), SelectedItem = FontCatalog.Current(chosen, current.Info.base_weight, current.Info.slant) };
        void ChooseFamily(string value) {
            if (string.Equals(value, FontCatalog.DisplayFamily(chosen), StringComparison.OrdinalIgnoreCase)) return;
            var face = FontCatalog.ForFamilyChange(value, variant.SelectedItem as FontFace);
            chosen = value; variant.ItemsSource = FontCatalog.Faces(value); variant.SelectedItem = face;
        }
        family.SelectionChanged += (_, _) => { if (family.SelectedItem is string value) ChooseFamily(value); };
        family.LostFocus += (_, _) => ChooseFamily(family.Text.Trim());
        var size = new NumberBox { Value = current.Info.size, Minimum = 1, Maximum = 256, Header = "Size (DIPs)", SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
        var stack = new StackPanel { Spacing = 12 }; stack.Children.Add(family); stack.Children.Add(variant); stack.Children.Add(size);
        var dialog = new ContentDialog { XamlRoot = root.XamlRoot, Title = "Font", Content = stack, PrimaryButtonText = "Apply", CloseButtonText = "Cancel" };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary) { ChooseFamily(family.Text.Trim()); view.SetFont(chosen, (float)size.Value, variant.SelectedItem as FontFace); }
    }
    private async Task ShowColor(uint property)
    {
        var picker = new ColorPicker { IsAlphaEnabled = true, IsColorSpectrumVisible = true };
        var dialog = new ContentDialog { XamlRoot = root.XamlRoot, Title = property == VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND ? "Text Color" : "Highlight Color", Content = picker, PrimaryButtonText = "Apply", SecondaryButtonText = "Default", CloseButtonText = "Cancel" };
        var result = await dialog.ShowAsync();
        if (result == ContentDialogResult.Primary) SetColor(property, picker.Color);
        else if (result == ContentDialogResult.Secondary) View?.DirectStyle(property, default, true);
    }
    private unsafe void SetColor(uint property, global::Windows.UI.Color color)
    { var v = CoreView.Enum(VIEM_STYLE_VALUE_COLOR, 0); v.color = new() { red = color.R / 255f, green = color.G / 255f, blue = color.B / 255f, alpha = color.A / 255f }; View?.DirectStyle(property, v); }
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
