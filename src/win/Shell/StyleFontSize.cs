using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    private readonly NumberBox fontSize = NumberEditor(82, 1, 256);
    private readonly ComboBox fontSizeUnit = new FontSizeUnitPicker() {
        Width = 52, MinWidth = 0, Padding = new Thickness(4, 0, 4, 0),
    };

    private sealed partial class FontSizeUnitPicker : ComboBox
    {
        protected override void OnApplyTemplate()
        {
            base.OnApplyTemplate();
            if (GetTemplateChild("DropDownGlyph") is FrameworkElement glyph
                && VisualTreeHelper.GetParent(glyph) is Grid layout) {
                // WinUI reserves 38 px for the arrow; short units need more of this 52 px control.
                layout.ColumnDefinitions[Grid.GetColumn(glyph)].Width = new GridLength(24);
                glyph.Margin = new Thickness(0, 0, 6, 0);
            }
        }
    }

    private void BuildFontSize(Panel row)
    {
        AutomationProperties.SetName(fontSize, "Size");
        AutomationProperties.SetName(fontSizeUnit, "Font size units");
        Property(row, "Size", VIEM_STYLE_PROPERTY_CHARACTER_SIZE, Inline(fontSize, fontSizeUnit), caption: false);
        refreshFields.Add(() => {
            bool enabled = selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS)
                && ShowsValue(VIEM_STYLE_PROPERTY_CHARACTER_SIZE);
            bool percentage = !IsBase && selected.Declares(VIEM_STYLE_PROPERTY_CHARACTER_SIZE)
                && selected.Properties[VIEM_STYLE_PROPERTY_CHARACTER_SIZE].declared.kind == VIEM_STYLE_VALUE_PERCENTAGE;
            double number = percentage
                ? selected.Properties[VIEM_STYLE_PROPERTY_CHARACTER_SIZE].declared.enum_value
                : selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number;
            fontSizeUnit.ItemsSource = IsBase ? new[] { "pt" } : new[] { "pt", "%" };
            fontSizeUnit.SelectedIndex = ShowsValue(VIEM_STYLE_PROPERTY_CHARACTER_SIZE) ? percentage ? 1 : 0 : -1;
            fontSizeUnit.IsEnabled = enabled && !IsBase;
            fontSize.Minimum = percentage ? 10 : Math.Min(1, number);
            // A relative size may resolve above the usual point-entry range.
            // Switching units must still show its actual resolved point size.
            fontSize.Maximum = percentage ? 1000 : Math.Max(256, number);
            fontSize.Value = ShowsValue(VIEM_STYLE_PROPERTY_CHARACTER_SIZE) ? number : double.NaN;
            fontSize.IsEnabled = enabled;
            ToolTipService.SetToolTip(fontSizeUnit, IsBase
                ? "Base Paragraph uses points."
                : selected.Namespace == 1
                    ? "Percentages use the font size of the based-on paragraph style."
                    : "Percentages use the font size of the underlying paragraph text.");
        });
        fontSize.ValueChanged += (_, _) => {
            if (loading || !double.IsFinite(fontSize.Value)) return;
            bool percentage = fontSizeUnit.SelectedIndex == 1;
            Try(() => {
                double value = fontSize.Value;
                if (percentage && (IsBase || value < 10 || value > 1000 || value != Math.Truncate(value)))
                    throw new ArgumentException("Enter a whole-number percentage from 10 to 1000.");
                view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_SIZE,
                    percentage ? CoreView.Enum(VIEM_STYLE_VALUE_PERCENTAGE, checked((uint)value)) : CoreView.Number((float)value));
            });
        };
        fontSizeUnit.SelectionChanged += (_, _) => {
            if (loading || fontSizeUnit.SelectedIndex < 0) return;
            bool percentage = fontSizeUnit.SelectedIndex == 1;
            Try(() => {
                if (percentage && IsBase) throw new ArgumentException("Base Paragraph font size must use points.");
                float points = selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number;
                if (!percentage) {
                    view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_SIZE, CoreView.Number(points));
                    return;
                }
                var reference = selected.Namespace == 1
                    ? sheet.Styles.FirstOrDefault(style => style.Namespace == 1 && style.Id == selected.Parent)
                    : sheet.Styles.FirstOrDefault(style => (style.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
                float basis = reference?.Value(VIEM_STYLE_PROPERTY_CHARACTER_SIZE).number ?? 0;
                if (!float.IsFinite(basis) || basis <= 0) throw new InvalidOperationException("The inherited font size is unavailable.");
                uint amount = (uint)Math.Clamp(Math.Round(points / (double)basis * 100, MidpointRounding.AwayFromZero), 10, 1000);
                view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_SIZE,
                    CoreView.Enum(VIEM_STYLE_VALUE_PERCENTAGE, amount));
            });
        };
    }

#if DEBUG
    internal NumberBox FontSizeControl => fontSize;
    internal ComboBox FontSizeUnitControl => fontSizeUnit;
#endif
}
