using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Windowing;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Viem.Windows.Rendering;
using Windows.Graphics;
using Windows.UI.Text;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

/// <summary>A modeless native inspector over the core-owned stylesheet.</summary>
internal sealed partial class StyleWindow : Window
{
    private CoreView view;
    private readonly Preferences preferences;
    private StyleSheet sheet = null!;
    private StyleDefinition selected = null!;
    private bool loading, updating, closed;
    private const int ClientWidth = 680;
    private readonly StackPanel root = new() { Padding = new(24, 16, 24, 14), Spacing = 6, Width = ClientWidth, VerticalAlignment = VerticalAlignment.Top };
    private readonly ComboBox stylePicker = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly TextBox name = new();
    private readonly TextBlock kind = new() { VerticalAlignment = VerticalAlignment.Center, Opacity = .65 };
    private readonly ComboBox parent = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly ComboBox next = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly Button delete = new() { Content = "Delete", Width = 68 };
    private readonly DropDownButton create = new() { Content = "New", Width = 72, VerticalAlignment = VerticalAlignment.Center };
    private readonly MenuFlyoutItem createParagraph = new() { Text = "Paragraph style" };
    private readonly Button visitParent = NavigationButton();
    private readonly Button visitNext = NavigationButton();
    private readonly Button restoreDefaults = new() { Content = "Restore Defaults" };
    private readonly StackPanel character = new() { Spacing = 12 };
    private readonly StackPanel paragraph = new() { Spacing = 8 };
    private readonly StackPanel block = new() { Spacing = 8 };
    private readonly CanvasControl preview = new() { Height = 200 };
    private BlockStylePreview? blockPreview;
    private readonly TextBlock error = new() { TextWrapping = TextWrapping.Wrap, FontSize = 12, Visibility = Visibility.Collapsed };
    private readonly List<Action> refreshFields = [];

