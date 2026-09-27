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
        using var document = new CoreDocument("{\\rtf1{\\stylesheet{\\s0 Paragraph;}{\\s1\\sbasedon0\\b\\fs48 Heading1;}{\\s2\\sbasedon0\\b\\fs36 Heading2;}{\\*\\cs1\\f1 Code;}}{\\fonttbl{\\f0 Times New Roman;}{\\f1 Courier New;}}\\s1 Title\\par\\s0 Body}"u8.ToArray(), format: VIEM_FORMAT_RTF);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        SetPoints(view, 1, "Paragraph", 20);
        byte[] originalTheme = preferences.ThemeStyleDefaults(VIEM_FORMAT_RTF);
        preferences.SaveThemeStyles(VIEM_FORMAT_RTF, view.ExportStyleDefaults());
        byte[] originalSource = document.Source(document.State.document_revision);
        var inspector = new StyleWindow(view, preferences, followCaret: false);
        inspector.Activate(); await Task.Delay(150);
        try
        {
            Check(((string[])inspector.FontSizeUnitControl.ItemsSource).SequenceEqual(new[] { "pt" })
                && !inspector.FontSizeUnitControl.IsEnabled,
                "Base Paragraph font size only offers points");
            Select(inspector, 1, "RtfP1");
            Check(inspector.FontSizeControl.Value == 24 && inspector.FontSizeUnitControl.SelectedIndex == 0,
                "an absolute heading size initially shows points");
            inspector.FontSizeUnitControl.SelectedIndex = 1;
            AssertSize(inspector.ThemeView, 1, "RtfP1", 120, 24);
            Check(document.Source(document.State.document_revision).SequenceEqual(originalSource),
                "changing theme font-size units preserves source and its undo history");
            double priorStatusSize = preferences.StatusFontSize;
            preferences.Set("theme", "statusFontSize", System.Text.Json.Nodes.JsonValue.Create(priorStatusSize == 11 ? 12 : 11));
            inspector.UndoThemeForTesting();
            Check(inspector.FontSizeUnitControl.SelectedIndex == 0 && inspector.FontSizeControl.Value == 24,
                "unrelated appearance changes preserve theme font-size Undo without touching source history");
            inspector.RedoThemeForTesting();
            preferences.Set("theme", "statusFontSize", System.Text.Json.Nodes.JsonValue.Create(priorStatusSize));
            AssertSize(inspector.ThemeView, 1, "RtfP1", 120, 24);
            Check(inspector.FontSizeControl.Value == 120 && inspector.FontSizeControl.Minimum == 10
                && inspector.FontSizeControl.Maximum == 1000 && inspector.FontSizeControl.SmallChange == 1,
                "percentage font sizes display their declaration with integer stepper bounds 10–1000");
            inspector.FontSizeControl.Value = 150;
            AssertSize(inspector.ThemeView, 1, "RtfP1", 150, 30);
            inspector.FontSizeControl.Value = 150.5;
            Check(inspector.Error.Length > 0 && inspector.FontSizeControl.Value == 150,
                "fractional percentage input is rejected and the last committed size remains visible");
            AssertSize(inspector.ThemeView, 1, "RtfP1", 150, 30);
            inspector.FontSizeControl.Value = 10;
            AssertSize(inspector.ThemeView, 1, "RtfP1", 10, 2);
            inspector.FontSizeControl.Value = 1000;
            AssertSize(inspector.ThemeView, 1, "RtfP1", 1000, 200);
            inspector.FontSizeControl.Value = 150;
            SetPoints(inspector.ThemeView, 1, "Paragraph", 40); inspector.RefreshForTesting();
            AssertSize(inspector.ThemeView, 1, "RtfP1", 150, 60);
            Select(inspector, 1, "RtfP1");
            Check(inspector.FontSizeControl.Value == 150, "changing the parent does not replace the percentage declaration with points");
            inspector.FontSizeUnitControl.SelectedIndex = 0;
            Check(inspector.FontSizeControl.Value == 60
                && Style(inspector.ThemeView, 1, "RtfP1").Properties[VIEM_STYLE_PROPERTY_CHARACTER_SIZE].declared.kind == VIEM_STYLE_VALUE_FLOAT,
                "switching back to points preserves the resolved appearance");

            string paragraphChild = inspector.ThemeView.CreateStyle(1, "Relative paragraph");
            inspector.ThemeView.EditStyleString(Style(inspector.ThemeView, 1, paragraphChild), VIEM_STYLE_EDIT_SET_PARENT, 0, "RtfP1"); inspector.RefreshForTesting();
            Select(inspector, 1, paragraphChild);
            var paragraphDeclaration = Children<CheckBox>(inspector.RootControl)
                .Single(box => AutomationProperties.GetName(box) == "Declare Size");
            paragraphDeclaration.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            inspector.FontSizeUnitControl.SelectedIndex = 1;
            AssertSize(inspector.ThemeView, 1, paragraphChild, 100, 60);
            inspector.FontSizeControl.Value = 150;
            AssertSize(inspector.ThemeView, 1, paragraphChild, 150, 90);
            SetPoints(inspector.ThemeView, 1, "RtfP1", 80); inspector.RefreshForTesting();
            AssertSize(inspector.ThemeView, 1, paragraphChild, 150, 120);

            // A character percentage uses the underlying paragraph, even when
            // its named character parent has an independent absolute size.
            SetPoints(inspector.ThemeView, 1, "Paragraph", 20); inspector.RefreshForTesting();
            string parent = inspector.ThemeView.CreateStyle(2, "Sized character parent");
            string child = inspector.ThemeView.CreateStyle(2, "Relative character");
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
            using var reopened = new CoreDocument([], format: VIEM_FORMAT_RTF);
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
        finally { inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_RTF, originalTheme); }
        await RtfConversion(pane, preferences);
    }

    private static async Task RtfConversion(EditorPane pane, Preferences preferences)
    {
        using var document = new CoreDocument("{\\rtf1 Text}"u8.ToArray(), format: VIEM_FORMAT_RTF);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        SetPoints(view, 1, "Paragraph", 14);
        string relative = view.CreateStyle(1, "Relative RTF paragraph");
        SetPoints(view, 1, relative, 14);
        byte[] originalTheme = preferences.ThemeStyleDefaults(VIEM_FORMAT_RTF);
        preferences.SaveThemeStyles(VIEM_FORMAT_RTF, view.ExportStyleDefaults());
        byte[] originalSource = document.Source(document.State.document_revision);
        var inspector = new StyleWindow(view, preferences, followCaret: false);
        inspector.Activate(); await Task.Delay(150);
        try
        {
            Select(inspector, 1, relative);
            inspector.FontSizeUnitControl.SelectedIndex = 1;
            inspector.FontSizeControl.Value = 90;
            AssertSize(inspector.ThemeView, 1, relative, 90, 12.6f);
            inspector.FontSizeUnitControl.SelectedIndex = 0;
            Check(inspector.Error.Length == 0 && inspector.FontSizeControl.Value == 12.5
                && Style(inspector.ThemeView, 1, relative).Properties[VIEM_STYLE_PROPERTY_CHARACTER_SIZE].declared.kind == VIEM_STYLE_VALUE_FLOAT,
                "RTF percentage-to-point conversion rounds to the nearest representable half-point");
            inspector.FontSizeUnitControl.SelectedIndex = 1;
            inspector.FontSizeControl.Value = 10;
            SetPoints(inspector.ThemeView, 1, "Paragraph", 1); inspector.RefreshForTesting();
            AssertSize(inspector.ThemeView, 1, relative, 10, .1f);
            inspector.FontSizeUnitControl.SelectedIndex = 0;
            Check(inspector.Error.Length == 0 && inspector.FontSizeControl.Value == .5 && inspector.FontSizeControl.Minimum == .5,
                "RTF percentage-to-point conversion retains the smallest positive half-point without field coercion");
            byte[] saved = inspector.ThemeView.ExportStyleDefaults();
            using var reopened = new CoreDocument([], format: VIEM_FORMAT_RTF);
            reopened.InitializeStyleDefaults(saved);
            using var reopenedView = new CoreView(reopened, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
            Check(document.Source(document.State.document_revision).SequenceEqual(originalSource), "theme size conversion keeps authored RTF source unchanged");
            Check(Style(reopenedView, 1, relative).Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number == .5f,
                "the converted half-point size survives RTF save/reopen");
        }
        finally { inspector.Close(); preferences.SaveThemeStyles(VIEM_FORMAT_RTF, originalTheme); }
    }
}
#endif
