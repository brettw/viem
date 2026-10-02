using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Rendering;
using Windows.UI;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    /// A real divider row: a disabled, hit-test-invisible ComboBoxItem drawn
    /// as a thin line rather than text. ComboBoxItem recognizes an item that
    /// is already a ComboBoxItem and uses it as its own container, so it
    /// keeps the platform's default chrome apart from the properties set
    /// here, and disabled containers are skipped by mouse and keyboard
    /// selection like any other disabled item. A new instance is required
    /// each call: a ComboBoxItem is a UIElement and can only sit in one
    /// ItemsSource/visual tree at a time.
    private static ComboBoxItem FontFamilySeparatorItem() => new() {
        IsEnabled = false, IsHitTestVisible = false, IsTabStop = false, Height = 9, Padding = new(0),
        Content = new Border { Height = 1, VerticalAlignment = VerticalAlignment.Center, Background = new SolidColorBrush(Color.FromArgb(96, 128, 128, 128)) },
    };
    private static object[] FontFamilyItems(string[] installed) => new object[] {
        FontCatalog.DisplayFamily(FontCatalog.SystemDefaultFamily), FontCatalog.DisplayFamily(FontCatalog.SystemMonospaceFamily), FontFamilySeparatorItem(),
    }.Concat(installed).ToArray();
    private readonly ComboBox fontFamily = new() { IsEditable = true, Width = 205, ItemsSource = FontFamilyItems(FontCatalog.Families) };
    private readonly ComboBox fontVariant = new() { Width = 140 };
    private string[]? listedFamilies;
    private FontFace[]? listedFaces;
    private FontVariationInfo? listedVariations;
    private FontFace? CurrentFace => FontCatalog.Named(sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES))) ?? FontCatalog.Current(sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)),
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
        Property(row, "Font family", VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES, Inline(fontFamily, fallback, fontVariant), caption: false);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(fontFamily, "Font family");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(fontVariant, "Variant");
        BuildFontSize(row);
        character.Children.Add(axisRows);
        refreshFields.Add(() => {
            string stored = sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES));
            string display = ShowsValue(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES) ? FontCatalog.DisplayFamily(stored) : "";
            // Include unavailable stored names in the native list. A Text-only
            // value set before ComboBox templating can appear blank. The two
            // portable system entries are already pinned at the top below.
            var installed = display.Length != 0 && FontCatalog.StorageFamily(display) == null
                && !FontCatalog.Families.Contains(display, StringComparer.OrdinalIgnoreCase)
                ? FontCatalog.Families.Append(display).OrderBy(f => f, StringComparer.CurrentCultureIgnoreCase).ToArray() : FontCatalog.Families;
            if (listedFamilies == null || !listedFamilies.SequenceEqual(installed)) {
                fontFamily.ItemsSource = FontFamilyItems(installed);
                listedFamilies = installed;
            }
            fontFamily.SelectedItem = fontFamily.Items.Cast<object>().OfType<string>().FirstOrDefault(f => string.Equals(f, display, StringComparison.OrdinalIgnoreCase));
            fontFamily.Text = display;
            var faces = FontCatalog.Faces(stored);
            var face = CurrentFace;
            var info = FontVariations.For(face);
            if (!ReferenceEquals(listedFaces, faces) || !ReferenceEquals(listedVariations, info)) {
                fontVariant.ItemsSource = info.Axes.Length == 0 ? faces
                    : faces.Cast<object>().Concat(info.Instances).Append("Custom").ToArray();
                listedFaces = faces; listedVariations = info;
            }
            if (info.Axes.Length == 0) { fontVariant.SelectedItem = face; }
            else {
                var values = BaseAxisValues(info);
                fontVariant.SelectedItem = (object?)info.Instances.FirstOrDefault(i => info.Axes.All(a => Math.Abs(i.Values[a.Tag] - values.GetValueOrDefault(a.Tag, a.Default)) < .001f)) ?? "Custom";
            }
            fontVariant.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS) && ShowsValue(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES) && faces.Length > 0;
            RefreshAxisControls(info);
        });
        fontFamily.SelectionChanged += (_, _) => { if (!loading && fontFamily.SelectedItem is string name) Try(() => SetFamily(name)); };
        fontFamily.LostFocus += (_, _) => { if (!loading && fontFamily.IsEnabled) Try(() => SetFamily(fontFamily.Text)); };
        fontVariant.SelectionChanged += (_, _) => { if (loading) return; if (fontVariant.SelectedItem is FontFace face) Try(() => SetFace(face)); else if (fontVariant.SelectedItem is FontInstance instance) Try(() => SetAxes(instance.Values)); };
    }
    private readonly StackPanel axisRows = new() { Spacing = 8 };
    private readonly Dictionary<string, (Slider Slider, TextBlock Value)> axisControls = new();
    private string axisControlIdentity = "";
    private Dictionary<string, float> CurrentAxisValues => FontVariations.Decode(sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_AXES)));
    private Dictionary<string, float> BaseAxisValues(FontVariationInfo info)
    {
        var values = FontVariations.Effective(info, CurrentAxisValues,
            selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value, false, 0);
        foreach (var (tag, value) in CurrentAxisValues) values[tag] = value;
        return values;
    }
    private void SetAxes(Dictionary<string, float> values) => view.EditStyleFont(selected, sheet.StringList(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)), CurrentFace, values);
    private void RefreshAxisControls(FontVariationInfo info)
    {
        string identity = CurrentFace?.Name ?? "";
        if (identity != axisControlIdentity) {
            axisControlIdentity = identity; axisRows.Children.Clear(); axisControls.Clear();
            var visible = info.Axes.Where(a => !a.Hidden && a.Maximum > a.Minimum).ToArray();
            for (int i = 0; i < visible.Length; i += 2) {
                var row = Row(axisRows);
                foreach (var axis in visible.Skip(i).Take(2)) {
                    var column = new StackPanel { Width = 280, Spacing = 2 };
                    var value = new TextBlock { FontSize = 11 };
                    var slider = new Slider { Minimum = axis.Minimum, Maximum = axis.Maximum, StepFrequency = 1, Width = 275 };
                    Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(slider, axis.Name);
                    column.Children.Add(value); column.Children.Add(slider); row.Children.Add(column);
                    axisControls.Add(axis.Tag, (slider, value));
                    bool gesture = false;
                    void Begin() { if (loading || gesture) return; gesture = true; BeginThemeHistoryGroup(); }
                    void End() { if (!gesture) return; gesture = false; EndThemeHistoryGroup(); }
                    slider.AddHandler(UIElement.PointerPressedEvent, new Microsoft.UI.Xaml.Input.PointerEventHandler((_, _) => Begin()), true);
                    slider.AddHandler(UIElement.PointerReleasedEvent, new Microsoft.UI.Xaml.Input.PointerEventHandler((_, _) => End()), true);
                    slider.AddHandler(UIElement.PointerCaptureLostEvent, new Microsoft.UI.Xaml.Input.PointerEventHandler((_, _) => End()), true);
                    slider.KeyDown += (_, _) => Begin(); slider.KeyUp += (_, _) => End(); slider.LostFocus += (_, _) => End(); slider.Unloaded += (_, _) => End();
                    slider.ValueChanged += (_, _) => {
                        if (loading || !slider.IsEnabled || identity != axisControlIdentity) return;
                        var values = BaseAxisValues(info);
                        values[axis.Tag] = (float)Math.Clamp(Math.Round(slider.Value, MidpointRounding.AwayFromZero), axis.Minimum, axis.Maximum);
                        Try(() => SetAxes(values));
                    };
                }
            }
        }
        var saved = BaseAxisValues(info);
        foreach (var axis in info.Axes) if (axisControls.TryGetValue(axis.Tag, out var control)) {
            float value = Math.Clamp(saved.GetValueOrDefault(axis.Tag, axis.Default), axis.Minimum, axis.Maximum);
            control.Slider.Value = value;
            control.Slider.IsEnabled = selected.Has(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS) && ShowsValue(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES);
            var effective = FontVariations.Effective(info, saved, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value + (selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_BOLD).enum_value != 0 ? 300 : 0), selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_BOLD).enum_value != 0, selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_SLANT).enum_value);
            control.Value.Text = axis.Name + " " + value.ToString("0") + (Math.Abs(effective[axis.Tag] - value) > .001f ? " (effective " + effective[axis.Tag].ToString("0") + ")" : "");
        }
        axisRows.Visibility = axisControls.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
    }
    private void SetFamily(string proposed)
    {
        string value = proposed.Trim(), chosen = sheet.String(selected.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES));
        if (value.Length == 0) return;
        if (selected.Declares(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)
            && (string.Equals(value, chosen, StringComparison.OrdinalIgnoreCase) || string.Equals(value, FontCatalog.DisplayFamily(chosen), StringComparison.OrdinalIgnoreCase))) return;
        // The two portable system entries store their generic request token
        // rather than a concrete resolved face, so the same declaration
        // renders correctly on macOS too.
        if (FontCatalog.StorageFamily(value) is { } portable) { view.EditStyleFont(selected, ReplacePrimary(portable), null); return; }
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
    internal Button VisitParent => visitParent;
    internal Button VisitNext => visitNext;
    internal ComboBox ParentPicker => parent;
    internal ComboBox NextPicker => next;
    internal Microsoft.UI.Xaml.FrameworkElement CharacterPanel => character;
    internal Microsoft.UI.Xaml.FrameworkElement ParagraphPanel => paragraph;
#endif
}
