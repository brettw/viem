using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    private ToggleMenuFlyoutItem plainMode = null!, markdownMode = null!, codeMode = null!, autoMode = null!;
    private MenuFlyoutSubItem codeLanguageMenu = null!, obscureLanguageModes = null!;
    private readonly List<(ToggleMenuFlyoutItem Item, string Language)> languageModes = new();
    private bool languageModesLoaded, obscureLanguageModesLoaded;
    private DocumentModeState? menuModeState;
    private FontIcon codeLanguageCheck = null!, obscureModeCheck = null!;

    private void BuildDocumentModeMenu()
    {
        plainMode = DocumentModeChoice("Plain text", "P", VIEM_DOCUMENT_MODE_PLAIN_TEXT);
        // Only opening a top-level Format or File flyout captures a document revision.
        // Loading nested language rows must retain that snapshot for validation.
        plainMode.Loaded += (_, _) => ValidateMenus();
        markdownMode = DocumentModeChoice("Markdown Preview", "M", VIEM_DOCUMENT_MODE_MARKDOWN);
        codeMode = DocumentModeChoice("Code", "C", VIEM_DOCUMENT_MODE_CODE);
        autoMode = DocumentModeChoice("Auto (Plain Text)", "A", VIEM_DOCUMENT_MODE_AUTO);
        obscureLanguageModes = Sub("Obscure languages", "O");
        obscureModeCheck = new FontIcon { FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Segoe Fluent Icons"), Glyph = "", FontSize = 12, Width = 16 };
        obscureLanguageModes.Icon = obscureModeCheck;
        obscureLanguageModes.Loaded += (_, _) => EnsureObscureDocumentLanguages();
        codeLanguageMenu = Sub("Code language", "G", autoMode, obscureLanguageModes, Separator());
        codeLanguageCheck = new FontIcon { FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Segoe Fluent Icons"), Glyph = "", FontSize = 12, Width = 16 };
        codeLanguageMenu.Icon = codeLanguageCheck;
        // Loading the Code language row means its File flyout is actually opening,
        // including pointer, access-key and UI Automation entry. Populate now
        // so a subsequent access key can open the complete submenu immediately.
        codeLanguageMenu.Loaded += (_, _) => EnsureDocumentLanguages();
    }

    private ToggleMenuFlyoutItem DocumentModeChoice(string name, string accessKey, uint mode, string language = "")
        => Toggle(name, accessKey, _ => {
            if (View is { } view && menuModeState is { } expected)
                view.SetDocumentMode(mode, language, expected);
            menusDirty = true; ValidateMenus();
        }, validateOnLoad: false);

    private void EnsureDocumentLanguages()
    {
        if (languageModesLoaded) return;
        using var startup = Diagnostics.StartupPerformance.Measure("menus.languages");
        foreach (var language in DocumentModes.Languages.Where(language => language.Primary))
        {
            var item = DocumentModeChoice(language.Name, "", VIEM_DOCUMENT_MODE_CODE, language.Id);
            languageModes.Add((item, language.Id)); codeLanguageMenu.Items.Add(item);
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
        plainMode.IsEnabled = markdownMode.IsEnabled = codeMode.IsEnabled = codeLanguageMenu.IsEnabled = state != null;
        plainMode.IsChecked = state?.Format == "plainText";
        markdownMode.IsChecked = state?.Format == "markdown";
        codeMode.IsChecked = state?.Format == "code";
        codeLanguageCheck.Glyph = codeMode.IsChecked ? "\uE73E" : "";
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
