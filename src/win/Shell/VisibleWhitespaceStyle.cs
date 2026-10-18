using System.Text;
using System.Text.Json.Nodes;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

// A native settings entry, never registered in the document/global stylesheet.
// Its distinct namespace cannot collide with an authored character style.
internal static class VisibleWhitespaceStyle
{
    internal static readonly StyleKey Key = new(3, "Visible whitespace");
    internal static readonly (uint Property, string Field, uint Kind)[] Fields = [
        (VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES, "font_families", VIEM_STYLE_VALUE_STRING_LIST),
        (VIEM_STYLE_PROPERTY_CHARACTER_FONT_FACE, "font_face", VIEM_STYLE_VALUE_STRING),
        (VIEM_STYLE_PROPERTY_CHARACTER_FONT_AXES, "font_axes", VIEM_STYLE_VALUE_STRING),
        (VIEM_STYLE_PROPERTY_CHARACTER_SIZE, "size", VIEM_STYLE_VALUE_FLOAT),
        (VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT, "weight", VIEM_STYLE_VALUE_UNSIGNED),
        (VIEM_STYLE_PROPERTY_CHARACTER_BOLD, "bold", VIEM_STYLE_VALUE_BOOLEAN),
        (VIEM_STYLE_PROPERTY_CHARACTER_SLANT, "slant", VIEM_STYLE_VALUE_FONT_SLANT),
        (VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND, "foreground", VIEM_STYLE_VALUE_COLOR),
        (VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND, "background", VIEM_STYLE_VALUE_COLOR),
        (VIEM_STYLE_PROPERTY_CHARACTER_UNDERLINE, "underline", VIEM_STYLE_VALUE_BOOLEAN),
        (VIEM_STYLE_PROPERTY_CHARACTER_STRIKETHROUGH, "strikethrough", VIEM_STYLE_VALUE_BOOLEAN),
        (VIEM_STYLE_PROPERTY_CHARACTER_SUPERSCRIPT, "superscript", VIEM_STYLE_VALUE_BOOLEAN),
        (VIEM_STYLE_PROPERTY_CHARACTER_SUBSCRIPT, "subscript", VIEM_STYLE_VALUE_BOOLEAN),
        (VIEM_STYLE_PROPERTY_CHARACTER_LANGUAGE, "language", VIEM_STYLE_VALUE_STRING),
        (VIEM_STYLE_PROPERTY_CHARACTER_DIRECTION, "direction", VIEM_STYLE_VALUE_WRITING_DIRECTION),
        (VIEM_STYLE_PROPERTY_CHARACTER_OPEN_TYPE_FEATURES, "open_type_features", VIEM_STYLE_VALUE_OPEN_TYPE_FEATURES),
        (VIEM_STYLE_PROPERTY_CHARACTER_LETTER_SPACING, "letter_spacing", VIEM_STYLE_VALUE_FLOAT)
    ];
    internal static string[] Slants => ["Upright", "Italic", "Oblique"];
    internal static string[] Directions => ["Natural", "LeftToRight", "RightToLeft"];

    internal static JsonObject RestoringDefaults(JsonObject current)
    {
        var result = (JsonObject)current.DeepClone();
        foreach (var (_, field, _) in Fields) result.Remove(field);
        foreach (var field in Preferences.DefaultVisibleWhitespaceStyle()) result[field.Key] = field.Value?.DeepClone();
        return result;
    }

    internal static StyleSheet Append(StyleSheet source, JsonObject style)
    {
        var strings = source.Strings.ToList(); var items = source.Items.ToList();
        ViemStyleStringRefV1 Text(string text) {
            var bytes = Encoding.UTF8.GetBytes(text);
            var reference = new ViemStyleStringRefV1 { offset = (ulong)strings.Count, length = (ulong)bytes.Length };
            strings.AddRange(bytes); return reference;
        }
        var basis = source.Styles.First(s => (s.Native.flags & VIEM_STYLE_DEFINITION_BASE_PARAGRAPH) != 0);
        var properties = new Dictionary<uint, ViemStylePropertyV1>();
        foreach (var (property, field, kind) in Fields)
        {
            var inherited = basis.Properties.GetValueOrDefault(property);
            var value = inherited.effective;
            bool declared = style[field] is { };
            if (style[field] is { } node)
            {
                value = new() { kind = kind };
                switch (kind)
                {
                    case VIEM_STYLE_VALUE_FLOAT: value.number = (float)node.GetValue<double>(); break;
                    case VIEM_STYLE_VALUE_UNSIGNED: value.enum_value = node.GetValue<uint>(); break;
                    case VIEM_STYLE_VALUE_BOOLEAN: value.enum_value = node.GetValue<bool>() ? 1u : 0u; break;
                    case VIEM_STYLE_VALUE_FONT_SLANT: value.enum_value = (uint)Math.Max(0, Array.IndexOf(Slants, node.GetValue<string>())); break;
                    case VIEM_STYLE_VALUE_WRITING_DIRECTION: value.enum_value = (uint)Math.Max(0, Array.IndexOf(Directions, node.GetValue<string>())); break;
                    case VIEM_STYLE_VALUE_COLOR:
                        value.color = new() { red = (float)node["red"]!.GetValue<double>(), green = (float)node["green"]!.GetValue<double>(),
                            blue = (float)node["blue"]!.GetValue<double>(), alpha = (float)node["alpha"]!.GetValue<double>() }; break;
                    case VIEM_STYLE_VALUE_STRING:
                        value.@string = Text(field == "font_axes" ? node.ToJsonString() : node.GetValue<string>()); break;
                    case VIEM_STYLE_VALUE_STRING_LIST:
                        value.first_item = (ulong)items.Count;
                        foreach (var family in node.AsArray()) items.Add(new() { kind = VIEM_STYLE_VALUE_ITEM_STRING, @string = Text(family!.GetValue<string>()) });
                        value.item_count = (ulong)items.Count - value.first_item; break;
                    case VIEM_STYLE_VALUE_OPEN_TYPE_FEATURES:
                        value.first_item = (ulong)items.Count;
                        foreach (var feature in node.AsObject()) items.Add(new() { kind = VIEM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE,
                            @string = Text(feature.Key), unsigned_value = feature.Value!.GetValue<uint>() });
                        value.item_count = (ulong)items.Count - value.first_item; break;
                }
            }
            properties[property] = new() { property = property, effective = value, declared = declared ? value : default,
                flags = (declared ? VIEM_STYLE_PROPERTY_DECLARED : 0) | (value.kind != VIEM_STYLE_VALUE_NONE ? VIEM_STYLE_PROPERTY_EFFECTIVE_PRESENT : 0),
                contributor_kind = declared ? VIEM_STYLE_CONTRIBUTOR_CHARACTER_STYLE : inherited.contributor_kind,
                contributor_namespace = declared ? Key.Namespace : inherited.contributor_namespace,
                contributor_style_id = declared ? Text(Key.Id) : inherited.contributor_style_id };
        }
        var definition = new StyleDefinition(new() { namespace_id = Key.Namespace, flags = VIEM_STYLE_DEFINITION_INTERNAL,
            origin = VIEM_STYLE_ORIGIN_GENERATED_CONFIGURATION, capabilities = VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS },
            Key.Id, Key.Id, "", "", properties);
        return new(source.Identity, [.. source.Styles, definition], strings.ToArray(), items.ToArray());
    }
}
