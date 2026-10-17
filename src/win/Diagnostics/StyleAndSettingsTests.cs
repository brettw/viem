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
        // A theme can request a variable face even in an empty document. Its
        // targeted metadata lookup is valid startup work; generic and missing
        // family lookups must not enumerate any additional face descriptions.
        int descriptions = FontCatalog.FaceDescriptionsRead;
        Check(!FontCatalog.FamilyListLoaded, "empty editor starts without loading font picker lists");
        Check(FontCatalog.Resolve("system-ui")?.Family == "Segoe UI" && FontCatalog.Resolve("Segoe UI")?.Family == "Segoe UI"
            && FontCatalog.FaceDescriptionsRead == descriptions && !FontCatalog.FamilyListLoaded,
            "Windows default and generic system fonts resolve to Segoe UI without reading face descriptions");
        Check(FontCatalog.Resolve("viem-missing-startup-font") == null
            && FontCatalog.FaceDescriptionsRead == descriptions && !FontCatalog.FamilyListLoaded,
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
        window.Activate(); pane.FocusEditor(); await SelfTestRunner.Until(() => GetForegroundWindow() == window.Hwnd);
        byte[] sourceBeforeInspector = pane.Document.Source(pane.Document.State.document_revision);
        await InputRoutingTests.Key(VirtualKey.F8); await SelfTestRunner.Until(() => window.StyleInspector is { } opened && GetForegroundWindow() == WinRT.Interop.WindowNative.GetWindowHandle(opened));
        var styles = window.StyleInspector ?? throw new InvalidOperationException("F8 did not open the style inspector.");
        // These chrome/control checks inspect Base Paragraph explicitly. F8 now
        // correctly starts at the caret's style, covered by the following tests.
        styles.StylePicker.SelectedItem = styles.StylePicker.Items.OfType<StyleDefinition>().Single(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
        await InputRoutingTests.Settle();
        var hwnd = WinRT.Interop.WindowNative.GetWindowHandle(styles);
        Check(GetForegroundWindow() == hwnd, "native F8 opens the inspector and retains foreground focus");
        Check(styles.AppWindow.Presenter is OverlappedPresenter { IsAlwaysOnTop: false }, "style inspector is a normal window, not always on top");
        Check(styles.AppWindow.Presenter is OverlappedPresenter { IsResizable: false, IsMaximizable: false }, "style inspector has fixed dimensions and cannot be maximized");
        Check(styles.FontFamilyControl.ActualHeight is > 0 and <= 28 && window.Menu.ActualHeight <= 32,
            $"compact resources reach editor menus and inspector controls ({window.Menu.ActualHeight}, {styles.FontFamilyControl.ActualHeight})");
        Check(Children<TextBox>(styles.FontFamilyControl).Any(t => t.Text == styles.FontFamilyControl.Text && t.Text.Length > 0), "style font is visible in the native editable picker on first opening");
        var configuredStyles = styles.ThemeView.Styles();
        var baseParagraph = configuredStyles.Styles.Single(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
        string configuredFamily = configuredStyles.StringList(baseParagraph.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)).First();
        Check(styles.FontFamilyControl.Text == FontCatalog.DisplayFamily(configuredFamily), "the font picker displays the configured Base Paragraph family");
        bool sfProInstalled = FontCatalog.Families.Contains("SF Pro", StringComparer.OrdinalIgnoreCase);
        Check(sfProInstalled ? FontCatalog.Resolve("SF Pro")?.Family == "SF Pro" : FontCatalog.Resolve("SF Pro") == null && FontCatalog.Faces("SF Pro").Length == 0,
            "SF Pro resolves only when installed and never aliases Segoe UI");
        Check(((IEnumerable<object>)styles.FontFamilyControl.ItemsSource).OfType<string>().Contains("SF Pro", StringComparer.OrdinalIgnoreCase) == sfProInstalled,
            "the default Windows font picker offers SF Pro only when installed");
        Check(!Children<TextBlock>(styles.RootControl).Any(t => t.Text == "Properties" || t.Text.StartsWith("Changes apply live")), "style inspector omits redundant headings and guidance");
        Check(styles.RootControl.ActualHeight <= styles.RootControl.XamlRoot.Size.Height + 1 && styles.RootControl.XamlRoot.Size.Height - styles.RootControl.ActualHeight < 24,
            "style inspector initially fits its content without spare bottom space");
        var characterSize = styles.AppWindow.ClientSize;
        styles.ParagraphTab.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space); await InputRoutingTests.Settle();
        Check(styles.ParagraphPanel.Visibility == Visibility.Visible && styles.CharacterPanel.Visibility == Visibility.Collapsed, "native Paragraph tab displays paragraph controls");
        Check(styles.AppWindow.ClientSize == characterSize, "character and paragraph tabs occupy the same fixed window size");
        await LineSpacingChecks(pane, styles, sourceBeforeInspector);
        Check(!Children<Button>(styles.RootControl).Any(b => b.Content as string == "Restore Defaults" && b.Visibility == Visibility.Visible), "prose theme styles do not offer a separate reset");
        Check(!styles.VisitParent.IsEnabled && !styles.VisitNext.IsEnabled, "base paragraph has no relationship navigation targets");
        window.Activate(); pane.FocusEditor(); await SelfTestRunner.Until(() => GetForegroundWindow() == window.Hwnd);
        Check(GetForegroundWindow() == window.Hwnd, "editor can be raised above the modeless style inspector");
        var menu = window.Menu.Items.Single(m => m.Title == "Style");
        new MenuBarItemAutomationPeer(menu).Expand(); await InputRoutingTests.Settle();
        var item = menu.Items.OfType<MenuFlyoutItem>().Single(i => i.Text == "Edit Styles…");
        new MenuFlyoutItemAutomationPeer(item).Invoke(); await SelfTestRunner.Until(() => GetForegroundWindow() == hwnd);
        Check(window.StyleInspector == styles && GetForegroundWindow() == hwnd, "Edit Styles menu reuses and raises the inspector without refocusing the editor");
        styles.Close();
        Check(KeyPolicy.Route(VirtualKey.F8, false, true, false).Kind == VIEM_KEY_FUNCTION
            && KeyPolicy.Route(VirtualKey.F8, false, false, false, true).Kind == VIEM_KEY_FUNCTION, "modified and literal-next F8 remain core function keys");
    }

    internal static async Task DocumentSwitchingChecks(Preferences preferences)
    {
        using var markdown = new CoreDocument("# Title `code` tail\n\nBody"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, markdown);
        App.Instance.Windows.Add(window); window.Activate();
        var first = window.ActivePane!;
        var firstView = await first.Ready;
        firstView.Place(7, VIEM_BOUNDARY_AFFINITY_DOWNSTREAM, markdown.State.document_revision, false);
        window.ShowStyles();
        var inspector = window.StyleInspector!;
        StyleDefinition Selected() => (StyleDefinition)inspector.StylePicker.SelectedItem;
        static bool HasInputFocus(EditorPane pane) => ReferenceEquals(
            Microsoft.UI.Xaml.Input.FocusManager.GetFocusedElement(pane.XamlRoot),
            Children<TextBox>(pane).Single(control => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(control) == "Viem document input"));
        try
        {
            Check(Selected().Id == "Code", "document-switch inspector starts at the Markdown caret's character style");
            window.Activate();
            var text = new CoreDocument("Another document"u8.ToArray(), format: VIEM_FORMAT_PLAIN_TEXT);
            var second = window.SplitPane(first, text);
            await second.Ready; await InputRoutingTests.Settle();
            var expected = second.View!.CurrentStyleEditorKey(second.View.Styles());
            Check(window.StyleInspector == inspector && Selected().Key == expected
                && inspector.Title.Contains("Plain Text"), "switching panes immediately retargets the existing inspector's family and current style"
                + $" (sameInspector={window.StyleInspector == inspector}, selected={Selected().Key}, expected={expected}, title={inspector.Title},"
                + $" activePaneIsSecond={window.ActivePane == second}, activeWindow={window.IsWindowActive}, foregroundIsWindow={GetForegroundWindow() == window.Hwnd},"
                + $" followed={inspector.FollowingStateForTesting}, secondView={second.View.Id})");
            Check(HasInputFocus(second), "splitting into another document gives its input native focus");
            Check(GetForegroundWindow() == window.Hwnd, "retargeting the inspector leaves keyboard focus in the document window");
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().First(s => s.Id != Selected().Id);
            var explicitChoice = Selected().Key;
            window.FocusPane(second); await InputRoutingTests.Settle();
            Check(Selected().Key == explicitChoice, "refocusing the same document preserves an explicit inspector choice");
            window.FocusPane(first); await InputRoutingTests.Settle();
            Check(Selected().Id == "Code" && inspector.Title.Contains("Markdown"), "switching back restores the originating document's caret style");
            window.FocusPane(second);
            await window.ClosePane(second, force: true); await InputRoutingTests.Settle();
            Check(window.StyleInspector == inspector && Selected().Id == "Code",
                "closing the followed pane keeps the inspector open and follows the remaining document");
            foreach (string command in new[] { "split", "new" }) {
                first.View!.Ex(command); await window.PendingEffectsForTesting;
                var added = window.Panes.Single(pane => pane != first);
                await added.Ready; await InputRoutingTests.Settle();
                Check(window.ActivePane == added && HasInputFocus(added),
                    $":{command} selects the new pane and completes its native input focus");
                await window.ClosePane(added, force: true);
            }
            var otherWindow = new EditorWindow(preferences, new CoreDocument("A separate window"u8.ToArray(), format: VIEM_FORMAT_PLAIN_TEXT));
            App.Instance.Windows.Add(otherWindow); otherWindow.Activate();
            try
            {
                await otherWindow.ActivePane!.Ready; await SelfTestRunner.Until(() => GetForegroundWindow() == otherWindow.Hwnd);
                Check(otherWindow.StyleInspector == inspector && inspector.Title.Contains("Plain Text"),
                    "switching document windows retargets the single application-wide inspector");
                Check(GetForegroundWindow() == otherWindow.Hwnd, "window switching does not raise the inspector");
                otherWindow.ShowStyles();
                Check(otherWindow.StyleInspector == inspector && window.StyleInspector == inspector,
                    "F8 in another window reuses the same inspector");
            }
            finally { await otherWindow.RequestCloseForTesting(); }
            window.Activate(); await SelfTestRunner.Until(() => GetForegroundWindow() == window.Hwnd);
            Check(Selected().Id == "Code", "returning to the original window restores its current style");
            Check(markdown.Source(markdown.State.document_revision).AsSpan().SequenceEqual("# Title `code` tail\n\nBody"u8),
                "document switching and inspector following leave source bytes unchanged");
        }
        finally { inspector.Close(); await window.RequestCloseForTesting(); }
    }

    private static async Task LineSpacingChecks(EditorPane pane, StyleWindow styles, byte[] sourceBeforeInspector)
    {
        // Picker rows retain their identity while editing. Their immutable
        // records are labels; inspect current declarations through the core.
        StyleDefinition SelectedStyle() => styles.ThemeView.Styles().Styles.Single(style =>
            style.Key == ((StyleDefinition)styles.StylePicker.SelectedItem).Key);
        // The line-spacing interaction starts from a declared Normal value;
        // bundled themes are free to choose a different multiplier.
        styles.ThemeView.EditStyle((StyleDefinition)styles.StylePicker.SelectedItem, VIEM_STYLE_EDIT_SET_DECLARATION,
            VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING,
            CoreView.Enum(VIEM_STYLE_VALUE_LINE_SPACING, VIEM_STYLE_LINE_SPACING_NORMAL));
        styles.CommitThemeForTesting();
        await InputRoutingTests.Settle();
        var lineSpacing = Children<ComboBox>(styles.ParagraphPanel).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Line spacing");
        var lineSpacingAmount = Children<NumberBox>(styles.ParagraphPanel).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Line spacing amount");
        var lineSpacingUnits = Children<TextBlock>(styles.ParagraphPanel).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Line spacing units");
        var firstLineIndent = Children<NumberBox>(styles.ParagraphPanel).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "First line");
        Check(!Children<NumberBox>(styles.ParagraphPanel).Any(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c)
                is "Start indent" or "End indent" or "Space before" or "Space after"),
            "paragraph settings retain first-line indent while block geometry uses the Block tab");
        var paragraphStyle = SelectedStyle();
        styles.ThemeView.EditStyle(paragraphStyle, VIEM_STYLE_EDIT_SET_DECLARATION,
            VIEM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT, CoreView.Number(23));
        styles.ThemeView.EditStyle(paragraphStyle, VIEM_STYLE_EDIT_SET_DECLARATION,
            VIEM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT, CoreView.Number(17));
        styles.CommitThemeForTesting(); styles.RefreshForTesting();
        paragraphStyle = SelectedStyle();
        Check(paragraphStyle.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT).number == 23
            && paragraphStyle.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT).number == 17,
            $"configured side indents survive theme reload (leading: {paragraphStyle.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT).number}, "
                + $"trailing: {paragraphStyle.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT).number}, style: {paragraphStyle.Id})");
        firstLineIndent.Value = 9; await InputRoutingTests.Settle();
        paragraphStyle = SelectedStyle();
        Check(paragraphStyle.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT).number == 23
            && paragraphStyle.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT).number == 17
            && paragraphStyle.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT).number == 9,
            $"editing the retained first-line control preserves configured side indents (leading: {paragraphStyle.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT).number}, "
                + $"trailing: {paragraphStyle.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT).number}, first: {paragraphStyle.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT).number}, "
                + $"control: {firstLineIndent.Value}, enabled: {firstLineIndent.IsEnabled}, declared: {paragraphStyle.Declares(VIEM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT)}, style: {paragraphStyle.Id})");
        Check(sourceBeforeInspector.AsSpan().SequenceEqual(pane.Document.Source(pane.Document.State.document_revision)),
            "opening and displaying style controls does not rewrite document source");
        Check(lineSpacing.SelectedIndex == 0 && lineSpacingAmount.IsEnabled && Math.Abs(lineSpacingAmount.Minimum - .1) < .0001
            && Math.Abs(lineSpacingAmount.Value - 1) < .0001
            && lineSpacingAmount.Text == "1" && Math.Abs(lineSpacingAmount.SmallChange - .1) < .0001 && lineSpacingUnits.Text == "\u00D7",
            "Normal line spacing is an enabled compact 1 multiplier with tenth steps");
        lineSpacingAmount.Value += lineSpacingAmount.SmallChange; await InputRoutingTests.Settle();
        Check(lineSpacing.SelectedIndex == 1 && Math.Abs(lineSpacingAmount.Value - 1.1) < .0001,
            "stepping Normal away from 1 changes it to a Multiple value by 0.1");
        lineSpacingAmount.Value = .7999999523162842; await InputRoutingTests.Settle();
        Check(lineSpacing.SelectedIndex == 1 && Math.Abs(lineSpacingAmount.Value - .8) < .0001
            && lineSpacingAmount.Text.Length <= 3 && lineSpacingAmount.Text.StartsWith("0") && lineSpacingAmount.Text.EndsWith("8"),
            "Multiple line spacing rounds float noise to one compact decimal place");
        lineSpacingAmount.Value = 1; await InputRoutingTests.Settle();
        Check(lineSpacing.SelectedIndex == 0 && lineSpacingAmount.IsEnabled && lineSpacingAmount.Text == "1",
            "a Multiple value of exactly 1 changes back to Normal without disabling its amount");
        lineSpacing.SelectedIndex = 1; await InputRoutingTests.Settle();
        Check(lineSpacing.SelectedIndex == 1 && Math.Abs(lineSpacingAmount.Value - 1.1) < .0001,
            "choosing Multiple from Normal starts at 1.1");
        lineSpacingAmount.Value -= lineSpacingAmount.SmallChange; await InputRoutingTests.Settle();
        Check(lineSpacing.SelectedIndex == 0 && Math.Abs(lineSpacingAmount.Value - 1) < .0001,
            "stepping a Multiple value down to 1 changes it to Normal");
        var nearNormal = CoreView.Enum(VIEM_STYLE_VALUE_LINE_SPACING, VIEM_STYLE_LINE_SPACING_MULTIPLIER);
        nearNormal.number = .99999994f;
        var revisionBeforeNearNormal = pane.Document.State.document_revision;
        styles.ThemeView.EditStyle((StyleDefinition)styles.StylePicker.SelectedItem, VIEM_STYLE_EDIT_SET_DECLARATION,
            VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING, nearNormal);
        styles.CommitThemeForTesting();
        await InputRoutingTests.Settle();
        Check(pane.Document.State.document_revision == revisionBeforeNearNormal && lineSpacing.SelectedIndex == 0
            && Math.Abs(lineSpacingAmount.Value - 1) < .0001 && lineSpacingAmount.Text == "1",
            "an existing multiplier that rounds to 1 displays as Normal without a presentation-time rewrite");
        lineSpacing.SelectedIndex = 1; await InputRoutingTests.Settle();
        Check(lineSpacing.SelectedIndex == 1 && Math.Abs(lineSpacingAmount.Value - 1.1) < .0001,
            "choosing Multiple from a normalized legacy 1 multiplier starts at 1.1");
        lineSpacing.SelectedIndex = 3; await InputRoutingTests.Settle();
        Check(lineSpacingAmount.Minimum == 0 && Math.Abs(lineSpacingAmount.SmallChange - 1) < .0001 && lineSpacingUnits.Text == "pt",
            "point-based line spacing permits zero, uses point units, and keeps whole-point steps");
        lineSpacingAmount.Value = 0; await InputRoutingTests.Settle();
        Check(lineSpacing.SelectedIndex == 3 && lineSpacingAmount.Value == 0 && lineSpacingAmount.Text == "0",
            "Exact line spacing accepts and compactly displays zero points");
        lineSpacingAmount.Value = 19.299999237060547; await InputRoutingTests.Settle();
        Check(Math.Abs(lineSpacingAmount.Value - 19.3) < .0001 && lineSpacingAmount.Text.Length <= 4
            && lineSpacingAmount.Text.StartsWith("19") && lineSpacingAmount.Text.EndsWith("3"),
            "point-based line spacing also rounds float noise to one compact decimal place");
        lineSpacingAmount.Value = 19; await InputRoutingTests.Settle();
        Check(lineSpacingAmount.Text == "19", "whole point line spacing omits an unnecessary fractional part");
        lineSpacing.SelectedIndex = 2; await InputRoutingTests.Settle();
        lineSpacingAmount.Value = 0; await InputRoutingTests.Settle();
        Check(lineSpacing.SelectedIndex == 2 && lineSpacingAmount.Minimum == 0 && lineSpacingAmount.Value == 0
            && lineSpacingAmount.Text == "0" && lineSpacingUnits.Text == "pt",
            "At least line spacing also accepts and compactly displays zero points");
        lineSpacing.SelectedIndex = 0; await InputRoutingTests.Settle();
    }

    internal static async Task RunSettings(EditorPane pane, EditorWindow window, Preferences preferences)
    {
        await StyleDefaultsLoadingTests.Run(preferences);
        window.Activate(); await window.ShowSettings(); await InputRoutingTests.Settle();
        var settings = window.SettingsInspector!;
        string? priorTheme = preferences.SelectedTheme, priorPath = preferences.SelectedThemePath;
        string paperPath = preferences.ThemeFiles.Single(file => file.Name == "Paper").Path; byte[] priorPaper = File.ReadAllBytes(paperPath);
        string discoveredThemePath = Path.Combine(preferences.ThemesDirectory, "Discovered-" + Guid.NewGuid().ToString("N")[..8] + ".json");
        var theme = preferences.Theme;
        byte[] source = pane.Document.Source(pane.Document.State.document_revision);
        bool priorMarkdownAutodetect = preferences.MarkdownAutodetect;
        try {
            Check(settings.Categories.Items.Count == 3 && settings.Categories.SelectedIndex == 1 && settings.CurrentPage.Visibility == Visibility.Visible, "settings opens Theme beside a three-category sidebar without Code");
            Check(settings.NewTheme.ActualHeight is > 0 and <= 32, "separate settings window inherits compact control density");
            Check(!Children<ComboBox>(settings.RootControl).Any(control => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(control) == "Status font"),
                "Settings retires the status font picker in favor of built-in styles");
            File.WriteAllBytes(discoveredThemePath, priorPaper);
            byte[] beforeDiscovery = File.ReadAllBytes(Path.Combine(preferences.DirectoryPath, "config.json"));
            settings.ThemePicker.IsDropDownOpen = true; await InputRoutingTests.Settle();
            Check(settings.ThemePicker.Items.OfType<Preferences.ThemeFile>().Any(file => file.Path == discoveredThemePath)
                && preferences.SelectedTheme == priorTheme && preferences.SelectedThemePath == priorPath
                && File.ReadAllBytes(Path.Combine(preferences.DirectoryPath, "config.json")).SequenceEqual(beforeDiscovery),
                "opening the theme dropdown discovers external files without changing selection or saving settings");
            settings.ThemePicker.IsDropDownOpen = false;
            settings.ThemePicker.SelectedItem = settings.ThemePicker.Items.OfType<Preferences.ThemeFile>().Single(file => file.Name == "Paper"); await InputRoutingTests.Settle();
            Check(settings.Error.Length == 0 && preferences.SelectedTheme == "Paper" && preferences.SelectedThemePath == paperPath
                && preferences.Theme.Background == Theme.Paper.Background, "Paper selection commits the named aggregate theme");
            Check(settings.DeleteTheme.Content as string == "Revert", "bundled themes offer Revert in Settings");
            Check(settings.ImportTheme.Content as string == "Import…" && settings.DeleteTheme.Parent is Panel themeRow
                && themeRow.Children.IndexOf(settings.ImportTheme) == themeRow.Children.IndexOf(settings.DeleteTheme) + 1,
                "Settings offers Import… to the right of Revert or Delete");
            settings.ThemePicker.SelectedItem = settings.ThemePicker.Items.OfType<Preferences.ThemeFile>().Single(file => file.Path == discoveredThemePath);
            Check(settings.DeleteTheme.Content as string == "Delete", "user themes offer Delete in Settings");
            settings.ThemePicker.SelectedItem = settings.ThemePicker.Items.OfType<Preferences.ThemeFile>().Single(file => file.Name == "Paper");
            var statusStyles = System.Text.Json.Nodes.JsonNode.Parse(preferences.ThemeStyleDefaults(VIEM_FORMAT_PLAIN_TEXT))!;
            var status = statusStyles["block_styles"]!.AsArray().Single(entry => entry!["id"]!.GetValue<string>() == "Status bar")!;
            status["character"]!["font_families"] = new System.Text.Json.Nodes.JsonArray("Consolas");
            status["character"]!["size"] = 14;
            preferences.SaveThemeStyles(VIEM_FORMAT_PLAIN_TEXT, System.Text.Encoding.UTF8.GetBytes(statusStyles.ToJsonString())); await InputRoutingTests.Settle();
            Check(preferences.StatusStyles.Status.Family == "Consolas" && preferences.StatusStyles.Status.Size == 14
                && settings.PreviewStatus.FontFamily.Source == "Consolas" && settings.PreviewStatus.FontSize == 14, "status font changes update preferences and the theme preview");
            settings.ThemePicker.SelectedItem = "Default"; await InputRoutingTests.Settle();
            Check(!settings.DeleteTheme.IsEnabled, "Default has no Revert or Delete action");
            Check(StyleDefaultsLoadingTests.MatchesBuiltInTheme(preferences)
                && settings.PreviewStatus.FontSize == preferences.StatusStyles.Status.Size, "Default selection restores built-in colors and status typography");
            for (int index = 0; index < 3; index++) {
                settings.Categories.SelectedIndex = index; await InputRoutingTests.Settle();
                Check(settings.CurrentPage.Visibility == Visibility.Visible && settings.CurrentPage.ActualHeight > 0, $"settings sidebar displays category {index + 1}");
            }
            var markdownTyping = Children<CheckBox>(settings.CurrentPage).Single(c => c.Content as string == "Automatically format typed Markdown");
            Check(markdownTyping.IsChecked == preferences.MarkdownAutodetect, "Editing reflects the Markdown typing preference");
            var indentation = preferences.Editing["indentation"] as System.Text.Json.Nodes.JsonObject;
            foreach (var (key, label, fallback) in new[] {
                ("autoindent", "Auto indent", true), ("expandtab", "Insert spaces for tabs", true), ("smarttab", "Smart tabs", true),
                ("continueCommentsOnEnter", "Continue comments with Enter", false), ("continueCommentsOnOpenLine", "Continue comments with o / O", false) }) {
                var checkbox = Children<CheckBox>(settings.CurrentPage).Single(c => c.Content as string == label);
                Check(checkbox.IsChecked == (indentation?[key]?.GetValue<bool>() ?? fallback),
                    $"Editing reflects the portable default or explicit setting for {key}");
            }
            markdownTyping.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space); await InputRoutingTests.Settle();
            Check(preferences.MarkdownAutodetect != priorMarkdownAutodetect
                && new Preferences(preferences.DirectoryPath).MarkdownAutodetect == preferences.MarkdownAutodetect,
                "the native Markdown typing checkbox persists its new value");
            Check(!Children<TextBlock>(settings.RootControl).Any(t => t.Text == "Code")
                && !Children<Button>(settings.RootControl).Any(b => b.Content as string == "Edit Code Styles…"),
                "settings has no Code section or Code Styles entrypoint");
            Check(source.AsSpan().SequenceEqual(pane.Document.Source(pane.Document.State.document_revision)), "settings edits do not modify document source");
        }
        finally {
            settings.Close(); File.Delete(discoveredThemePath); Preferences.AtomicWrite(paperPath, priorPaper);
            preferences.Set("editing", "markdownAutodetect", priorMarkdownAutodetect);
            var json = Preferences.ThemeJson(theme);
            preferences.SelectTheme(priorTheme, priorPath);
            preferences.SetSections(new() { ["theme"] = json }); window.Activate(); pane.FocusEditor();
        }
    }
    internal static async Task FontChecks(EditorPane pane, Preferences preferences)
    {
        Check(FontCatalog.Families.SequenceEqual(FontCatalog.Families.OrderBy(f => f, StringComparer.CurrentCultureIgnoreCase)), "font family menus use culture-aware alphabetical order");
        var faces = FontCatalog.Faces("Segoe UI");
        var face = faces.First(f => f.Weight == 700 && f.Slant == FontStyle.Italic);
        int descriptions = FontCatalog.FaceDescriptionsRead;
        Check(FontCatalog.Faces("segoe ui").SequenceEqual(faces) && FontCatalog.FaceDescriptionsRead == descriptions,
            "repeated family variant requests reuse cached discovery");
        Check(FontCatalog.Match(face.PortableFamily, face.PortableStyle) == face, "portable family and subfamily lookup retains exact face metadata");
        descriptions = FontCatalog.FaceDescriptionsRead;
        Check(FontCatalog.Match(face.PortableFamily.ToLowerInvariant(), face.PortableStyle.ToLowerInvariant()) == face && FontCatalog.FaceDescriptionsRead == descriptions,
            "repeated family and subfamily requests reuse cached discovery case-insensitively");
        Check(FontCatalog.Current(face.PortableFamily, face.Weight, 1) == face && FontCatalog.Current(face.PortableFamily, 617, 1) == null, "font variants match effective traits and leave unknown combinations unresolved");
        Check(FontCatalog.Resolve(face.Name) == null, "a PostScript face name is not interpreted as a saved family");
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
        view.EditStyleFont(style, [face.PortableFamily, "serif"], face);
        var sheet = view.Styles(); style = sheet.Styles.Single(s => s.Id == id);
        Check(sheet.StringList(style.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)).SequenceEqual(new[] { face.PortableFamily, "serif" })
            && sheet.String(style.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FACE)) == face.PortableStyle
            && style.Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value == 700 && !style.Declares(VIEM_STYLE_PROPERTY_CHARACTER_SLANT), "font variant persists exact face and base weight without declaring semantic Italic");
        var changed = view.Layout();
        Check(!CoreView.SameLayout(prior.Info.identity, changed.Info.identity) && view.Provider.ShapedCharacters > shaped
            && changed.Clusters.SelectMany(c => view.Provider.RenderedFontNames(c.render_run.identifier)).Contains(face.Name), "font change invalidates layout and DirectWrite renders the selected face");
        Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(before), "theme font variants leave Markdown source unchanged");
        byte[] originalThemeStyles = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
        preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, view.ExportStyleDefaults());
        var inspector = new StyleWindow(view, preferences); inspector.Activate(); await InputRoutingTests.Settle();
        try {
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == id);
            Check(!inspector.ParagraphTab.IsEnabled, "character styles disable the paragraph tab");
            Check(inspector.FontVariantControl.SelectedItem as FontFace == face, "style inspector displays the stored font variant");
            var regular = faces.First(f => f.Weight == 400 && f.Slant == FontStyle.Normal);
            inspector.FontVariantControl.SelectedItem = regular; await InputRoutingTests.Settle();
            sheet = inspector.ThemeView.Styles(); style = sheet.Styles.Single(s => s.Id == id);
            Check(inspector.Error.Length == 0 && sheet.StringList(style.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)).SequenceEqual(new[] { regular.PortableFamily, "serif" })
                && style.Value(VIEM_STYLE_PROPERTY_CHARACTER_SLANT).enum_value == 0, "choosing a variant in the native picker applies its traits and preserves fallbacks");
            inspector.ThemeView.EditStyle(style, VIEM_STYLE_EDIT_CLEAR_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES, default); inspector.RefreshForTesting();
            Check(inspector.FontFamilyControl.Text.Length == 0 && !inspector.FontFamilyControl.IsEnabled, "inherited font fields are empty until overridden");
            var declare = Children<CheckBox>(inspector.RootControl).Single(c => Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(c) == "Declare Font family");
            declare.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            Check(inspector.Error.Length == 0 && inspector.FontFamilyControl.IsEnabled
                && inspector.ThemeView.Styles().Styles.Single(s => s.Id == id).Declares(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES), "an inherited font family can be enabled with the native declaration checkbox");
            inspector.ThemeView.EditStyleFont(style, ["viem-missing-font", "serif"], null); inspector.RefreshForTesting(); await InputRoutingTests.Settle();
            Check(inspector.FontFamilyControl.Text == "viem-missing-font" && inspector.FontVariantControl.SelectedItem == null,
                "unavailable document fonts remain visible without selecting a substitute variant");
            var familyItemsSource = ((IEnumerable<object>)inspector.FontFamilyControl.ItemsSource).ToArray();
            Check(familyItemsSource[2] is ComboBoxItem { IsEnabled: false, IsHitTestVisible: false },
                "the font family picker's third row is a real disabled, unselectable separator, not text");
            var familyItems = familyItemsSource.OfType<string>().ToArray();
            Check(familyItems.Take(2).SequenceEqual(new[] { "System Default", "System Monospace" })
                && familyItems.Skip(2).SequenceEqual(familyItems.Skip(2).OrderBy(f => f, StringComparer.CurrentCultureIgnoreCase)),
                "the font family picker lists the two system entries first, then a native separator, then installed fonts in order");
            inspector.FontFamilyControl.SelectedItem = "System Default"; await InputRoutingTests.Settle();
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
            new ButtonAutomationPeer(inspector.VisitParent).Invoke(); await InputRoutingTests.Settle();
            Check(((StyleDefinition)inspector.StylePicker.SelectedItem).Id == parentId
                && inspector.ParentPicker.Items.OfType<StyleDefinition>().Any(s => s.Id == childId),
                "parent navigation selects the referenced style while inheritance choices retain descendants for apply-time cycle validation");
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == childId);
            new ButtonAutomationPeer(inspector.VisitNext).Invoke(); await InputRoutingTests.Settle();
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
        byte[] sourceAfterCodeAssignment = doc.Source(doc.State.document_revision);
        int count = 0;
        foreach (string family in families)
        {
            Check(FontCatalog.Families.Contains(family), $"{family} appears in the font picker");
            var faces = FontCatalog.Faces(family);
            Check(faces.Length == 2 && faces.Any(f => f.Slant == FontStyle.Normal) && faces.Any(f => f.Slant == FontStyle.Italic), $"{family} exposes exactly its two variable designs");
            foreach (var face in faces)
            {
                VariableFontTests.SourceChecks(face, "flightline/FlightlineCode-" + (face.Slant == FontStyle.Normal ? "Regular" : "Italic") + "-VF.ttf");
                view.EditStyleFont(style, [face.PortableFamily, "serif"], face);
                style = view.Styles().Styles.Single(s => s.Id == "Code");
                var layout = view.Layout();
                Check(layout.Clusters.SelectMany(c => view.Provider.RenderedFontNames(c.render_run.identifier)).Contains(face.Name),
                    $"DirectWrite shapes the original {face.Name} design");
                count++;
            }
        }
        Check(count == 2, "both native Flightline variable designs render through the provider");
        Check(doc.Source(doc.State.document_revision).AsSpan().SequenceEqual(sourceAfterCodeAssignment), "font selection preserves source after the explicit Code-style assignment");
        Check(!System.Text.Encoding.UTF8.GetString(view.ExportStyleDefaults()).Contains("file:", StringComparison.OrdinalIgnoreCase),
            "saved font styles contain portable names rather than resource URIs");
    }
    internal static async Task CodeStyleChecks(EditorPane pane, Preferences preferences)
    {
        using var doc = new CoreDocument("A Code style sample."u8.ToArray(), format: VIEM_FORMAT_CODE);
        using var view = new CoreView(doc, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        byte[] before = view.ExportStyleDefaults(), source = doc.Source(doc.State.document_revision);
        var inspector = new StyleWindow(view, preferences); inspector.Activate(); await InputRoutingTests.Settle();
        try {
            var restore = Children<Button>(inspector.RootControl).Single(b => b.Content as string == "Restore Defaults");
            Check(restore.Visibility == Visibility.Visible, "Code theme styles expose Restore Defaults");
            Check(Children<CheckBox>(inspector.CharacterPanel).All(c => c.IsChecked == true && !c.IsEnabled), "base paragraph overrides are checked and fixed while values remain editable");
            string CreateStyle(string name)
            {
                var sheet = view.Styles();
                try { return view.CreateCodeStyle(name); }
                catch (Exception error)
                {
                    string snapshot = FrontendSmokeTests.ReportPath + ".code-style-before.json";
                    File.WriteAllBytes(snapshot, view.ExportStyleDefaults());
                    var matches = sheet.Styles.Where(s => s.Name == name || s.Id == name).Select(s => $"{s.Namespace}:{s.Id}:{s.Name}");
                    throw new InvalidOperationException($"Create Code style '{name}', sheet revision {sheet.Identity.style_sheet_revision}, existing matches [{string.Join(", ", matches)}], snapshot {snapshot}", error);
                }
            }
            string parentId = CreateStyle("Parent"), childId = CreateStyle("Child");
            var child = view.Styles().Styles.Single(s => s.Id == childId);
            view.EditStyleString(child, VIEM_STYLE_EDIT_SET_PARENT, 0, parentId); inspector.RefreshForTesting();
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == childId);
            byte[] related = view.ExportStyleDefaults();
            new ButtonAutomationPeer(inspector.VisitParent).Invoke(); await InputRoutingTests.Settle();
            Check(((StyleDefinition)inspector.StylePicker.SelectedItem).Id == parentId
                && inspector.ParentPicker.Items.OfType<StyleDefinition>().Any(s => s.Id == childId),
                "Code parent navigation selects the referenced style while inheritance choices retain descendants for apply-time cycle validation");
            Check(related.AsSpan().SequenceEqual(view.ExportStyleDefaults()), "Code parent navigation does not edit shared styles");
            new ButtonAutomationPeer(restore).Invoke(); await InputRoutingTests.Settle();
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
            inspector.CharacterTab.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space); await InputRoutingTests.Settle();
            inspector.ParagraphTab.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space); await InputRoutingTests.Settle();
            Check(inspector.ParagraphPanel.Visibility == Visibility.Visible, "Code Styles also exposes base paragraph properties");
        }
        finally {
            inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_CODE, before);
        }
    }
}
#endif
