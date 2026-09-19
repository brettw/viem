using Viem.Windows.Interop;
using Viem.Windows.Rendering;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed record StyleDefinition(ViemStyleDefinitionV1 Native, string Id, string Name, string Parent, string Next, Dictionary<uint, ViemStylePropertyV1> Properties)
{
    public override string ToString() => Name;
    public uint Namespace => Native.namespace_id;
    public bool Has(uint capability) => (Native.capabilities & capability) != 0;
    public bool Declares(uint property) => Properties.TryGetValue(property, out var p) && (p.flags & VIEM_STYLE_PROPERTY_DECLARED) != 0;
    public ViemStyleValueV1 Value(uint property) => Properties.TryGetValue(property, out var p) ? p.effective : default;
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
    public uint DecorationState(uint property)
    { uint state = 0; Check(viem_core_view_decoration_state(Document.Handle, Id, property, &state), "Read decoration"); return state; }
    public void DirectStyle(uint property, ViemStyleEditValueV1 value, bool clear = false)
    {
        if (clear) value = New<ViemStyleEditValueV1>();
        var selection = LogicalSelection();
        Apply(o => { var request = New<ViemDirectStyleEditV1>(); request.property = property; request.operation = clear ? VIEM_STYLE_EDIT_CLEAR_DECLARATION : VIEM_STYLE_EDIT_SET_DECLARATION;
            request.expected_selection = selection; request.value = value; return viem_core_view_edit_direct_style(Document.Handle, Id, &request, o); });
    }
    public void DirectFontFamily(string family)
    {
        using var arena = new NativeArena();
        var item = New<ViemStyleEditValueItemV1>(); item.kind = VIEM_STYLE_VALUE_ITEM_STRING; item.text = arena.Utf8(family);
        var value = New<ViemStyleEditValueV1>(); value.kind = VIEM_STYLE_VALUE_STRING_LIST; value.items = &item; value.item_count = 1;
        DirectStyle(VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES, value);
    }
    public static ViemStyleEditValueV1 Number(float number) { var v = New<ViemStyleEditValueV1>(); v.kind = VIEM_STYLE_VALUE_FLOAT; v.number = number; return v; }
    public static ViemStyleEditValueV1 Enum(uint kind, uint value) { var v = New<ViemStyleEditValueV1>(); v.kind = kind; v.enum_value = value; return v; }
    public (ViemTypographyInfoV1 Info, string Family) Typography()
    {
        ulong revision = Document.State.document_revision;
        var info = New<ViemTypographyInfoV1>(); uint status = viem_core_view_typography_export(Document.Handle, Id, revision, &info, null, 0, null, 0);
        if (status != VIEM_STATUS_BUFFER_TOO_SMALL) Check(status, "Read typography");
        var family = new byte[checked((int)info.font_family_bytes)]; var features = new ViemOpenTypeFeatureV1[checked((int)info.feature_count)];
        fixed (byte* p = family) fixed (ViemOpenTypeFeatureV1* f = features) Check(viem_core_view_typography_export(Document.Handle, Id, revision, &info, p, (ulong)family.Length, f, (ulong)features.Length), "Read typography");
        return (info, System.Text.Encoding.UTF8.GetString(family));
    }
    public void SetFont(string family, float size, FontFace? face = null)
    {
        using var arena = new NativeArena(); var item = New<ViemStyleEditValueItemV1>(); item.kind = VIEM_STYLE_VALUE_ITEM_STRING; item.text = arena.Utf8(face?.Name ?? family);
        var font = New<ViemStyleEditValueV1>(); font.kind = VIEM_STYLE_VALUE_STRING_LIST; font.items = &item; font.item_count = 1;
        var selection = LogicalSelection(); var requests = new ViemDirectStyleEditV1[face == null ? 2 : 4];
        for (int i = 0; i < requests.Length; i++) { requests[i] = New<ViemDirectStyleEditV1>(); requests[i].operation = VIEM_STYLE_EDIT_SET_DECLARATION; requests[i].expected_selection = selection; }
        requests[0].property = VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES; requests[0].value = font;
        requests[1].property = VIEM_STYLE_PROPERTY_CHARACTER_SIZE; requests[1].value = Number(size);
        if (face != null) {
            requests[2].property = VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT; requests[2].value = Enum(VIEM_STYLE_VALUE_UNSIGNED, face.Weight);
            requests[3].property = VIEM_STYLE_PROPERTY_CHARACTER_SLANT; requests[3].value = Enum(VIEM_STYLE_VALUE_FONT_SLANT, face.Slant == global::Windows.UI.Text.FontStyle.Normal ? 0u : face.Slant == global::Windows.UI.Text.FontStyle.Italic ? 1u : 2u);
        }
        Apply(o => { fixed (ViemDirectStyleEditV1* p = requests) return viem_core_view_edit_direct_character_batch(Document.Handle, Id, p, (ulong)requests.Length, o); });
    }
    public void SetParagraph(uint level) => Apply(o => { var r = New<ViemSetParagraphStyleV1>(); r.level = level; r.expected_selection = LogicalSelection(); return viem_core_view_set_paragraph_style(Document.Handle, Id, &r, o); });
    public void SetList(uint style) => Apply(o => { var r = New<ViemSetListStyleV1>(); r.style = style; r.expected_selection = LogicalSelection(); return viem_core_view_set_list_style(Document.Handle, Id, &r, o); });
    public void IndentList(bool unindent) => Apply(o => { var r = New<ViemListIndentV1>(); r.unindent = unindent ? 1u : 0u; r.expected_selection = LogicalSelection(); return viem_core_view_indent_list(Document.Handle, Id, &r, o); });
    public uint ListCapabilities() { var selection = LogicalSelection(); uint caps = 0; Check(viem_core_view_list_indent_capabilities(Document.Handle, Id, &selection, &caps), "Read list actions"); return caps; }
    public void Format(uint format, bool convert = false) => Send((c, o, e) => { var state = Document.State; var r = New<ViemSetFormatV1>(); r.document_id = state.document_id; r.document_revision = state.document_revision; r.format = format; r.operation = convert ? 1u : 0u; return viem_core_view_set_format_with_effects(Document.Handle, Id, &r, o, e); });
    public void SetEncoding(uint encoding) => Send((c, o, e) => { var state = Document.State; var r = New<ViemSetEncodingV1>(); r.document_id = state.document_id; r.document_revision = state.document_revision; r.encoding = encoding; return viem_core_view_set_encoding_with_effects(Document.Handle, Id, &r, o, e); });
    public void FileFormat(uint format) => Apply(o => { var state = Document.State; var r = New<ViemSetFileFormatV1>(); r.document_id = state.document_id; r.document_revision = state.document_revision; r.file_format = format; return viem_core_view_set_file_format(Document.Handle, Id, &r, o); });
    public StyleSheet Styles()
    {
        bool global = UsesGlobalStyles;
        var info = New<ViemStyleSheetInfoV1>(); Check(global ? viem_code_style_sheet_info(&info) : viem_core_style_sheet_info(Document.Handle, &info), "Read styles");
        var definitions = new ViemStyleDefinitionV1[checked((int)info.definition_count)];
        var properties = new ViemStylePropertyV1[checked((int)info.property_count)];
        var items = new ViemStyleValueItemV1[checked((int)info.value_item_count)];
        var dependencies = new ViemStyleDependencyV1[checked((int)info.dependency_count)];
        var strings = new byte[checked((int)info.string_bytes)]; var identity = info.identity;
        fixed (ViemStyleDefinitionV1* d = definitions) fixed (ViemStylePropertyV1* p = properties) fixed (ViemStyleValueItemV1* v = items) fixed (ViemStyleDependencyV1* r = dependencies) fixed (byte* s = strings)
            Check(global ? viem_code_copy_style_sheet(&identity, d, (ulong)definitions.Length, p, (ulong)properties.Length, v, (ulong)items.Length, r, (ulong)dependencies.Length, s, (ulong)strings.Length, &info)
                : viem_core_copy_style_sheet(Document.Handle, &identity, d, (ulong)definitions.Length, p, (ulong)properties.Length, v, (ulong)items.Length, r, (ulong)dependencies.Length, s, (ulong)strings.Length, &info), "Copy styles");
        return new(identity, definitions.Select(d => new StyleDefinition(d, Abi.Text(strings, d.stable_id), Abi.Text(strings, d.display_name), Abi.Text(strings, d.parent_id), Abi.Text(strings, d.next_style_id),
            properties.Skip(checked((int)d.first_property)).Take(checked((int)d.property_count)).ToDictionary(p => p.property))).ToArray(), strings, items);
    }
    public void EditStyle(StyleDefinition style, uint operation, uint property, ViemStyleEditValueV1 value, ViemStyleEditGroupV1? group = null)
    {
        // Clear operations still require a sized, empty ABI value, including
        // relationship clears whose UI previously supplied a string value.
        if (operation is VIEM_STYLE_EDIT_CLEAR_DECLARATION or VIEM_STYLE_EDIT_CLEAR_PARENT or VIEM_STYLE_EDIT_CLEAR_NEXT_STYLE) value = New<ViemStyleEditValueV1>();
        using var arena = new NativeArena(); var request = New<ViemStyleEditV1>(); request.identity = Styles().Identity;
        request.namespace_id = style.Namespace; request.style_id = arena.Utf8(style.Id); request.operation = operation; request.property = property; request.value = value;
        if (UsesGlobalStyles) { var info = New<ViemStyleSheetInfoV1>(); Check(viem_code_edit_style(&request, &info), "Edit Code style"); Document.NotifyChanged(); Refresh(); }
        else { var copy = request; Apply(o => { var r = copy; if (group is { } token) return viem_core_view_edit_style_in_group(Document.Handle, Id, &token, &r, o); return viem_core_view_edit_style(Document.Handle, Id, &r, o); }); }
    }
    public void EditStyleFont(StyleDefinition style, string[] families, FontFace? face)
    {
        using var arena = new NativeArena();
        var items = families.Select(f => { var item = New<ViemStyleEditValueItemV1>(); item.kind = VIEM_STYLE_VALUE_ITEM_STRING; item.text = arena.Utf8(f); return item; }).ToArray();
        var value = New<ViemStyleEditValueV1>(); value.kind = VIEM_STYLE_VALUE_STRING_LIST; value.items = arena.Copy<ViemStyleEditValueItemV1>(items); value.item_count = (ulong)items.Length;
        ViemStyleEditGroupV1? group = null;
        if (face != null && !UsesGlobalStyles) { var identity = Styles().Identity; var token = New<ViemStyleEditGroupV1>(); Check(viem_core_view_begin_style_edit_group(Document.Handle, Id, &identity, &token), "Begin font change"); group = token; }
        try {
            EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_FONT_FAMILIES, value, group);
            if (face != null) {
                EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_WEIGHT, Enum(VIEM_STYLE_VALUE_UNSIGNED, face.Weight), group);
                EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, VIEM_STYLE_PROPERTY_CHARACTER_SLANT, Enum(VIEM_STYLE_VALUE_FONT_SLANT, face.Slant == global::Windows.UI.Text.FontStyle.Normal ? 0u : face.Slant == global::Windows.UI.Text.FontStyle.Italic ? 1u : 2u), group);
            }
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
    public void AssignStyle(uint space, string id)
    {
        using var arena = new NativeArena(); var slice = arena.Utf8(id); var identity = Styles().Identity;
        Apply(o => { var r = New<ViemAssignStyleV1>(); r.@namespace = space; r.identity = identity; r.style_id = slice; r.expected_selection = LogicalSelection(); return viem_core_view_assign_style(Document.Handle, Id, &r, o); });
    }
    public string CreateStyle(uint space, string name)
    {
        using var arena = new NativeArena(); var r = New<ViemCreateStyleV1>(); r.@namespace = space; r.identity = Styles().Identity;
        string id = Document.State.format == VIEM_FORMAT_RTF ? $"Rtf{(space == 1 ? "P" : "C")}{Enumerable.Range(1, 32766).First(n => !Styles().Styles.Any(s => s.Id == $"Rtf{(space == 1 ? "P" : "C")}{n}"))}" : "Style" + Guid.NewGuid().ToString("N");
        r.style_id = arena.Utf8(id); r.display_name = arena.Utf8(name);
        if (UsesGlobalStyles) { var info = New<ViemStyleSheetInfoV1>(); Check(viem_code_create_style(&r, &info), "Create style"); Document.NotifyChanged(); Refresh(); }
        else { var copy = r; Apply(o => { var request = copy; return viem_core_view_create_style(Document.Handle, Id, &request, o); }); }
        return id;
    }
    public void DeleteStyle(StyleDefinition style)
    {
        using var arena = new NativeArena(); var r = New<ViemDeleteStyleV1>(); r.@namespace = style.Namespace; r.style_id = arena.Utf8(style.Id); r.identity = Styles().Identity;
        if (UsesGlobalStyles) { var info = New<ViemStyleSheetInfoV1>(); Check(viem_code_delete_style(&r, &info), "Delete style"); Document.NotifyChanged(); Refresh(); }
        else { var copy = r; Apply(o => { var request = copy; return viem_core_view_delete_style(Document.Handle, Id, &request, o); }); }
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
        if (property == VIEM_STYLE_PROPERTY_CHARACTER_BACKGROUND && effective.kind == VIEM_STYLE_VALUE_NONE) value.kind = VIEM_STYLE_VALUE_COLOR;
        EditStyle(style, VIEM_STYLE_EDIT_SET_DECLARATION, property, value);
    }
}
