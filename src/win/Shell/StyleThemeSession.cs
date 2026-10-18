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
    private byte[] sessionThemeContext = Array.Empty<byte>();
    private bool themeSessionNeedsRefresh;
    private void CreateThemeSession(uint? requestedFormat = null)
    {
        themeSessionNeedsRefresh = true;
        uint format = requestedFormat ?? styleDocument.State.format;
        var document = new CoreDocument([], format: format);
        CoreView? next = null;
        try
        {
            if (format != VIEM_FORMAT_CODE) document.InitializeStyleDefaults(preferences.ThemeStyleContext(format));
#if DEBUG
            ThemeSessionCreationForTesting?.Invoke();
#endif
            next = new CoreView(document, CanvasDevice.GetSharedDevice(), DispatcherQueue, 640, 200);
            // Complete fallible configuration queries before publishing the
            // candidate. A failed preview must leave the existing view usable.
            byte[] styles = preferences.ThemeStyleDefaults(format);
            byte[] context = preferences.ThemeStyleContext(format);
            string nextFamily = Preferences.StyleFamily(format);
            if (sessionFamily != nextFamily) SwitchThemeHistory(nextFamily, format);
            var oldView = view; var oldDocument = styleDocument;
            view = next; styleDocument = document; sessionFamily = nextFamily;
            sessionThemeName = preferences.SelectedTheme; sessionThemePath = preferences.SelectedThemePath;
            sessionThemeStyles = styles; sessionThemeContext = context;
            next = null; themeSessionNeedsRefresh = false;
            oldView?.Dispose(); oldDocument?.Dispose();
        }
        catch { next?.Dispose(); if (!ReferenceEquals(styleDocument, document)) document.Dispose(); throw; }
    }
    private void RememberThemeSession()
    {
        sessionThemeName = preferences.SelectedTheme; sessionThemePath = preferences.SelectedThemePath;
        sessionThemeStyles = preferences.ThemeStyleDefaults(styleDocument.State.format);
        sessionThemeContext = preferences.ThemeStyleContext(styleDocument.State.format);
    }
    private void SelectedThemeChanged()
    {
        if (closed) return;
        // Own edits are already applied in this session. Keep the comparison
        // current so a later palette or other-family edit retains this history.
        if (updating) { RememberThemeSession(); return; }
        bool sameTarget = sessionThemeName == preferences.SelectedTheme && sessionThemePath == preferences.SelectedThemePath;
        if (!themeSessionNeedsRefresh && sameTarget && sessionThemeContext.AsSpan().SequenceEqual(preferences.ThemeStyleContext(styleDocument.State.format))) return;
        DismissColorPickers(commit: false);
        if (!sameTarget || !sessionThemeStyles.AsSpan().SequenceEqual(preferences.ThemeStyleDefaults(styleDocument.State.format))) ClearThemeHistory();
        var key = selected?.Key;
        try { CreateThemeSession(); Load(key); }
        catch (Exception exception) { themeSessionNeedsRefresh = true; ShowStyleFailure(exception); }
    }
    private bool PrepareThemeEdit()
    {
        if (!themeSessionNeedsRefresh) return true;
        // The action was formed from controls belonging to the old session.
        // Refresh them, then discard that action rather than replaying it onto
        // a newer configuration or overwriting another editor's saved changes.
        DismissColorPickers(commit: false);
        try {
            CreateThemeSession(); Load(selected?.Key);
            error.Text = "Theme styles were refreshed. Review the current values and repeat the edit.";
            error.Visibility = Microsoft.UI.Xaml.Visibility.Visible;
        }
        catch (Exception exception) { themeSessionNeedsRefresh = true; ShowStyleFailure(exception); }
        return false;
    }
    private void ShowStyleFailure(Exception exception, Exception? recovery = null)
    {
        static string Bounded(string text) => text.Length <= 600 ? text : text[..600] + "…";
        error.Text = Bounded(exception.Message) + (recovery == null ? "" : "\nStyle preview: " + Bounded(recovery.Message));
        error.Visibility = Microsoft.UI.Xaml.Visibility.Visible;
    }
#if DEBUG
    internal Action? ThemeSessionCreationForTesting { get; set; }
    internal Microsoft.UI.Xaml.Controls.ComboBox DocumentPicker => documentPicker;
    internal CoreView ThemeView => view;
    internal void CommitThemeForTesting() => preferences.SaveThemeStyles(view.Document.State.format, view.ExportStyleDefaults());
    internal void RefreshForTesting() => Load(selected?.Key);
    internal bool EditPropertyForTesting(uint property, float number)
        => Try(() => EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, property, CoreView.Number(number)));
    internal bool ClearPropertyForTesting(uint property)
        => Try(() => EditStyle(selected, VIEM_STYLE_EDIT_CLEAR_DECLARATION, property, default));
#endif
}
