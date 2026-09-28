#if DEBUG
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Interop;
using Viem.Windows.Shell;
using Windows.System;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Diagnostics;

internal static class StyleFontSizeTests
{
    private static void Check(bool condition, string message)
    {
        if (!condition) throw new InvalidOperationException(message);
        FrontendSmokeTests.UiChecks.Add(message);
    }
    private static IEnumerable<T> Children<T>(DependencyObject root) where T : DependencyObject
    {
        if (root is T match) yield return match;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
            foreach (var child in Children<T>(VisualTreeHelper.GetChild(root, i))) yield return child;
    }
    private static StyleDefinition Style(CoreView view, uint space, string id)
        => view.Styles().Styles.Single(style => style.Namespace == space && style.Id == id);
    private static void Select(StyleWindow inspector, uint space, string id)
        => inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>()
            .Single(style => style.Namespace == space && style.Id == id);
    private static void SetPoints(CoreView view, uint space, string id, float size)
        => view.EditStyle(Style(view, space, id), VIEM_STYLE_EDIT_SET_DECLARATION,
            VIEM_STYLE_PROPERTY_CHARACTER_SIZE, CoreView.Number(size));
    private static void AssertSize(CoreView view, uint space, string id, uint percent, float points)
    {
        var property = Style(view, space, id).Properties[VIEM_STYLE_PROPERTY_CHARACTER_SIZE];
        Check(property.declared.kind == VIEM_STYLE_VALUE_PERCENTAGE && property.declared.enum_value == percent
            && property.effective.kind == VIEM_STYLE_VALUE_FLOAT && Math.Abs(property.effective.number - points) < .001,
            $"{id} retains {percent}% while resolving to {points} points");
    }

