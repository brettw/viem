using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private ToggleMenuFlyoutItem plainMode = null!, markdownMode = null!, autoMode = null!;
    private MenuFlyoutSubItem codeMode = null!;
    private readonly List<(ToggleMenuFlyoutItem Item, string Language)> languageModes = new();
    private DocumentModeState? menuModeState;
    private FontIcon codeModeCheck = null!;

    private void BuildDocumentModeMenu()
    {
        ToggleMenuFlyoutItem Choice(string name, uint mode, string language = "")
            => Toggle(name, _ => {
                if (View is { } view && menuModeState is { } expected)
                    view.SetDocumentMode(mode, language, preferences.MarkdownFormattedView, expected);
                menusDirty = true; ValidateMenus();
            });
        plainMode = Choice("Plain text", VIEM_DOCUMENT_MODE_PLAIN_TEXT);
        markdownMode = Choice("Markdown", VIEM_DOCUMENT_MODE_MARKDOWN);
        autoMode = Choice("Auto (Plain Text)", VIEM_DOCUMENT_MODE_AUTO);
        codeMode = Sub("Code", autoMode, Separator());
        codeModeCheck = new FontIcon { FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Segoe Fluent Icons"), Glyph = "", FontSize = 12, Width = 16 };
        codeMode.Icon = codeModeCheck;
        foreach (var language in DocumentModes.Languages)
        {
            var item = Choice(language.Name, VIEM_DOCUMENT_MODE_CODE, language.Id);
            languageModes.Add((item, language.Id)); codeMode.Items.Add(item);
        }
    }

    private void RefreshDocumentModeMenu()
    {
        if (plainMode == null) return;
        menuModeState = View == null ? null : DocumentModes.Read(ActivePane!.Document);
        var state = menuModeState;
        plainMode.IsEnabled = markdownMode.IsEnabled = codeMode.IsEnabled = state != null;
        plainMode.IsChecked = state?.Format == "plainText";
        markdownMode.IsChecked = state?.Format == "markdown";
        codeModeCheck.Glyph = state?.Format == "code" ? "\uE73E" : "";
        autoMode.Text = "Auto (" + (state?.DetectedName ?? "Plain Text") + ")";
        autoMode.IsChecked = state?.Automatic == true;
        foreach (var (item, language) in languageModes)
            item.IsChecked = state?.Format == "code" && state.Automatic == false && state.Language == language;
    }
}
