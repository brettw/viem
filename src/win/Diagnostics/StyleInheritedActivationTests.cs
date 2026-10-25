#if DEBUG
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Viem.Windows.Core;
using Viem.Windows.Editor;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static partial class StyleInspectorBehaviorTests
{
    internal static async Task InheritedActivation(EditorPane pane, Preferences preferences)
    {
        byte[] originalTheme = preferences.ThemeStyleDefaults(VIEM_FORMAT_MARKDOWN);
        byte[] source = "# Title\n\nBody"u8.ToArray();
        using var document = new CoreDocument(source, format: VIEM_FORMAT_MARKDOWN);
        preferences.AttachThemeDocument(document);
        using var view = new CoreView(document, pane.Canvas.Device, pane.DispatcherQueue, 700, 400);
        var inspector = new StyleWindow(view, preferences, followCaret: false);
        inspector.Activate();
        await InputRoutingTests.Settle();
        try {
            inspector.StylePicker.SelectedItem = inspector.StylePicker.Items.OfType<StyleDefinition>().Single(s => s.Id == "Heading2");
            T Field<T>(string name) where T : Control => Children<T>(inspector.RootControl)
                .Single(control => AutomationProperties.GetName(control) == name);
            void Clear(uint property) {
                inspector.ThemeView.EditStyle(Selected(inspector), VIEM_STYLE_EDIT_CLEAR_DECLARATION, property, default);
                inspector.RefreshForTesting();
            }
            async Task Click(FrameworkElement control) {
                control.StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false });
                await InputRoutingTests.Settle();
                await InputRoutingTests.Drag(inspector, control,
                    new global::Windows.Foundation.Point[] { new(.5, .5) }, _ => { }, focusTarget: false);
            }
            async Task CloseChoice(ComboBox combo) {
                var closed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
                void Closed(object? sender, object args) => closed.TrySetResult();
                combo.DropDownClosed += Closed;
                try {
                    combo.IsDropDownOpen = false;
                    await closed.Task.WaitAsync(TimeSpan.FromSeconds(5));
                    await InputRoutingTests.Settle();
                }
                finally { combo.DropDownClosed -= Closed; }
            }
            void Declared(uint property, string label) => Check(Selected(inspector).Declares(property)
                && Children<CheckBox>(inspector.RootControl).Any(c => AutomationProperties.GetName(c) == "Declare " + label && c.IsChecked == true)
                && inspector.Error.Length == 0,
                "clicking inherited " + label + " checks its declaration without an error"
                + $" (declared={Selected(inspector).Declares(property)}, error={inspector.Error})");

            Clear(VIEM_STYLE_PROPERTY_CHARACTER_SIZE);
            var size = inspector.FontSizeControl;
            Check(!size.IsEnabled, "inherited size starts disabled");
            float inheritedSize = Selected(inspector).Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number;
            await Click(size);
            Declared(VIEM_STYLE_PROPERTY_CHARACTER_SIZE, "Size");
            var input = Children<TextBox>(size).Single(t => t.Name == "InputBox");
            Check(size.IsEnabled && size.Value == inheritedSize && input.Text.Length > 0
                && input.SelectionLength == input.Text.Length && ReferenceEquals(FocusManager.GetFocusedElement(input.XamlRoot), input),
                "one click declares the inherited measurement, focuses it and selects its full text");
            Clear(VIEM_STYLE_PROPERTY_CHARACTER_DIRECTION);
            var direction = Children<ComboBox>(inspector.CharacterPanel).Single(c => AutomationProperties.GetName(c) == "Direction");
            await Click(direction);
            Declared(VIEM_STYLE_PROPERTY_CHARACTER_DIRECTION, "Direction");
            Check(direction.IsDropDownOpen, "one click opens an inherited choice dropdown");
            await CloseChoice(direction);

            Clear(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND);
            var color = Field<Button>("Text Color");
            var flyout = (Flyout)color.Flyout;
            bool opened = false;
            flyout.Opened += (_, _) => opened = true;
            await Click(color);
            Check(await SelfTestRunner.Until(() => opened), "one click opens the inherited color picker");
            Declared(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND, "Text Color");
            await Close(flyout);

        }
        finally {
            inspector.Close();
            preferences.SaveThemeStyles(VIEM_FORMAT_MARKDOWN, originalTheme);
        }
    }
}
#endif
