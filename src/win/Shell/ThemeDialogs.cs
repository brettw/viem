using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

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
