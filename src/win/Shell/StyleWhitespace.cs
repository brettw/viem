using System.Text;
using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Shell;

internal sealed partial class StyleWindow
{
    private JsonObject? whitespaceStyle, whitespaceCandidate;
    private ulong whitespaceRevision, whitespaceEditRevision;
    private StyleSheet? whitespaceBaseSheet, whitespaceInspectorSheet;
    private bool IsWhitespaceStyle => selected?.Key == VisibleWhitespaceStyle.Key;
    private static byte[] WhitespaceBytes(JsonObject style) => Encoding.UTF8.GetBytes(style.ToJsonString());

    internal void SelectVisibleWhitespace()
    {
        DismissColorPickers();
        CreateThemeSession(VIEM_FORMAT_CODE);
        DetachView(); followsCaret = false;
        RefreshWhitespaceSession(); Load(VisibleWhitespaceStyle.Key);
    }

    private void RefreshWhitespaceSession()
    {
        var current = preferences.VisibleWhitespaceStyle;
        if (JsonNode.DeepEquals(current, whitespaceStyle)) return;
        bool external = whitespaceStyle != null && !updating;
        whitespaceStyle = current; whitespaceRevision = checked(whitespaceRevision + 1);
        whitespaceBaseSheet = whitespaceInspectorSheet = null;
        if (!external) return;
        DismissColorPickers(commit: false);
        // An external marker edit invalidates only marker history. Ordinary
        // theme history still belongs to its original settings target.
        themeUndo.RemoveAll(edit => edit.Selection == VisibleWhitespaceStyle.Key);
        var retained = themeRedo.Where(edit => edit.Selection != VisibleWhitespaceStyle.Key).Reverse().ToArray();
        themeRedo.Clear(); foreach (var edit in retained) themeRedo.Push(edit);
        foreach (string family in familyHistory.Keys.ToArray())
        {
            var history = familyHistory[family];
            familyHistory[family] = history with {
                Undo = history.Undo.Where(edit => edit.Selection != VisibleWhitespaceStyle.Key).ToArray(),
                Redo = history.Redo.Where(edit => edit.Selection != VisibleWhitespaceStyle.Key).ToArray()
            };
        }
        if (themeHistoryGroup?.Selection == VisibleWhitespaceStyle.Key) { themeHistoryGroup = null; themeHistoryGroupDepth = 0; }
        RefreshThemeHistory();
        if (styleDocument?.State.format == VIEM_FORMAT_CODE && !loading && selected != null) Load(selected.Key);
    }

    private StyleSheet InspectorStyles(StyleSheet source)
    {
        if (view.Document.State.format != VIEM_FORMAT_CODE) return source;
        if (whitespaceStyle == null) RefreshWhitespaceSession();
        if (ReferenceEquals(source, whitespaceBaseSheet) && whitespaceInspectorSheet != null) return whitespaceInspectorSheet;
        whitespaceBaseSheet = source;
        return whitespaceInspectorSheet = VisibleWhitespaceStyle.Append(source, whitespaceStyle!);
    }

    private bool TryWhitespaceEdit(Action action, bool reload)
    {
        ulong expectedRevision = whitespaceRevision;
        try
        {
            preferences.RefreshVisibleWhitespaceStyle();
            if (expectedRevision != whitespaceRevision)
                throw new InvalidOperationException("Visible whitespace changed in another settings owner. Review the current values and repeat the edit.");
            if (reload) DismissColorPickers();
            var before = (JsonObject)whitespaceStyle!.DeepClone();
            whitespaceCandidate = (JsonObject)before.DeepClone(); whitespaceEditRevision = whitespaceRevision;
            updating = true; action();
            if (!JsonNode.DeepEquals(before, whitespaceCandidate))
            {
                preferences.SaveVisibleWhitespaceStyle(before, whitespaceCandidate);
                RecordThemeEdit(WhitespaceBytes(before), WhitespaceBytes(whitespaceCandidate), VisibleWhitespaceStyle.Key);
                RefreshWhitespaceSession();
            }
            if (reload) Load(VisibleWhitespaceStyle.Key); else RefreshCommittedColors();
            error.Text = ""; error.Visibility = Visibility.Collapsed;
            return true;
        }
        catch (Exception exception)
        {
            updating = false;
            try { preferences.RefreshVisibleWhitespaceStyle(); RefreshWhitespaceSession(); Load(VisibleWhitespaceStyle.Key); }
            catch (Exception recovery) { ShowStyleFailure(exception, recovery); return false; }
            ShowStyleFailure(exception); return false;
        }
        finally { whitespaceCandidate = null; updating = false; }
    }

