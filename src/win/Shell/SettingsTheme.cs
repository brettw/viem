using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Rendering;
using Windows.UI;

namespace Viem.Windows.Shell;

internal sealed partial class SettingsWindow
{
    private readonly Dictionary<string, ColorPicker> colors = [];
    private readonly Dictionary<string, Border> swatches = [];
    private readonly ComboBox statusFont = new() { IsEditable = true, HorizontalAlignment = HorizontalAlignment.Stretch, VerticalAlignment = VerticalAlignment.Center, ItemsSource = new[] { "System" }.Concat(FontCatalog.Families).ToArray() };
    private readonly NumberBox statusSize = new() { Minimum = 8, Maximum = 32, Width = 92, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
    private readonly TextBlock previewTitle = new() { Text = "A space for your words.", FontSize = 20, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold };
    private readonly TextBlock previewSelection = new() { Text = "Keep your own rhythm.", FontSize = 15, Padding = new(3, 1, 3, 1) };
    private readonly TextBlock previewStatus = new() { Text = "NORMAL      ○ Ln 1, Col 1     UTF-8     Markdown", Padding = new(12, 5, 12, 5) };
    private readonly Border previewBody = new() { Height = 108 }, previewFooter = new(), selectionPaint = new(), caretPaint = new() { Width = 11, Height = 22, Margin = new(2, 0, 0, 0), VerticalAlignment = VerticalAlignment.Center };
    private readonly ComboBox themePicker = new() { MinWidth = 230, HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly Button newTheme = new() { Content = "New…" }, deleteTheme = new() { Content = "Delete" };

    private void BuildTheme(StackPanel page)
    {
        var themes = Row(page); themes.Children.Add(themePicker); themes.Children.Add(newTheme); themes.Children.Add(deleteTheme);
        AutomationProperties.SetName(themePicker, "Theme");
        themePicker.SelectionChanged += (_, _) => { if (!loading) { Commit(() => { if (themePicker.SelectedItem is Preferences.ThemeFile file) preferences.SelectTheme(file.Name, file.Path); else preferences.SelectTheme(null); }); RefreshThemeControls(); } };
        themePicker.DropDownOpened += (_, _) => Commit(RefreshThemeControls);
        newTheme.Click += async (_, _) => { try { await ThemeDialogs.Create(preferences, root.XamlRoot, root.RequestedTheme); } catch (Exception exception) { Commit(() => throw exception); } };
        deleteTheme.Click += async (_, _) => { try { await ThemeDialogs.RevertOrDelete(preferences, root.XamlRoot, root.RequestedTheme); } catch (Exception exception) { Commit(() => throw exception); } };
        var preview = new StackPanel { Spacing = 12, Padding = new(18, 16, 18, 16) };
        var title = Row(preview); title.Spacing = 0; title.Children.Add(previewTitle); title.Children.Add(caretPaint);
        selectionPaint.Child = previewSelection; selectionPaint.HorizontalAlignment = HorizontalAlignment.Left; preview.Children.Add(selectionPaint);
        previewBody.Child = preview; previewFooter.Child = previewStatus;
        var sample = new StackPanel(); sample.Children.Add(previewBody); sample.Children.Add(previewFooter);
        page.Children.Add(new Border { Child = sample, CornerRadius = new(6), BorderThickness = new(1), BorderBrush = sectionBorder });
        var editor = ThemeSection(page, "Editor");
        var theme = preferences.Theme;
        var row = ColorRow(editor);
        ColorControl(row, 0, "foreground", "Text", theme.Foreground); ColorControl(row, 1, "background", "Canvas", theme.Background);
        row = ColorRow(editor);
        ColorControl(row, 0, "caret", "Caret", theme.Caret); ColorControl(row, 1, "selection", "Selection", theme.Selection);
        var status = ThemeSection(page, "Status bar"); row = ColorRow(status);
        ColorControl(row, 0, "statusForeground", "Text", theme.StatusForeground); ColorControl(row, 1, "statusBackground", "Background", theme.StatusBackground);
        var fontRow = new Grid { ColumnSpacing = 10 };
        foreach (var width in new[] { GridLength.Auto, new GridLength(1, GridUnitType.Star), GridLength.Auto, GridLength.Auto }) fontRow.ColumnDefinitions.Add(new() { Width = width });
        fontRow.Children.Add(new TextBlock { Text = "Font", VerticalAlignment = VerticalAlignment.Center, Margin = new(0, 0, 4, 0) });
        statusFont.Text = preferences.Get("theme", "statusFontFamily", "System"); statusSize.Value = preferences.StatusFontSize;
        statusFont.SelectedItem = ((string[])statusFont.ItemsSource).FirstOrDefault(f => string.Equals(f, statusFont.Text, StringComparison.OrdinalIgnoreCase));
        fontRow.Children.Add(statusFont); Grid.SetColumn(statusFont, 1); fontRow.Children.Add(statusSize); Grid.SetColumn(statusSize, 2);
        var units = new TextBlock { Text = "pt", VerticalAlignment = VerticalAlignment.Center, Opacity = .7 }; fontRow.Children.Add(units); Grid.SetColumn(units, 3); status.Children.Add(fontRow);
        AutomationProperties.SetName(statusFont, "Status font"); AutomationProperties.SetName(statusSize, "Status font size");
        statusFont.SelectionChanged += (_, _) => { if (!loading && statusFont.SelectedItem is string family) { statusFont.Text = family; SaveTheme(); } };
        statusFont.LostFocus += (_, _) => SaveTheme();
        statusSize.ValueChanged += (_, _) => { if (double.IsFinite(statusSize.Value)) SaveTheme(); };
        RefreshThemeControls();
        preferences.ThemesChanged += RefreshThemeControls;
        Closed += (_, _) => preferences.ThemesChanged -= RefreshThemeControls;
    }
    private StackPanel ThemeSection(Panel page, string heading)
    {
        var fields = new StackPanel { Spacing = 10 };
        fields.Children.Add(new TextBlock { Text = heading, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        page.Children.Add(new Border { Child = fields, Padding = new(14), BorderThickness = new(1), BorderBrush = sectionBorder, CornerRadius = new(6) }); return fields;
    }
    private static Grid ColorRow(Panel parent)
    { var row = new Grid { ColumnSpacing = 24 }; row.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); parent.Children.Add(row); return row; }
    private void ColorControl(Grid row, int column, string key, string label, Color color)
    {
        var picker = new ColorPicker { Color = color, IsAlphaEnabled = true, Width = 290 }; colors.Add(key, picker);
        var chip = new Border { Width = 28, Height = 12, CornerRadius = new(2), Background = new SolidColorBrush(color) }; swatches.Add(key, chip);
        var button = new Button { Content = chip, Padding = new(5), MinWidth = 40, Flyout = new Flyout { Content = picker } };
        AutomationProperties.SetName(button, "Choose " + label.ToLowerInvariant() + " color");
        var pair = new Grid { ColumnSpacing = 12 }; pair.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); pair.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        pair.Children.Add(new TextBlock { Text = label, VerticalAlignment = VerticalAlignment.Center }); pair.Children.Add(button); Grid.SetColumn(button, 1); Grid.SetColumn(pair, column); row.Children.Add(pair);
        // Preview while dragging, persist once the picker closes.
        picker.ColorChanged += (_, _) => RefreshThemePreview(); button.Flyout.Closed += (_, _) => SaveTheme();
    }
    private void RefreshThemeControls()
    {
        preferences.EnsureCurrentThemeExists();
        bool wasLoading = loading; loading = true;
        try
        {
            var entries = preferences.ThemeFiles;
            themePicker.ItemsSource = entries.Cast<object>().Append("Default").ToArray();
            themePicker.SelectedItem = entries.FirstOrDefault(file => file.Path == preferences.SelectedThemePath) as object ?? "Default";
            deleteTheme.Content = preferences.SelectedThemeIsBundled ? "Revert" : "Delete";
            deleteTheme.IsEnabled = preferences.SelectedTheme != null;
            var theme = preferences.Theme;
            colors["foreground"].Color = theme.Foreground; colors["background"].Color = theme.Background;
            colors["caret"].Color = theme.Caret; colors["selection"].Color = theme.Selection;
            colors["statusForeground"].Color = theme.StatusForeground; colors["statusBackground"].Color = theme.StatusBackground;
            statusFont.Text = preferences.Get("theme", "statusFontFamily", "System"); statusSize.Value = preferences.StatusFontSize;
            statusFont.SelectedItem = ((string[])statusFont.ItemsSource).FirstOrDefault(f => string.Equals(f, statusFont.Text, StringComparison.OrdinalIgnoreCase));
            RefreshThemePreview();
        }
        finally { loading = wasLoading; }
    }
    private void SaveTheme()
    {
        if (loading) return;
        if (preferences.EnsureCurrentThemeExists()) { RefreshThemeControls(); return; }
        RefreshThemePreview();
        Commit(() => {
            var theme = new JsonObject(); foreach (var pair in colors) theme[pair.Key] = Preferences.ColorJson(pair.Value.Color);
            theme["statusFontFamily"] = statusFont.Text.Trim(); theme["statusFontSize"] = statusSize.Value;
            preferences.SetSections(new JsonObject { ["theme"] = theme });
        });
        RefreshThemeControls();
    }
    private void RefreshThemePreview()
    {
        if (colors.Count != 6) return;
        SolidColorBrush Paint(string key) => new(colors[key].Color);
        previewBody.Background = Paint("background"); previewTitle.Foreground = previewSelection.Foreground = Paint("foreground");
        caretPaint.Background = Paint("caret"); selectionPaint.Background = Paint("selection");
        previewFooter.Background = Paint("statusBackground"); previewStatus.Foreground = Paint("statusForeground");
        previewStatus.FontFamily = new FontFamily(FontCatalog.Resolve(statusFont.Text)?.Family ?? statusFont.Text);
        if (double.IsFinite(statusSize.Value)) previewStatus.FontSize = statusSize.Value;
        foreach (var pair in swatches) pair.Value.Background = Paint(pair.Key);
    }
#if DEBUG
    internal ComboBox ThemePicker => themePicker;
    internal Button NewTheme => newTheme;
    internal Button DeleteTheme => deleteTheme;
    internal ComboBox StatusFont => statusFont;
    internal NumberBox StatusSize => statusSize;
    internal TextBlock PreviewStatus => previewStatus;
#endif
}
