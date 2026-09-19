using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    private readonly ComboBox fontFamily = new() { IsEditable = true, PlaceholderText = "System default", Width = 235, ItemsSource = FontCatalog.Families };
    private readonly ComboBox fontVariant = new() { Width = 165, PlaceholderText = "Custom / inherited" };
    private FontFace? CurrentFace => FontCatalog.Current(sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)),
        selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SLANT).enum_value);
    private void BuildFontRow()
    {
        var row = Row(character);
        Property(row, "Font family", VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES, fontFamily,
            () => SetFamily(string.IsNullOrWhiteSpace(fontFamily.Text) ? "Segoe UI" : fontFamily.Text));
        Property(row, "Variant", VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT, fontVariant, () => {
            if (fontVariant.SelectedItem is FontFace face) SetFace(face);
            else view.EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT,
                Core.CoreView.Enum(VIEM_STYLE_VALUE_UNSIGNED, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value));
        });
        Number(row, "Size", VIEM_STYLE_PROPERTY_CHARACTER_SIZE, 16, 1, 256);
        refreshFields.Add(() => {
            string stored = sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES));
            string display = FontCatalog.DisplayFamily(stored);
            fontFamily.SelectedItem = FontCatalog.Families.FirstOrDefault(f => string.Equals(f, display, StringComparison.OrdinalIgnoreCase));
            fontFamily.Text = display;
            fontVariant.ItemsSource = FontCatalog.Faces(stored);
            fontVariant.SelectedItem = selected.Declares(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT) ? CurrentFace : null;
            fontVariant.IsEnabled = selected.Declares(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT) && ((FontFace[])fontVariant.ItemsSource).Length > 0;
        });
        fontFamily.SelectionChanged += (_, _) => { if (!loading && fontFamily.SelectedItem is string name) Try(() => SetFamily(name)); };
        fontFamily.LostFocus += (_, _) => { if (!loading && fontFamily.IsEnabled) Try(() => SetFamily(fontFamily.Text)); };
        fontVariant.SelectionChanged += (_, _) => { if (!loading && fontVariant.SelectedItem is FontFace face) Try(() => SetFace(face)); };
    }
    private void SetFamily(string proposed)
    {
        string value = proposed.Trim(), chosen = sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES));
        if (value.Length == 0) return;
        if (selected.Declares(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)
            && (string.Equals(value, chosen, StringComparison.OrdinalIgnoreCase) || string.Equals(value, FontCatalog.DisplayFamily(chosen), StringComparison.OrdinalIgnoreCase))) return;
        if (FontCatalog.ForFamilyChange(value, CurrentFace) is { } face) SetFace(face);
        else view.EditStyleFont(selected, ReplacePrimary(value), null);
    }
    private string[] ReplacePrimary(string value) => new[] { value }.Concat(sheet.StringList(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)).Skip(1)).ToArray();
    private void SetFace(FontFace face) => view.EditStyleFont(selected, ReplacePrimary(face.Name), face);
#if DEBUG
    internal ComboBox FontFamilyControl => fontFamily;
    internal ComboBox FontVariantControl => fontVariant;
    internal Microsoft.UI.Xaml.FrameworkElement RootControl => root;
    internal ComboBox StylePicker => stylePicker;
    internal Microsoft.UI.Xaml.Controls.Primitives.ToggleButton ParagraphTab => paragraphTab;
    internal string Error => error.Text;
#endif
}
