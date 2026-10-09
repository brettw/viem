using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal enum StyleIconKind { Paragraph, Character, Block, Builtin }

/// <summary>One style-type vocabulary for inspector pickers and toolbar selectors.</summary>
internal static class StyleIcons
{
    private const string Corners = "M2 2H6V3.3H3.3V6H2Z M10 2H14V6H12.7V3.3H10Z M2 10H3.3V12.7H6V14H2Z M12.7 10H14V14H10V12.7H12.7Z";

    internal static Canvas ScriptIcon(ButtonBase owner, bool raised)
    {
        var canvas = new Canvas { Width = 18, Height = 18 };
        var text = new TextBlock { Text = "x", FontFamily = new FontFamily("Times New Roman"), FontSize = 13 * .9, FontStyle = global::Windows.UI.Text.FontStyle.Italic };
        var number = new TextBlock { Text = "2", FontSize = 7 };
        foreach (var glyph in new[] { text, number })
            glyph.SetBinding(TextBlock.ForegroundProperty, new Microsoft.UI.Xaml.Data.Binding { Source = owner, Path = new PropertyPath("Foreground") });
        Canvas.SetLeft(text, 3); Canvas.SetTop(text, 3);
        Canvas.SetLeft(number, 10.5); Canvas.SetTop(number, raised ? 0 : 9);
        canvas.Children.Add(text); canvas.Children.Add(number);
        return canvas;
    }

    internal static StyleIconKind Kind(StyleDefinition? style, uint fallbackNamespace = 1)
    {
        if (style == null) return fallbackNamespace == 2 ? StyleIconKind.Character : StyleIconKind.Paragraph;
        if ((style.Native.flags & VIEM_STYLE_DEFINITION_INTERNAL_LIST) != 0 || style.Native.role == VIEM_STYLE_ROLE_LIST) return StyleIconKind.Paragraph;
        if ((style.Native.flags & VIEM_STYLE_DEFINITION_INTERNAL) != 0
            || style.Native.origin == VIEM_STYLE_ORIGIN_SYNTHETIC_READ_ONLY) return StyleIconKind.Builtin;
        return style.Namespace == 2 ? StyleIconKind.Character : style.Native.role == VIEM_STYLE_ROLE_PARAGRAPH ? StyleIconKind.Paragraph : StyleIconKind.Block;
    }

    internal static string TypeName(StyleIconKind kind) => kind switch { StyleIconKind.Paragraph => "Paragraph style", StyleIconKind.Character => "Character style", StyleIconKind.Block => "Block style", _ => "Built-in style" };

    internal static IconElement Icon(StyleIconKind kind)
    {
        IconElement icon = kind switch {
            StyleIconKind.Paragraph => new FontIcon { Glyph = "¶", FontFamily = new FontFamily("Times New Roman"), FontSize = 16 },
            StyleIconKind.Character => new FontIcon { Glyph = "a", FontFamily = new FontFamily("Times New Roman"), FontWeight = Microsoft.UI.Text.FontWeights.Bold, FontSize = 16 },
            StyleIconKind.Block => (PathIcon)XamlReader.Load("<PathIcon xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation' Width='16' Height='16' Data='" + Corners + "'/>"),
            _ => new FontIcon { Glyph = "\uE713", FontSize = 16 },
        };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(icon, TypeName(kind));
        return icon;
    }

    internal static StackPanel Label(string name, StyleIconKind kind)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
        row.Children.Add(Icon(kind));
        row.Children.Add(new TextBlock { Text = name, VerticalAlignment = VerticalAlignment.Center, TextTrimming = TextTrimming.CharacterEllipsis });
        return row;
    }

    internal sealed class PickerTemplates : DataTemplateSelector
    {
        private readonly Dictionary<StyleIconKind, DataTemplate> templates = Enum.GetValues<StyleIconKind>().ToDictionary(kind => kind, Template);
        private readonly DataTemplate plain = (DataTemplate)XamlReader.Load("<DataTemplate xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation'><TextBlock Text='{Binding}'/></DataTemplate>");
        private static DataTemplate Template(StyleIconKind kind)
        {
            string icon = kind switch {
                StyleIconKind.Paragraph => "<FontIcon Glyph='¶' FontFamily='Times New Roman' FontSize='16'/>",
                StyleIconKind.Character => "<FontIcon Glyph='a' FontFamily='Times New Roman' FontWeight='Bold' FontSize='16'/>",
                StyleIconKind.Block => "<PathIcon Width='16' Height='16' Data='" + Corners + "'/>",
                _ => "<FontIcon Glyph='&#xE713;' FontSize='16'/>"
            };
            return (DataTemplate)XamlReader.Load("<DataTemplate xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation'><StackPanel Orientation='Horizontal' Spacing='6'>" + icon + "<TextBlock Text='{Binding}' VerticalAlignment='Center'/></StackPanel></DataTemplate>");
        }
        protected override DataTemplate SelectTemplateCore(object item) => item switch {
            StyleDefinition style => templates[Kind(style)],
            "Default Paragraph" => templates[StyleIconKind.Character],
            "Same Style" => templates[StyleIconKind.Paragraph],
            _ => plain
        };
        protected override DataTemplate SelectTemplateCore(object item, DependencyObject container) => SelectTemplateCore(item);
    }
}
