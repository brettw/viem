using System.Numerics;
using System.Text;
using System.Text.Json;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Text;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Windows.Foundation;
using Windows.UI;
using Windows.UI.Text;
using static Viem.Windows.Interop.Abi;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Rendering;

internal sealed unsafe partial class DirectWriteProvider
{
    private sealed record MarkerFont(string Family, FontStretch Stretch, float Size, ushort Weight, FontStyle Slant, float Spacing, string Language, string Features)
    {
        public static MarkerFont From(ViemResolvedTextStyleV1 style, float scale)
        {
            string features = "{}";
            if (style.feature_count != 0)
            {
                var values = new Dictionary<string, uint>();
                for (ulong i = 0; i < style.feature_count; i++) values[Encoding.ASCII.GetString(new ReadOnlySpan<byte>(style.features[i].tag, 4))] = style.features[i].value;
                features = JsonSerializer.Serialize(values, FontFeaturesJsonContext.Default.Features);
            }
            var resolved = ResolveFont(style);
            return new(resolved.Family, resolved.Stretch, style.size * scale, (ushort)style.weight, DirectWriteProvider.Slant(style.slant), style.letter_spacing * scale,
                style.has_language != 0 ? Text(style.language) : "", features);
        }
    }

    // Markers inherit their contributor's font and paint. Only marker ink is
    // fitted into the existing slot; text, row widths and hit testing stay intact.
    public void DrawWhitespace(CanvasDrawingSession drawing, WhitespaceExport export, LayoutSnapshot snapshot, ViemViewportStateV1 viewport, Func<ulong, Color> foreground, Color fallback)
    {
        if (export.Markers.Length == 0) return;
        var style = export.Style;
        JsonElement? Value(string key) => style.TryGetProperty(key, out var value) && value.ValueKind != JsonValueKind.Null ? value : null;
        Color ColorValue(JsonElement color) => Color.FromArgb((byte)Math.Clamp(color.GetProperty("alpha").GetSingle() * 255, 0, 255), (byte)Math.Clamp(color.GetProperty("red").GetSingle() * 255, 0, 255), (byte)Math.Clamp(color.GetProperty("green").GetSingle() * 255, 0, 255), (byte)Math.Clamp(color.GetProperty("blue").GetSingle() * 255, 0, 255));
        var rows = snapshot.Rows.ToDictionary(r => r.row_index);
        var clusters = snapshot.Clusters.GroupBy(c => c.row_index).ToDictionary(g => g.Key, g => g.OrderBy(c => c.x).ToArray());
        var layouts = new Dictionary<(MarkerFont Font, string Text), CanvasTextLayout>();
        try
        {
            foreach (var marker in export.Markers)
            {
                if (marker.Width <= 0 || marker.Height <= 0 || !rows.TryGetValue(marker.Row, out var row)) continue;
                var slot = new Rect(marker.X - viewport.left, marker.Y - viewport.top, marker.Width, marker.Height);
                var cluster = default(ViemPositionedClusterV1);
                if (clusters.TryGetValue(marker.Row, out var contributors) && contributors.Length > 0)
                {
                    int lower = 0, upper = contributors.Length;
                    while (lower < upper) { int middle = (lower + upper) / 2; if (contributors[middle].x <= marker.X) lower = middle + 1; else upper = middle; }
                    cluster = contributors[Math.Max(0, lower - 1)];
                }
                var inherited = resources.TryGetValue(cluster.render_run.identifier, out var resource) ? resource.MarkerFont
                    : new MarkerFont("Segoe UI", FontStretch.Normal, Math.Max(1, row.ascent + row.descent), 400, FontStyle.Normal, 0, 0, "", "{}");
                string family = inherited.Family;
                var stretch = inherited.Stretch;
                if (Value("font_families") is JsonElement names)
                    foreach (var name in names.EnumerateArray())
                    {
                        if (FontCatalog.Resolve(name.GetString()!) is not { } resolved) continue;
                        family = resolved.Family; stretch = resolved.Stretch; break;
                    }
                ushort weight = Value("weight")?.GetUInt16() ?? inherited.Weight;
                if (Value("bold")?.GetBoolean() == true) weight = weight < 350 ? (ushort)400 : weight < 550 ? (ushort)700 : (ushort)900;
                else if (Value("bold")?.GetBoolean() == false && weight >= 600 && Value("weight") == null) weight = 400;
                var font = inherited with {
                    Family = family, Stretch = stretch, Size = Value("size") is JsonElement size ? size.GetSingle() * viewport.scale : inherited.Size, Weight = weight,
                    Slant = Value("slant")?.GetString() switch { "Upright" => FontStyle.Normal, "Italic" => FontStyle.Italic, "Oblique" => FontStyle.Oblique, _ => inherited.Slant },
                    Spacing = Value("letter_spacing") is JsonElement spacing ? spacing.GetSingle() * viewport.scale : inherited.Spacing,
                    Language = Value("language")?.GetString() ?? inherited.Language, Features = Value("open_type_features")?.GetRawText() ?? inherited.Features
                };
                if (!layouts.TryGetValue((font, marker.Text), out var layout))
                {
                    using var format = new CanvasTextFormat { FontFamily = font.Family, FontStretch = font.Stretch, FontSize = font.Size, FontWeight = new FontWeight { Weight = font.Weight }, FontStyle = font.Slant, WordWrapping = CanvasWordWrapping.NoWrap,
                        Direction = Value("direction")?.GetString() == "RightToLeft" ? CanvasTextDirection.RightToLeftThenTopToBottom : CanvasTextDirection.LeftToRightThenTopToBottom };
                    layout = new CanvasTextLayout(device, marker.Text, format, 1, 10000);
                    layouts.Add((font, marker.Text), layout);
                    layout.SetCharacterSpacing(0, marker.Text.Length, 0, font.Spacing, 0);
                    if (font.Language.Length > 0) layout.SetLocaleName(0, marker.Text.Length, font.Language);
                    layout.SetUnderline(0, marker.Text.Length, Value("underline")?.GetBoolean() == true);
                    layout.SetStrikethrough(0, marker.Text.Length, Value("strikethrough")?.GetBoolean() == true);
                    using var features = JsonDocument.Parse(font.Features); using var typography = new CanvasTypography();
                    foreach (var feature in features.RootElement.EnumerateObject())
                    { string tag = feature.Name; typography.AddFeature((CanvasTypographyFeatureName)((uint)tag[0] | ((uint)tag[1] << 8) | ((uint)tag[2] << 16) | ((uint)tag[3] << 24)), feature.Value.GetUInt32()); }
                    layout.SetTypography(0, marker.Text.Length, typography);
                }
                var bounds = layout.LayoutBounds; var metrics = layout.LineMetrics[0];
                float fit = (float)Math.Min(1, Math.Min(slot.Width / Math.Max(.001, bounds.Width), slot.Height / Math.Max(.001, bounds.Height)));
                float baseline = Math.Clamp(row.baseline - viewport.top, (float)slot.Y + metrics.Baseline * fit, (float)slot.Bottom - (metrics.Height - metrics.Baseline) * fit);
                using var clip = drawing.CreateLayer(1, slot);
                if (Value("background") is JsonElement background) drawing.FillRectangle(slot, ColorValue(background));
                var previous = drawing.Transform;
                try
                {
                    drawing.Transform = Matrix3x2.CreateScale(fit) * Matrix3x2.CreateTranslation((float)slot.X - (float)bounds.X * fit, baseline - metrics.Baseline * fit) * previous;
                    var color = Value("foreground") is JsonElement specified ? ColorValue(specified) : cluster.text_end > cluster.text_start ? foreground(cluster.text_start) : fallback;
                    drawing.DrawTextLayout(layout, 0, 0, color);
                }
                finally { drawing.Transform = previous; }
            }
        }
        finally { foreach (var layout in layouts.Values) layout.Dispose(); }
    }
}
