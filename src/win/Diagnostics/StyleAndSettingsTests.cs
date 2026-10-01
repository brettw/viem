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
    internal static void StartupFontChecks()
    {
        Check(!FontCatalog.FamilyListLoaded && FontCatalog.FaceDescriptionsRead == 0,
            "empty editor starts without loading font picker lists or enumerating font faces");
        Check(FontCatalog.Resolve("system-ui")?.Family == "Segoe UI" && FontCatalog.Resolve("Segoe UI")?.Family == "Segoe UI",
            "Windows default and generic system fonts resolve to Segoe UI");
        Check(FontCatalog.Resolve("viem-missing-startup-font") == null && FontCatalog.Named("viem-missing-startup-font") == null
            && FontCatalog.FaceDescriptionsRead == 0 && !FontCatalog.FamilyListLoaded,
            "missing document fonts do not trigger a system-wide face scan");
    }
    private static IEnumerable<T> Children<T>(DependencyObject root) where T : DependencyObject
    {
        if (root is T match) yield return match;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
            foreach (var child in Children<T>(VisualTreeHelper.GetChild(root, i))) yield return child;
    }
    internal static async Task Run(EditorPane pane, EditorWindow window, Preferences preferences)
    {
        window.Activate(); pane.FocusEditor(); await Task.Delay(100);
        byte[] sourceBeforeInspector = pane.Document.Source(pane.Document.State.document_revision);
        await InputRoutingTests.Key(VirtualKey.F8); await Task.Delay(350);
        var styles = window.StyleInspector ?? throw new InvalidOperationException("F8 did not open the style inspector.");
        // These chrome/control checks inspect Base Paragraph explicitly. F8 now
        // correctly starts at the caret's style, covered by the following tests.
        styles.StylePicker.SelectedItem = styles.StylePicker.Items.OfType<StyleDefinition>().Single(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
        await Task.Delay(100);
        var hwnd = WinRT.Interop.WindowNative.GetWindowHandle(styles);
        Check(GetForegroundWindow() == hwnd, "native F8 opens the inspector and retains foreground focus");
        Check(styles.AppWindow.Presenter is OverlappedPresenter { IsAlwaysOnTop: false }, "style inspector is a normal window, not always on top");
        Check(styles.AppWindow.Presenter is OverlappedPresenter { IsResizable: false, IsMaximizable: false }, "style inspector has fixed dimensions and cannot be maximized");
        Check(styles.FontFamilyControl.ActualHeight is > 0 and <= 28 && window.Menu.ActualHeight <= 32,
            $"compact resources reach editor menus and inspector controls ({window.Menu.ActualHeight}, {styles.FontFamilyControl.ActualHeight})");
        Check(Children<TextBox>(styles.FontFamilyControl).Any(t => t.Text == styles.FontFamilyControl.Text && t.Text.Length > 0), "style font is visible in the native editable picker on first opening");
        Check(styles.FontFamilyControl.Text == "System Default", "the default theme style displays the portable System Default font in the picker");
        bool sfProInstalled = FontCatalog.Families.Contains("SF Pro", StringComparer.OrdinalIgnoreCase);
        Check(sfProInstalled ? FontCatalog.Resolve("SF Pro")?.Family == "SF Pro" : FontCatalog.Resolve("SF Pro") == null && FontCatalog.Faces("SF Pro").Length == 0,
            "SF Pro resolves only when installed and never aliases Segoe UI");
        Check(((IEnumerable<object>)styles.FontFamilyControl.ItemsSource).OfType<string>().Contains("SF Pro", StringComparer.OrdinalIgnoreCase) == sfProInstalled,
            "the default Windows font picker offers SF Pro only when installed");
        Check(!Children<TextBlock>(styles.RootControl).Any(t => t.Text == "Properties" || t.Text.StartsWith("Changes apply live")), "style inspector omits redundant headings and guidance");
        Check(styles.RootControl.ActualHeight <= styles.RootControl.XamlRoot.Size.Height + 1 && styles.RootControl.XamlRoot.Size.Height - styles.RootControl.ActualHeight < 24,
            "style inspector initially fits its content without spare bottom space");
        await WindowCapture.Save(hwnd, pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".styles.png");
        var characterSize = styles.AppWindow.ClientSize;
        styles.ParagraphTab.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space); await Task.Delay(200);
        Check(styles.ParagraphPanel.Visibility == Visibility.Visible && styles.CharacterPanel.Visibility == Visibility.Collapsed, "native Paragraph tab displays paragraph controls");
        Check(styles.AppWindow.ClientSize == characterSize, "character and paragraph tabs occupy the same fixed window size");
        await LineSpacingChecks(pane, styles, sourceBeforeInspector);
        Check(!Children<Button>(styles.RootControl).Any(b => b.Content as string == "Restore Defaults" && b.Visibility == Visibility.Visible), "prose theme styles do not offer a separate reset");
        Check(!styles.VisitParent.IsEnabled && !styles.VisitNext.IsEnabled, "base paragraph has no relationship navigation targets");
        await WindowCapture.Save(hwnd, pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".styles-paragraph.png");
        window.Activate(); pane.FocusEditor(); await Task.Delay(100);
        Check(GetForegroundWindow() == window.Hwnd, "editor can be raised above the modeless style inspector");
        var menu = window.Menu.Items.Single(m => m.Title == "Style");
        new MenuBarItemAutomationPeer(menu).Expand(); await Task.Delay(100);
        var item = menu.Items.OfType<MenuFlyoutItem>().Single(i => i.Text == "Edit Styles…");
        new MenuFlyoutItemAutomationPeer(item).Invoke(); await Task.Delay(200);
        Check(window.StyleInspector == styles && GetForegroundWindow() == hwnd, "Edit Styles menu reuses and raises the inspector without refocusing the editor");
        styles.Close();
        Check(KeyPolicy.Route(VirtualKey.F8, false, true, false).Kind == VIEM_KEY_FUNCTION
            && KeyPolicy.Route(VirtualKey.F8, false, false, false, true).Kind == VIEM_KEY_FUNCTION, "modified and literal-next F8 remain core function keys");
        await FontChecks(pane, preferences);
        await StyleFontSizeTests.Run(pane, preferences);
        await FormatMenuTests.Run(preferences);
        await DocumentModeMenuTests.Run(preferences);
        await ListInteractionTests.Run(preferences);
        await CodeStyleChecks(pane, preferences);
        await StyleInspectorBehaviorTests.Run(pane, preferences);

        await RunSettings(pane, window, preferences);
    }

    internal static async Task RunLineSpacing(EditorPane pane, Preferences preferences)
    {
        byte[] sourceBeforeInspector = pane.Document.Source(pane.Document.State.document_revision);
        var styles = new StyleWindow(pane.View!, preferences, followCaret: false);
        try {
            styles.Activate(); await Task.Delay(200);
            styles.StylePicker.SelectedItem = styles.StylePicker.Items.OfType<StyleDefinition>()
                .Single(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
            styles.ParagraphTab.IsChecked = true; await Task.Delay(100);
            Check(styles.ParagraphPanel.Visibility == Visibility.Visible, "focused line-spacing diagnostics display paragraph controls");
            await LineSpacingChecks(pane, styles, sourceBeforeInspector);
        }
        finally { styles.Close(); }
    }

    private static async Task LineSpacingChecks(EditorPane pane, StyleWindow styles, byte[] sourceBeforeInspector)
    {
        var lineSpacing = Children<ComboBox>(styles.ParagraphPanel).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Line spacing");
        var lineSpacingAmount = Children<NumberBox>(styles.ParagraphPanel).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Line spacing amount");
        var lineSpacingUnits = Children<TextBlock>(styles.ParagraphPanel).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Line spacing units");
        var spaceBefore = Children<NumberBox>(styles.ParagraphPanel).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Space before");
        var spaceAfter = Children<NumberBox>(styles.ParagraphPanel).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Space after");
        var startIndent = Children<NumberBox>(styles.ParagraphPanel).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Start indent");
        Check(spaceBefore.Width < startIndent.Width && spaceAfter.Width < startIndent.Width,
            "paragraph spacing fields are narrower to preserve the fixed inspector width");
        Check(sourceBeforeInspector.AsSpan().SequenceEqual(pane.Document.Source(pane.Document.State.document_revision)),
            "opening and displaying style controls does not rewrite document source");
        Check(lineSpacing.SelectedIndex == 0 && lineSpacingAmount.IsEnabled && Math.Abs(lineSpacingAmount.Minimum - .1) < .0001
            && Math.Abs(lineSpacingAmount.Value - 1) < .0001
            && lineSpacingAmount.Text == "1" && Math.Abs(lineSpacingAmount.SmallChange - .1) < .0001 && lineSpacingUnits.Text == "\u00D7",
            "Normal line spacing is an enabled compact 1 multiplier with tenth steps");
        lineSpacingAmount.Value += lineSpacingAmount.SmallChange; await Task.Delay(100);
        Check(lineSpacing.SelectedIndex == 1 && Math.Abs(lineSpacingAmount.Value - 1.1) < .0001,
            "stepping Normal away from 1 changes it to a Multiple value by 0.1");
        lineSpacingAmount.Value = .7999999523162842; await Task.Delay(100);
        Check(lineSpacing.SelectedIndex == 1 && Math.Abs(lineSpacingAmount.Value - .8) < .0001
            && lineSpacingAmount.Text.Length <= 3 && lineSpacingAmount.Text.StartsWith("0") && lineSpacingAmount.Text.EndsWith("8"),
            "Multiple line spacing rounds float noise to one compact decimal place");
        lineSpacingAmount.Value = 1; await Task.Delay(100);
        Check(lineSpacing.SelectedIndex == 0 && lineSpacingAmount.IsEnabled && lineSpacingAmount.Text == "1",
            "a Multiple value of exactly 1 changes back to Normal without disabling its amount");
        lineSpacing.SelectedIndex = 1; await Task.Delay(100);
        Check(lineSpacing.SelectedIndex == 1 && Math.Abs(lineSpacingAmount.Value - 1.1) < .0001,
            "choosing Multiple from Normal starts at 1.1");
        lineSpacingAmount.Value -= lineSpacingAmount.SmallChange; await Task.Delay(100);
        Check(lineSpacing.SelectedIndex == 0 && Math.Abs(lineSpacingAmount.Value - 1) < .0001,
            "stepping a Multiple value down to 1 changes it to Normal");
        var nearNormal = CoreView.Enum(VIEM_STYLE_VALUE_LINE_SPACING, VIEM_STYLE_LINE_SPACING_MULTIPLIER);
        nearNormal.number = .99999994f;
        var revisionBeforeNearNormal = pane.Document.State.document_revision;
        styles.ThemeView.EditStyle((StyleDefinition)styles.StylePicker.SelectedItem, VIEM_STYLE_EDIT_SET_DECLARATION,
            VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING, nearNormal);
        styles.CommitThemeForTesting();
        await Task.Delay(100);
        Check(pane.Document.State.document_revision == revisionBeforeNearNormal && lineSpacing.SelectedIndex == 0
            && Math.Abs(lineSpacingAmount.Value - 1) < .0001 && lineSpacingAmount.Text == "1",
            "an existing multiplier that rounds to 1 displays as Normal without a presentation-time rewrite");
        lineSpacing.SelectedIndex = 1; await Task.Delay(100);
        Check(lineSpacing.SelectedIndex == 1 && Math.Abs(lineSpacingAmount.Value - 1.1) < .0001,
            "choosing Multiple from a normalized legacy 1 multiplier starts at 1.1");
        lineSpacing.SelectedIndex = 3; await Task.Delay(100);
        Check(lineSpacingAmount.Minimum == 0 && Math.Abs(lineSpacingAmount.SmallChange - 1) < .0001 && lineSpacingUnits.Text == "pt",
            "point-based line spacing permits zero, uses point units, and keeps whole-point steps");
        lineSpacingAmount.Value = 0; await Task.Delay(100);
        Check(lineSpacing.SelectedIndex == 3 && lineSpacingAmount.Value == 0 && lineSpacingAmount.Text == "0",
            "Exact line spacing accepts and compactly displays zero points");
        lineSpacingAmount.Value = 19.299999237060547; await Task.Delay(100);
        Check(Math.Abs(lineSpacingAmount.Value - 19.3) < .0001 && lineSpacingAmount.Text.Length <= 4
            && lineSpacingAmount.Text.StartsWith("19") && lineSpacingAmount.Text.EndsWith("3"),
            "point-based line spacing also rounds float noise to one compact decimal place");
        lineSpacingAmount.Value = 19; await Task.Delay(100);
        Check(lineSpacingAmount.Text == "19", "whole point line spacing omits an unnecessary fractional part");
        lineSpacing.SelectedIndex = 2; await Task.Delay(100);
        lineSpacingAmount.Value = 0; await Task.Delay(100);
        Check(lineSpacing.SelectedIndex == 2 && lineSpacingAmount.Minimum == 0 && lineSpacingAmount.Value == 0
            && lineSpacingAmount.Text == "0" && lineSpacingUnits.Text == "pt",
            "At least line spacing also accepts and compactly displays zero points");
        lineSpacing.SelectedIndex = 0; await Task.Delay(100);
    }

    internal static async Task RunSettings(EditorPane pane, EditorWindow window, Preferences preferences)
    {
        await StyleDefaultsLoadingTests.Run(preferences);
        window.Activate(); await window.ShowSettings(); await Task.Delay(350);
        var settings = window.SettingsInspector!;
        string? priorTheme = preferences.SelectedTheme, priorPath = preferences.SelectedThemePath;
        string paperPath = preferences.ThemeFiles.Single(file => file.Name == "Paper").Path; byte[] priorPaper = File.ReadAllBytes(paperPath);
        string discoveredThemePath = Path.Combine(preferences.ThemesDirectory, "Discovered-" + Guid.NewGuid().ToString("N")[..8] + ".json");
        var theme = preferences.Theme; string font = preferences.Get("theme", "statusFontFamily", "System"); double size = preferences.StatusFontSize;
        byte[] source = pane.Document.Source(pane.Document.State.document_revision);
        try {
            Check(settings.Categories.Items.Count == 3 && settings.Categories.SelectedIndex == 1 && settings.CurrentPage.Visibility == Visibility.Visible, "settings opens Theme beside a three-category sidebar without Code");
            await WindowCapture.Save(WinRT.Interop.WindowNative.GetWindowHandle(settings), pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".settings.png");
            Check(settings.StatusFont.ActualHeight is > 0 and <= 28 && settings.NewTheme.ActualHeight <= 32, $"separate settings window inherits compact control density ({settings.StatusFont.ActualHeight}, {settings.NewTheme.ActualHeight})");
            Check(Children<TextBox>(settings.StatusFont).Any(t => t.Text == settings.StatusFont.Text && t.Text.Length > 0), "status font appears in the editable picker on first opening");
            File.WriteAllBytes(discoveredThemePath, priorPaper);
            byte[] beforeDiscovery = File.ReadAllBytes(Path.Combine(preferences.DirectoryPath, "config.json"));
            settings.ThemePicker.IsDropDownOpen = true; await Task.Delay(100);
            Check(settings.ThemePicker.Items.OfType<Preferences.ThemeFile>().Any(file => file.Path == discoveredThemePath)
                && preferences.SelectedTheme == priorTheme && preferences.SelectedThemePath == priorPath
                && File.ReadAllBytes(Path.Combine(preferences.DirectoryPath, "config.json")).SequenceEqual(beforeDiscovery),
                "opening the theme dropdown discovers external files without changing selection or saving settings");
            settings.ThemePicker.IsDropDownOpen = false;
            settings.ThemePicker.SelectedItem = settings.ThemePicker.Items.OfType<Preferences.ThemeFile>().Single(file => file.Name == "Paper"); await Task.Delay(100);
            Check(settings.Error.Length == 0 && preferences.SelectedTheme == "Paper" && preferences.SelectedThemePath == paperPath
                && preferences.Theme.Background == Theme.Paper.Background, "Paper selection commits the named aggregate theme");
            settings.StatusFont.SelectedItem = "Consolas"; settings.StatusSize.Value = 14; await Task.Delay(100);
            Check(preferences.StatusFontFamily == "Consolas" && preferences.StatusFontSize == 14
                && settings.PreviewStatus.FontFamily.Source == "Consolas" && settings.PreviewStatus.FontSize == 14, "status font changes update preferences and the theme preview");
            settings.ThemePicker.SelectedItem = "Default"; await Task.Delay(100);
            Check(preferences.Theme == Theme.Midnight && preferences.StatusFontSize == 11 && settings.StatusFont.Text == "System", "Default selection restores built-in colors and status typography");
            for (int index = 0; index < 3; index++) {
                settings.Categories.SelectedIndex = index; await Task.Delay(100);
                Check(settings.CurrentPage.Visibility == Visibility.Visible && settings.CurrentPage.ActualHeight > 0, $"settings sidebar displays category {index + 1}");
            }
            Check(!Children<TextBlock>(settings.RootControl).Any(t => t.Text == "Code")
                && !Children<Button>(settings.RootControl).Any(b => b.Content as string == "Edit Code Styles…"),
                "settings has no Code section or Code Styles entrypoint");
            Check(source.AsSpan().SequenceEqual(pane.Document.Source(pane.Document.State.document_revision)), "settings edits do not modify document source");
        }
        finally {
            settings.Close(); File.Delete(discoveredThemePath); Preferences.AtomicWrite(paperPath, priorPaper);
            var json = Preferences.ThemeJson(theme); json["statusFontFamily"] = font; json["statusFontSize"] = size;
            preferences.SelectTheme(priorTheme, priorPath);
            preferences.SetSections(new() { ["theme"] = json }); window.Activate(); pane.FocusEditor();
        }
    }
    private static async Task FontChecks(EditorPane pane, Preferences preferences)
    {
        Check(FontCatalog.Families.SequenceEqual(FontCatalog.Families.OrderBy(f => f, StringComparer.CurrentCultureIgnoreCase)), "font family menus use culture-aware alphabetical order");
        var faces = FontCatalog.Faces("Segoe UI");
        var face = faces.First(f => f.Weight == 700 && f.Slant == FontStyle.Italic);
        int descriptions = FontCatalog.FaceDescriptionsRead;
        Check(FontCatalog.Faces("segoe ui").SequenceEqual(faces) && FontCatalog.FaceDescriptionsRead == descriptions,
            "repeated family variant requests reuse cached discovery");
        Check(FontCatalog.Named(face.Name) == face, "indexed PostScript lookup retains exact face metadata");
        descriptions = FontCatalog.FaceDescriptionsRead;
        Check(FontCatalog.Named(face.Name.ToLowerInvariant()) == face && FontCatalog.FaceDescriptionsRead == descriptions,
            "repeated PostScript requests reuse cached discovery case-insensitively");
        Check(FontCatalog.Current(face.Name, face.Weight, 1) == face && FontCatalog.Current(face.Name, 617, 1) == null, "font variants match effective traits and leave unknown combinations unresolved");
        Check(FontCatalog.ForFamilyChange("Consolas", face)?.StyleName == face.StyleName, "family changes preserve a matching variant name");
        Check(FontCatalog.Faces("system-ui").Length > 1 && FontCatalog.Faces("viem-missing-font").Length == 0, "font variants resolve generic families without substituting unknown fonts");
        Check(FontCatalog.Faces("ui-monospace").Length > 1, "the portable monospace token also resolves real font variants");
        Check(FontCatalog.DisplayFamily("system-ui") == "System Default" && FontCatalog.DisplayFamily("ui-monospace") == "System Monospace",
            "the style dialog labels the two portable system tokens instead of a resolved font name");
        Check(FontCatalog.StorageFamily("System Default") == "system-ui" && FontCatalog.StorageFamily("System Monospace") == "ui-monospace"
            && FontCatalog.StorageFamily("Segoe UI") == null, "only the picker's own two labels reverse to a portable token");
        BundledFontChecks(pane);
        using var doc = new CoreDocument("A sample for font selection."u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        string id = "Code";
        var style = view.Styles().Styles.Single(s => s.Id == id);
        view.EditStyleFont(style, ["Segoe UI", "serif"], null);
        view.SelectAll(); view.AssignStyle(2, id);
        byte[] before = doc.Source(doc.State.document_revision);
        var prior = view.Layout(); long shaped = view.Provider.ShapedCharacters;
        view.EditStyleFont(style, [face.Name, "serif"], face);
        var sheet = view.Styles(); style = sheet.Styles.Single(s => s.Id == id);
        Check(sheet.StringList(style.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)).SequenceEqual(new[] { face.Name, "serif" })
            && style.Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value == 700 && !style.Declares(VIEM_STYLE_PROPERTY_CHARACTER_SLANT), "font variant persists exact face and base weight without declaring semantic Italic");
        var changed = view.Layout();
        Check(!CoreView.SameLayout(prior.Info.identity, changed.Info.identity) && view.Provider.ShapedCharacters > shaped
            && changed.Clusters.SelectMany(c => view.Provider.RenderedFontNames(c.render_run.identifier)).Contains(face.Name), "font change invalidates layout and DirectWrite renders the selected face");
        Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before), "theme font variants leave Markdown source unchanged");
        byte[] originalThemeStyles = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
        preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, view.ExportStyleDefaults());
        var inspector = new StyleWindow(view, preferences); inspector.Activate(); await Task.Delay(200);
        try {
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == id);
            Check(!inspector.ParagraphTab.IsEnabled, "character styles disable the paragraph tab");
            Check(inspector.FontVariantControl.SelectedItem as FontFace == face, "style inspector displays the stored font variant");
            await Task.Delay(150);
            await WindowCapture.Save(WinRT.Interop.WindowNative.GetWindowHandle(inspector), pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".styles-font.png");
            var regular = faces.First(f => f.Weight == 400 && f.Slant == FontStyle.Normal);
            inspector.FontVariantControl.SelectedItem = regular; await Task.Delay(100);
            sheet = inspector.ThemeView.Styles(); style = sheet.Styles.Single(s => s.Id == id);
            Check(inspector.Error.Length == 0 && sheet.StringList(style.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)).SequenceEqual(new[] { regular.Name, "serif" })
                && style.Value(VIEM_STYLE_PROPERTY_CHARACTER_SLANT).enum_value == 0, "choosing a variant in the native picker applies its traits and preserves fallbacks");
            inspector.ThemeView.EditStyle(style, VIEM_STYLE_EDIT_CLEAR_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES, default); inspector.RefreshForTesting();
            Check(inspector.FontFamilyControl.Text.Length == 0 && !inspector.FontFamilyControl.IsEnabled, "inherited font fields are empty until overridden");
            var declare = Children<CheckBox>(inspector.RootControl).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Declare Font family");
            declare.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            Check(inspector.Error.Length == 0 && inspector.FontFamilyControl.IsEnabled
                && inspector.ThemeView.Styles().Styles.Single(s => s.Id == id).Declares(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES), "an inherited font family can be enabled with the native declaration checkbox");
            inspector.ThemeView.EditStyleFont(style, ["viem-missing-font", "serif"], null); inspector.RefreshForTesting(); await Task.Delay(100);
            Check(inspector.FontFamilyControl.Text == "viem-missing-font" && inspector.FontVariantControl.SelectedItem == null,
                "unavailable document fonts remain visible without selecting a substitute variant");
            var familyItemsSource = ((IEnumerable<object>)inspector.FontFamilyControl.ItemsSource).ToArray();
            Check(familyItemsSource[2] is ComboBoxItem { IsEnabled: false, IsHitTestVisible: false },
                "the font family picker's third row is a real disabled, unselectable separator, not text");
            var familyItems = familyItemsSource.OfType<string>().ToArray();
            Check(familyItems.Take(2).SequenceEqual(new[] { "System Default", "System Monospace" })
                && familyItems.Skip(2).SequenceEqual(familyItems.Skip(2).OrderBy(f => f, StringComparer.CurrentCultureIgnoreCase)),
                "the font family picker lists the two system entries first, then a native separator, then installed fonts in order");
            inspector.FontFamilyControl.SelectedItem = "System Default"; await Task.Delay(100);
            sheet = inspector.ThemeView.Styles(); style = sheet.Styles.Single(s => s.Id == id);
            Check(inspector.Error.Length == 0 && sheet.StringList(style.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)).SequenceEqual(new[] { "system-ui", "serif" }),
                "choosing System Default in the native picker stores its portable token, not a resolved font name, and keeps the fallback tail");
            Check(inspector.FontFamilyControl.Text == "System Default", "the stored portable token displays as its picker label again");
            string parentId = "Heading1", childId = "Heading2";
            var child = inspector.ThemeView.Styles().Styles.Single(s => s.Id == childId);
            inspector.ThemeView.EditStyleString(child, VIEM_STYLE_EDIT_SET_PARENT, 0, parentId);
            inspector.ThemeView.EditStyleString(child, VIEM_STYLE_EDIT_SET_NEXT_STYLE, 0, parentId); inspector.RefreshForTesting();
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == childId);
            byte[] related = doc.Source(doc.State.document_revision);
            new ButtonAutomationPeer(inspector.VisitParent).Invoke(); await Task.Delay(100);
            Check(((StyleDefinition)inspector.StylePicker.SelectedItem).Id == parentId
                && !((object[])inspector.ParentPicker.ItemsSource).OfType<StyleDefinition>().Any(s => s.Id == childId), "parent arrow selects the referenced style and parent choices exclude descendants");
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == childId);
            new ButtonAutomationPeer(inspector.VisitNext).Invoke(); await Task.Delay(100);
            Check(((StyleDefinition)inspector.StylePicker.SelectedItem).Id == parentId && related.AsSpan().SequenceEqual(doc.Source(doc.State.document_revision)), "next paragraph arrow navigates without editing styles");
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == childId);
            inspector.ParagraphTab.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            var declareAlignment = Children<CheckBox>(inspector.ParagraphPanel).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Declare Alignment");
            declareAlignment.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            byte[] alignmentBefore = doc.Source(doc.State.document_revision);
            var center = Children<Microsoft.UI.Xaml.Controls.Primitives.ToggleButton>(inspector.ParagraphPanel).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Align center");
            center.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            Check(inspector.Error.Length == 0 && inspector.ThemeView.Styles().Styles.Single(s => s.Id == childId).Value(VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT).enum_value == 3, "native paragraph alignment buttons apply the chosen alignment");
            Check(alignmentBefore.AsSpan().SequenceEqual(doc.Source(doc.State.document_revision)), "theme paragraph alignment leaves authored source unchanged");
        }
        finally { inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, originalThemeStyles); }
    }

    private static void BundledFontChecks(EditorPane pane)
    {
        string[] families = ["Flightline Code"];
        using var doc = new CoreDocument("Writing 0123"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        var style = view.Styles().Styles.Single(s => s.Id == "Code");
        view.SelectAll(); view.AssignStyle(2, "Code");
        int count = 0;
        foreach (string family in families)
        {
            Check(FontCatalog.Families.Contains(family), $"bundled {family} appears in the font picker");
            var faces = FontCatalog.Faces(family);
            Check(faces.Length == 12, $"bundled {family} exposes every variant");
            foreach (var face in faces)
            {
                Check(face.Source is { IsFile: true } && File.Exists(face.Source.LocalPath), $"{face.Name} resolves to a bundled file");
                string rendering = FontCatalog.RenderingFamily(face.Family, face.Weight, face.Slant, face.Stretch);
                Check(rendering == face.Source!.AbsoluteUri + "#" + face.Family, $"{face.Name} selects its bundled variant file");
                view.EditStyleFont(style, [face.Name, "serif"], face);
                style = view.Styles().Styles.Single(s => s.Id == "Code");
                var layout = view.Layout();
                Check(layout.Clusters.SelectMany(c => view.Provider.RenderedFontNames(c.render_run.identifier)).Contains(face.Name),
                    $"DirectWrite shapes {face.Name} from app-local resources");
                count++;
            }
        }
        Check(count == 12, "all 12 bundled static font faces render through the native provider");
        VariableFontTests.RecursiveFontChecks(pane);
        Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual("Writing 0123"u8), "bundled font selection preserves document source");
        Check(!System.Text.Encoding.UTF8.GetString(view.ExportStyleDefaults()).Contains("file:", StringComparison.OrdinalIgnoreCase),
            "saved font styles contain portable names rather than resource URIs");
    }
    private static async Task CodeStyleChecks(EditorPane pane, Preferences preferences)
    {
        using var doc = new CoreDocument("A Code style sample."u8.ToArray(), format: VIEM_FORMAT_CODE);
        using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        byte[] before = view.ExportStyleDefaults(), source = doc.Source(doc.State.document_revision);
        var inspector = new StyleWindow(view, preferences); inspector.Activate(); await Task.Delay(200);
        try {
            var restore = Children<Button>(inspector.RootControl).Single(b => b.Content as string == "Restore Defaults");
            Check(restore.Visibility == Visibility.Visible, "Code theme styles expose Restore Defaults");
            Check(Children<CheckBox>(inspector.CharacterPanel).All(c => c.IsChecked == true && !c.IsEnabled), "base paragraph overrides are checked and fixed while values remain editable");
            string parentId = view.CreateCodeStyle("Parent"), childId = view.CreateCodeStyle("Child");
            var child = view.Styles().Styles.Single(s => s.Id == childId);
            view.EditStyleString(child, VIEM_STYLE_EDIT_SET_PARENT, 0, parentId); inspector.RefreshForTesting();
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == childId);
            byte[] related = view.ExportStyleDefaults();
            new ButtonAutomationPeer(inspector.VisitParent).Invoke(); await Task.Delay(100);
            Check(((StyleDefinition)inspector.StylePicker.SelectedItem).Id == parentId
                && !((object[])inspector.ParentPicker.ItemsSource).OfType<StyleDefinition>().Any(s => s.Id == childId), "parent arrow selects the referenced style and parent choices exclude descendants");
            Check(related.AsSpan().SequenceEqual(view.ExportStyleDefaults()), "Code parent navigation does not edit shared styles");
            new ButtonAutomationPeer(restore).Invoke(); await Task.Delay(100);
            Check(inspector.Error.Length == 0 && !view.Styles().Styles.Any(style => style.Id == childId || style.Id == parentId),
                "Restore Defaults replaces only the current theme's Code styles");
            inspector.UndoThemeForTesting();
            Check(related.AsSpan().SequenceEqual(view.ExportStyleDefaults()), "Code theme defaults restoration is one settings Undo");
            inspector.RedoThemeForTesting();
            Check(!view.Styles().Styles.Any(style => style.Id == childId || style.Id == parentId)
                && source.AsSpan().SequenceEqual(doc.Source(doc.State.document_revision)),
                "Code theme defaults Redo preserves source bytes");
            var builtins = view.Styles();
            var comment = builtins.Styles.Single(s => s.Id == "syntax:Comment");
            Check(!builtins.Styles.Any(s => s.Id.StartsWith("syntax:@", StringComparison.Ordinal))
                && builtins.Styles.Single(s => s.Id == "syntax:Comment.documentation").Parent == comment.Id,
                "Tree-sitter captures share canonical styles and dotted captures derive directly from their parent");
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == comment.Id);
            var colorDeclaration = Children<CheckBox>(inspector.RootControl).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Declare Text Color");
            Check(colorDeclaration.IsChecked == true && comment.Declares(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND),
                "the built-in Comment color is a visible declaration");
            colorDeclaration.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            Check(inspector.Error.Length == 0 && colorDeclaration.IsChecked == false
                && !view.Styles().Styles.Single(s => s.Id == comment.Id).Declares(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND),
                "unchecking the built-in Comment color resumes inheritance");
            byte[] cleared = preferences.ThemeStyleDefaults(VIEM_FORMAT_CODE);
            view.ReplaceCodeStyles(cleared);
            Check(!view.Styles().Styles.Single(s => s.Id == comment.Id).Declares(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND),
                "saved Code styles do not restore a cleared built-in declaration");
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == "Paragraph");
            inspector.CharacterTab.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space); await Task.Delay(200);
            await WindowCapture.Save(WinRT.Interop.WindowNative.GetWindowHandle(inspector), pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".code-styles-character.png");
            inspector.ParagraphTab.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space); await Task.Delay(200);
            Check(inspector.ParagraphPanel.Visibility == Visibility.Visible, "Code Styles also exposes base paragraph properties");
            await WindowCapture.Save(WinRT.Interop.WindowNative.GetWindowHandle(inspector), pane.Canvas.Device, FrontendSmokeTests.ReportPath + ".code-styles-paragraph.png");
        }
        finally {
            inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_CODE, before);
        }
    }
}
#endif
