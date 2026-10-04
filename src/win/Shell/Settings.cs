using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;

namespace Viem.Windows.Shell;

internal sealed partial class SettingsWindow : Window
{
    private readonly Preferences preferences;
    private readonly Grid root = new();
    private readonly ListView categories = new() { SelectionMode = ListViewSelectionMode.Single, Margin = new(8, 0, 8, 0) };
    private readonly Grid pages = new();
    private readonly TextBlock error = new() { TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
    private readonly Border sidebar = new();
    private readonly SolidColorBrush sectionBorder = new();
    private readonly List<ScrollViewer> sections = [];
    private bool loading = true;

    internal SettingsWindow(Preferences preferences)
    {
        this.preferences = preferences;
        Title = "Settings"; Content = root;
        root.ColumnDefinitions.Add(new() { Width = new(164) }); root.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        var navigation = new StackPanel { Spacing = 12 };
        navigation.Children.Add(new TextBlock { Text = "SETTINGS", FontSize = 11, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, Opacity = .7, Margin = new(20, 24, 0, 0) });
        navigation.Children.Add(categories); sidebar.Child = navigation; root.Children.Add(sidebar);
        var body = new Grid(); body.RowDefinitions.Add(new() { Height = new(1, GridUnitType.Star) }); body.RowDefinitions.Add(new() { Height = GridLength.Auto });
        body.Children.Add(pages); body.Children.Add(error); Grid.SetRow(error, 1); error.Margin = new(24, 0, 24, 16);
        Grid.SetColumn(body, 1); root.Children.Add(body);
        BuildView(Page("View", "\uE7F4", "Arrange your writing space."));
        BuildTheme(Page("Theme", "\uE790", "Make a comfortable space for writing. Changes apply to every window."));
        BuildEditing(Page("Editing", "\uE70F", "Configure typing, indentation, and whitespace."));
        categories.SelectionChanged += (_, _) => {
            if (categories.SelectedIndex == 1 && root.IsLoaded) OpenThemeSection();
            for (int i = 0; i < sections.Count; i++) sections[i].Visibility = i == categories.SelectedIndex ? Visibility.Visible : Visibility.Collapsed;
        };
        root.Loaded += (_, _) => { if (categories.SelectedIndex == 1) OpenThemeSection(); };
        categories.SelectedIndex = 1;
        ApplyAppearance(); preferences.Changed += ApplyAppearance;
        Closed += (_, _) => preferences.Changed -= ApplyAppearance;
        WindowSizing.Resize(this, 800, 730);
        loading = false;
    }
    private void OpenThemeSection() => Commit(() => { preferences.RestoreMissingBundledThemes(); RefreshThemeControls(); });
    internal void SelectThemeCategory() => categories.SelectedIndex = 1;

    private StackPanel Page(string title, string glyph, string description)
    {
        var label = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        label.Children.Add(new FontIcon { Glyph = glyph, FontSize = 16 }); label.Children.Add(new TextBlock { Text = title });
        categories.Items.Add(new ListViewItem { Content = label });
        var panel = new StackPanel { Spacing = 16, Padding = new(24, 20, 24, 24) };
        var heading = new StackPanel { Spacing = 6 };
        heading.Children.Add(new TextBlock { Text = title, FontSize = 24, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        heading.Children.Add(new TextBlock { Text = description, TextWrapping = TextWrapping.Wrap, Opacity = .7 }); panel.Children.Add(heading);
        var scroll = new ScrollViewer { Content = panel, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Visibility = Visibility.Collapsed };
        sections.Add(scroll); pages.Children.Add(scroll); return panel;
    }
    private void ApplyAppearance()
    {
        root.RequestedTheme = preferences.Midnight ? ElementTheme.Dark : ElementTheme.Light;
        root.Background = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(32, 32, 32) : Theme.Rgb(250, 250, 250));
        sidebar.Background = new SolidColorBrush(preferences.Midnight ? Theme.Rgb(27, 27, 27) : Theme.Rgb(240, 240, 240));
        sectionBorder.Color = preferences.Midnight ? Theme.Rgb(58, 58, 58) : Theme.Rgb(216, 216, 216);
        WindowSizing.Appearance(this, preferences.Midnight);
    }
    private void Commit(Action update)
    {
        if (loading) return;
        try { update(); error.Text = ""; error.Visibility = Visibility.Collapsed; }
        catch (Exception exception) { error.Text = exception.Message; error.Visibility = Visibility.Visible; }
    }
    private static StackPanel Row(Panel parent)
    { var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 }; parent.Children.Add(row); return row; }
    private NumberBox Number(Panel parent, string label, double value, double min, double max, Action<double> set, double width = 140)
    {
        var box = new NumberBox { Header = label, Value = value, Minimum = min, Maximum = max, Width = width, HorizontalAlignment = HorizontalAlignment.Left, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
        parent.Children.Add(box); box.ValueChanged += (_, _) => { if (double.IsFinite(box.Value)) Commit(() => set(box.Value)); }; return box;
    }
    private void Check(Panel parent, string label, bool value, Action<bool> set)
    { var box = new CheckBox { Content = label, IsChecked = value }; parent.Children.Add(box); box.Click += (_, _) => Commit(() => set(box.IsChecked == true)); }
    private void Choice(Panel parent, string label, string[] items, int selected, Action<int> set)
    { var box = new ComboBox { Header = label, ItemsSource = items, SelectedIndex = selected, HorizontalAlignment = HorizontalAlignment.Stretch }; parent.Children.Add(box); box.SelectionChanged += (_, _) => Commit(() => set(box.SelectedIndex)); }
    private void Text(Panel parent, string label, string text, Action<string> set, bool multiline = false)
    { var box = new TextBox { Header = label, Text = text, AcceptsReturn = multiline, TextWrapping = multiline ? TextWrapping.Wrap : TextWrapping.NoWrap, MinHeight = multiline ? 100 : 0 }; parent.Children.Add(box); box.LostFocus += (_, _) => Commit(() => set(box.Text)); }
    private void BuildView(StackPanel page)
    {
        page.Children.Add(new TextBlock { Text = "Text margins", FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        var row = Row(page);
        foreach (string edge in new[] { "top", "left", "bottom", "right" })
            Number(row, char.ToUpperInvariant(edge[0]) + edge[1..], preferences.Margin(edge), 0, 1000, n => preferences.Set("view", "margins", new JsonObject { [edge] = n }), 112);
        Check(page, "Show status bar", preferences.ShowStatus, value => preferences.Set("appearance", "showStatusBar", value));
        Check(page, "Show menu bar", preferences.ShowMenu, value => preferences.Set("windows", "showMenu", value));
    }
    private void BuildEditing(StackPanel page)
    {
        Check(page, "Caret hover effect", preferences.CaretHoverEffect, value => preferences.Set("editing", "caretHoverEffect", value));
        Check(page, "Smart quotes", preferences.SmartQuotes, value => preferences.Set("editing", "smartQuotes", value));
        Check(page, "Automatically format typed Markdown", preferences.MarkdownAutodetect, value => preferences.Set("editing", "markdownAutodetect", value));
        page.Children.Add(new TextBlock { Text = "In Markdown Formatted view, completed Markdown spans and block prefixes become formatting. Use Ctrl-Q before a character to keep it literal.", TextWrapping = TextWrapping.Wrap, Opacity = .7 });
        Number(page, "Text width (columns for gq / gw)", preferences.TextWidth, 1, uint.MaxValue, n => preferences.Set("editing", "textWidth", n), 250);
        var editing = preferences.Editing;
        var indentation = editing["indentation"] as JsonObject ?? new JsonObject();
        void Indent(string key, JsonNode? value) => preferences.Set("editing", "indentation", new JsonObject { [key] = value });
        page.Children.Add(new TextBlock { Text = "Indentation and tabs", FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        var row = Row(page);
        foreach (var (key, title, fallback, min) in new[] { ("tabstop", "Tab stop", 2, 1), ("shiftwidth", "Shift width", 2, 0), ("softtabstop", "Soft tab stop", 2, -1) })
            Number(row, title, indentation[key]?.GetValue<int>() ?? fallback, min, 1024, n => Indent(key, n));
        foreach (var (key, title) in new[] { ("autoindent", "Auto indent"), ("expandtab", "Insert spaces for tabs"), ("smarttab", "Smart tabs"), ("continueCommentsOnEnter", "Continue comments with Enter"), ("continueCommentsOnOpenLine", "Continue comments with o / O") })
            Check(page, title, indentation[key]?.GetValue<bool>() ?? true, value => Indent(key, value));
        var whitespace = editing["whitespacePresentation"] as JsonObject ?? new JsonObject(); var visible = whitespace["visibleWhitespace"] as JsonObject ?? new JsonObject();
        void Whitespace(string key, JsonNode? value) => preferences.Set("editing", "whitespacePresentation", new JsonObject { [key] = value });
        page.Children.Add(new TextBlock { Text = "Whitespace", FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        Choice(page, "Code indentation width", ["Paragraph en", "Spaces"], whitespace["codeWhitespace"]?.GetValue<string>() == "spaces" ? 1 : 0, n => Whitespace("codeWhitespace", n == 0 ? "paragraphEn" : "spaces"));
        Choice(page, "Other formats indentation width", ["Spaces", "Paragraph en"], whitespace["otherWhitespace"]?.GetValue<string>() == "paragraphEn" ? 1 : 0, n => Whitespace("otherWhitespace", n == 0 ? "spaces" : "paragraphEn"));
        Number(page, "Wrapped line indent (Code)", whitespace["codeWrappedLineIndent"]?.GetValue<int>() ?? 4, 0, 1024, n => Whitespace("codeWrappedLineIndent", n), 250);
        Check(page, "Show invisible characters", visible["enabled"]?.GetValue<bool>() ?? true, value => Whitespace("visibleWhitespace", new JsonObject { ["enabled"] = value }));
        Text(page, "Visible whitespace (listchars)", visible["listchars"]?.GetValue<string>() ?? "tab:>-,trail:*,extends:>,precedes:<", value => Whitespace("visibleWhitespace", new JsonObject { ["listchars"] = value }));
    }
#if DEBUG
    internal FrameworkElement RootControl => root;
    internal ListView Categories => categories;
    internal ScrollViewer CurrentPage => sections[categories.SelectedIndex];
    internal string Error => error.Text;
#endif
}
