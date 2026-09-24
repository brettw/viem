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
        => inspector.StylePicker.SelectedItem = ((StyleDefinition[])inspector.StylePicker.ItemsSource)
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
    private static unsafe void IncludeStyles(CoreView view)
    {
        view.Apply(outcome => {
            var state = view.Document.State;
            var request = New<ViemSetIncludeStyleDefinitionsV1>();
            request.enabled = 1; request.document_id = state.document_id; request.document_revision = state.document_revision;
            return viem_core_view_set_include_style_definitions(view.Document.Handle, view.Id, &request, outcome);
        });
    }
    internal static async Task Run(EditorPane pane, Preferences preferences)
    {
        using var document = new CoreDocument("<h1>Title</h1><p>Body</p><!--keep-->"u8.ToArray(), format: VIEM_FORMAT_HTML);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        IncludeStyles(view);
        SetPoints(view, 1, "Paragraph", 20);
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
            AssertSize(view, 1, "Heading1", 120, 24);
            view.Undo();
            Check(inspector.FontSizeUnitControl.SelectedIndex == 0 && inspector.FontSizeControl.Value == 24,
                "one undo restores the prior font-size unit and value");
            view.Redo();
            AssertSize(view, 1, "Heading1", 120, 24);
            Check(inspector.FontSizeControl.Value == 120 && inspector.FontSizeControl.Minimum == 10
                && inspector.FontSizeControl.Maximum == 1000 && inspector.FontSizeControl.SmallChange == 1,
                "percentage font sizes display their declaration with integer stepper bounds 10–1000");
            inspector.FontSizeControl.Value = 150;
            AssertSize(view, 1, "Heading1", 150, 30);
            inspector.FontSizeControl.Value = 150.5;
            Check(inspector.Error.Length > 0 && inspector.FontSizeControl.Value == 150,
                "fractional percentage input is rejected and the last committed size remains visible");
            AssertSize(view, 1, "Heading1", 150, 30);
            inspector.FontSizeControl.Value = 10;
            AssertSize(view, 1, "Heading1", 10, 2);
            inspector.FontSizeControl.Value = 1000;
            AssertSize(view, 1, "Heading1", 1000, 200);
            inspector.FontSizeControl.Value = 150;
            SetPoints(view, 1, "Paragraph", 40);
            AssertSize(view, 1, "Heading1", 150, 60);
            Select(inspector, 1, "Heading1");
            Check(inspector.FontSizeControl.Value == 150, "changing the parent does not replace the percentage declaration with points");
            inspector.FontSizeUnitControl.SelectedIndex = 0;
            Check(inspector.FontSizeControl.Value == 60
                && Style(view, 1, "Heading1").Properties[VIEM_STYLE_PROPERTY_CHARACTER_SIZE].declared.kind == VIEM_STYLE_VALUE_FLOAT,
                "switching back to points preserves the resolved appearance");

            string paragraphChild = view.CreateStyle(1, "Relative paragraph");
            view.EditStyleString(Style(view, 1, paragraphChild), VIEM_STYLE_EDIT_SET_PARENT, 0, "Heading1");
            Select(inspector, 1, paragraphChild);
            var paragraphDeclaration = Children<CheckBox>(inspector.RootControl)
                .Single(box => AutomationProperties.GetName(box) == "Declare Size");
            paragraphDeclaration.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            inspector.FontSizeUnitControl.SelectedIndex = 1;
            AssertSize(view, 1, paragraphChild, 100, 60);
            inspector.FontSizeControl.Value = 150;
            AssertSize(view, 1, paragraphChild, 150, 90);
            SetPoints(view, 1, "Heading1", 80);
            AssertSize(view, 1, paragraphChild, 150, 120);

            // A character percentage uses the underlying paragraph, even when
            // its named character parent has an independent absolute size.
            SetPoints(view, 1, "Paragraph", 20);
            string parent = view.CreateStyle(2, "Sized character parent");
            string child = view.CreateStyle(2, "Relative character");
            SetPoints(view, 2, parent, 40);
            view.EditStyleString(Style(view, 2, child), VIEM_STYLE_EDIT_SET_PARENT, 0, parent);
            Select(inspector, 2, child);
            var declaration = Children<CheckBox>(inspector.RootControl)
                .Single(box => AutomationProperties.GetName(box) == "Declare Size");
            declaration.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            inspector.FontSizeUnitControl.SelectedIndex = 1;
            AssertSize(view, 2, child, 200, 40);
            SetPoints(view, 2, parent, 70);
            AssertSize(view, 2, child, 200, 40);
            SetPoints(view, 1, "Paragraph", 30);
            AssertSize(view, 2, child, 200, 60);
            Select(inspector, 2, child);
            Check(inspector.FontSizeControl.Value == 200 && inspector.FontSizeUnitControl.SelectedIndex == 1,
                "character percentage controls retain the authored percentage after inherited sizes change");

            byte[] saved = document.Source(document.State.document_revision);
            using var reopened = new CoreDocument(saved, format: VIEM_FORMAT_HTML);
            using var reopenedView = new CoreView(reopened, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
            AssertSize(reopenedView, 1, paragraphChild, 150, 120);
            AssertSize(reopenedView, 2, child, 200, 60);
            declaration.Focus(FocusState.Programmatic); await InputRoutingTests.Key(VirtualKey.Space);
            Check(!Style(view, 2, child).Declares(VIEM_STYLE_PROPERTY_CHARACTER_SIZE)
                && !inspector.FontSizeControl.IsEnabled && !inspector.FontSizeUnitControl.IsEnabled
                && double.IsNaN(inspector.FontSizeControl.Value),
                "unchecking font size clears the percentage declaration and disables both value and units");
            Check(Math.Abs(Style(view, 2, child).Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number - 70) < .001,
                "clearing a character percentage resumes its named parent's absolute size");
        }
        finally { inspector.Close(); }
        await RtfConversion(pane, preferences);
    }

    private static async Task RtfConversion(EditorPane pane, Preferences preferences)
    {
        using var document = new CoreDocument("{\\rtf1 Text}"u8.ToArray(), format: VIEM_FORMAT_RTF);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        SetPoints(view, 1, "Paragraph", 14);
        string relative = view.CreateStyle(1, "Relative RTF paragraph");
        SetPoints(view, 1, relative, 14);
        var inspector = new StyleWindow(view, preferences, followCaret: false);
        inspector.Activate(); await Task.Delay(150);
        try
        {
            Select(inspector, 1, relative);
            inspector.FontSizeUnitControl.SelectedIndex = 1;
            inspector.FontSizeControl.Value = 90;
            AssertSize(view, 1, relative, 90, 12.6f);
            inspector.FontSizeUnitControl.SelectedIndex = 0;
            Check(inspector.Error.Length == 0 && inspector.FontSizeControl.Value == 12.5
                && Style(view, 1, relative).Properties[VIEM_STYLE_PROPERTY_CHARACTER_SIZE].declared.kind == VIEM_STYLE_VALUE_FLOAT,
                "RTF percentage-to-point conversion rounds to the nearest representable half-point");
            inspector.FontSizeUnitControl.SelectedIndex = 1;
            inspector.FontSizeControl.Value = 10;
            SetPoints(view, 1, "Paragraph", 1);
            AssertSize(view, 1, relative, 10, .1f);
            inspector.FontSizeUnitControl.SelectedIndex = 0;
            Check(inspector.Error.Length == 0 && inspector.FontSizeControl.Value == .5 && inspector.FontSizeControl.Minimum == .5,
                "RTF percentage-to-point conversion retains the smallest positive half-point without field coercion");
            byte[] saved = document.Source(document.State.document_revision);
            using var reopened = new CoreDocument(saved, format: VIEM_FORMAT_RTF);
            using var reopenedView = new CoreView(reopened, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
            Check(Style(reopenedView, 1, relative).Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number == .5f,
                "the converted half-point size survives RTF save/reopen");
        }
        finally { inspector.Close(); }
    }
}
#endif
