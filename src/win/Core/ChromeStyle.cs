using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using System.Text.Json;
using Viem.Windows.Interop;
using Viem.Windows.Rendering;
using Windows.Foundation;
using Windows.UI;
using Windows.UI.Text;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Core;

// Native chrome consumes the same resolved properties as document blocks.
// Retain values rather than references into a stylesheet's export arena.
internal readonly record struct ChromeEdges(float Left, float Top, float Right, float Bottom)
{
    internal float Horizontal => Left + Right;
    internal float Vertical => Top + Bottom;
}

internal sealed record ChromeStyle(string Family, float Size, ushort Weight, FontStyle Slant,
    string Face, string Axes, string Features, float Tracking, Color Foreground, Color Background,
    ChromeEdges Margin, ChromeEdges Padding, ChromeEdges Border,
    Color LeftBorder, Color TopBorder, Color RightBorder, Color BottomBorder, uint Alignment, uint BaseDirection)
{
    internal (string Family, float Size, ushort Weight, FontStyle Slant, string Face, string Axes, string Features, float Tracking) FontIdentity
        => (Family, Size, Weight, Slant, Face, Axes, Features, Tracking);
    internal static ChromeStyle Fallback(Color foreground, Color background, bool status = false) => new(
        "Segoe UI", 11, 400, FontStyle.Normal, "", "", "{}", 0, foreground, background,
        default, status ? new(10, 6, 10, 6) : new(4, 0, 4, 0),
        status ? new(0, 1, 0, 0) : new(0, 0, 1, 0), foreground, foreground, foreground, foreground, 2, 0);

    internal ChromeStyle Scaled(float scale)
    {
        if (!float.IsFinite(scale) || scale <= 0 || scale == 1) return this;
        ChromeEdges Edges(ChromeEdges value) => new(value.Left * scale, value.Top * scale, value.Right * scale, value.Bottom * scale);
        return this with { Size = Size * scale, Tracking = Tracking * scale,
            Margin = Edges(Margin), Padding = Edges(Padding), Border = Edges(Border) };
    }

    internal static ChromeStyle Resolve(StyleSheet sheet, string id, ChromeStyle fallback)
        => Resolve(sheet, sheet.Styles.FirstOrDefault(s => s.Namespace == 1 && s.Id == id), fallback);

    internal static ChromeStyle ResolveLineNumber(StyleSheet sheet, bool current, ChromeStyle fallback)
    {
        const uint required = VIEM_STYLE_DEFINITION_INTERNAL | VIEM_STYLE_DEFINITION_LINE_NUMBER;
        return Resolve(sheet, sheet.Styles.FirstOrDefault(style => style.Namespace == 1
            && (style.Native.flags & required) == required
            && ((style.Native.flags & VIEM_STYLE_DEFINITION_CURRENT_LINE_NUMBER) != 0) == current), fallback);
    }

    private static ChromeStyle Resolve(StyleSheet sheet, StyleDefinition? style, ChromeStyle fallback)
    {
        if (style == null) return fallback;
        float Number(uint property) => float.IsFinite(style.Value(property).number) ? style.Value(property).number : 0;
        float Nonnegative(uint property) => Math.Max(0, Number(property));
        Color Paint(uint property, Color missing) => style.Value(property).kind == VIEM_STYLE_VALUE_COLOR
            ? ConvertColor(style.Value(property).color) : missing;
        var foreground = style.UsesThemeForeground ? fallback.Foreground : Paint(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND, fallback.Foreground);
        Color Edge(uint property) => style.UsesTextColor(property) ? foreground : Paint(property, foreground);
        string face = sheet.String(style.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FACE));
        string family = sheet.StringList(style.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES))
            .FirstOrDefault(candidate => FontCatalog.Resolve(candidate, face) != null) ?? fallback.Family;
        uint weight = style.Value(VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT).enum_value;
        if (style.Value(VIEM_STYLE_PROPERTY_CHARACTER_BOLD).enum_value != 0) weight = Math.Min(1000, weight + 300);
        float size = Number(VIEM_STYLE_PROPERTY_CHARACTER_SIZE);
        var featureValue = style.Value(VIEM_STYLE_PROPERTY_CHARACTER_OPEN_TYPE_FEATURES);
        var features = sheet.Items.Skip(checked((int)featureValue.first_item)).Take(checked((int)featureValue.item_count))
            .Where(item => item.kind == VIEM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE)
            .ToDictionary(item => Abi.Text(sheet.Strings, item.@string), item => item.unsigned_value);
        return new(family is "System" or "system-ui" ? "Segoe UI" : family,
            size > 0 ? size : fallback.Size, (ushort)Math.Clamp(weight, 1u, 999u),
            style.Value(VIEM_STYLE_PROPERTY_CHARACTER_SLANT).enum_value switch { 1 => FontStyle.Italic, 2 => FontStyle.Oblique, _ => FontStyle.Normal },
            face, sheet.String(style.Value(VIEM_STYLE_PROPERTY_CHARACTER_FONT_AXES)),
            JsonSerializer.Serialize(features, FontFeaturesJsonContext.Default.Features),
            Number(VIEM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING), foreground,
            Paint(VIEM_STYLE_PROPERTY_BLOCK_BACKGROUND, Microsoft.UI.Colors.Transparent),
            new(Number(VIEM_STYLE_PROPERTY_BLOCK_MARGIN_LEFT), Number(VIEM_STYLE_PROPERTY_BLOCK_MARGIN_TOP), Number(VIEM_STYLE_PROPERTY_BLOCK_MARGIN_RIGHT), Number(VIEM_STYLE_PROPERTY_BLOCK_MARGIN_BOTTOM)),
            new(Nonnegative(VIEM_STYLE_PROPERTY_BLOCK_PADDING_LEFT), Nonnegative(VIEM_STYLE_PROPERTY_BLOCK_PADDING_TOP), Nonnegative(VIEM_STYLE_PROPERTY_BLOCK_PADDING_RIGHT), Nonnegative(VIEM_STYLE_PROPERTY_BLOCK_PADDING_BOTTOM)),
            new(Nonnegative(VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_WIDTH), Nonnegative(VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_WIDTH), Nonnegative(VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_WIDTH), Nonnegative(VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_WIDTH)),
            Edge(VIEM_STYLE_PROPERTY_BLOCK_BORDER_LEFT_COLOR), Edge(VIEM_STYLE_PROPERTY_BLOCK_BORDER_TOP_COLOR), Edge(VIEM_STYLE_PROPERTY_BLOCK_BORDER_RIGHT_COLOR), Edge(VIEM_STYLE_PROPERTY_BLOCK_BORDER_BOTTOM_COLOR),
            style.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_ALIGNMENT).enum_value,
            style.Value(VIEM_STYLE_PROPERTY_PARAGRAPH_BASE_DIRECTION).enum_value);
    }

    internal CanvasTextLayout Layout(CanvasDevice device, string text, float? size = null)
    {
        var resolved = FontCatalog.Resolve(Family, Face) ?? (Family: "Segoe UI", Stretch: FontStretch.Normal);
        var face = Face.Length > 0 ? FontCatalog.Match(Family, Face)
            : FontCatalog.RenderingFace(resolved.Family, Weight, Slant, resolved.Stretch, discoverSystem: Axes.Length > 2);
        using var format = new CanvasTextFormat {
            FontFamily = FontCatalog.RenderingFamily(resolved.Family, Weight, Slant, resolved.Stretch, face),
            FontStretch = resolved.Stretch, FontSize = size ?? Size, FontWeight = new() { Weight = Weight },
            FontStyle = Slant, WordWrapping = CanvasWordWrapping.NoWrap
        };
        var layout = new CanvasTextLayout(device, text, format, 1_000_000, 1_000_000);
        if (text.Length > 0) {
            var axes = FontCatalog.NamedCoordinates(face, Face);
            foreach (var entry in FontVariations.Decode(Axes)) axes[entry.Key] = entry.Value;
            FontVariations.Apply(layout, 0, text.Length,
                FontVariations.Effective(FontVariations.For(face), axes, Weight, false, Slant == FontStyle.Normal ? 0u : 1u), face);
            layout.SetCharacterSpacing(0, text.Length, 0, Tracking, 0);
            using var typography = new CanvasTypography();
            foreach (var feature in JsonSerializer.Deserialize(Features, FontFeaturesJsonContext.Default.Features) ?? []) {
                if (feature.Key.Length != 4 || feature.Key == "kern") continue;
                uint tag = (uint)feature.Key[0] | (uint)feature.Key[1] << 8 | (uint)feature.Key[2] << 16 | (uint)feature.Key[3] << 24;
                typography.AddFeature((CanvasTypographyFeatureName)tag, feature.Value);
            }
            typography.AddFeature((CanvasTypographyFeatureName)FontVariations.Tag("kern"), 1);
            layout.SetTypography(0, text.Length, typography);
        }
        return layout;
    }

    internal void DrawBox(CanvasDrawingSession drawing, Rect bounds)
    {
        if (Background.A > 0) drawing.FillRectangle(bounds, Background);
        if (Border.Left > 0) drawing.FillRectangle(new(bounds.X, bounds.Y, Math.Min(bounds.Width, Border.Left), bounds.Height), LeftBorder);
        if (Border.Right > 0) drawing.FillRectangle(new(Math.Max(bounds.X, bounds.Right - Border.Right), bounds.Y, Math.Min(bounds.Width, Border.Right), bounds.Height), RightBorder);
        if (Border.Top > 0) drawing.FillRectangle(new(bounds.X, bounds.Y, bounds.Width, Math.Min(bounds.Height, Border.Top)), TopBorder);
        if (Border.Bottom > 0) drawing.FillRectangle(new(bounds.X, Math.Max(bounds.Y, bounds.Bottom - Border.Bottom), bounds.Width, Math.Min(bounds.Height, Border.Bottom)), BottomBorder);
    }

    private static Color ConvertColor(ViemRgbaV1 value)
    {
        static byte Component(float v) => (byte)Math.Round(Math.Clamp(v, 0, 1) * 255);
        return Color.FromArgb(Component(value.alpha), Component(value.red), Component(value.green), Component(value.blue));
    }
}
