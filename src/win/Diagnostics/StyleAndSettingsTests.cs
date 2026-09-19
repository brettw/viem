#if DEBUG
using System.Runtime.InteropServices;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Input;
using Viem.Windows.Rendering;
using Viem.Windows.Shell;
using Windows.System;
using Windows.UI.Text;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class StyleAndSettingsTests
{
    [DllImport("user32.dll")] private static extern nint GetForegroundWindow();
    private static void Check(bool condition, string name)
    { if (!condition) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
    private static IEnumerable<T> Children<T>(DependencyObject root) where T : DependencyObject
    {
        if (root is T match) yield return match;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
            foreach (var child in Children<T>(VisualTreeHelper.GetChild(root, i))) yield return child;
    }
    internal static async Task Run(EditorPane pane, EditorWindow window, Preferences preferences)
    {
        window.Activate(); pane.FocusEditor(); await Task.Delay(100);
        await InputRoutingTests.Key(VirtualKey.F8); await Task.Delay(350);
        var styles = window.StyleInspector ?? throw new InvalidOperationException("F8 did not open the style inspector.");
        var hwnd = WinRT.Interop.WindowNative.GetWindowHandle(styles);
        Check(GetForegroundWindow() == hwnd, "native F8 opens the inspector and retains foreground focus");
        Check(styles.AppWindow.Presenter is OverlappedPresenter { IsAlwaysOnTop: false }, "style inspector is a normal window, not always on top");
        Check(styles.FontFamilyControl.ActualHeight is > 0 and <= 28 && window.Menu.ActualHeight <= 32,
            $"compact resources reach editor menus and inspector controls ({window.Menu.ActualHeight}, {styles.FontFamilyControl.ActualHeight})");
        Check(!Children<TextBlock>(styles.RootControl).Any(t => t.Text == "Properties" || t.Text.StartsWith("Changes apply live")), "style inspector omits redundant headings and guidance");
        Check(styles.RootControl.ActualHeight <= styles.RootControl.XamlRoot.Size.Height + 1 && styles.RootControl.XamlRoot.Size.Height - styles.RootControl.ActualHeight < 24,
            "style inspector initially fits its content without spare bottom space");
        await WindowCapture.Save(hwnd, pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".styles.png");
        new ToggleButtonAutomationPeer(styles.ParagraphTab).Toggle(); await Task.Delay(100);
        await WindowCapture.Save(hwnd, pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".styles-paragraph.png");
        window.Activate(); pane.FocusEditor(); await Task.Delay(100);
        Check(GetForegroundWindow() == window.Hwnd, "editor can be raised above the modeless style inspector");
        var menu = window.Menu.Items.Single(m => m.Title == "Paragraph");
        new MenuBarItemAutomationPeer(menu).Expand(); await Task.Delay(100);
        var item = menu.Items.OfType<MenuFlyoutItem>().Single(i => i.Text == "Edit Styles…");
        new MenuFlyoutItemAutomationPeer(item).Invoke(); await Task.Delay(200);
        Check(window.StyleInspector == styles && GetForegroundWindow() == hwnd, "Edit Styles menu reuses and raises the inspector without refocusing the editor");
        styles.Close();
        Check(KeyPolicy.Route(VirtualKey.F8, false, true, false).Kind == VIEM_KEY_FUNCTION
            && KeyPolicy.Route(VirtualKey.F8, false, false, false, true).Kind == VIEM_KEY_FUNCTION, "modified and literal-next F8 remain core function keys");
        await FontChecks(pane, preferences);

        window.Activate(); await window.ShowSettings(); await Task.Delay(350);
        var settings = window.SettingsInspector!;
        var theme = preferences.Theme; string font = preferences.Get("theme", "statusFontFamily", "System"); double size = preferences.StatusFontSize;
        byte[] source = pane.Document.Source(pane.Document.State.document_revision);
        try {
            Check(settings.Categories.Items.Count == 4 && settings.Categories.SelectedIndex == 1 && settings.CurrentPage.Visibility == Visibility.Visible, "settings opens Theme beside a four-category sidebar");
            await WindowCapture.Save(WinRT.Interop.WindowNative.GetWindowHandle(settings), pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".settings.png");
            Check(settings.StatusFont.ActualHeight is > 0 and <= 28 && settings.PaperPreset.ActualHeight <= 32, $"separate settings window inherits compact control density ({settings.StatusFont.ActualHeight}, {settings.PaperPreset.ActualHeight})");
            Check(Children<TextBox>(settings.StatusFont).Any(t => t.Text == settings.StatusFont.Text && t.Text.Length > 0), "status font appears in the editable picker on first opening");
            new ButtonAutomationPeer(settings.PaperPreset).Invoke(); await Task.Delay(100);
            Check(settings.Error.Length == 0 && preferences.Theme == Theme.Paper, "Paper preset commits all theme colors together");
            settings.StatusFont.SelectedItem = "Consolas"; settings.StatusSize.Value = 14; await Task.Delay(100);
            Check(preferences.StatusFontFamily == "Consolas" && preferences.StatusFontSize == 14
                && settings.PreviewStatus.FontFamily.Source == "Consolas" && settings.PreviewStatus.FontSize == 14, "status font changes update preferences and the theme preview");
            new ButtonAutomationPeer(settings.RestoreDefaults).Invoke(); await Task.Delay(100);
            Check(preferences.Theme == Theme.Midnight && preferences.StatusFontSize == 11 && settings.StatusFont.Text == "System", "Restore Defaults resets colors and status typography");
            for (int index = 0; index < 4; index++) {
                settings.Categories.SelectedIndex = index; await Task.Delay(100);
                Check(settings.CurrentPage.Visibility == Visibility.Visible && settings.CurrentPage.ActualHeight > 0, $"settings sidebar displays category {index + 1}");
            }
            var associations = Children<TextBox>(settings.CurrentPage).Single(t => t.Header as string == "Filename associations (JSON)");
            string original = associations.Text;
            associations.Focus(FocusState.Programmatic); associations.Text = "invalid JSON";
            settings.Categories.Focus(FocusState.Programmatic); await Task.Delay(100);
            Check(settings.Error.Length > 0 && System.Text.Encoding.UTF8.GetString(preferences.Associations) == original, "invalid settings edits report an error and preserve saved values");
            associations.Focus(FocusState.Programmatic); associations.Text = original; settings.Categories.Focus(FocusState.Programmatic); await Task.Delay(100);
            Check(settings.Error.Length == 0, "correcting an invalid settings edit clears the error");
            Check(source.AsSpan().SequenceEqual(pane.Document.Source(pane.Document.State.document_revision)), "settings edits do not modify document source");
        }
        finally {
            settings.Close();
            var json = Preferences.ThemeJson(theme); json["statusFontFamily"] = font; json["statusFontSize"] = size;
            preferences.SetSections(new() { ["theme"] = json }); window.Activate(); pane.FocusEditor();
        }
    }
    private static async Task FontChecks(EditorPane pane, Preferences preferences)
    {
        Check(FontCatalog.Families.SequenceEqual(FontCatalog.Families.OrderBy(f => f, StringComparer.CurrentCultureIgnoreCase)), "font family menus use culture-aware alphabetical order");
        var faces = FontCatalog.Faces("Segoe UI");
        var face = faces.First(f => f.Weight == 700 && f.Slant == FontStyle.Italic);
        Check(FontCatalog.Current(face.Name, face.Weight, 1) == face && FontCatalog.Current(face.Name, 617, 1) == null, "font variants match effective traits and leave unknown combinations unresolved");
        Check(FontCatalog.ForFamilyChange("Consolas", face)?.StyleName == face.StyleName, "family changes preserve a matching variant name");
        Check(FontCatalog.Faces("system-ui").Length > 1 && FontCatalog.Faces("viem-missing-font").Length == 0, "font variants resolve generic families without substituting unknown fonts");
        foreach (string family in FontCatalog.Families.Where(f => f.Contains("Flightline", StringComparison.OrdinalIgnoreCase)))
            Check(FontCatalog.Faces(family).Length > 1, $"installed {family} exposes its font variants");
        using var doc = new CoreDocument("<p>A sample for font selection.</p>"u8.ToArray(), format: VIEM_FORMAT_HTML);
        using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        string id = view.CreateStyle(2, "Font test");
        var style = view.Styles().Styles.Single(s => s.Id == id);
        view.EditStyleFont(style, ["Segoe UI", "serif"], null);
        view.SelectAll(); view.AssignStyle(2, id);
        byte[] before = doc.Source(doc.State.document_revision);
        var prior = view.Layout(); long shaped = view.Provider.ShapedCharacters;
        view.EditStyleFont(style, [face.Name, "serif"], face);
        var sheet = view.Styles(); style = sheet.Styles.Single(s => s.Id == id);
        Check(sheet.StringList(style.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)).SequenceEqual(new[] { face.Name, "serif" })
            && style.Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value == 700 && style.Value(VIEM_STYLE_PROPERTY_CHARACTER_SLANT).enum_value == 1, "font variant persists exact face, weight, slant and fallback families");
        var changed = view.Layout();
        Check(!CoreView.SameLayout(prior.Info.identity, changed.Info.identity) && view.Provider.ShapedCharacters > shaped
            && changed.Clusters.SelectMany(c => view.Provider.RenderedFontNames(c.render_run.identifier)).Contains(face.Name), "font change invalidates layout and DirectWrite renders the selected face");
        view.Undo(); Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before), "one undo restores all properties of a font variant change");
        view.Redo(); Check(view.Typography().Family == face.Name, "redo restores the selected font variant");
        var inspector = new StyleWindow(view, preferences); inspector.Activate(); await Task.Delay(200);
        try {
            inspector.StylePicker.SelectedItem = ((StyleDefinition[])inspector.StylePicker.ItemsSource).Single(s => s.Id == id);
            Check(inspector.FontVariantControl.SelectedItem as FontFace == face, "style inspector displays the stored font variant");
            await Task.Delay(150);
            await WindowCapture.Save(WinRT.Interop.WindowNative.GetWindowHandle(inspector), pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".styles-font.png");
            var regular = faces.First(f => f.Weight == 400 && f.Slant == FontStyle.Normal);
            inspector.FontVariantControl.SelectedItem = regular; await Task.Delay(100);
            sheet = view.Styles(); style = sheet.Styles.Single(s => s.Id == id);
            Check(inspector.Error.Length == 0 && sheet.StringList(style.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)).SequenceEqual(new[] { regular.Name, "serif" })
                && style.Value(VIEM_STYLE_PROPERTY_CHARACTER_SLANT).enum_value == 0, "choosing a variant in the native picker applies its traits and preserves fallbacks");
            view.EditStyle(style, VIEM_STYLE_EDIT_CLEAR_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES, default);
            var declare = Children<CheckBox>(inspector.RootControl).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Declare Font family");
            declare.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            Check(inspector.Error.Length == 0 && inspector.FontFamilyControl.IsEnabled
                && view.Styles().Styles.Single(s => s.Id == id).Declares(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES), "an inherited font family can be enabled with the native declaration checkbox");
            view.EditStyleFont(style, ["viem-missing-font", "serif"], null); await Task.Delay(100);
            Check(inspector.FontFamilyControl.Text == "viem-missing-font" && inspector.FontVariantControl.SelectedItem == null,
                "unavailable document fonts remain visible without selecting a substitute variant");
        }
        finally { inspector.Close(); }
        view.SelectAll(); byte[] directBefore = doc.Source(doc.State.document_revision);
        view.SetFont(face.Family, 18, face);
        Check(view.Typography().Family == face.Name && view.Typography().Info.base_weight == 700 && view.Typography().Info.slant == 1, "direct font action applies family, size and variant as a batch");
        view.Undo(); Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(directBefore), "one undo restores a direct font batch");
        foreach (string family in FontCatalog.Families.Where(f => f.Contains("Flightline", StringComparison.OrdinalIgnoreCase))) {
            var variant = FontCatalog.Faces(family).First(f => f.Weight != 400 && f.Slant == FontStyle.Normal);
            view.SelectAll(); view.SetFont(family, 18, variant);
            Check(view.Layout().Clusters.SelectMany(c => view.Provider.RenderedFontNames(c.render_run.identifier)).Contains(variant.Name), $"DirectWrite renders the selected {family} {variant.StyleName} face");
        }
    }
}
#endif
