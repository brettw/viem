#if DEBUG
using System.Text;
using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class VisibleWhitespaceStyleTests
{
    private static void Check(bool condition, string name)
    { if (!condition) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
    private static IEnumerable<T> Children<T>(DependencyObject root) where T : DependencyObject
    {
        if (root is T match) yield return match;
        for (int index = 0; index < VisualTreeHelper.GetChildrenCount(root); index++)
            foreach (var child in Children<T>(VisualTreeHelper.GetChild(root, index))) yield return child;
    }

    internal static async Task Run(Preferences preferences)
    {
        var original = preferences.VisibleWhitespaceStyle;
        var initial = (JsonObject)original.DeepClone(); initial["size"] = 17; initial["futureMarkerField"] = "retain";
        preferences.SaveVisibleWhitespaceStyle(original, initial);
        const string source = "# Heading\n\nText with trailing spaces  ";
        var document = new CoreDocument(Encoding.UTF8.GetBytes(source), format: VIEM_FORMAT_MARKDOWN);
        var window = new EditorWindow(preferences, document);
        App.Instance.Windows.Add(window); window.Activate();
        var pane = window.ActivePane!; var view = await pane.Ready;
        StyleWindow? inspector = null;
        try
        {
            byte[] themeBefore = preferences.ThemeStyleDefaults(VIEM_FORMAT_CODE);
            ulong codeRevision = CoreThemes.CodeStyleRevision;
            window.ShowStyles(); inspector = window.StyleInspector!;
            Check(inspector.DocumentPicker.SelectedIndex == 1, "whitespace routing starts with the ordinary Markdown inspector");
            await window.ShowSettings();
            var settings = window.SettingsInspector!; settings.Categories.SelectedIndex = 2;
            await InputRoutingTests.Settle();
            var button = Children<Button>(settings.RootControl).Single(button => AutomationProperties.GetName(button) == "Edit visible whitespace style");
            new ButtonAutomationPeer(button).Invoke(); await InputRoutingTests.Settle();
            Check(ReferenceEquals(window.StyleInspector, inspector) && inspector.DocumentPicker.SelectedIndex == 2
                && inspector.StylePicker.SelectedItem is StyleDefinition { Namespace: 3, Id: "Visible whitespace" },
                "Settings Edit Style reuses the ordinary inspector with Code and Visible whitespace selected");
            Check(!inspector.ParentPicker.IsEnabled && !inspector.NextPicker.IsEnabled
                && !inspector.ParagraphTab.IsEnabled && !inspector.FontSizeUnitControl.IsEnabled
                && inspector.FontSizeUnitControl.Items.Cast<string>().SequenceEqual(new[] { "pt" }),
                "visible whitespace uses character controls without parent assignment, paragraph settings or percentage sizes");
            Check(!inspector.ThemeView.Styles().Styles.Any(style => style.Namespace == 3),
                "the settings-backed whitespace style is absent from the real global Code stylesheet");
            var exported = inspector.ThemeView.Styles();
            var defaultMarker = VisibleWhitespaceStyle.Append(exported, Preferences.DefaultVisibleWhitespaceStyle()).Styles.Last();
            var defaultColor = defaultMarker.Value(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND).color;
            Check(defaultColor.red == 0 && defaultColor.green == 0 && defaultColor.blue == (float)(139d / 255) && defaultColor.alpha == 1,
                "a fresh profile's unparsed default JSON supplies the visible whitespace foreground");
            var basis = exported.Styles.First(style => (style.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
            var properties = new Dictionary<uint, Viem.Windows.Interop.ViemStylePropertyV1>(basis.Properties);
            var foreground = properties[VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND];
            foreground.flags = VIEM_STYLE_PROPERTY_DECLARED | VIEM_STYLE_PROPERTY_EFFECTIVE_PRESENT;
            foreground.effective = new() { kind = VIEM_STYLE_VALUE_COLOR, color = new() { red = 1, green = 0, blue = 0, alpha = 1 } };
            foreground.declared = foreground.effective; foreground.contributor_kind = VIEM_STYLE_CONTRIBUTOR_BLOCK_STYLE;
            foreground.contributor_namespace = 1; foreground.contributor_style_id = basis.Native.stable_id;
            properties[VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND] = foreground;
            var explicitRed = exported with { Styles = exported.Styles.Select(style => style.Key == basis.Key ? style with { Properties = properties } : style).ToArray() };
            var inheritedMarker = VisibleWhitespaceStyle.Append(explicitRed, new JsonObject()).Styles.Last();
            Check(!inheritedMarker.UsesThemeForeground && inheritedMarker.Value(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND).color.red == 1
                && inheritedMarker.Properties[VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND].contributor_style_id.Equals(basis.Native.stable_id),
                "an undeclared marker foreground retains its Code base color and contributor instead of substituting the theme palette");
            window.ShowVisibleWhitespaceStyle();
            Check(ReferenceEquals(window.StyleInspector, inspector), "repeated whitespace settings actions reuse the same style window");
            inspector.FontSizeControl.Value = 23;
            Check(preferences.VisibleWhitespaceStyle["size"]!.GetValue<float>() == 23
                && preferences.VisibleWhitespaceStyle["futureMarkerField"]!.GetValue<string>() == "retain",
                "ordinary size controls persist whitespace settings while retaining unknown style fields");
            using (var disk = System.Text.Json.JsonDocument.Parse(File.ReadAllBytes(Path.Combine(preferences.DirectoryPath, "config.json"))))
                Check(disk.RootElement.GetProperty("editing").GetProperty("whitespacePresentation").GetProperty("visibleWhitespace")
                    .GetProperty("style").GetProperty("size").GetSingle() == 23,
                    "visible whitespace changes retain their config.json location");
            Check(view.Whitespace(view.Layout().Info).Style.GetProperty("size").GetSingle() == 23,
                "settings-backed style changes update an open document's whitespace presentation live");
            inspector.UndoThemeForTesting();
            Check(preferences.VisibleWhitespaceStyle["size"]!.GetValue<float>() == 17 && inspector.FontSizeControl.Value == 17,
                "the ordinary inspector Undo button restores the previous whitespace setting");
            inspector.RedoThemeForTesting();
            Check(preferences.VisibleWhitespaceStyle["size"]!.GetValue<float>() == 23 && inspector.FontSizeControl.Value == 23,
                "the ordinary inspector Redo button reapplies the whitespace setting");
            var marker = (StyleDefinition)inspector.StylePicker.SelectedItem;
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().First(style => style.Namespace == 2);
            Check(!inspector.ParentPicker.Items.OfType<StyleDefinition>().Any(style => style.Key == marker.Key),
                "normal character styles cannot inherit from the settings-backed marker definition");
            inspector.SelectVisibleWhitespace();
            ulong markerRevision = inspector.WhitespaceSettingsRevision;
            inspector.DocumentPicker.SelectedIndex = 1;
            var external = preferences.VisibleWhitespaceStyle; external["size"] = 29;
            preferences.SaveVisibleWhitespaceStyle(preferences.VisibleWhitespaceStyle, external);
            external["size"] = 23; preferences.SaveVisibleWhitespaceStyle(preferences.VisibleWhitespaceStyle, external);
            inspector.SelectVisibleWhitespace();
            Check(inspector.WhitespaceSettingsRevision > markerRevision && inspector.FontSizeControl.Value == 23,
                "external settings changes and ABA restoration advance marker identity and refresh the inspector");
            inspector.UndoThemeForTesting();
            Check(preferences.VisibleWhitespaceStyle["size"]!.GetValue<float>() == 23,
                "external marker changes retire cached Code-family marker undo instead of restoring it after a stylesheet switch");

            // Change the file through a distinct settings owner: the next
            // action must refresh and reject, not overwrite that newer style.
            var another = new Preferences(preferences.DirectoryPath);
            var otherStyle = another.VisibleWhitespaceStyle; otherStyle["size"] = 31;
            another.SaveVisibleWhitespaceStyle(another.VisibleWhitespaceStyle, otherStyle);
            inspector.FontSizeControl.Value = 25;
            Check(preferences.VisibleWhitespaceStyle["size"]!.GetValue<float>() == 31 && inspector.FontSizeControl.Value == 31
                && inspector.Error.Contains("changed", StringComparison.OrdinalIgnoreCase),
                "a stale whitespace edit refreshes the controls and leaves another owner's stored setting intact");
            var reset = Children<Button>(inspector.RootControl).Single(button => button.Content as string == "Restore Defaults");
            new ButtonAutomationPeer(reset).Invoke();
            Check(JsonNode.DeepEquals(preferences.VisibleWhitespaceStyle, VisibleWhitespaceStyle.RestoringDefaults(initial)),
                "Restore Defaults on Visible whitespace resets known marker properties while preserving unknown fields");
            inspector.UndoThemeForTesting();
            Check(preferences.VisibleWhitespaceStyle["size"]!.GetValue<float>() == 31,
                "restoring marker defaults participates in ordinary inspector undo");
            Check(preferences.ThemeStyleDefaults(VIEM_FORMAT_CODE).AsSpan().SequenceEqual(themeBefore)
                && CoreThemes.CodeStyleRevision == codeRevision && !document.IsDirty
                && Encoding.UTF8.GetString(document.Source(document.State.document_revision)) == source,
                "whitespace style editing, routing, defaults and undo change neither Code theme styles nor document source");
        }
        finally
        {
            inspector?.Close(); window.SettingsInspector?.Close();
            await window.ClosePane(pane, force: true); App.Instance.Windows.Remove(window);
            preferences.SaveVisibleWhitespaceStyle(preferences.VisibleWhitespaceStyle, original);
        }
    }
}
#endif
