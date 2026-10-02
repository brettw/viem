using System.Text;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed record SelectedStyles(ViemStyleSheetIdentityV1 Identity, uint Flags, string Paragraph, string Character)
{
    public bool ParagraphMixed => (Flags & VIEM_SELECTED_STYLE_PARAGRAPH_MIXED) != 0;
    public bool CharacterMixed => (Flags & VIEM_SELECTED_STYLE_CHARACTER_MIXED) != 0;
    public uint ListState(uint kind)
    {
        uint own = kind == VIEM_LIST_STYLE_BULLET ? VIEM_SELECTED_STYLE_HAS_BULLETS : VIEM_SELECTED_STYLE_HAS_NUMBERING;
        uint other = kind == VIEM_LIST_STYLE_BULLET ? VIEM_SELECTED_STYLE_HAS_NUMBERING : VIEM_SELECTED_STYLE_HAS_BULLETS;
        return (Flags & own) == 0 ? VIEM_SEMANTIC_STYLE_STATE_OFF
            : (Flags & (other | VIEM_SELECTED_STYLE_HAS_NON_LIST)) != 0 ? VIEM_SEMANTIC_STYLE_STATE_MIXED : VIEM_SEMANTIC_STYLE_STATE_ON;
    }
}

internal sealed record StyleChoice(StyleKey Key, string Name, bool Enabled, bool Selected);

internal sealed unsafe partial class CoreView
{
    public bool HasFormattingSelection => Presentation.mode is VIEM_MODE_NORMAL or VIEM_MODE_INSERT or VIEM_MODE_REPLACE
        or VIEM_MODE_VISUAL_CHARACTER or VIEM_MODE_VISUAL_LINE or VIEM_MODE_SELECT_CHARACTER or VIEM_MODE_SELECT_LINE
        or VIEM_MODE_SELECTION_CHARACTER or VIEM_MODE_SELECTION_LINE;

    public SelectedStyles SelectedNamedStyles()
    {
        var state = Document.State;
        if (!HasFormattingSelection)
        {
            var unavailable = New<ViemStyleSheetIdentityV1>();
            unavailable.document_id = state.document_id; unavailable.document_revision = state.document_revision; unavailable.style_sheet_revision = state.style_sheet_revision;
            return new(unavailable, VIEM_SELECTED_STYLE_PARAGRAPH_MIXED | VIEM_SELECTED_STYLE_CHARACTER_MIXED, "", "");
        }
        var info = New<ViemSelectedStylesInfoV1>();
        uint status = viem_core_view_selected_styles_export(Document.Handle, Id, state.document_revision, &info, null, 0);
        if (status != VIEM_STATUS_BUFFER_TOO_SMALL) Check(status, "Read selected styles");
        var bytes = new byte[checked((int)(info.paragraph_id_bytes + info.character_id_bytes))];
        fixed (byte* p = bytes) Check(viem_core_view_selected_styles_export(Document.Handle, Id, state.document_revision, &info, p, (ulong)bytes.Length), "Read selected styles");
        var identity = New<ViemStyleSheetIdentityV1>();
        identity.document_id = info.document_id; identity.document_revision = info.document_revision; identity.style_sheet_revision = info.style_sheet_revision;
        int split = checked((int)info.paragraph_id_bytes);
        return new(identity, info.flags, Encoding.UTF8.GetString(bytes.AsSpan(0, split)), Encoding.UTF8.GetString(bytes.AsSpan(split)));
    }

    // Shared by native menus and toolbar; names are labels, never action IDs.
    public static StyleChoice[] StyleChoices(StyleSheet sheet, SelectedStyles selected)
    {
        bool matches = sheet.Identity.Equals(selected.Identity);
        var choices = sheet.Styles.Where(s => s.Namespace is 1 or 2
            && (s.Native.flags & VIEM_STYLE_DEFINITION_INTERNAL) == 0
            && ((s.Native.flags & VIEM_STYLE_DEFINITION_INTERNAL_LIST) == 0 || matches && s.Id == selected.Paragraph))
            .OrderBy(s => s.Name, StringComparer.CurrentCultureIgnoreCase)
            .Select(s => new StyleChoice(s.Key, s.Name, (s.Namespace == 2 || selected.Paragraph is not ("Table cell" or "Table header")) && (s.Has(VIEM_STYLE_CAPABILITY_ASSIGN) || HeadingLevel(s.Key) != null),
                matches && (s.Namespace == 1 ? !selected.ParagraphMixed && selected.Paragraph == s.Id : !selected.CharacterMixed && selected.Character == s.Id))).ToList();
        choices.Insert(0, new(new(2, ""), "Default Paragraph", true, matches && !selected.CharacterMixed && selected.Character.Length == 0));
        return choices.ToArray();
    }

    internal static uint? HeadingLevel(StyleKey key) => key.Namespace != 1 ? null : key.Id == "Paragraph" ? 0
        : key.Id.Length == 8 && key.Id.StartsWith("Heading", StringComparison.Ordinal) && key.Id[7] is >= '1' and <= '6' ? (uint)(key.Id[7] - '0') : null;

    public void ChooseStyle(StyleKey key, ViemStyleSheetIdentityV1 expected)
    {
        // An open native menu retains its original catalogue; a stale action
        // must not silently apply that identity to another document/revision.
        if (!HasFormattingSelection) return;
        var selected = SelectedNamedStyles();
        if (!selected.Identity.Equals(expected)) return;
        if (!StyleChoices(Styles(expected), selected).Any(c => c.Key == key && c.Enabled)) return;
        if (HeadingLevel(key) is { } level) SetParagraph(level);
        else AssignStyle(key.Namespace, key.Id, expected);
    }
}
