using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private ToggleMenuFlyoutItem plainMode = null!, markdownMode = null!, autoMode = null!;
    private MenuFlyoutSubItem codeMode = null!, obscureLanguageModes = null!;
    private readonly List<(ToggleMenuFlyoutItem Item, string Language)> languageModes = new();
    private bool languageModesLoaded, obscureLanguageModesLoaded;
    private DocumentModeState? menuModeState;
    private FontIcon codeModeCheck = null!, obscureModeCheck = null!;

    private void BuildDocumentModeMenu()
    {
        plainMode = DocumentModeChoice("Plain text", "P", VIEM_DOCUMENT_MODE_PLAIN_TEXT);
        // Only opening the top-level View flyout captures a document revision.
        // Loading nested language rows must retain that snapshot for validation.
        plainMode.Loaded += (_, _) => ValidateMenus();
        markdownMode = DocumentModeChoice("Markdown", "M", VIEM_DOCUMENT_MODE_MARKDOWN);
        autoMode = DocumentModeChoice("Auto (Plain Text)", "A", VIEM_DOCUMENT_MODE_AUTO);
        obscureLanguageModes = Sub("Obscure languages", "O");
        obscureModeCheck = new FontIcon { FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Segoe Fluent Icons"), Glyph = "", FontSize = 12, Width = 16 };
        obscureLanguageModes.Icon = obscureModeCheck;
        obscureLanguageModes.Loaded += (_, _) => EnsureObscureDocumentLanguages();
        codeMode = Sub("Code", "C", autoMode, obscureLanguageModes, Separator());
        codeModeCheck = new FontIcon { FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Segoe Fluent Icons"), Glyph = "", FontSize = 12, Width = 16 };
        codeMode.Icon = codeModeCheck;
        // Loading the Code row means its View flyout is actually opening,
        // including pointer, access-key and UI Automation entry. Populate now
        // so a subsequent C key can open the complete submenu immediately.
        codeMode.Loaded += (_, _) => EnsureDocumentLanguages();
    }

    private ToggleMenuFlyoutItem DocumentModeChoice(string name, string accessKey, uint mode, string language = "")
        => Toggle(name, accessKey, _ => {
            if (View is { } view && menuModeState is { } expected)
                view.SetDocumentMode(mode, language, preferences.MarkdownFormattedView, expected);
            menusDirty = true; ValidateMenus();
        }, validateOnLoad: false);

    private void EnsureDocumentLanguages()
    {
        if (languageModesLoaded) return;
        using var startup = Diagnostics.StartupPerformance.Measure("menus.languages");
        foreach (var language in DocumentModes.Languages.Where(language => language.Primary))
        {
            var item = DocumentModeChoice(language.Name, "", VIEM_DOCUMENT_MODE_CODE, language.Id);
            languageModes.Add((item, language.Id)); codeMode.Items.Add(item);
        }
        languageModesLoaded = true;
        RefreshDocumentLanguageChecks();
    }

    private void EnsureObscureDocumentLanguages()
    {
        if (obscureLanguageModesLoaded) return;
        using var startup = Diagnostics.StartupPerformance.Measure("menus.obscureLanguages");
        foreach (var language in DocumentModes.Languages.Where(language => !language.Primary))
        {
            var item = DocumentModeChoice(language.Name, "", VIEM_DOCUMENT_MODE_CODE, language.Id);
            languageModes.Add((item, language.Id)); obscureLanguageModes.Items.Add(item);
        }
        obscureLanguageModesLoaded = true;
        RefreshDocumentLanguageChecks();
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
        RefreshDocumentLanguageChecks();
    }

    private void RefreshDocumentLanguageChecks()
    {
        var state = menuModeState;
        obscureModeCheck.Glyph = state?.Format == "code" && state.Automatic == false
            && DocumentModes.Languages.Any(language => !language.Primary && language.Id == state.Language) ? "\uE73E" : "";
        foreach (var (item, language) in languageModes)
            item.IsChecked = state?.Format == "code" && state.Automatic == false && state.Language == language;
    }
}
