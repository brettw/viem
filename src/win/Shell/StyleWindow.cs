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
    private readonly CanvasControl preview = new() { Height = 140 };
    private readonly TextBlock error = new() { TextWrapping = TextWrapping.Wrap, FontSize = 12, Visibility = Visibility.Collapsed };
    private readonly List<Action> refreshFields = [];

    public StyleWindow(CoreView view, Preferences preferences)
    {
        this.view = view; this.preferences = preferences;
        Title = view.UsesGlobalStyles ? "Code Styles" : "Document Styles";
        var scroll = new ScrollViewer { Content = root, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, RequestedTheme = preferences.Midnight ? ElementTheme.Dark : ElementTheme.Light, Background = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(32, 32, 32) : Theme.Rgb(250, 250, 250)) };
        Content = scroll;
        root.RequestedTheme = preferences.Midnight ? ElementTheme.Dark : ElementTheme.Light;
        WindowSizing.Resize(this, ClientWidth, 640);
        if (AppWindow.Presenter is OverlappedPresenter presenter) { presenter.IsResizable = false; presenter.IsMaximizable = false; }
        // Fit after layout, including collapsed guidance/error rows. Both tabs
        // share a fixed formatting area so switching tabs never resizes the window.
        root.SizeChanged += (_, _) => { if (!closed && root.ActualHeight > 0) WindowSizing.FitClient(this, ClientWidth, (int)Math.Ceiling(root.ActualHeight)); };
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
        { var item = space == 1 ? createParagraph : new MenuFlyoutItem { Text = title }; item.Click += (_, _) => Try(() => { string id = view.CreateStyle(space, "New " + title); Load(id); }); createMenu.Items.Add(item); }
        create.Flyout = createMenu;
        Field(properties, "Style", choiceRow); Field(properties, "Name", name); Field(properties, "Style type", kind); Field(properties, "Based on", Relationship(parent, visitParent)); Field(properties, "Next paragraph", Relationship(next, visitNext));
        Add(properties);
        Add(availability);
        Add(Separator());
        var tabs = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Center, Spacing = 2 };
        tabs.Children.Add(characterTab); tabs.Children.Add(paragraphTab); Add(tabs);
        var fields = new Grid(); fields.Children.Add(character); fields.Children.Add(paragraph);
        Add(new Border { Child = fields, Height = 202, Padding = new(12, 12, 12, 10), BorderBrush = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(58, 58, 58) : Theme.Rgb(210, 210, 210)), BorderThickness = new(1), CornerRadius = new(6) });
        characterTab.Click += (_, _) => SelectTab(false); paragraphTab.Click += (_, _) => SelectTab(true); SelectTab(false);
        characterTab.Checked += (_, _) => SelectTab(false); paragraphTab.Checked += (_, _) => SelectTab(true);
        BuildCharacter(); BuildParagraph();
        Add(new Border { Child = preview, BorderBrush = new SolidColorBrush(Theme.Rgb(64, 70, 80)), BorderThickness = new(1), CornerRadius = new(5) });
        preview.Draw += (_, e) => DrawPreview(e.DrawingSession);
        Add(error);
        var buttons = new Grid(); buttons.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); buttons.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        restoreDefaults.HorizontalAlignment = HorizontalAlignment.Left;
        restoreDefaults.Click += (_, _) => Try(() => view.ReplaceCodeStyles([])); buttons.Children.Add(restoreDefaults);
        var close = new Button { Content = "Close", MinWidth = 82 }; close.Click += (_, _) => Close(); buttons.Children.Add(close); Grid.SetColumn(close, 1); Add(buttons);
        visitParent.Click += (_, _) => Navigate(selected.Parent);
        visitNext.Click += (_, _) => Navigate(selected.Next);
        stylePicker.SelectionChanged += (_, _) => { if (!loading && stylePicker.SelectedItem is StyleDefinition style) Load(style.Id); };
        name.LostFocus += (_, _) => { if (!loading && name.Text != selected.Name) Try(() => view.EditStyleString(selected, VIEM_STYLE_EDIT_SET_DISPLAY_NAME, 0, name.Text)); };
        parent.SelectionChanged += (_, _) => { if (loading) return; Try(() => { string id = parent.SelectedItem is StyleDefinition p ? p.Id : ""; view.EditStyleString(selected, id.Length == 0 ? VIEM_STYLE_EDIT_CLEAR_PARENT : VIEM_STYLE_EDIT_SET_PARENT, 0, id); }); };
        next.SelectionChanged += (_, _) => { if (loading) return; Try(() => { string id = next.SelectedItem is StyleDefinition p ? p.Id : ""; view.EditStyleString(selected, id.Length == 0 ? VIEM_STYLE_EDIT_CLEAR_NEXT_STYLE : VIEM_STYLE_EDIT_SET_NEXT_STYLE, 0, id); }); };
        delete.Click += (_, _) => Try(() => { view.DeleteStyle(selected); Load(); });
        view.Document.Changed += DocumentChanged;
        view.Disposed += Close;
        Closed += (_, _) => { closed = true; view.Document.Changed -= DocumentChanged; view.Disposed -= Close; preview.RemoveFromVisualTree(); };
        Load();
    }
    private readonly TextBlock availability = new() { FontSize = 12, Opacity = .65, TextWrapping = TextWrapping.Wrap };
    private readonly ToggleButton characterTab = new() { Content = "Character", FontSize = 13, Width = 130, Padding = new(12, 3, 12, 3) };
    private readonly ToggleButton paragraphTab = new() { Content = "Paragraph", FontSize = 13, Width = 130, Padding = new(12, 3, 12, 3) };
    private void SelectTab(bool paragraphSelected)
    {
        characterTab.IsChecked = !paragraphSelected; paragraphTab.IsChecked = paragraphSelected;
        character.Visibility = paragraphSelected ? Visibility.Collapsed : Visibility.Visible;
        paragraph.Visibility = paragraphSelected ? Visibility.Visible : Visibility.Collapsed;
    }
    internal void Retarget(CoreView nextView)
    {
        if (view == nextView) return;
        view.Document.Changed -= DocumentChanged; view.Disposed -= Close;
        view = nextView; view.Document.Changed += DocumentChanged; view.Disposed += Close;
        Load();
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
    private void Navigate(string id)
    {
        if (id.Length == 0 || id == selected.Id) return;
        if (name.Text != selected.Name) Try(() => view.EditStyleString(selected, VIEM_STYLE_EDIT_SET_DISPLAY_NAME, 0, name.Text));
        if (error.Visibility != Visibility.Visible) Load(id);
    }
    private void UpdateNavigation(Button button, string id, string relationship)
    {
        var destination = sheet.Styles.FirstOrDefault(s => s.Id == id && s.Id != selected.Id);
        button.IsEnabled = destination != null;
        string label = destination == null ? "Go to " + relationship : "Go to " + destination.Name;
        AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label);
    }
    private bool IsDescendant(StyleDefinition style)
    {
        var visited = new HashSet<string>();
        while (visited.Add(style.Id)) {
            if (style.Id == selected.Id) return true;
            var ancestor = sheet.Styles.FirstOrDefault(s => s.Id == style.Parent);
            if (ancestor == null) break; style = ancestor;
        }
        return false;
    }
    private static void Field(Grid grid, string label, FrameworkElement value)
    { int row = grid.RowDefinitions.Count; grid.RowDefinitions.Add(new() { Height = GridLength.Auto, MinHeight = 24 }); var text = new TextBlock { Text = label, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Right }; Grid.SetRow(text, row); grid.Children.Add(text); Grid.SetRow(value, row); Grid.SetColumn(value, 1); grid.Children.Add(value); }
    private void DocumentChanged() { if (!updating && !closed) Load(selected?.Id); }
    private void Load(string? id = null)
    {
        if (view.Id == 0) { EnableChildren(root, false); return; }
        loading = true;
        try
        {
            sheet = view.Styles();
            Title = view.UsesGlobalStyles ? "Code Styles" : "Document Styles";
            availability.Text = view.UsesGlobalStyles ? "Shared by every Code document. Changes are saved to code_style.json." : "";
            availability.Visibility = view.UsesGlobalStyles ? Visibility.Visible : Visibility.Collapsed;
            restoreDefaults.Visibility = availability.Visibility;
            create.IsEnabled = view.UsesGlobalStyles || view.Document.State.format is VIEM_FORMAT_HTML or VIEM_FORMAT_HTML_SOURCE or VIEM_FORMAT_RTF;
            createParagraph.Visibility = view.UsesGlobalStyles ? Visibility.Collapsed : Visibility.Visible;
            var styles = sheet.Styles.Where(s => s.Native.role != VIEM_STYLE_ROLE_DOCUMENT && (s.Native.flags & VIEM_STYLE_DEFINITION_INTERNAL) == 0).ToArray();
            selected = styles.FirstOrDefault(s => s.Id == id) ?? styles.FirstOrDefault(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0) ?? styles[0];
            stylePicker.ItemsSource = styles; stylePicker.SelectedItem = selected;
            name.Text = selected.Name; name.IsReadOnly = !selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME); kind.Text = selected.Namespace == 1 ? "Paragraph" : "Character";
            var parents = new object[] { selected.Namespace == 2 ? "Default Paragraph" : "None" }.Concat(styles.Where(s => s.Namespace == selected.Namespace && !IsDescendant(s))).ToArray(); parent.ItemsSource = parents; parent.SelectedItem = parents.OfType<StyleDefinition>().FirstOrDefault(s => s.Id == selected.Parent) ?? parents[0]; parent.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_EDIT_PARENT);
            var following = new object[] { "Same Style" }.Concat(styles.Where(s => s.Namespace == 1)).ToArray(); next.ItemsSource = following; next.SelectedItem = following.OfType<StyleDefinition>().FirstOrDefault(s => s.Id == selected.Next) ?? following[0]; next.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_EDIT_NEXT_STYLE);
            delete.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_DELETE);
            UpdateNavigation(visitParent, selected.Parent, "parent style"); UpdateNavigation(visitNext, selected.Next, "next paragraph style");
            paragraphTab.IsEnabled = selected.Namespace == 1;
            if (!paragraphTab.IsEnabled) SelectTab(false);
            EnableChildren(paragraph, true); EnableChildren(character, true);
            foreach (var refresh in refreshFields) refresh();
            if (selected.Namespace != 1 || !selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS)) EnableChildren(paragraph, false);
            if (!selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS)) EnableChildren(character, false);
            preview.Invalidate();
        }
        finally { loading = false; }
    }
    private static void EnableChildren(Panel panel, bool enabled)
    {
        foreach (var child in panel.Children) { if (child is Control control) control.IsEnabled = enabled; if (child is Panel nested) EnableChildren(nested, enabled); }
    }
    private void Try(Action action)
    {
        if (loading || closed) return;
        updating = true;
        byte[]? before = null;
        try
        {
            if (view.UsesGlobalStyles) before = view.ExportStyleDefaults();
            error.Text = ""; error.Visibility = Visibility.Collapsed; action(); string id = selected.Id;
            if (view.UsesGlobalStyles) { Preferences.AtomicWrite(Path.Combine(preferences.DirectoryPath, "code_style.json"), view.ExportStyleDefaults()); foreach (var doc in App.Instance.Windows.SelectMany(w => w.Panes).Select(p => p.Document).Distinct()) doc.NotifyChanged(); }
            Load(id);
        }
        catch (Exception e) {
            if (before != null) view.ReplaceCodeStyles(before);
            error.Text = e.Message; error.Visibility = Visibility.Visible; Load(selected.Id);
        }
        finally { updating = false; }
    }
    private void DrawPreview(CanvasDrawingSession drawing)
    {
        drawing.Clear(preferences.Theme.Background);
        if (selected == null) return;
        string family = sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES));
        var resolved = FontCatalog.Resolve(family) ?? (Family: "Segoe UI", Stretch: FontStretch.Normal);
        using var format = new CanvasTextFormat { FontFamily = resolved.Family, FontStretch = resolved.Stretch, FontSize = Math.Max(8, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number), WordWrapping = CanvasWordWrapping.Wrap };
        uint weight = selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value;
        if (selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_BOLD).enum_value != 0) weight = Math.Min(1000, weight + 300);
        format.FontWeight = new FontWeight { Weight = (ushort)Math.Clamp(weight, 1, 999) };
        format.FontStyle = selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SLANT).enum_value switch { 1 => FontStyle.Italic, 2 => FontStyle.Oblique, _ => FontStyle.Normal };
        format.HorizontalAlignment = selected.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT).enum_value switch { 2 => CanvasHorizontalAlignment.Right, 3 => CanvasHorizontalAlignment.Center, _ => CanvasHorizontalAlignment.Left };
        const string sample = "A calm writing surface shaped with the selected style,\nwith line spacing and alignment visible.";
        using var layout = new CanvasTextLayout(preview.Device, sample, format, Math.Max(1, (float)preview.ActualWidth - 40), 76);
        layout.SetUnderline(0, sample.Length, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE).enum_value != 0);
        layout.SetStrikethrough(0, sample.Length, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH).enum_value != 0);
        layout.SetCharacterSpacing(0, sample.Length, 0, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING).number, 0);
        using var context = new CanvasTextFormat { FontFamily = "Segoe UI", FontSize = 12 };
        var muted = preferences.Theme.Foreground; muted.A = 190;
        drawing.DrawText("Previous paragraph gives the style context.", 20, 20, muted, context);
        drawing.DrawTextLayout(layout, 20, 40, preferences.Theme.Foreground);
        drawing.DrawText("Following paragraph shows spacing and inheritance.", 20, Math.Min(114, 44 + (float)layout.LayoutBounds.Height), muted, context);
    }
}