    internal static async Task Run(EditorPane pane, Preferences preferences)
    {
        using var document = new CoreDocument("# Title\n\nBody"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        SetPoints(view, 1, "Paragraph", 20);
        SetPoints(view, 1, "Heading1", 24);
        byte[] originalTheme = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
        preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, view.ExportStyleDefaults());
        byte[] originalSource = document.Source(document.State.document_revision);
        var inspector = new StyleWindow(view, preferences, followCaret: false);
        inspector.Activate(); await Task.Delay(150);
        try
        {
            Check(((string[])inspector.FontSizeUnitControl.ItemsSource).SequenceEqual(new[] { "pt" })
                && !inspector.FontSizeUnitControl.IsEnabled,
                "Base Paragraph font size only offers points");
            Select(inspector, 1, "Heading1");
            Check(inspector.FontSizeControl.Value == 24 && inspector.FontSizeUnitControl.SelectedIndex == 0,
                "an absolute heading size initially shows points");
            inspector.FontSizeUnitControl.SelectedIndex = 1;
            AssertSize(inspector.ThemeView, 1, "Heading1", 120, 24);
            Check(document.Source(document.State.document_revision).SequenceEqual(originalSource),
                "changing theme font-size units preserves source and its undo history");
            double priorStatusSize = preferences.StatusFontSize;
            preferences.Set("theme", "statusFontSize", System.Text.Json.Nodes.JsonValue.Create(priorStatusSize == 11 ? 12 : 11));
            inspector.UndoThemeForTesting();
            Check(inspector.FontSizeUnitControl.SelectedIndex == 0 && inspector.FontSizeControl.Value == 24,
                "unrelated appearance changes preserve theme font-size Undo without touching source history");
            inspector.RedoThemeForTesting();
            preferences.Set("theme", "statusFontSize", System.Text.Json.Nodes.JsonValue.Create(priorStatusSize));
            AssertSize(inspector.ThemeView, 1, "Heading1", 120, 24);
            Check(inspector.FontSizeControl.Value == 120 && inspector.FontSizeControl.Minimum == 10
                && inspector.FontSizeControl.Maximum == 1000 && inspector.FontSizeControl.SmallChange == 1,
                "percentage font sizes display their declaration with integer stepper bounds 10–1000");
            inspector.FontSizeControl.Value = 150;
            AssertSize(inspector.ThemeView, 1, "Heading1", 150, 30);
            inspector.FontSizeControl.Value = 150.5;
            Check(inspector.Error.Length > 0 && inspector.FontSizeControl.Value == 150,
                "fractional percentage input is rejected and the last committed size remains visible");
            AssertSize(inspector.ThemeView, 1, "Heading1", 150, 30);
            inspector.FontSizeControl.Value = 10;
            AssertSize(inspector.ThemeView, 1, "Heading1", 10, 2);
            inspector.FontSizeControl.Value = 1000;
            AssertSize(inspector.ThemeView, 1, "Heading1", 1000, 200);
            inspector.FontSizeControl.Value = 150;
            SetPoints(inspector.ThemeView, 1, "Paragraph", 40); inspector.RefreshForTesting();
            AssertSize(inspector.ThemeView, 1, "Heading1", 150, 60);
            Select(inspector, 1, "Heading1");
            Check(inspector.FontSizeControl.Value == 150, "changing the parent does not replace the percentage declaration with points");
            inspector.FontSizeUnitControl.SelectedIndex = 0;
            Check(inspector.FontSizeControl.Value == 60
                && Style(inspector.ThemeView, 1, "Heading1").Properties[VIEM_STYLE_PROPERTY_CHARACTER_SIZE].declared.kind == VIEM_STYLE_VALUE_FLOAT,
                "switching back to points preserves the resolved appearance");

            string paragraphChild = "Heading2";
            inspector.ThemeView.EditStyle(Style(inspector.ThemeView, 1, paragraphChild), VIEM_STYLE_EDIT_CLEAR_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_SIZE, default);
            inspector.ThemeView.EditStyleString(Style(inspector.ThemeView, 1, paragraphChild), VIEM_STYLE_EDIT_SET_PARENT, 0, "Heading1"); inspector.RefreshForTesting();
            Select(inspector, 1, paragraphChild);
            var paragraphDeclaration = Children<CheckBox>(inspector.RootControl)
                .Single(box => AutomationProperties.GetName(box) == "Declare Size");
            paragraphDeclaration.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            inspector.FontSizeUnitControl.SelectedIndex = 1;
            AssertSize(inspector.ThemeView, 1, paragraphChild, 100, 60);
            inspector.FontSizeControl.Value = 150;
            AssertSize(inspector.ThemeView, 1, paragraphChild, 150, 90);
            SetPoints(inspector.ThemeView, 1, "Heading1", 80); inspector.RefreshForTesting();
            AssertSize(inspector.ThemeView, 1, paragraphChild, 150, 120);

            // A character percentage uses the underlying paragraph, even when
            // its named character parent has an independent absolute size.
            SetPoints(inspector.ThemeView, 1, "Paragraph", 20); inspector.RefreshForTesting();
            string parent = "Link";
            string child = "Code";
            inspector.ThemeView.EditStyle(Style(inspector.ThemeView, 2, child), VIEM_STYLE_EDIT_CLEAR_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_SIZE, default);
            SetPoints(inspector.ThemeView, 2, parent, 40); inspector.RefreshForTesting();
            inspector.ThemeView.EditStyleString(Style(inspector.ThemeView, 2, child), VIEM_STYLE_EDIT_SET_PARENT, 0, parent); inspector.RefreshForTesting();
            Select(inspector, 2, child);
            var declaration = Children<CheckBox>(inspector.RootControl)
                .Single(box => AutomationProperties.GetName(box) == "Declare Size");
            declaration.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            inspector.FontSizeUnitControl.SelectedIndex = 1;
            AssertSize(inspector.ThemeView, 2, child, 200, 40);
            SetPoints(inspector.ThemeView, 2, parent, 70); inspector.RefreshForTesting();
            AssertSize(inspector.ThemeView, 2, child, 200, 40);
            SetPoints(inspector.ThemeView, 1, "Paragraph", 30); inspector.RefreshForTesting();
            AssertSize(inspector.ThemeView, 2, child, 200, 60);
            Select(inspector, 2, child);
            Check(inspector.FontSizeControl.Value == 200 && inspector.FontSizeUnitControl.SelectedIndex == 1,
                "character percentage controls retain the authored percentage after inherited sizes change");

            byte[] saved = inspector.ThemeView.ExportStyleDefaults();
            using var reopened = new CoreDocument([], format: VIEM_FORMAT_MARKDOWN);
            reopened.InitializeStyleDefaults(saved);
            using var reopenedView = new CoreView(reopened, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
            AssertSize(reopenedView, 1, paragraphChild, 150, 120);
            AssertSize(reopenedView, 2, child, 200, 60);
            declaration.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            Check(!Style(inspector.ThemeView, 2, child).Declares(VIEM_STYLE_PROPERTY_CHARACTER_SIZE)
                && !inspector.FontSizeControl.IsEnabled && !inspector.FontSizeUnitControl.IsEnabled
                && double.IsNaN(inspector.FontSizeControl.Value),
                "unchecking font size clears the percentage declaration and disables both value and units");
            Check(Math.Abs(Style(inspector.ThemeView, 2, child).Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number - 70) < .001,
                "clearing a character percentage resumes its named parent's absolute size");
        }
        finally { inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, originalTheme); }
    }
}
#endif