    private void EditStyle(StyleDefinition style, uint operation, uint property, ViemStyleEditValueV1 value, ViemStyleEditGroupV1? group = null)
    {
        if (style.Key != VisibleWhitespaceStyle.Key) { view.EditStyle(style, operation, property, value, group); return; }
        if (whitespaceCandidate == null || whitespaceEditRevision != whitespaceRevision)
            throw new InvalidOperationException("The visible whitespace style changed. Repeat the edit.");
        var field = VisibleWhitespaceStyle.Fields.SingleOrDefault(field => field.Property == property);
        if (field.Field == null || operation is not (VIEM_STYLE_EDIT_SET_DECLARATION or VIEM_STYLE_EDIT_CLEAR_DECLARATION))
            throw new InvalidOperationException("Visible whitespace supports character declarations only.");
        if (operation == VIEM_STYLE_EDIT_CLEAR_DECLARATION)
        {
            whitespaceCandidate.Remove(field.Field);
            if (property == VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES)
                foreach (string associated in new[] { "font_face", "font_axes", "weight" }) whitespaceCandidate.Remove(associated);
            return;
        }
        if (value.kind != field.Kind) throw new InvalidOperationException("Visible whitespace font size uses points, and its properties must use supported character values.");
        JsonNode? candidate = value.kind switch {
            VIEM_STYLE_VALUE_FLOAT => JsonValue.Create(value.number),
            VIEM_STYLE_VALUE_UNSIGNED => JsonValue.Create(value.enum_value),
            VIEM_STYLE_VALUE_BOOLEAN => JsonValue.Create(value.enum_value != 0),
            VIEM_STYLE_VALUE_FONT_SLANT when value.enum_value < 3 => JsonValue.Create(VisibleWhitespaceStyle.Slants[value.enum_value]),
            VIEM_STYLE_VALUE_WRITING_DIRECTION when value.enum_value < 3 => JsonValue.Create(VisibleWhitespaceStyle.Directions[value.enum_value]),
            VIEM_STYLE_VALUE_COLOR => new JsonObject { ["red"] = value.color.red, ["green"] = value.color.green,
                ["blue"] = value.color.blue, ["alpha"] = value.color.alpha },
            _ => throw new InvalidOperationException("Unsupported visible whitespace character value.")
        };
        whitespaceCandidate[field.Field] = candidate;
        if (value.enum_value != 0 && property is VIEM_STYLE_PROPERTY_CHARACTER_SUPERSCRIPT or VIEM_STYLE_PROPERTY_CHARACTER_SUBSCRIPT)
            whitespaceCandidate[property == VIEM_STYLE_PROPERTY_CHARACTER_SUPERSCRIPT ? "subscript" : "superscript"] = false;
    }

    private void EditStyleFont(StyleDefinition style, string[] families, FontFace? face, Dictionary<string, float>? axes = null, string? subfamily = null)
    {
        if (style.Key != VisibleWhitespaceStyle.Key) { view.EditStyleFont(style, families, face, axes, subfamily); return; }
        if (whitespaceCandidate == null || whitespaceEditRevision != whitespaceRevision)
            throw new InvalidOperationException("The visible whitespace style changed. Repeat the edit.");
        string selectedFace = subfamily ?? face?.PortableStyle ?? "";
        if (face != null && families.Length > 0) families = new[] { face.PortableFamily }.Concat(families.Skip(1)).ToArray();
        if (axes == null) { axes = FontVariations.For(face).Defaults; if (face != null && axes.ContainsKey("wght")) axes["wght"] = face.Weight; }
        whitespaceCandidate["font_families"] = new JsonArray(families.Select(family => (JsonNode?)JsonValue.Create(family)).ToArray());
        whitespaceCandidate["font_face"] = selectedFace;
        var values = new JsonObject(); foreach (var (tag, number) in axes) values[tag] = number;
        whitespaceCandidate["font_axes"] = values;
        whitespaceCandidate["weight"] = (uint)Math.Clamp(Math.Round(axes.GetValueOrDefault("wght", face?.Weight ?? 400)), 1, 1000);
    }

    private void DeclareEffectiveWhitespace(uint property)
    {
        var effective = selected.Value(property);
        var value = CoreView.Enum(effective.kind, effective.enum_value);
        value.number = effective.number; value.color = effective.color;
        if (property == VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND && effective.kind == VIEM_STYLE_VALUE_NONE) value.kind = VIEM_STYLE_VALUE_COLOR;
        EditStyle(selected, VIEM_STYLE_EDIT_SET_DECLARATION, property, value);
    }

    private void RestoreWhitespaceDefaults() => TryWhitespaceEdit(() => whitespaceCandidate = VisibleWhitespaceStyle.RestoringDefaults(whitespaceCandidate!), reload: true);

    private bool ReplayWhitespaceHistory(ThemeStyleEdit edit, bool redo)
    {
        try
        {
            var expected = JsonNode.Parse(redo ? edit.Before : edit.After)!.AsObject();
            var replacement = JsonNode.Parse(redo ? edit.After : edit.Before)!.AsObject();
            preferences.SaveVisibleWhitespaceStyle(expected, replacement);
            RefreshWhitespaceSession(); Load(VisibleWhitespaceStyle.Key);
            return true;
        }
        catch (Exception exception)
        {
            updating = false;
            try { preferences.RefreshVisibleWhitespaceStyle(); RefreshWhitespaceSession(); }
            catch (Exception recovery) { ShowStyleFailure(exception, recovery); return false; }
            ShowStyleFailure(exception); return false;
        }
    }
#if DEBUG
    internal ulong WhitespaceSettingsRevision => whitespaceRevision;
#endif
}