    public StyleWindow(CoreView initialView, Preferences preferences, bool followCaret = true)
    {
        view = initialView; this.preferences = preferences;
        Title = view.UsesGlobalStyles ? "Code Styles" : "Document Styles";
        var scroll = new ScrollViewer { Content = root, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, RequestedTheme = preferences.Midnight ? ElementTheme.Dark : ElementTheme.Light, Background = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(32, 32, 32) : Theme.Rgb(250, 250, 250)) };
        Content = scroll;
        root.RequestedTheme = preferences.Midnight ? ElementTheme.Dark : ElementTheme.Light;
        if (AppWindow.Presenter is OverlappedPresenter presenter) { presenter.IsResizable = false; presenter.IsMaximizable = false; }
        preview.ClearColor = preferences.Theme.Background;
        WindowSizing.Appearance(this, preferences.Midnight);
        var properties = new Grid { RowSpacing = 5, ColumnSpacing = 10, Margin = new(12, 0, 12, 4) };
        properties.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); properties.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        var choiceRow = new Grid { ColumnSpacing = 6 };
        choiceRow.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); choiceRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); choiceRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        choiceRow.Children.Add(stylePicker);
        Grid.SetColumn(create, 1); choiceRow.Children.Add(create); Grid.SetColumn(delete, 2); choiceRow.Children.Add(delete);
        stylePicker.VerticalAlignment = delete.VerticalAlignment = VerticalAlignment.Center;
        var createMenu = new MenuFlyout();
        foreach (var (title, space) in new[] { ("Paragraph style", 1u), ("Character style", 2u) })
        { var item = space == 1 ? createParagraph : new MenuFlyoutItem { Text = title }; item.Click += (_, _) => Try(() => { string id = view.CreateStyle(space, "New " + title); CancelCaretFollow(); Load(new(space, id)); }); createMenu.Items.Add(item); }
        create.Flyout = createMenu;
        Field(properties, "Style", choiceRow); Field(properties, "Name", name); Field(properties, "Style type", kind); Field(properties, "Based on", Relationship(parent, visitParent)); Field(properties, "Next paragraph", Relationship(next, visitNext));
        Add(properties);
        Add(availability);
        Add(Separator());
        var tabs = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Center, Spacing = 2 };
        tabs.Children.Add(characterTab); tabs.Children.Add(paragraphTab); tabs.Children.Add(blockTab); Add(tabs);
        var fields = new Grid(); fields.Children.Add(character); fields.Children.Add(paragraph); fields.Children.Add(block);
        Add(new Border { Child = new ScrollViewer { Content = fields, VerticalScrollBarVisibility = ScrollBarVisibility.Auto }, Height = 320, Padding = new(12, 12, 12, 10), BorderBrush = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(58, 58, 58) : Theme.Rgb(210, 210, 210)), BorderThickness = new(1), CornerRadius = new(6) });
        characterTab.Click += (_, _) => SelectTab(false); paragraphTab.Click += (_, _) => SelectTab(true); SelectTab(false);
        characterTab.Checked += (_, _) => SelectTab(false); paragraphTab.Checked += (_, _) => SelectTab(true);
        blockTab.Click += (_, _) => SelectTab(false, true); blockTab.Checked += (_, _) => SelectTab(false, true);
        BuildCharacter(); BuildParagraph(); BuildBlock();
        Add(new Border { Child = preview, BorderBrush = new SolidColorBrush(Theme.Rgb(64, 70, 80)), BorderThickness = new(1), CornerRadius = new(5) });
        preview.Draw += (_, e) => DrawPreview(e.DrawingSession);
        Add(error);
        var buttons = new Grid(); buttons.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); buttons.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        restoreDefaults.HorizontalAlignment = HorizontalAlignment.Left;
        restoreDefaults.Click += (_, _) => Try(() => view.ReplaceCodeStyles([])); buttons.Children.Add(restoreDefaults);
        var close = new Button { Content = "Close", MinWidth = 82 }; close.Click += (_, _) => Close(); buttons.Children.Add(close); Grid.SetColumn(close, 1); Add(buttons);
        visitParent.Click += (_, _) => Navigate(new(selected.Namespace, selected.Parent));
        visitNext.Click += (_, _) => Navigate(new(1, selected.Next));
        stylePicker.SelectionChanged += (_, _) => { if (!loading && stylePicker.SelectedItem is StyleDefinition style) Navigate(style.Key); };
        name.LostFocus += (_, _) => { if (!loading && name.Text != selected.Name) Try(() => view.EditStyleString(selected, VIEM_STYLE_EDIT_SET_DISPLAY_NAME, 0, name.Text)); };
        parent.SelectionChanged += (_, _) => { if (loading) return; Try(() => { string id = parent.SelectedItem is StyleDefinition p ? p.Id : ""; view.EditStyleString(selected, id.Length == 0 ? VIEM_STYLE_EDIT_CLEAR_PARENT : VIEM_STYLE_EDIT_SET_PARENT, 0, id); }); };
        next.SelectionChanged += (_, _) => { if (loading) return; Try(() => { string id = next.SelectedItem is StyleDefinition p ? p.Id : ""; view.EditStyleString(selected, id.Length == 0 ? VIEM_STYLE_EDIT_CLEAR_NEXT_STYLE : VIEM_STYLE_EDIT_SET_NEXT_STYLE, 0, id); }); };
        delete.Click += (_, _) => Try(() => { view.DeleteStyle(selected); Load(); });
        AttachView(followCaret);
        preferences.Changed += ThemeChanged;
        AppWindow.Closing += (_, args) => {
            if (visibleColorPickers.Count == 0) return;
            args.Cancel = true; Close();
        };
        Closed += (_, _) => { DetachView(); DismissColorPickers(); closed = true; preferences.Changed -= ThemeChanged; blockPreview?.Dispose(); blockPreview = null; preview.RemoveFromVisualTree(); };
        Load(followCaret: followCaret);
    }
    private readonly TextBlock availability = new() { FontSize = 12, Opacity = .65, TextWrapping = TextWrapping.Wrap };
    private readonly ToggleButton characterTab = new() { Content = "Character", FontSize = 13, Width = 130, Padding = new(12, 3, 12, 3) };
    private readonly ToggleButton paragraphTab = new() { Content = "Paragraph", FontSize = 13, Width = 130, Padding = new(12, 3, 12, 3) };
    private readonly ToggleButton blockTab = new() { Content = "Block", FontSize = 13, Width = 130, Padding = new(12, 3, 12, 3) };
    private void SelectTab(bool paragraphSelected, bool blockSelected = false)
    {
        characterTab.IsChecked = !paragraphSelected && !blockSelected; paragraphTab.IsChecked = paragraphSelected; blockTab.IsChecked = blockSelected;
        character.Visibility = paragraphSelected || blockSelected ? Visibility.Collapsed : Visibility.Visible;
        paragraph.Visibility = paragraphSelected ? Visibility.Visible : Visibility.Collapsed;
        block.Visibility = blockSelected ? Visibility.Visible : Visibility.Collapsed;
    }
    internal void Retarget(CoreView nextView)
    {
        DismissColorPickers();
        if (!CommitPendingName()) return;
        DetachView();
        view = nextView; AttachView(true);
        Load(followCaret: true);
    }
    private void Add(FrameworkElement item) => root.Children.Add(item);
    private Border Separator() => new() { Height = 1, Background = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(58, 58, 58) : Theme.Rgb(210, 210, 210)) };
    private static Button NavigationButton() => new() { Content = "↗", Width = 30, Padding = new(0), FontSize = 18, VerticalAlignment = VerticalAlignment.Center };
    private static Grid Relationship(ComboBox picker, Button visit)
    {
        var row = new Grid { ColumnSpacing = 6 };
        row.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        row.Children.Add(picker); Grid.SetColumn(visit, 1); row.Children.Add(visit); return row;
    }
    private void Navigate(StyleKey key)
    {
        if (key.Id.Length == 0) return;
        DismissColorPickers();
        if (!CommitPendingName()) return;
        CancelCaretFollow();
        if (key != selected.Key) Load(key);
    }
    private void UpdateNavigation(Button button, StyleKey key, string relationship)
    {
        var destination = sheet.Styles.FirstOrDefault(s => s.Key == key && s.Key != selected.Key);
        button.IsEnabled = destination != null;
        string label = destination == null ? "Go to " + relationship : "Go to " + destination.Name;
        AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label);
    }
    private bool IsDescendant(StyleDefinition style)
    {
        var visited = new HashSet<StyleKey>();
        while (visited.Add(style.Key)) {
            if (style.Key == selected.Key) return true;
            var ancestor = sheet.Styles.FirstOrDefault(s => s.Id == style.Parent && s.Namespace == style.Namespace);
            if (ancestor == null) break; style = ancestor;
        }
        return false;
    }
    private static void Field(Grid grid, string label, FrameworkElement value)
    { int row = grid.RowDefinitions.Count; grid.RowDefinitions.Add(new() { Height = GridLength.Auto, MinHeight = 24 }); var text = new TextBlock { Text = label, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Right }; Grid.SetRow(text, row); grid.Children.Add(text); Grid.SetRow(value, row); Grid.SetColumn(value, 1); grid.Children.Add(value); }
    private void Load(StyleKey? key = null, bool followCaret = false, StyleSheet? snapshot = null)
    {
        if (view.Id == 0) { EnableChildren(root, false); return; }
        loading = true;
        try
        {
            sheet = snapshot ?? view.Styles();
            if (followCaret) key = CurrentCaretStyle(sheet);
#if DEBUG
            StyleLoads++;
#endif
            Title = view.UsesGlobalStyles ? "Code Styles" : "Document Styles";
            availability.Text = view.UsesGlobalStyles ? "Shared by every Code document. Changes are saved to code_style.json." : "";
            availability.Visibility = view.UsesGlobalStyles ? Visibility.Visible : Visibility.Collapsed;
            restoreDefaults.Visibility = availability.Visibility;
            create.IsEnabled = view.UsesGlobalStyles || view.Document.State.format is VIEM_FORMAT_RTF;
            createParagraph.Visibility = view.UsesGlobalStyles ? Visibility.Collapsed : Visibility.Visible;
            var styles = sheet.Styles.Where(s => s.Native.role != VIEM_STYLE_ROLE_DOCUMENT && (s.Native.flags & VIEM_STYLE_DEFINITION_INTERNAL) == 0).ToArray();
            var chosen = styles.FirstOrDefault(s => s.Key == key) ?? styles.FirstOrDefault(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0) ?? styles[0];
            if (selected != null && (selected.Id != chosen.Id || selected.Namespace != chosen.Namespace)) DismissColorPickers();
            selected = chosen;
            var catalogue = new List<object>();
            foreach (var (title, entries) in new[] {
                ("Paragraph", styles.Where(s => s.Native.role == VIEM_STYLE_ROLE_PARAGRAPH)),
                ("Container", styles.Where(s => s.Native.role >= VIEM_STYLE_ROLE_QUOTE)),
                ("Character", styles.Where(s => s.Namespace == 2))
            }) {
                var group = entries.OrderBy(s => s.Name, StringComparer.CurrentCultureIgnoreCase).ToArray();
                if (group.Length == 0) continue;
                catalogue.Add(new ComboBoxItem { Content = title, IsEnabled = false });
                catalogue.AddRange(group);
            }
            stylePicker.ItemsSource = catalogue; stylePicker.SelectedItem = selected;
            name.Text = selected.Name; name.IsReadOnly = !selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME); kind.Text = selected.Namespace == 2 ? "Character" : selected.Native.role >= VIEM_STYLE_ROLE_QUOTE ? "Container" : "Paragraph";
            var parents = new object[] { selected.Namespace == 2 ? "Default Paragraph" : "None" }.Concat(styles.Where(s => s.Namespace == selected.Namespace && (s.Native.role == selected.Native.role || (selected.Native.role >= VIEM_STYLE_ROLE_QUOTE && (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0)) && !IsDescendant(s))).ToArray(); parent.ItemsSource = parents; parent.SelectedItem = parents.OfType<StyleDefinition>().FirstOrDefault(s => s.Id == selected.Parent) ?? parents[0]; parent.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_EDIT_PARENT);
            var following = new object[] { "Same Style" }.Concat(styles.Where(s => s.Native.role == VIEM_STYLE_ROLE_PARAGRAPH)).ToArray(); next.ItemsSource = following; next.SelectedItem = following.OfType<StyleDefinition>().FirstOrDefault(s => s.Id == selected.Next) ?? following[0]; next.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_EDIT_NEXT_STYLE);
            delete.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_DELETE);
            UpdateNavigation(visitParent, new(selected.Namespace, selected.Parent), "parent style"); UpdateNavigation(visitNext, new(1, selected.Next), "next paragraph style");
            paragraphTab.IsEnabled = selected.Namespace == 1;
            if (!paragraphTab.IsEnabled) SelectTab(false);
            blockTab.IsEnabled = selected.Namespace == 1;
            if (selected.Namespace != 1 && blockTab.IsChecked == true) SelectTab(false);
            EnableChildren(paragraph, true); EnableChildren(character, true); EnableChildren(block, true);
            foreach (var refresh in refreshFields) refresh();
            if (selected.Namespace != 1 || !selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS)) EnableChildren(paragraph, false);
            if (!selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS)) EnableChildren(character, false);
            if (selected.Namespace != 1 || !selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS)) EnableChildren(block, false);
            // Measure the populated controls before the first Activate. Resizing
            // from SizeChanged exposed successive startup layouts and flashes.
            // Later loads resize only when guidance/error rows change the size.
            root.Measure(new global::Windows.Foundation.Size(ClientWidth, double.PositiveInfinity));
            if (!AppWindow.IsVisible)
            {
                // Initial template bindings settle during arrange. Complete that
                // hidden layout before using DesiredSize for the native window.
                root.Arrange(new global::Windows.Foundation.Rect(0, 0, ClientWidth, root.DesiredSize.Height));
                root.UpdateLayout();
                root.Measure(new global::Windows.Foundation.Size(ClientWidth, double.PositiveInfinity));
            }
            WindowSizing.FitClient(this, ClientWidth, (int)Math.Ceiling(root.DesiredSize.Height));
            preview.Invalidate();
        }
        finally { loading = false; }
    }
    private static void EnableChildren(Panel panel, bool enabled)
    {
        foreach (var child in panel.Children) { if (child is Control control) control.IsEnabled = enabled; if (child is Panel nested) EnableChildren(nested, enabled); }
    }
    private bool Try(Action action, bool reload = true)
    {
        if (loading || closed) return false;
        if (reload) DismissColorPickers();
        updating = true;
        byte[]? before = null;
        try
        {
            if (view.UsesGlobalStyles) before = view.ExportStyleDefaults();
            error.Text = ""; error.Visibility = Visibility.Collapsed; action(); var key = selected.Key;
            if (view.UsesGlobalStyles) { Preferences.AtomicWrite(Path.Combine(preferences.DirectoryPath, "code_style.json"), view.ExportStyleDefaults()); foreach (var doc in App.Instance.Windows.SelectMany(w => w.Panes).Select(p => p.Document).Distinct()) doc.NotifyChanged(); }
            if (reload) Load(key); else RefreshCommittedColors();
            return true;
        }
        catch (Exception e) {
            if (before != null) view.ReplaceCodeStyles(before);
            error.Text = e.Message; error.Visibility = Visibility.Visible;
            if (reload) Load(selected.Key); else RefreshCommittedColors();
            return false;
        }
        finally { updating = false; }
    }
    private void DrawPreview(CanvasDrawingSession drawing)
    {
        drawing.Clear(preferences.Theme.Background);
        if (selected == null) return;
        if (selected.Namespace == 1) {
            try {
                if (blockPreview?.Role != selected.Native.role) {
                    blockPreview?.Dispose(); blockPreview = null;
                    blockPreview = new(selected.Native.role, preview.Device, DispatcherQueue);
                }
                blockPreview.Update(sheet, selected, preferences.Theme.Foreground);
                blockPreview.Draw(drawing, (float)preview.ActualWidth, (float)preview.ActualHeight, preferences.Theme.Foreground);
            } catch (Exception exception) {
                using var message = new CanvasTextFormat { FontSize = 12 };
                drawing.DrawText("Style preview: " + exception.Message, 12, 12, preferences.Theme.Foreground, message);
            }
            return;
        }
        blockPreview?.Dispose(); blockPreview = null;
        string family = sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES));
        var resolved = FontCatalog.Resolve(family) ?? (Family: "Segoe UI", Stretch: FontStretch.Normal);
        float size = selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number;
        uint script = selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SCRIPT_POSITION).enum_value;
        using var format = new CanvasTextFormat { FontFamily = resolved.Family, FontStretch = resolved.Stretch, FontSize = DirectWriteProvider.ScriptSize(size, script), WordWrapping = CanvasWordWrapping.Wrap };
        uint weight = selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value;
        if (selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_BOLD).enum_value != 0) weight = Math.Min(1000, weight + 300);
        format.FontWeight = new FontWeight { Weight = (ushort)Math.Clamp(weight, 1, 999) };
        format.FontStyle = selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SLANT).enum_value switch { 1 => FontStyle.Italic, 2 => FontStyle.Oblique, _ => FontStyle.Normal };
        format.HorizontalAlignment = selected.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT).enum_value switch { 2 => CanvasHorizontalAlignment.Right, 3 => CanvasHorizontalAlignment.Center, _ => CanvasHorizontalAlignment.Left };
        string sample = selected.Native.role >= VIEM_STYLE_ROLE_QUOTE
            ? "A first paragraph inside this container.\n\nA second paragraph shares its block box."
            : "A calm writing surface shaped with the selected style,\nwith line spacing and alignment visible.";
        float Edge(uint key) => selected.Namespace == 1 ? Math.Clamp(selected.Value(key).number, 0, 24) : 0;
        float left = Edge(VIEM_STYLE_PROPERTY_BLOCK_MARGIN_LEFT), right = Edge(VIEM_STYLE_PROPERTY_BLOCK_MARGIN_RIGHT);
        float borderLeft = Edge(VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH), borderRight = Edge(VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_WIDTH);
        float borderTop = Edge(VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_WIDTH), borderBottom = Edge(VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_WIDTH);
        float paddingLeft = Edge(VIEM_STYLE_PROPERTY_BLOCK_PADDING_LEFT), paddingRight = Edge(VIEM_STYLE_PROPERTY_BLOCK_PADDING_RIGHT);
        float boxX = 20 + left, boxY = 36 + Edge(VIEM_STYLE_PROPERTY_BLOCK_MARGIN_TOP);
        float boxWidth = Math.Max(1, (float)preview.ActualWidth - 40 - left - right);
        float textX = boxX + borderLeft + paddingLeft;
        float textY = boxY + borderTop + Edge(VIEM_STYLE_PROPERTY_BLOCK_PADDING_TOP);
        using var layout = new CanvasTextLayout(preview.Device, sample, format, Math.Max(1, boxWidth - borderLeft - borderRight - paddingLeft - paddingRight), 76);
        layout.SetUnderline(0, sample.Length, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE).enum_value != 0);
        layout.SetStrikethrough(0, sample.Length, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH).enum_value != 0);
        layout.SetCharacterSpacing(0, sample.Length, 0, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING).number, 0);
        using var context = new CanvasTextFormat { FontFamily = "Segoe UI", FontSize = 12 };
        var muted = preferences.Theme.Foreground; muted.A = 190;
        drawing.DrawText("Previous paragraph gives the style context.", 20, 20, muted, context);
        float sampleY = textY - DirectWriteProvider.ScriptOffset(size, script);
        float boxHeight = textY - boxY + (float)layout.LayoutBounds.Height + Edge(VIEM_STYLE_PROPERTY_BLOCK_PADDING_BOTTOM) + borderBottom;
        var blockBackground = PreviewColor(VIEM_STYLE_PROPERTY_BLOCK_BACKGROUND);
        if (blockBackground.A > 0) drawing.FillRectangle(boxX, boxY, boxWidth, boxHeight, blockBackground);
        if (borderTop > 0) drawing.FillRectangle(boxX, boxY, boxWidth, borderTop, PreviewColor(VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_COLOR));
        if (borderBottom > 0) drawing.FillRectangle(boxX, boxY + boxHeight - borderBottom, boxWidth, borderBottom, PreviewColor(VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_COLOR));
        if (borderLeft > 0) drawing.FillRectangle(boxX, boxY, borderLeft, boxHeight, PreviewColor(VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_COLOR));
        if (borderRight > 0) drawing.FillRectangle(boxX + boxWidth - borderRight, boxY, borderRight, boxHeight, PreviewColor(VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_COLOR));
        var background = PreviewColor(VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND);
        if (background.A > 0) drawing.FillRectangle(textX, sampleY, (float)layout.LayoutBounds.Width, (float)layout.LayoutBounds.Height, background);
        drawing.DrawTextLayout(layout, textX, sampleY, PreviewColor(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND));
        drawing.DrawText("Following paragraph shows spacing and inheritance.", 20, boxY + boxHeight + 6 + Edge(VIEM_STYLE_PROPERTY_BLOCK_MARGIN_BOTTOM), muted, context);
    }
}
