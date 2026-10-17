using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Windows.System;

namespace Viem.Windows.Shell;

internal static class ThemeDialogs
{
    public static async Task Create(Preferences preferences, XamlRoot root, ElementTheme appearance)
    {
        var name = new TextBox { Header = "Name", PlaceholderText = "Theme name", MaxLength = 64 };
        var error = new TextBlock { TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
        var content = new StackPanel { Spacing = 8 }; content.Children.Add(name); content.Children.Add(error);
        var dialog = new ContentDialog { XamlRoot = root, RequestedTheme = appearance, Title = "New theme", Content = content,
            PrimaryButtonText = "Create", CloseButtonText = "Cancel", DefaultButton = ContentDialogButton.Primary };
        dialog.PrimaryButtonClick += (_, args) => {
            try { preferences.CreateTheme(name.Text); }
            catch (Exception exception) { args.Cancel = true; error.Text = exception.Message; error.Visibility = Visibility.Visible; }
        };
        await dialog.ShowAsync();
    }
    /// Import Vim Theme: a list of discovered color schemes with Select File…,
    /// Cancel and Import. Choosing a file imports it immediately.
    public static async Task ImportVim(Preferences preferences, XamlRoot root, ElementTheme appearance,
        Func<string?, Task<string?>> pickFile, string[]? directories = null)
    {
        directories ??= VimColorSchemes.Directories();
        var schemes = VimColorSchemes.Available(directories);
        string places = string.Join(" or ", directories.Select(VimColorSchemes.Display));
        var message = new TextBlock { TextWrapping = TextWrapping.Wrap, Opacity = .7, Text = schemes.Length == 0
            ? $"No Vim color schemes were found in {places}. Select a .vim file to import."
            : $"Choose a color scheme from {places}, or select another .vim file. Its GUI colors become a new theme." };
        var list = new ListView { ItemsSource = schemes, SelectionMode = ListViewSelectionMode.Single, Height = 240, BorderThickness = new(1),
            BorderBrush = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["ControlStrokeColorDefaultBrush"] };
        AutomationProperties.SetName(list, "Vim color schemes");
        var error = new TextBlock { TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
        var selectFile = new Button { Content = "Select File…" };
        var cancel = new Button { Content = "Cancel" };
        var import = new Button { Content = "Import" };
        if (Application.Current.Resources.TryGetValue("AccentButtonStyle", out var accent) && accent is Style style) import.Style = style;
        var buttons = new Grid { ColumnSpacing = 8 };
        foreach (var width in new[] { GridLength.Auto, new GridLength(1, GridUnitType.Star), GridLength.Auto, GridLength.Auto }) buttons.ColumnDefinitions.Add(new() { Width = width });
        buttons.Children.Add(selectFile); buttons.Children.Add(cancel); buttons.Children.Add(import);
        Grid.SetColumn(cancel, 2); Grid.SetColumn(import, 3);
        var content = new StackPanel { Spacing = 12, Width = 420 };
        foreach (var child in new UIElement[] { message, list, error, buttons }) content.Children.Add(child);
        // The dialog's own buttons are in a fixed order; this row places Select
        // File… apart from Cancel and Import, as on macOS.
        var dialog = new ContentDialog { XamlRoot = root, RequestedTheme = appearance, Title = "Import Vim Theme", Content = content };
        if (schemes.Length > 0) list.SelectedIndex = 0;
        import.IsEnabled = list.SelectedItem != null;
        list.SelectionChanged += (_, _) => import.IsEnabled = list.SelectedItem != null;
        void Import(string path)
        {
            try { preferences.ImportVimTheme(VimColorSchemes.Read(path), Path.GetFileNameWithoutExtension(path)); dialog.Hide(); }
            catch (Exception exception) { error.Text = exception.Message; error.Visibility = Visibility.Visible; }
        }
        void ImportSelection() { if (list.SelectedItem is VimColorSchemes.Scheme scheme) Import(scheme.Path); }
        import.Click += (_, _) => ImportSelection();
        list.DoubleTapped += (_, _) => ImportSelection();
        cancel.Click += (_, _) => dialog.Hide();
        content.KeyDown += (_, args) => {
            if (args.Key == VirtualKey.Escape) { dialog.Hide(); args.Handled = true; }
            else if (args.Key == VirtualKey.Enter && list.SelectedItem != null) { ImportSelection(); args.Handled = true; }
        };
        selectFile.Click += async (_, _) => {
            try { if (await pickFile(directories.FirstOrDefault(Directory.Exists)) is { } path) Import(path); }
            catch (Exception exception) { error.Text = exception.Message; error.Visibility = Visibility.Visible; }
        };
#if DEBUG
        ImportVimForTesting?.Invoke(new(dialog, list, selectFile, cancel, import, error));
#endif
        await dialog.ShowAsync();
    }
#if DEBUG
    internal sealed record ImportVimControls(ContentDialog Dialog, ListView List, Button SelectFile, Button Cancel, Button Import, TextBlock Error);
    internal static Action<ImportVimControls>? ImportVimForTesting { get; set; }
#endif
    public static async Task RevertOrDelete(Preferences preferences, XamlRoot root, ElementTheme appearance)
    {
        if (preferences.SelectedTheme == null) return;
        string name = preferences.SelectedTheme;
        string? path = preferences.SelectedThemePath;
        bool revert = preferences.SelectedThemeIsBundled;
        var dialog = new ContentDialog { XamlRoot = root, RequestedTheme = appearance, Title = revert ? "Revert theme" : "Delete theme",
            Content = revert ? $"Restore ‘{name}’ to the bundled version? Your changes to this theme will be replaced." : $"Delete ‘{name}’? Viem will switch to Default.", PrimaryButtonText = revert ? "Revert" : "Delete", CloseButtonText = "Cancel", DefaultButton = ContentDialogButton.Close };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary && preferences.SelectedThemePath == path) {
            if (revert) preferences.RevertTheme(); else preferences.DeleteTheme();
        }
    }
}
