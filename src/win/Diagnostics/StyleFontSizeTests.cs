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

    private static readonly (string Name, uint Property, float Value)[] BlockMeasurements = [
        ("Top margin", VIEM_STYLE_PROPERTY_BLOCK_MARGIN_TOP, 21.5f),
        ("Right margin", VIEM_STYLE_PROPERTY_BLOCK_MARGIN_RIGHT, 22.5f),
        ("Bottom margin", VIEM_STYLE_PROPERTY_BLOCK_MARGIN_BOTTOM, 23.5f),
        ("Left margin", VIEM_STYLE_PROPERTY_BLOCK_MARGIN_LEFT, 24.5f),
        ("Top padding", VIEM_STYLE_PROPERTY_BLOCK_PADDING_TOP, 15.5f),
        ("Right padding", VIEM_STYLE_PROPERTY_BLOCK_PADDING_RIGHT, 16.5f),
        ("Bottom padding", VIEM_STYLE_PROPERTY_BLOCK_PADDING_BOTTOM, 17.5f),
        ("Left padding", VIEM_STYLE_PROPERTY_BLOCK_PADDING_LEFT, 18.5f),
        ("Top border weight", VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_WIDTH, 2.5f),
        ("Right border weight", VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_WIDTH, 3.5f),
        ("Bottom border weight", VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_WIDTH, 4.5f),
        ("Left border weight", VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH, 5.5f),
    ];

    private static async Task MeasurementClicks(StyleWindow inspector)
    {
        var tabs = Children<Microsoft.UI.Xaml.Controls.Primitives.ToggleButton>(inspector.RootControl)
            .Where(button => button.Content is "Character" or "Block").ToArray();
        byte[] before = inspector.ThemeView.ExportStyleDefaults();
        foreach (string tab in new[] { "Character", "Block" }) {
            var button = tabs.Single(button => button.Content as string == tab);
            button.Focus(FocusState.Programmatic);
            await InputRoutingTests.Key(VirtualKey.Space);
            NumberBox[] numbers = tab == "Character" ? [inspector.FontSizeControl]
                : BlockMeasurements.Select(measurement => Children<NumberBox>(inspector.RootControl)
                    .Single(number => AutomationProperties.GetName(number) == measurement.Name)).ToArray();
            foreach (var number in numbers) {
                string label = AutomationProperties.GetName(number);
                var input = Children<TextBox>(number).Single(input => input.Name == "InputBox");
                Check(input.Text.Length > 0, label + " exposes an explicit numeric fixture");
                button.Focus(FocusState.Programmatic);
                await InputRoutingTests.Drag(inspector, input, [new(.8, .5)], _ => { }, focusTarget: false);
                Check(input.SelectionStart == 0 && input.SelectionLength == input.Text.Length,
                    label + " first pointer click selects the whole value after release");
            }
            // Exercise native editing on a representative field from each tab.
            var representative = Children<TextBox>(numbers.Last()).Single(input => input.Name == "InputBox");
            string name = AutomationProperties.GetName(numbers.Last());
            await Task.Delay((int)InputRoutingTests.GetDoubleClickTime() + 20);
            await InputRoutingTests.Drag(inspector, representative, [new(.8, .5)], _ => { }, focusTarget: false);
            Check(representative.SelectionLength == 0, name + " second pointer click places a caret");
            representative.Select(1, 0);
            await InputRoutingTests.Key(VirtualKey.Right, shift: true);
            Check(representative.SelectionStart == 1 && representative.SelectionLength == 1,
                name + " retains ordinary keyboard selection after its first click");
            button.Focus(FocusState.Programmatic);
            representative.Focus(FocusState.Keyboard);
            representative.Select(1, 0);
            await Task.Delay(60);
            Check(representative.SelectionStart == 1 && representative.SelectionLength == 0,
                name + " does not replace a keyboard-focused caret with deferred pointer selection");
            button.Focus(FocusState.Programmatic);
            await Task.Delay((int)InputRoutingTests.GetDoubleClickTime() + 20);
            (int Start, int Length) nativeDragSelection = default;
            await InputRoutingTests.Drag(inspector, representative, [new(.99, .5), new(.72, .5)], step => {
                if (step == 1) nativeDragSelection = (representative.SelectionStart, representative.SelectionLength);
            }, focusTarget: false);
            Check(nativeDragSelection.Length > 0 && nativeDragSelection.Length < representative.Text.Length
                && (representative.SelectionStart, representative.SelectionLength) == nativeDragSelection,
                name + " entering with a drag preserves the native partial selection after release");
            button.Focus(FocusState.Programmatic);
            await Task.Delay((int)InputRoutingTests.GetDoubleClickTime() + 20);
            await InputRoutingTests.Drag(inspector, representative, [new(.8, .5)], _ => { }, focusTarget: false);
            Check(representative.SelectionStart == 0 && representative.SelectionLength == representative.Text.Length,
                name + " selects the whole number again when the pointer reenters");
        }
        Check(inspector.ThemeView.ExportStyleDefaults().SequenceEqual(before),
            "measurement focus, clicks and selection do not edit theme values");

        // A real replacement should commit one new number, with no old digits
        // retained. Restore the explicit fixture before the percentage tests.
        await InputRoutingTests.Text("7");
        await InputRoutingTests.Key(VirtualKey.Enter);
        Check(Style(inspector.ThemeView, 1, "Paragraph").Value(VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH).number == 7,
            "typing after a block measurement's entry click replaces its entire value");
        inspector.ThemeView.EditStyle(Style(inspector.ThemeView, 1, "Paragraph"), VIEM_STYLE_EDIT_SET_DECLARATION,
            VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH, CoreView.Number(5.5f));
        inspector.RefreshForTesting();
        var character = tabs.Single(button => button.Content as string == "Character");
        character.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
        var size = Children<TextBox>(inspector.FontSizeControl).Single(input => input.Name == "InputBox");
        await InputRoutingTests.Drag(inspector, size, [new(.8, .5)], _ => { }, focusTarget: false);
        await InputRoutingTests.Text("27"); await InputRoutingTests.Key(VirtualKey.Enter);
        Check(Style(inspector.ThemeView, 1, "Paragraph").Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number == 27,
            "typing after a font-size entry click replaces its entire value");
        SetPoints(inspector.ThemeView, 1, "Paragraph", 20); inspector.RefreshForTesting();
    }

    internal static async Task Run(EditorPane pane, Preferences preferences)
    {
        using var document = new CoreDocument("# Title\n\nBody"u8.ToArray(), format: VIEM_FORMAT_MARKDOWN);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        SetPoints(view, 1, "Paragraph", 20);
        SetPoints(view, 1, "Heading1", 24);
        foreach (var measurement in BlockMeasurements)
            view.EditStyle(Style(view, 1, "Paragraph"), VIEM_STYLE_EDIT_SET_DECLARATION,
                measurement.Property, CoreView.Number(measurement.Value));
        byte[] originalTheme = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
        preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, view.ExportStyleDefaults());
        byte[] originalSource = document.Source(document.State.document_revision);
        var inspector = new StyleWindow(view, preferences, followCaret: false);
        inspector.Activate(); await Task.Delay(150);
        try
        {
            await MeasurementClicks(inspector);
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
