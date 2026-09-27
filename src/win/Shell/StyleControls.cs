using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Windows.UI.Text;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    private bool IsBase => (selected.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0;
    private bool ShowsValue(uint property) => IsBase || selected.Declares(property);
    private static StackPanel Row(StackPanel target)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        target.Children.Add(row); return row;
    }
    private StackPanel Property(Panel row, string label, uint property, FrameworkElement editor, bool caption = true, bool reserveCaption = false)
    {
        var group = new StackPanel { Spacing = 3 };
        if (caption || reserveCaption) {
            var title = new TextBlock { Text = caption ? label : "", FontSize = 11, Opacity = .65, Height = 17 };
            title.Tapped += (_, _) => { if (!loading && selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS) && !ShowsValue(property)) Try(() => DeclareEffective(property)); };
            group.Children.Add(title);
        }
        var body = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, Height = 28 };
        var enabled = new CheckBox { MinWidth = 0, MinHeight = 0, Padding = new(0), VerticalAlignment = VerticalAlignment.Center };
        editor.VerticalAlignment = VerticalAlignment.Center;
        AutomationProperties.SetName(enabled, "Declare " + label); AutomationProperties.SetName(editor, label);
        ToolTipService.SetToolTip(enabled, "Override inherited");
        body.Children.Add(enabled); body.Children.Add(editor); group.Children.Add(body); row.Children.Add(group);
        refreshFields.Add(() => {
            bool editable = selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS);
            enabled.IsChecked = ShowsValue(property); enabled.IsEnabled = editable && !IsBase;
            SetEnabled(editor, editable && ShowsValue(property));
        });
        enabled.Click += (_, _) => {
            if (loading) return;
            Try(() => { if (enabled.IsChecked == true) DeclareEffective(property);
                else view.EditStyle(selected, VIEM_STYLE_EDIT_CLEAR_DECLARATION, property, default); });
        };
        return group;
    }
    private static void SetEnabled(FrameworkElement element, bool enabled)
    {
        if (element is Control control) control.IsEnabled = enabled;
        else if (element is Panel panel) EnableChildren(panel, enabled);
    }
    private static StackPanel Inline(params UIElement[] elements)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        foreach (var element in elements) row.Children.Add(element); return row;
    }
    private static TextBlock Unit(string text) => new() { Text = text, FontSize = 11, Opacity = .65, VerticalAlignment = VerticalAlignment.Center };
    private static FontIcon Icon(string glyph) => new() { Glyph = glyph, FontSize = 16, Opacity = .7, VerticalAlignment = VerticalAlignment.Center, Width = 18 };
    private void Number(Panel row, string label, uint property, float min = -1000, float max = 1000, bool caption = true,
        string? icon = null, double? width = null, double? groupWidth = null)
    {
        var value = new NumberBox { Width = width ?? (caption ? 104 : 82), Minimum = min, Maximum = max, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
        AutomationProperties.SetName(value, label);
        var body = Inline(value, Unit("pt"));
        if (icon != null) body.Children.Insert(0, Icon(icon));
        var group = Property(row, label, property, body, caption);
        if (caption) group.Width = groupWidth ?? 188;
        refreshFields.Add(() => value.Value = ShowsValue(property) ? selected.Value(property).number : double.NaN);
        value.ValueChanged += (_, _) => { if (!loading && double.IsFinite(value.Value)) Try(() => view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, property, CoreView.Number((float)value.Value))); };
    }
    private void Choice(Panel row, string label, uint property, uint valueKind, string[] choices, uint[] values)
    {
        var combo = new ComboBox { Width = 162, ItemsSource = choices };
        Property(row, label, property, combo);
        refreshFields.Add(() => combo.SelectedIndex = ShowsValue(property) ? Array.IndexOf(values, selected.Value(property).enum_value) : -1);
        combo.SelectionChanged += (_, _) => { if (!loading && combo.SelectedIndex >= 0) Try(() => view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, property, CoreView.Enum(valueKind, values[combo.SelectedIndex]))); };
    }
    private void Boolean(Panel row, string label, uint property)
    {
        var button = new ToggleButton { Content = label, Width = 32, MinWidth = 0, Padding = new(0), FontSize = 14 };
        if (label == "B") button.FontWeight = Microsoft.UI.Text.FontWeights.Bold;
        if (label == "I") button.FontStyle = FontStyle.Italic;
        if (label is "U" or "S") button.Content = new TextBlock { Text = label, TextDecorations = label == "U" ? TextDecorations.Underline : TextDecorations.Strikethrough, FontSize = 14 };
        Property(row, label, property, button, caption: false, reserveCaption: true);
        refreshFields.Add(() => button.IsChecked = ShowsValue(property) && selected.Value(property).enum_value != 0);
        button.Click += (_, _) => Try(() => view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, property,
            CoreView.Enum(property == VIEM_STYLE_PROPERTY_CHARACTER_SLANT ? VIEM_STYLE_VALUE_FONT_SLANT : VIEM_STYLE_VALUE_BOOLEAN, button.IsChecked == true ? 1u : 0u)));
    }
    private void ScriptPosition(Panel row)
    {
        var buttons = Inline();
        foreach (var (label, text, position) in new[] {
            ("Superscript", "x²", VIEM_SCRIPT_POSITION_SUPERSCRIPT),
            ("Subscript", "x₂", VIEM_SCRIPT_POSITION_SUBSCRIPT) }) {
            var button = new ToggleButton { Content = text, Width = 36, MinWidth = 0, Padding = new(0), FontSize = 16 };
            AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label); buttons.Children.Add(button);
            refreshFields.Add(() => button.IsChecked = ShowsValue(VIEM_STYLE_PROPERTY_CHARACTER_SCRIPT_POSITION)
                && selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SCRIPT_POSITION).enum_value == position);
            button.Click += (_, _) => Try(() => view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION,
                VIEM_STYLE_PROPERTY_CHARACTER_SCRIPT_POSITION, CoreView.Enum(VIEM_STYLE_VALUE_SCRIPT_POSITION,
                    button.IsChecked == true ? position : VIEM_SCRIPT_POSITION_NORMAL)));
        }
        Property(row, "Superscript / Subscript", VIEM_STYLE_PROPERTY_CHARACTER_SCRIPT_POSITION, buttons);
    }
    private void SetColor(uint property, global::Windows.UI.Color color, ViemStyleEditGroupV1? group = null)
    {
        var value = CoreView.Enum(VIEM_STYLE_VALUE_COLOR, 0); value.color = new() { red = color.R / 255f, green = color.G / 255f, blue = color.B / 255f, alpha = color.A / 255f };
        view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, property, value, group);
    }
    private void BuildCharacter()
    {
        BuildFontRow();
        var row = Row(character);
        Boolean(row, "B", VIEM_STYLE_PROPERTY_CHARACTER_BOLD); Boolean(row, "I", VIEM_STYLE_PROPERTY_CHARACTER_SLANT);
        Boolean(row, "U", VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE); Boolean(row, "S", VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH);
        ColorControl(row, "Text Color", VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND); ColorControl(row, "Background Color", VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND);
        character.Children.Add(Separator());
        row = Row(character); Number(row, "Tracking", VIEM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING, icon: "\uE8D2"); ScriptPosition(row);
    }
    private void BuildBlock()
    {
        ColorControl(Row(block), "Background color", VIEM_STYLE_PROPERTY_BLOCK_BACKGROUND);
        foreach (var side in new[] {
            ("Top", VIEM_STYLE_PROPERTY_BLOCK_MARGIN_TOP, VIEM_STYLE_PROPERTY_BLOCK_PADDING_TOP, VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_WIDTH, VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_COLOR),
            ("Right", VIEM_STYLE_PROPERTY_BLOCK_MARGIN_RIGHT, VIEM_STYLE_PROPERTY_BLOCK_PADDING_RIGHT, VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_WIDTH, VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_COLOR),
            ("Bottom", VIEM_STYLE_PROPERTY_BLOCK_MARGIN_BOTTOM, VIEM_STYLE_PROPERTY_BLOCK_PADDING_BOTTOM, VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_WIDTH, VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_COLOR),
            ("Left", VIEM_STYLE_PROPERTY_BLOCK_MARGIN_LEFT, VIEM_STYLE_PROPERTY_BLOCK_PADDING_LEFT, VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH, VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_COLOR) }) {
            var row = Row(block);
            Number(row, side.Item1 + " margin", side.Item2, width: 70, groupWidth: 138);
            Number(row, side.Item1 + " padding", side.Item3, min: 0, width: 70, groupWidth: 138);
            Number(row, side.Item1 + " border weight", side.Item4, min: 0, width: 70, groupWidth: 138);
            ColorControl(row, side.Item1 + " border color", side.Item5);
        }
    }

    private void BuildParagraph()
    {
        var top = new Grid(); top.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); top.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        paragraph.Children.Add(top);
        var alignment = Inline(); var buttons = new List<ToggleButton>();
        foreach (var (label, glyph, value) in new[] { ("Align start", "\uE8E4", 1u), ("Align center", "\uE8E3", 3u), ("Align end", "\uE8E2", 2u) }) {
            var button = new ToggleButton { Content = new FontIcon { Glyph = glyph, FontSize = 16 }, Width = 36, MinWidth = 0, Padding = new(0) };
            AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label); alignment.Children.Add(button); buttons.Add(button);
            button.Click += (_, _) => Try(() => view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT, CoreView.Enum(VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT, value)));
            refreshFields.Add(() => button.IsChecked = ShowsValue(VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT) && selected.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT).enum_value == value);
        }
        Property(top, "Alignment", VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT, alignment, caption: false, reserveCaption: true);
        Choice(top, "Direction", VIEM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION, VIEM_STYLE_VALUE_WRITING_DIRECTION, ["Automatic", "Left to Right", "Right to Left"], [0, 1, 2]); Grid.SetColumn((FrameworkElement)top.Children.Last(), 1);
        paragraph.Children.Add(Separator());
        var row = Row(paragraph);
        Number(row, "Start indent", VIEM_STYLE_PROPERTY_PARAGRAPH_LEADING_INDENT, icon: "\uE8A0"); Number(row, "End indent", VIEM_STYLE_PROPERTY_PARAGRAPH_TRAILING_INDENT, icon: "\uE89F"); Number(row, "First line", VIEM_STYLE_PROPERTY_PARAGRAPH_FIRST_LINE_INDENT, icon: "\uE8A0");
        row = Row(paragraph);
        var spacing = new ComboBox { ItemsSource = new[] { "Normal", "Multiple", "At least", "Exact" }, Width = 112, MinWidth = 0 };
        var amount = new NumberBox { Minimum = .01, Maximum = 1000, Width = 60, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
        var amountUnit = Unit("");
        AutomationProperties.SetName(spacing, "Line spacing"); AutomationProperties.SetName(amount, "Line spacing amount");
        AutomationProperties.SetName(amountUnit, "Line spacing units");
        Property(row, "Line spacing", VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING, Inline(spacing, amount, amountUnit));
        refreshFields.Add(() => {
            var v = selected.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING);
            int spacingIndex = ShowsValue(VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING) ? (int)v.enum_value - 1 : -1;
            if (spacingIndex == 1 && RoundLineSpacing(v.number) == 1) spacingIndex = 0;
            spacing.SelectedIndex = spacingIndex;
            amount.IsEnabled = ShowsValue(VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING) && spacing.SelectedIndex >= 0;
            amount.Minimum = spacing.SelectedIndex is 0 or 1 ? .1 : 0;
            amount.SmallChange = spacing.SelectedIndex is 0 or 1 ? .1 : 1;
            amount.Value = amount.IsEnabled ? spacing.SelectedIndex == 0 ? 1 : RoundLineSpacing(v.number) : double.NaN;
            amountUnit.Text = spacing.SelectedIndex switch { 0 or 1 => "\u00D7", 2 or 3 => "pt", _ => "" };
        });
        spacing.SelectionChanged += (_, _) => {
            if (loading || spacing.SelectedIndex < 0) return;
            var current = selected.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING);
            bool currentDisplaysAsNormal = current.enum_value == VIEM_STYLE_LINE_SPACING_NORMAL
                || current.enum_value == VIEM_STYLE_LINE_SPACING_MULTIPLIER && RoundLineSpacing(current.number) == 1;
            float number = spacing.SelectedIndex == 0 ? 0 : spacing.SelectedIndex == 1 && currentDisplaysAsNormal ? 1.1f
                : current.enum_value == (uint)spacing.SelectedIndex + 1 && current.number > 0 ? current.number
                : spacing.SelectedIndex == 1 ? 1.1f : selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number;
            Try(() => SetLineSpacing((uint)spacing.SelectedIndex + 1, number));
        };
        amount.ValueChanged += (_, _) => {
            if (loading || !double.IsFinite(amount.Value) || spacing.SelectedIndex < 0) return;
            double number = RoundLineSpacing(amount.Value);
            uint kind = (uint)spacing.SelectedIndex + 1;
            if (spacing.SelectedIndex is 0 or 1) {
                number = Math.Max(.1, number);
                kind = number == 1 ? VIEM_STYLE_LINE_SPACING_NORMAL : VIEM_STYLE_LINE_SPACING_MULTIPLIER;
            }
            Try(() => SetLineSpacing(kind, (float)number));
        };
    }
    private static double RoundLineSpacing(double number) => Math.Round(number, 1, MidpointRounding.AwayFromZero);
    private void SetLineSpacing(uint kind, float number)
    {
        if (kind == VIEM_STYLE_LINE_SPACING_MULTIPLIER) {
            number = (float)Math.Max(.1, RoundLineSpacing(number));
            if (number == 1) kind = VIEM_STYLE_LINE_SPACING_NORMAL;
        } else if (kind is VIEM_STYLE_LINE_SPACING_AT_LEAST or VIEM_STYLE_LINE_SPACING_EXACT) {
            number = (float)RoundLineSpacing(number);
        }
        var value = CoreView.Enum(VIEM_STYLE_VALUE_LINE_SPACING, kind); value.number = number;
        view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_PARAGRAPH_LINE_SPACING, value);
    }
}
