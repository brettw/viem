using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
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
    private readonly Grid root = new() { Padding = new(20), RowSpacing = 12, VerticalAlignment = VerticalAlignment.Top };
    private readonly ComboBox stylePicker = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly TextBox name = new();
    private readonly TextBlock kind = new();
    private readonly ComboBox parent = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly ComboBox next = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly Button delete = new() { Content = "Delete" };
    private readonly StackPanel character = new() { Spacing = 12 };
    private readonly StackPanel paragraph = new() { Spacing = 12 };
    private readonly CanvasControl preview = new() { Height = 140 };
    private readonly TextBlock error = new() { TextWrapping = TextWrapping.Wrap, FontSize = 12, Visibility = Visibility.Collapsed };
    private readonly List<Action> refreshFields = [];

    public StyleWindow(CoreView view, Preferences preferences)
    {
        this.view = view; this.preferences = preferences;
        Title = view.UsesGlobalStyles ? "Code Styles" : "Document Styles";
        var scroll = new ScrollViewer { Content = root, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, RequestedTheme = preferences.Midnight ? ElementTheme.Dark : ElementTheme.Light, Background = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(32, 32, 32) : Theme.Rgb(250, 250, 250)) };
        scroll.SizeChanged += (_, _) => root.Width = Math.Max(680, scroll.ActualWidth);
        Content = scroll;
        root.RequestedTheme = preferences.Midnight ? ElementTheme.Dark : ElementTheme.Light;
        WindowSizing.Resize(this, 760, 740);
        bool fitted = false;
        root.SizeChanged += (_, _) => { if (!fitted && root.ActualHeight > 0) { fitted = true; WindowSizing.FitClient(this, 760, (int)Math.Ceiling(root.ActualHeight)); } };
        WindowSizing.Appearance(this, preferences.Midnight);
        var properties = new Grid { RowSpacing = 8, ColumnSpacing = 12 };
        properties.ColumnDefinitions.Add(new() { Width = new(115) }); properties.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        var choiceRow = new Grid { ColumnSpacing = 8 };
        choiceRow.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); choiceRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); choiceRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        choiceRow.Children.Add(stylePicker);
        var create = new DropDownButton { Content = "New" }; Grid.SetColumn(create, 1); choiceRow.Children.Add(create); Grid.SetColumn(delete, 2); choiceRow.Children.Add(delete);
        var createMenu = new MenuFlyout();
        foreach (var (title, space) in new[] { ("Paragraph style", 1u), ("Character style", 2u) })
        { var item = new MenuFlyoutItem { Text = title }; item.Click += (_, _) => Try(() => { string id = view.CreateStyle(space, "New " + title); Load(id); }); createMenu.Items.Add(item); }
        create.Flyout = createMenu;
        Field(properties, "Style", choiceRow); Field(properties, "Name", name); Field(properties, "Style type", kind); Field(properties, "Based on", parent); Field(properties, "Next paragraph", next);
        Add(properties);
        Add(availability);
        var tabs = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Center, Spacing = 12, Margin = new(0, 6, 0, 4) };
        tabs.Children.Add(characterTab); tabs.Children.Add(paragraphTab); Add(tabs);
        var fields = new Grid(); fields.Children.Add(character); fields.Children.Add(paragraph);
        Add(new Border { Child = fields, Padding = new(16), BorderBrush = new SolidColorBrush(Theme.Rgb(64, 64, 64)), BorderThickness = new(1), CornerRadius = new(6) });
        characterTab.Click += (_, _) => SelectTab(false); paragraphTab.Click += (_, _) => SelectTab(true); SelectTab(false);
        characterTab.Checked += (_, _) => SelectTab(false); paragraphTab.Checked += (_, _) => SelectTab(true);
        BuildCharacter(); BuildParagraph();
        Add(new Border { Child = preview, BorderBrush = new SolidColorBrush(Theme.Rgb(64, 70, 80)), BorderThickness = new(1), CornerRadius = new(5) });
        preview.Draw += (_, e) => DrawPreview(e.DrawingSession);
        Add(error);
        var buttons = new Grid(); buttons.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); buttons.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var close = new Button { Content = "Close", MinWidth = 90 }; close.Click += (_, _) => Close(); buttons.Children.Add(close); Grid.SetColumn(close, 1); Add(buttons);
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
    private readonly TextBlock availability = new() { FontSize = 12, TextWrapping = TextWrapping.Wrap };
    private readonly ToggleButton characterTab = new() { Content = "Character", FontSize = 14, MinWidth = 145, Padding = new(24, 3, 24, 3) };
    private readonly ToggleButton paragraphTab = new() { Content = "Paragraph", FontSize = 14, MinWidth = 145, Padding = new(24, 3, 24, 3) };
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
    private void Add(FrameworkElement item) { root.RowDefinitions.Add(new() { Height = GridLength.Auto }); Grid.SetRow(item, root.RowDefinitions.Count - 1); root.Children.Add(item); }
    private static void Field(Grid grid, string label, FrameworkElement value)
    { int row = grid.RowDefinitions.Count; grid.RowDefinitions.Add(new() { Height = GridLength.Auto }); var text = new TextBlock { Text = label, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Right }; Grid.SetRow(text, row); grid.Children.Add(text); Grid.SetRow(value, row); Grid.SetColumn(value, 1); grid.Children.Add(value); }
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
            var styles = sheet.Styles.Where(s => s.Native.role != VIEM_STYLE_ROLE_DOCUMENT && (s.Native.flags & VIEM_STYLE_DEFINITION_INTERNAL) == 0).ToArray();
            selected = styles.FirstOrDefault(s => s.Id == id) ?? styles.FirstOrDefault(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0) ?? styles[0];
            stylePicker.ItemsSource = styles; stylePicker.SelectedItem = selected;
            name.Text = selected.Name; name.IsReadOnly = !selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME); kind.Text = selected.Namespace == 1 ? "Paragraph" : "Character";
            var parents = new object[] { "None" }.Concat(styles.Where(s => s.Namespace == selected.Namespace && s.Id != selected.Id)).ToArray(); parent.ItemsSource = parents; parent.SelectedItem = parents.OfType<StyleDefinition>().FirstOrDefault(s => s.Id == selected.Parent) ?? parents[0]; parent.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_EDIT_PARENT);
            var following = new object[] { "Same Style" }.Concat(styles.Where(s => s.Namespace == 1)).ToArray(); next.ItemsSource = following; next.SelectedItem = following.OfType<StyleDefinition>().FirstOrDefault(s => s.Id == selected.Next) ?? following[0]; next.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_EDIT_NEXT_STYLE);
            delete.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_DELETE);
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
        try
        {
            error.Text = ""; error.Visibility = Visibility.Collapsed; action(); string id = selected.Id;
            if (view.UsesGlobalStyles) { Preferences.AtomicWrite(Path.Combine(preferences.DirectoryPath, "code_style.json"), view.ExportStyleDefaults()); foreach (var doc in App.Instance.Windows.SelectMany(w => w.Panes).Select(p => p.Document).Distinct()) doc.NotifyChanged(); }
            Load(id);
        }
        catch (Exception e) { error.Text = e.Message; error.Visibility = Visibility.Visible; Load(selected.Id); }
        finally { updating = false; }
    }
    private StackPanel Row(StackPanel target)
    { var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 16 }; target.Children.Add(row); return row; }
    private void Property(StackPanel row, string label, uint property, Control editor, Action set)
    {
        var group = new StackPanel { Spacing = 6 };
        group.Children.Add(new TextBlock { Text = label, FontSize = 12 });
        var body = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        var enabled = new CheckBox { MinWidth = 0, Padding = new(0), VerticalAlignment = VerticalAlignment.Center };
        AutomationProperties.SetName(enabled, "Declare " + label); AutomationProperties.SetName(editor, label);
        ToolTipService.SetToolTip(enabled, "Checked: this style declares the value. Unchecked: inherit it.");
        body.Children.Add(enabled); body.Children.Add(editor); group.Children.Add(body); row.Children.Add(group);
        refreshFields.Add(() => { enabled.IsChecked = selected.Declares(property); editor.IsEnabled = enabled.IsChecked == true; });
        enabled.Click += (_, _) => { if (loading) return; Try(() => { if (enabled.IsChecked == true) set(); else view.EditStyle(selected, VIEM_STYLE_EDIT_CLEAR_DECLARATION, property, default); }); };
    }
    private void Number(StackPanel row, string label, uint property, float fallback = 0, float min = -1000, float max = 1000)
    {
        var value = new NumberBox { Width = 110, Minimum = min, Maximum = max, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
        bool unsigned = property == VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT;
        void Set() => view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, property, unsigned ? CoreView.Enum(VIEM_STYLE_VALUE_UNSIGNED, checked((uint)value.Value)) : CoreView.Number((float)value.Value));
        Property(row, label, property, value, Set);
        refreshFields.Add(() => value.Value = selected.Properties.ContainsKey(property) ? unsigned ? selected.Value(property).enum_value : selected.Value(property).number : fallback);
        value.ValueChanged += (_, _) => { if (!loading && double.IsFinite(value.Value)) Try(Set); };
    }
    private void Choice(StackPanel row, string label, uint property, uint kind, string[] choices, uint[] values)
    {
        var combo = new ComboBox { Width = 180, ItemsSource = choices };
        void Set() => view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, property, CoreView.Enum(kind, values[Math.Max(0, combo.SelectedIndex)]));
        Property(row, label, property, combo, Set);
        refreshFields.Add(() => combo.SelectedIndex = Math.Max(0, Array.IndexOf(values, selected.Value(property).enum_value)));
        combo.SelectionChanged += (_, _) => { if (!loading) Try(Set); };
    }
    private void Boolean(StackPanel row, string label, uint property)
    {
        var button = new ToggleButton { Content = label, MinWidth = 36, FontSize = 14 };
        if (label == "B") button.FontWeight = Microsoft.UI.Text.FontWeights.Bold;
        if (label == "I") button.FontStyle = FontStyle.Italic;
        if (label == "U") button.Content = new TextBlock { Text = "U", TextDecorations = TextDecorations.Underline, FontSize = 14 };
        void Set() => view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, property, CoreView.Enum(property == VIEM_STYLE_PROPERTY_CHARACTER_SLANT ? VIEM_STYLE_VALUE_FONT_SLANT : VIEM_STYLE_VALUE_BOOLEAN, button.IsChecked == true ? 1u : 0u));
        Property(row, label, property, button, Set); refreshFields.Add(() => button.IsChecked = selected.Value(property).enum_value != 0);
        button.Click += (_, _) => Try(Set);
    }
    private void ColorControl(StackPanel row, string label, uint property)
    {
        var button = new Button { Width = 70, Content = "…" };
        var picker = new ColorPicker { IsAlphaEnabled = true, Width = 300 };
        var flyout = new Flyout { Content = picker }; button.Flyout = flyout;
        void Set() => SetColor(property, picker.Color);
        Property(row, label, property, button, Set);
        refreshFields.Add(() => { var color = selected.Value(property).color; picker.Color = global::Windows.UI.Color.FromArgb((byte)(color.alpha * 255), (byte)(color.red * 255), (byte)(color.green * 255), (byte)(color.blue * 255)); button.Background = new SolidColorBrush(picker.Color); });
        flyout.Closed += (_, _) => { if (!loading && selected.Declares(property)) Try(Set); };
    }
    private unsafe void SetColor(uint property, global::Windows.UI.Color color)
    { var value = CoreView.Enum(VIEM_STYLE_VALUE_COLOR, 0); value.color = new() { red = color.R / 255f, green = color.G / 255f, blue = color.B / 255f, alpha = color.A / 255f }; view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, property, value); }
    private void BuildCharacter()
    {
        BuildFontRow();
        var row = Row(character); Boolean(row, "B", VIEM_STYLE_PROPERTY_CHARACTER_BOLD); Boolean(row, "I", VIEM_STYLE_PROPERTY_CHARACTER_SLANT); Boolean(row, "U", VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE); Boolean(row, "S", VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH);
        ColorControl(row, "Text Color", VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND); ColorControl(row, "Background", VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND);
        row = Row(character); Number(row, "Tracking", VIEM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING); Number(row, "Baseline", VIEM_STYLE_PROPERTY_CHARACTER_BASELINE_SHIFT);
    }
    private void BuildParagraph()
    {
        var row = Row(paragraph);
        Choice(row, "Alignment", VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT, VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT, ["Start", "Center", "End"], [1, 3, 2]);
        Choice(row, "Direction", VIEM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION, VIEM_STYLE_VALUE_WRITING_DIRECTION, ["Automatic", "Left to Right", "Right to Left"], [0, 1, 2]);
        row = Row(paragraph); Number(row, "Start indent", VIEM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT); Number(row, "End indent", VIEM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT); Number(row, "First line", VIEM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT);
        row = Row(paragraph); Number(row, "Space before", VIEM_STYLE_PROPERTY_PARAGRAPH_SPACING_BEFORE, min: 0); Number(row, "Space after", VIEM_STYLE_PROPERTY_PARAGRAPH_SPACING_AFTER, min: 0);
        var spacing = new ComboBox { ItemsSource = new[] { "Normal", "Multiplier", "At least", "Exact" }, Width = 150 };
        var amount = new NumberBox { Minimum = 0, Maximum = 1000, Width = 80, VerticalAlignment = VerticalAlignment.Bottom, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
        void Set() => SetLineSpacing((uint)Math.Max(1, spacing.SelectedIndex + 1), (float)amount.Value);
        Property(row, "Line spacing", VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING, spacing, Set); row.Children.Add(amount);
        refreshFields.Add(() => { var v = selected.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING); spacing.SelectedIndex = Math.Max(0, (int)v.enum_value - 1); amount.Value = v.number; amount.IsEnabled = selected.Declares(VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING) && spacing.SelectedIndex != 0; });
        spacing.SelectionChanged += (_, _) => { if (!loading) Try(Set); }; amount.ValueChanged += (_, _) => { if (!loading && double.IsFinite(amount.Value)) Try(Set); };
    }
    private unsafe void SetLineSpacing(uint kind, float number) { var value = CoreView.Enum(VIEM_STYLE_VALUE_LINE_SPACING, kind); value.number = number; view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING, value); }
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
        using var layout = new CanvasTextLayout(preview.Device, "A calm writing surface shaped with the selected style,\nwith the selected font and alignment.", format, Math.Max(1, (float)preview.ActualWidth - 40), 120);
        drawing.DrawTextLayout(layout, 20, 40, preferences.Theme.Foreground);
    }
}
