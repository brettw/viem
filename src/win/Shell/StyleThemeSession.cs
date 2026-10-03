using Microsoft.Graphics.Canvas;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    private CoreView documentView = null!;
    private CoreDocument styleDocument = null!;
    private string sessionFamily = "";
    private string ThemeStyleTitle => "Theme styles — " + (sessionFamily switch {
        "code" => "Code", "markdown" => "Markdown", _ => "Plain Text"
    }) + " — " + preferences.ThemeDisplayName;
    private string? sessionThemeName, sessionThemePath;
    private byte[] sessionThemeStyles = Array.Empty<byte>();
    private void CreateThemeSession(uint? requestedFormat = null)
    {
        uint sourceFormat = requestedFormat ?? styleDocument.State.format;
        uint format = sourceFormat == VIEM_FORMAT_MARKDOWN_SOURCE ? VIEM_FORMAT_MARKDOWN : sourceFormat;
        var document = new CoreDocument([], format: format);
        try
        {
            if (format != VIEM_FORMAT_CODE) document.InitializeStyleDefaults(preferences.ThemeStyleDefaults(format));
            var next = new CoreView(document, CanvasDevice.GetSharedDevice(), DispatcherQueue, 640, 200);
            var oldView = view; var oldDocument = styleDocument;
            string nextFamily = Preferences.StyleFamily(format);
            if (sessionFamily != nextFamily) SwitchThemeHistory(nextFamily, format);
            view = next; styleDocument = document; sessionFamily = nextFamily;
            RememberThemeSession();
            oldView?.Dispose(); oldDocument?.Dispose();
        }
        catch { document.Dispose(); throw; }
    }
    private void RememberThemeSession()
    {
        sessionThemeName = preferences.SelectedTheme; sessionThemePath = preferences.SelectedThemePath;
        sessionThemeStyles = preferences.ThemeStyleDefaults(styleDocument.State.format);
    }
    private void SelectedThemeChanged()
    {
        if (closed) return;
        // Own edits are already applied in this session. Keep the comparison
        // current so a later palette or other-family edit retains this history.
        if (updating) { RememberThemeSession(); return; }
        if (sessionThemeName == preferences.SelectedTheme && sessionThemePath == preferences.SelectedThemePath
            && sessionThemeStyles.AsSpan().SequenceEqual(preferences.ThemeStyleDefaults(styleDocument.State.format))) return;
        DismissColorPickers(commit: false); ClearThemeHistory();
        var key = selected?.Key;
        CreateThemeSession(); Load(key);
    }
#if DEBUG
    internal Microsoft.UI.Xaml.Controls.ComboBox DocumentPicker => documentPicker;
    internal CoreView ThemeView => view;
    internal void CommitThemeForTesting() => preferences.SaveThemeStyles(view.Document.State.format, view.ExportStyleDefaults());
    internal void RefreshForTesting() => Load(selected?.Key);
    internal bool EditPropertyForTesting(uint property, float number)
        => Try(() => view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, property, CoreView.Number(number)));
#endif
}
