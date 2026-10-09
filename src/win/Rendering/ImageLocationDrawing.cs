using System.Text.Json;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using Viem.Windows.Interop;
using Windows.Foundation;
using Windows.UI;
using Windows.UI.Text;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Rendering;

internal sealed unsafe partial class DirectWriteProvider
{
#if DEBUG
    internal float ImageLocationFontSize(ViemRenderRunHandleV1 handle) => resources.TryGetValue(handle.identifier, out var resource) ? resource.MarkerFont.Size : 0;
#endif
    public void DrawImageLocation(CanvasDrawingSession drawing, ViemRenderRunHandleV1 handle, string text, Rect bounds, Color color, uint paintFlags)
    {
        if (handle.owner != owner || handle.metrics_generation != Generation || !resources.TryGetValue(handle.identifier, out var resource)) return;
        if (text.Length > 4096) text = text[..(char.IsHighSurrogate(text[4095]) ? 4095 : 4096)] + "…";
        var font = resource.MarkerFont;
        var selected = FontCatalog.Match(font.Family, font.Face);
        var slant = font.Slant == FontStyle.Normal ? selected?.Slant ?? font.Slant : font.Slant;
        var face = selected != null && selected.Slant == slant && (FontVariations.For(selected).Axes.Length != 0 || selected.Weight == font.Weight)
            ? selected : FontCatalog.RenderingFace(font.Family, font.Weight, slant, font.Stretch, true);
        using var format = new CanvasTextFormat {
            FontFamily = FontCatalog.RenderingFamily(font.Family, font.Weight, slant, font.Stretch, face), FontStretch = font.Stretch,
            FontSize = font.Size, FontWeight = new FontWeight { Weight = font.Weight }, FontStyle = slant,
            WordWrapping = CanvasWordWrapping.Wrap, VerticalAlignment = CanvasVerticalAlignment.Center
        };
        using var layout = new CanvasTextLayout(device, text, format, (float)bounds.Width, (float)bounds.Height);
        layout.SetCharacterSpacing(0, text.Length, 0, font.Spacing, 0);
        layout.SetUnderline(0, text.Length, (paintFlags & VIEM_TEXT_PAINT_UNDERLINE) != 0);
        layout.SetStrikethrough(0, text.Length, (paintFlags & VIEM_TEXT_PAINT_STRIKETHROUGH) != 0);
        var axes = FontCatalog.NamedCoordinates(face, font.Face);
        foreach (var (tag, value) in FontVariations.Decode(font.Axes)) axes[tag] = value;
        if (axes.Count != 0)
        {
            axes = FontVariations.Effective(FontVariations.For(face), axes, font.Weight, false, (uint)font.Slant);
            FontVariations.Apply(layout, 0, text.Length, axes, face);
        }
        if (font.Language.Length > 0) layout.SetLocaleName(0, text.Length, font.Language);
        using var features = JsonDocument.Parse(font.Features);
        using var typography = new CanvasTypography();
        foreach (var feature in features.RootElement.EnumerateObject())
        {
            string tag = feature.Name;
            typography.AddFeature((CanvasTypographyFeatureName)((uint)tag[0] | (uint)tag[1] << 8 | (uint)tag[2] << 16 | (uint)tag[3] << 24), feature.Value.GetUInt32());
        }
        layout.SetTypography(0, text.Length, typography);
        drawing.DrawTextLayout(layout, (float)bounds.X, (float)bounds.Y, color);
    }
}
