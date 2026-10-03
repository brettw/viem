using Viem.Windows.Interop;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal readonly record struct StyleKey(uint Namespace, string Id);

internal sealed record StyleDefinition(ViemStyleDefinitionV1 Native, string Id, string Name, string Parent, string Next, Dictionary<uint, ViemStylePropertyV1> Properties)
{
    public override string ToString() => Name;
    public uint Namespace => Native.namespace_id;
    public StyleKey Key => new(Namespace, Id);
    public bool Has(uint capability) => (Native.capabilities & capability) != 0;
    public bool Declares(uint property) => Properties.TryGetValue(property, out var p) && (p.flags & VIEM_STYLE_PROPERTY_DECLARED) != 0;
    public ViemStyleValueV1 Value(uint property) => Properties.TryGetValue(property, out var p) ? p.effective : default;
    public bool UsesThemeForeground => !Properties.TryGetValue(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND, out var property)
        || (!Declares(VIEM_STYLE_PROPERTY_CHARACTER_FOREGROUND) && property.contributor_style_id.length == 0
            && property.contributor_kind == VIEM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY);
}
internal sealed record StyleSheet(ViemStyleSheetIdentityV1 Identity, StyleDefinition[] Styles, byte[] Strings, ViemStyleValueItemV1[] Items)
{
    public string[] StringList(ViemStyleValueV1 value) => value.kind == VIEM_STYLE_VALUE_STRING_LIST
        ? Items.Skip(checked((int)value.first_item)).Take(checked((int)value.item_count)).Select(i => Text(Strings, i.@string)).ToArray() : [];
    public string String(ViemStyleValueV1 value) => value.kind == VIEM_STYLE_VALUE_STRING_LIST && value.item_count > 0
        ? Text(Strings, Items[checked((int)value.first_item)].@string) : Text(Strings, value.@string);
}

