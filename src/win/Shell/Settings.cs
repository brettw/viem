using System.Text.Json;
using System.Text.Json.Nodes;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private ContentDialog CreateSettings()
    {
        var tabs = new Pivot();
        StackPanel Page(string name)
        {
            var panel = new StackPanel { Spacing = 12, Margin = new(0, 8, 0, 8) };
            tabs.Items.Add(new PivotItem { Header = name, Content = new ScrollViewer { Content = panel, MaxHeight = 440, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled } }); return panel;
        }
        var view = Page("View"); var themePage = Page("Theme"); var editingPage = Page("Editing"); var codePage = Page("Code");
        var marginFields = new Dictionary<string, NumberBox>();
        var marginRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        foreach (string edge in new[] { "top", "left", "bottom", "right" })
        { var box = new NumberBox { Header = char.ToUpperInvariant(edge[0]) + edge[1..], Value = preferences.Margin(edge), Minimum = 0, Maximum = 1000, Width = 84 }; marginFields.Add(edge, box); marginRow.Children.Add(box); }
        view.Children.Add(new TextBlock { Text = "Text margins (DIPs)" }); view.Children.Add(marginRow);
        var status = new ToggleSwitch { Header = "Show status bar", IsOn = preferences.ShowStatus }; view.Children.Add(status);
        var menus = new ToggleSwitch { Header = "Show menu bar", IsOn = preferences.ShowMenu }; view.Children.Add(menus);
        var theme = preferences.Theme;
        var colors = new Dictionary<string, ColorPicker>();
        var preview = new TextBlock { Text = "A calm writing surface", Padding = new(12), FontSize = 18 };
        var previewStatus = new TextBlock { Text = "NORMAL                         Ln 1, Col 1", Padding = new(8) };
        var previewBox = new StackPanel(); var previewBody = new Border { Child = preview }; var previewFooter = new Border { Child = previewStatus }; previewBox.Children.Add(previewBody); previewBox.Children.Add(previewFooter);
        void RefreshThemePreview()
        { if (colors.Count < 6) return; preview.Foreground = new SolidColorBrush(colors["foreground"].Color); previewBody.Background = new SolidColorBrush(colors["background"].Color); previewStatus.Foreground = new SolidColorBrush(colors["statusForeground"].Color); previewFooter.Background = new SolidColorBrush(colors["statusBackground"].Color); }
        var preset = new ComboBox { Header = "Preset", ItemsSource = new[] { "Custom", "Midnight", "Paper" }, SelectedIndex = 0 }; themePage.Children.Add(preset);
        foreach (var (key, title, color) in new[] { ("foreground", "Text", theme.Foreground), ("background", "Canvas", theme.Background), ("caret", "Caret", theme.Caret), ("selection", "Selection", theme.Selection), ("statusForeground", "Status text", theme.StatusForeground), ("statusBackground", "Status background", theme.StatusBackground) })
        {
            var picker = new ColorPicker { Color = color, IsAlphaEnabled = true, Width = 290 };
            colors[key] = picker;
            var button = new Button { Content = title, HorizontalAlignment = HorizontalAlignment.Stretch, Flyout = new Flyout { Content = picker } }; themePage.Children.Add(button);
            picker.ColorChanged += (_, _) => RefreshThemePreview();
        }
        preset.SelectionChanged += (_, _) => { if (preset.SelectedIndex < 1) return; var value = Preferences.ThemeJson(preset.SelectedIndex == 1 ? Theme.Midnight : Theme.Paper); foreach (var pair in colors) { var c = value[pair.Key]!; pair.Value.Color = Theme.Rgb((byte)(c["red"]!.GetValue<double>() * 255), (byte)(c["green"]!.GetValue<double>() * 255), (byte)(c["blue"]!.GetValue<double>() * 255), (byte)(c["alpha"]!.GetValue<double>() * 255)); } };
        var statusFont = new ComboBox { Header = "Status font", IsEditable = true, ItemsSource = CanvasTextFormat.GetSystemFontFamilies(), Text = preferences.StatusFontFamily };
        var statusSize = new NumberBox { Header = "Status font size", Value = preferences.StatusFontSize, Minimum = 8, Maximum = 32 };
        themePage.Children.Add(statusFont); themePage.Children.Add(statusSize); themePage.Children.Add(previewBox); RefreshThemePreview();
        statusFont.LostFocus += (_, _) => previewStatus.FontFamily = new FontFamily(statusFont.Text);
        statusSize.ValueChanged += (_, _) => { if (double.IsFinite(statusSize.Value)) previewStatus.FontSize = statusSize.Value; };
        var editing = preferences.Editing;
        var quotes = new ToggleSwitch { Header = "Smart quotes", IsOn = preferences.SmartQuotes }; editingPage.Children.Add(quotes);
        var width = new NumberBox { Header = "Text width (columns for gq / gw)", Value = preferences.TextWidth, Minimum = 1, Maximum = uint.MaxValue }; editingPage.Children.Add(width);
        var indentation = editing["indentation"] as JsonObject ?? new JsonObject(); var indentNumbers = new Dictionary<string, NumberBox>(); var indentBooleans = new Dictionary<string, CheckBox>();
        editingPage.Children.Add(new TextBlock { Text = "Indentation and tabs", FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        foreach (var (key, title, fallback, min) in new[] { ("tabstop", "Tab stop", 2, 1), ("shiftwidth", "Shift width", 2, 0), ("softtabstop", "Soft tab stop", 2, -1) })
        { var box = new NumberBox { Header = title, Value = indentation[key]?.GetValue<int>() ?? fallback, Minimum = min, Maximum = 1024 }; indentNumbers[key] = box; editingPage.Children.Add(box); }
        foreach (var (key, title) in new[] { ("autoindent", "Auto indent"), ("expandtab", "Insert spaces for tabs"), ("smarttab", "Smart tabs"), ("continueCommentsOnEnter", "Continue comments with Enter"), ("continueCommentsOnOpenLine", "Continue comments with o / O") })
        { var box = new CheckBox { Content = title, IsChecked = indentation[key]?.GetValue<bool>() ?? true }; indentBooleans[key] = box; editingPage.Children.Add(box); }
        var whitespace = editing["whitespacePresentation"] as JsonObject ?? new JsonObject(); var visible = whitespace["visibleWhitespace"] as JsonObject ?? new JsonObject();
        editingPage.Children.Add(new TextBlock { Text = "Whitespace", FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        var codeWhitespace = new ComboBox { Header = "Code indentation width", ItemsSource = new[] { "Paragraph en", "Spaces" }, SelectedIndex = whitespace["codeWhitespace"]?.GetValue<string>() == "spaces" ? 1 : 0 };
        var otherWhitespace = new ComboBox { Header = "Other formats indentation width", ItemsSource = new[] { "Spaces", "Paragraph en" }, SelectedIndex = whitespace["otherWhitespace"]?.GetValue<string>() == "paragraphEn" ? 1 : 0 };
        var wrapIndent = new NumberBox { Header = "Wrapped line indent (Code)", Value = whitespace["codeWrappedLineIndent"]?.GetValue<int>() ?? 4, Minimum = 0, Maximum = 1024 };
        var showWhitespace = new CheckBox { Content = "Show invisible characters", IsChecked = visible["enabled"]?.GetValue<bool>() ?? true };
        var listchars = new TextBox { Header = "Visible whitespace (listchars)", Text = visible["listchars"]?.GetValue<string>() ?? "tab:>-,trail:*,extends:>,precedes:<" };
        editingPage.Children.Add(codeWhitespace); editingPage.Children.Add(otherWhitespace); editingPage.Children.Add(wrapIndent); editingPage.Children.Add(showWhitespace); editingPage.Children.Add(listchars);
        var syntax = new TextBox { Header = "Vim syntax directory (optional)", Text = preferences.VimDirectory }; codePage.Children.Add(syntax);
        codePage.Children.Add(new TextBlock { Text = "Bundled Tree-sitter languages work without a Vim installation. The directory supplies fallback syntax files.", TextWrapping = TextWrapping.Wrap });
        var associations = new TextBox { Header = "Filename associations (JSON)", Text = System.Text.Encoding.UTF8.GetString(preferences.Associations), AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 80 }; codePage.Children.Add(associations);
        var error = new TextBlock { TextWrapping = TextWrapping.Wrap };
        var body = new StackPanel { Width = 400, Spacing = 8 }; body.Children.Add(tabs); body.Children.Add(error); body.Children.Add(new TextBlock { Text = preferences.DirectoryPath, TextWrapping = TextWrapping.Wrap, FontSize = 11, IsTextSelectionEnabled = true });
        var dialog = new ContentDialog { XamlRoot = root.XamlRoot, RequestedTheme = root.RequestedTheme, Title = "Settings", Content = body, PrimaryButtonText = "Apply", CloseButtonText = "Cancel" };
        dialog.PrimaryButtonClick += (_, args) => {
            try
            {
                var themeJson = new JsonObject(); foreach (var pair in colors) themeJson[pair.Key] = Preferences.ColorJson(pair.Value.Color);
                themeJson["statusFontFamily"] = statusFont.Text; themeJson["statusFontSize"] = statusSize.Value;
                var indent = new JsonObject(); foreach (var pair in indentNumbers) indent[pair.Key] = pair.Value.Value; foreach (var pair in indentBooleans) indent[pair.Key] = pair.Value.IsChecked == true;
                var margins = new JsonObject(); foreach (var pair in marginFields) margins[pair.Key] = pair.Value.Value;
                var changes = new JsonObject {
                    ["theme"] = themeJson, ["view"] = new JsonObject { ["margins"] = margins }, ["appearance"] = new JsonObject { ["showStatusBar"] = status.IsOn }, ["windows"] = new JsonObject { ["showMenu"] = menus.IsOn },
                    ["editing"] = new JsonObject { ["smartQuotes"] = quotes.IsOn, ["textWidth"] = width.Value, ["indentation"] = indent,
                        ["whitespacePresentation"] = new JsonObject { ["codeWhitespace"] = codeWhitespace.SelectedIndex == 0 ? "paragraphEn" : "spaces", ["otherWhitespace"] = otherWhitespace.SelectedIndex == 0 ? "spaces" : "paragraphEn", ["codeWrappedLineIndent"] = wrapIndent.Value, ["visibleWhitespace"] = new JsonObject { ["enabled"] = showWhitespace.IsChecked == true, ["listchars"] = listchars.Text } } },
                    ["code"] = new JsonObject { ["vimSyntaxDirectory"] = syntax.Text, ["filenameAssociations"] = JsonNode.Parse(associations.Text) }
                };
                preferences.SetSections(changes);
            }
            catch (Exception exception) { error.Text = exception.Message; args.Cancel = true; }
        };
        return dialog;
    }
}
