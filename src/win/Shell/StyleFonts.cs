using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    private readonly ComboBox fontFamily = new() { IsEditable = true, Width = 205, ItemsSource = FontCatalog.Families };
    private readonly ComboBox fontVariant = new() { Width = 140 };
    private FontFace? CurrentFace => FontCatalog.Current(sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)),
        selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SLANT).enum_value);
    private void BuildFontRow()
    {
        var row = Row(character);
        var fallback = new Button { Content = "…", Width = 28, MinWidth = 0, Padding = new(0) };
        var families = new TextBox { AcceptsReturn = true, Width = 300, MinHeight = 100, Header = "Font families in fallback order" };
        var apply = new Button { Content = "Apply", HorizontalAlignment = Microsoft.UI.Xaml.HorizontalAlignment.Right };
        var content = new StackPanel { Spacing = 8 }; content.Children.Add(families); content.Children.Add(apply);
        var flyout = new Flyout { Content = content }; fallback.Flyout = flyout;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(fallback, "Edit fallback fonts"); ToolTipService.SetToolTip(fallback, "Edit fallback fonts");
        flyout.Opened += (_, _) => families.Text = string.Join(Environment.NewLine, sheet.StringList(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)));
        apply.Click += (_, _) => {
            var values = families.Text.Split(['\r', '\n'], StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries);
            if (values.Length == 0) return;
            if (!values.SequenceEqual(sheet.StringList(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)))) Try(() => view.EditStyleFont(selected, values, null));
            flyout.Hide();
        };
        Property(row, "Font family", VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES, Inline(fontFamily, fallback), caption: false);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(fontFamily, "Font family");
        Property(row, "Variant", VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT, fontVariant, caption: false);
        BuildFontSize(row);
        refreshFields.Add(() => {
            string stored = sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES));
            string display = ShowsValue(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES) ? FontCatalog.DisplayFamily(stored) : "";
            // Include portable or unavailable stored names in the native list.
            // A Text-only value set before ComboBox templating can appear blank.
            var names = display.Length != 0 && !FontCatalog.Families.Contains(display, StringComparer.OrdinalIgnoreCase)
                ? FontCatalog.Families.Append(display).OrderBy(f => f, StringComparer.CurrentCultureIgnoreCase).ToArray() : FontCatalog.Families;
            fontFamily.ItemsSource = names;
            fontFamily.SelectedItem = names.FirstOrDefault(f => string.Equals(f, display, StringComparison.OrdinalIgnoreCase));
            fontFamily.Text = display;
            fontVariant.ItemsSource = FontCatalog.Faces(stored);
            fontVariant.SelectedItem = ShowsValue(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT) ? CurrentFace : null;
            fontVariant.IsEnabled = ShowsValue(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT) && ((FontFace[])fontVariant.ItemsSource).Length > 0;
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
    internal Microsoft.UI.Xaml.Controls.Primitives.ToggleButton CharacterTab => characterTab;
    internal string Error => error.Text;
    internal Button RestoreDefaults => restoreDefaults;
    internal Button VisitParent => visitParent;
    internal Button VisitNext => visitNext;
    internal ComboBox ParentPicker => parent;
    internal ComboBox NextPicker => next;
    internal Microsoft.UI.Xaml.FrameworkElement CharacterPanel => character;
    internal Microsoft.UI.Xaml.FrameworkElement ParagraphPanel => paragraph;
#endif
}