internal sealed unsafe partial class CoreView
{
    private StyleSheet? cachedStyles;
    // Opening and idle caret following share the macOS named-style policy.
    // Query only retained core assignments/runs, never typography or syntax work.
    public StyleKey? CurrentStyleEditorKey(StyleSheet sheet)
    {
        var state = Document.State;
        var info = New<ViemSelectedStylesInfoV1>();
        uint status = viem_core_view_selected_styles_export(Document.Handle, Id, state.document_revision, &info, null, 0);
        if (status != VIEM_STATUS_BUFFER_TOO_SMALL) Check(status, "Read selected styles");
        var bytes = new byte[checked((int)(info.paragraph_id_bytes + info.character_id_bytes))];
        fixed (byte* p = bytes) Check(viem_core_view_selected_styles_export(Document.Handle, Id, state.document_revision, &info, p, (ulong)bytes.Length), "Read selected styles");
        if (info.document_id != state.document_id || info.document_revision != state.document_revision
            || info.style_sheet_revision != sheet.Identity.style_sheet_revision
            || (!UsesGlobalStyles && (sheet.Identity.document_id != state.document_id || sheet.Identity.document_revision != state.document_revision))) return null;
        int split = checked((int)info.paragraph_id_bytes);
        string paragraph = System.Text.Encoding.UTF8.GetString(bytes.AsSpan(0, split));
        string character = System.Text.Encoding.UTF8.GetString(bytes.AsSpan(split));
        if ((info.flags & VIEM_SELECTED_STYLE_CHARACTER_MIXED) == 0 && character.Length > 0
            && sheet.Styles.Any(s => s.Id == character && s.Namespace == 2)) return new(2, character);
        if ((info.flags & VIEM_SELECTED_STYLE_PARAGRAPH_MIXED) == 0 && paragraph.Length > 0
            && sheet.Styles.Any(s => s.Id == paragraph && s.Namespace == 1)) return new(1, paragraph);
        return null;
    }
    public bool UsesGlobalStyles => Document.State.format == VIEM_FORMAT_CODE;
    public ViemLogicalSelectionIdentityV1 LogicalSelection()
    { var value = New<ViemLogicalSelectionIdentityV1>(); Check(viem_core_view_list_selection(Document.Handle, Id, &value), "Read formatting selection"); return value; }
    public ViemSemanticStylePresentationV1 SemanticStyle(uint style)
    { var value = New<ViemSemanticStylePresentationV1>(); Check(viem_core_view_semantic_style_presentation(Document.Handle, Id, style, &value), "Read formatting"); return value; }
    public void ToggleSemantic(uint style)
    {
        var presentation = SemanticStyle(style);
        Apply(o => { var request = New<ViemSetSemanticStyleV1>(); request.style = style; request.enabled = presentation.state == VIEM_SEMANTIC_STYLE_STATE_ON ? 0u : 1u; request.expected_selection = presentation.selection;
            return viem_core_view_set_semantic_style(Document.Handle, Id, &request, o); });
    }
    public bool CanFormatStrikethrough => HasFormattingSelection
        && Document.State.format is VIEM_FORMAT_MARKDOWN or VIEM_FORMAT_MARKDOWN_SOURCE
        && (LogicalSelection().kind is VIEM_LOGICAL_SELECTION_KIND_CHARACTER or VIEM_LOGICAL_SELECTION_KIND_LINE
            || Presentation.mode is VIEM_MODE_INSERT or VIEM_MODE_REPLACE);
    public uint StrikethroughState()
    { uint state = 0; Check(viem_core_view_strikethrough_state(Document.Handle, Id, &state), "Read strikethrough"); return state; }
    public void ToggleStrikethrough()
    {
        var selection = LogicalSelection();
        byte enabled = StrikethroughState() != VIEM_SEMANTIC_STYLE_STATE_ON ? (byte)1 : (byte)0;
        Apply(o => { var expected = selection; return viem_core_view_set_strikethrough(Document.Handle, Id, &expected, enabled, o); });
    }
    public static ViemStyleEditValueV1 Number(float number) { var v = New<ViemStyleEditValueV1>(); v.kind = VIEM_STYLE_VALUE_FLOAT; v.number = number; return v; }
    public static ViemStyleEditValueV1 Enum(uint kind, uint value) { var v = New<ViemStyleEditValueV1>(); v.kind = kind; v.enum_value = value; return v; }
    public void SetParagraph(uint level) => Apply(o => { var r = New<ViemSetParagraphStyleV1>(); r.level = level; r.expected_selection = LogicalSelection(); return viem_core_view_set_paragraph_style(Document.Handle, Id, &r, o); });
    public void SetList(uint style) => Apply(o => { var r = New<ViemSetListStyleV1>(); r.style = style; r.expected_selection = LogicalSelection(); return viem_core_view_set_list_style(Document.Handle, Id, &r, o); });
    public void IndentList(bool unindent) => Apply(o => { var r = New<ViemListIndentV1>(); r.unindent = unindent ? 1u : 0u; r.expected_selection = LogicalSelection(); return viem_core_view_indent_list(Document.Handle, Id, &r, o); });
    public uint ListCapabilities() { var selection = LogicalSelection(); uint caps = 0; Check(viem_core_view_list_indent_capabilities(Document.Handle, Id, &selection, &caps), "Read list actions"); return caps; }
    public void SetMarkdownSource(bool source) => Send((c, o, e) => { var state = Document.State; var r = New<ViemSetMarkdownSourceV1>(); r.document_id = state.document_id; r.document_revision = state.document_revision; r.source = source ? 1u : 0u; return viem_core_view_set_markdown_source_with_effects(Document.Handle, Id, &r, o, e); });
    public void SetEncoding(uint encoding) => Send((c, o, e) => { var state = Document.State; var r = New<ViemSetEncodingV1>(); r.document_id = state.document_id; r.document_revision = state.document_revision; r.encoding = encoding; return viem_core_view_set_encoding_with_effects(Document.Handle, Id, &r, o, e); });
    public void FileFormat(uint format) => Apply(o => { var state = Document.State; var r = New<ViemSetFileFormatV1>(); r.document_id = state.document_id; r.document_revision = state.document_revision; r.file_format = format; return viem_core_view_set_file_format(Document.Handle, Id, &r, o); });
    public ViemStyleSheetIdentityV1 StyleIdentity()
    {
        var identity = New<ViemStyleSheetIdentityV1>();
        Check(viem_core_style_sheet_identity(UsesGlobalStyles ? 0 : Document.Handle, &identity), "Read style identity");
        return identity;
    }
#if DEBUG
    internal int StyleExports { get; private set; }
#endif
    public StyleSheet Styles(ViemStyleSheetIdentityV1? expected = null)
    {
        // Toolbar callers already queried the exact identity with selected styles.
        // Avoid exporting and allocating the catalogue on every cursor move.
        if (expected is { } identityKey && cachedStyles is { } cached && cached.Identity.Equals(identityKey)) return cached;
        var actual = StyleIdentity();
        if (cachedStyles is { } retained && retained.Identity.Equals(actual)) return retained;
#if DEBUG
        StyleExports++;
#endif
        bool global = UsesGlobalStyles;
        var info = New<ViemStyleSheetInfoV1>(); Check(global ? viem_code_style_sheet_info(&info) : viem_core_style_sheet_info(Document.Handle, &info), "Read styles");
        if (cachedStyles is { } current && current.Identity.Equals(info.identity)) return current;
        var definitions = new ViemStyleDefinitionV1[checked((int)info.definition_count)];
        var properties = new ViemStylePropertyV1[checked((int)info.property_count)];
        var items = new ViemStyleValueItemV1[checked((int)info.value_item_count)];
        var dependencies = new ViemStyleDependencyV1[checked((int)info.dependency_count)];
        var strings = new byte[checked((int)info.string_bytes)]; var identity = info.identity;
        fixed (ViemStyleDefinitionV1* d = definitions) fixed (ViemStylePropertyV1* p = properties) fixed (ViemStyleValueItemV1* v = items) fixed (ViemStyleDependencyV1* r = dependencies) fixed (byte* s = strings)
            Check(global ? viem_code_copy_style_sheet(&identity, d, (ulong)definitions.Length, p, (ulong)properties.Length, v, (ulong)items.Length, r, (ulong)dependencies.Length, s, (ulong)strings.Length, &info)
                : viem_core_copy_style_sheet(Document.Handle, &identity, d, (ulong)definitions.Length, p, (ulong)properties.Length, v, (ulong)items.Length, r, (ulong)dependencies.Length, s, (ulong)strings.Length, &info), "Copy styles");
        return cachedStyles = new(identity, definitions.Select(d => new StyleDefinition(d, Abi.Text(strings, d.stable_id), Abi.Text(strings, d.display_name), Abi.Text(strings, d.parent_id), Abi.Text(strings, d.next_style_id),
            properties.Skip(checked((int)d.first_property)).Take(checked((int)d.property_count)).ToDictionary(p => p.property))).ToArray(), strings, items);
    }
    public void EditStyle(StyleDefinition style, uint operation, uint property, ViemStyleEditValueV1 value, ViemStyleEditGroupV1? group = null)
    {
        // Clear operations still require a sized, empty ABI value, including
        // relationship clears whose UI previously supplied a string value.
        if (operation is VIEM_STYLE_EDIT_CLEAR_DECLARATION or VIEM_STYLE_EDIT_CLEAR_PARENT or VIEM_STYLE_EDIT_CLEAR_NEXT_STYLE) value = New<ViemStyleEditValueV1>();
        using var arena = new NativeArena(); var request = New<ViemStyleEditV1>(); request.identity = StyleIdentity();
        request.namespace_id = style.Namespace; request.style_id = arena.Utf8(style.Id); request.operation = operation; request.property = property; request.value = value;
        if (UsesGlobalStyles) { var info = New<ViemStyleSheetInfoV1>(); Check(viem_code_edit_style(&request, &info), "Edit Code style"); Document.NotifyChanged(); Refresh(); }
        else { var copy = request; Apply(o => { var r = copy; if (group is { } token) return viem_core_view_edit_style_in_group(Document.Handle, Id, &token, &r, o); return viem_core_view_edit_style(Document.Handle, Id, &r, o); }); }
    }
    public ViemStyleEditGroupV1? BeginStyleEditGroup()
    {
        if (UsesGlobalStyles) return null;
        var identity = StyleIdentity(); var token = New<ViemStyleEditGroupV1>();
        Check(viem_core_view_begin_style_edit_group(Document.Handle, Id, &identity, &token), "Begin style change");
        return token;
    }
    public void EndStyleEditGroup(ViemStyleEditGroupV1? group)
    {
        if (Id == 0 || group is not { } token) return;
        uint status = viem_core_view_end_style_edit_group(Document.Handle, Id, &token);
        // Ordinary editor commands (including undo) already finalize the group.
        if (status != VIEM_STATUS_INVALID_STYLE_EDIT_GROUP) Check(status, "End style change");
    }
    public void EditStyleFont(StyleDefinition style, string[] families, FontFace? face, Dictionary<string, float>? axes = null, string? subfamily = null)
    {
        string selectedFace = subfamily ?? face?.PortableStyle ?? "";
        if (face != null && families.Length > 0) families = new[] { face.PortableFamily }.Concat(families.Skip(1)).ToArray();
        if (axes == null) { axes = FontVariations.For(face).Defaults; if (face != null && axes.ContainsKey("wght")) axes["wght"] = face.Weight; }
        if (axes.Count > 64 || axes.Any(v => v.Key.Length != 4 || v.Key.Any(c => c < 32 || c > 126) || !float.IsFinite(v.Value)))
            throw new ArgumentException("Font axis tags must be four ASCII characters with finite coordinates.", nameof(axes));
        using var arena = new NativeArena();
        var items = families.Select(f => { var item = New<ViemStyleEditValueItemV1>(); item.kind = VIEM_STYLE_VALUE_ITEM_STRING; item.text = arena.Utf8(f); return item; }).ToArray();
        var value = New<ViemStyleEditValueV1>(); value.kind = VIEM_STYLE_VALUE_STRING_LIST; value.items = arena.Copy<ViemStyleEditValueItemV1>(items); value.item_count = (ulong)items.Length;
        ViemStyleEditGroupV1? group = null;
        if (!UsesGlobalStyles) { var identity = StyleIdentity(); var token = New<ViemStyleEditGroupV1>(); Check(viem_core_view_begin_style_edit_group(Document.Handle, Id, &identity, &token), "Begin font change"); group = token; }
        try {
            EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES, value, group);
            var faceValue = New<ViemStyleEditValueV1>(); faceValue.kind = VIEM_STYLE_VALUE_STRING; faceValue.text = arena.Utf8(selectedFace);
            EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_FONT_FACE, faceValue, group);
            var axisValue = New<ViemStyleEditValueV1>(); axisValue.kind = VIEM_STYLE_VALUE_STRING; axisValue.text = arena.Utf8(FontVariations.Encode(axes));
            EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_FONT_AXES, axisValue, group);
            uint weight = (uint)Math.Clamp(Math.Round(axes.GetValueOrDefault("wght", face?.Weight ?? 400)), 1, 1000);
            EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT, Enum(VIEM_STYLE_VALUE_UNSIGNED, weight), group);
        }
        finally { if (group is { } token) Check(viem_core_view_end_style_edit_group(Document.Handle, Id, &token), "End font change"); }
    }
    public void EditStyleString(StyleDefinition style, uint operation, uint property, string text, bool list = false)
    {
        using var arena = new NativeArena(); var value = New<ViemStyleEditValueV1>(); value.kind = list ? VIEM_STYLE_VALUE_STRING_LIST : VIEM_STYLE_VALUE_STRING; value.text = arena.Utf8(text);
        var item = New<ViemStyleEditValueItemV1>(); item.kind = VIEM_STYLE_VALUE_ITEM_STRING; item.text = value.text;
        if (list) { value.items = &item; value.item_count = 1; value.text = default; }
        EditStyle(style, operation, property, value);
    }
    public void AssignStyle(uint space, string id, ViemStyleSheetIdentityV1? expected = null)
    {
        using var arena = new NativeArena(); var slice = arena.Utf8(id); var identity = expected ?? StyleIdentity();
        Apply(o => { var r = New<ViemAssignStyleV1>(); r.@namespace = space; r.identity = identity; r.style_id = slice; r.expected_selection = LogicalSelection(); return viem_core_view_assign_style(Document.Handle, Id, &r, o); });
    }
    public string CreateCodeStyle(string name)
    {
        if (!UsesGlobalStyles) throw new InvalidOperationException("Only Code styles support new definitions.");
        using var arena = new NativeArena(); var r = New<ViemCreateStyleV1>(); r.@namespace = VIEM_STYLE_NAMESPACE_CHARACTER; r.identity = Styles().Identity;
        string id = "Style" + Guid.NewGuid().ToString("N");
        r.style_id = arena.Utf8(id); r.display_name = arena.Utf8(name);
        var info = New<ViemStyleSheetInfoV1>(); Check(viem_code_create_style(&r, &info), "Create style"); Document.NotifyChanged(); Refresh();
        return id;
    }
    public void DeleteStyle(StyleDefinition style)
    {
        using var arena = new NativeArena(); var r = New<ViemDeleteStyleV1>(); r.@namespace = style.Namespace; r.style_id = arena.Utf8(style.Id); r.identity = Styles().Identity;
        if (!UsesGlobalStyles) throw new InvalidOperationException("Only Code styles support removing definitions.");
        var info = New<ViemStyleSheetInfoV1>(); Check(viem_code_delete_style(&r, &info), "Delete style"); Document.NotifyChanged(); Refresh();
    }
    public byte[] ExportStyleDefaults() { ulong revision = Document.State.document_revision; return UsesGlobalStyles ? Copy(viem_code_export_style_json) : Copy((p, n, r) => viem_core_export_style_defaults(Document.Handle, revision, p, n, r)); }
    public void ReplaceCodeStyles(byte[] json)
    {
        if (!UsesGlobalStyles) throw new InvalidOperationException("Only Code Styles have shared defaults.");
        fixed (byte* bytes = json) Check(viem_code_replace_style_json(bytes, (ulong)json.Length), "Restore Code styles");
        Document.NotifyChanged(); Refresh();
    }
    public void DeclareEffectiveStyle(StyleDefinition style, uint property, StyleSheet sheet)
    {
        var effective = style.Value(property);
        if (property == VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES) { EditStyleFont(style, sheet.StringList(effective), null); return; }
        var value = New<ViemStyleEditValueV1>(); value.kind = effective.kind; value.number = effective.number; value.enum_value = effective.enum_value; value.color = effective.color;
        // An absent background is a transparent declaration when overridden.
        if (property is VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND or VIEM_STYLE_PROPERTY_BLOCK_BACKGROUND && effective.kind == VIEM_STYLE_VALUE_NONE) value.kind = VIEM_STYLE_VALUE_COLOR;
        EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, property, value);
    }
}
