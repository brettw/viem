using System.Text;
using System.Text.Json;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed record PromptState(ViemCommandLineInfoV1 Info, string Text, ulong Anchor, ulong Active);
internal sealed record CompletionState(ViemCompletionInfoV1 Info, string[] Items);
internal sealed record WhitespaceMarker(string Text, ulong Row, float X, float Y, float Width, float Height);
internal sealed record WhitespaceExport(JsonElement Style, WhitespaceMarker[] Markers, bool Enabled);

internal sealed unsafe partial class CoreView
{
    public PromptState Prompt()
    {
        var info = New<ViemCommandLineInfoV1>(); Check(viem_core_view_command_line_info(Document.Handle, Id, &info), "Read prompt");
        var expected = info.identity; var selection = New<ViemCommandLineSelectionV1>();
        Check(viem_core_view_command_line_selection(Document.Handle, Id, &expected, &selection), "Read prompt selection");
        var text = new byte[checked((int)info.utf8_length)];
        fixed (byte* p = text) Check(viem_core_view_copy_command_line(Document.Handle, Id, &expected, p, (ulong)text.Length, &info), "Read prompt text");
        return new(info, Encoding.UTF8.GetString(text), selection.anchor_utf8_offset, selection.active_utf8_offset);
    }
    public void EditPrompt(PromptState state, ulong start, ulong end, string? replacement = null)
    {
        byte[] text = Encoding.UTF8.GetBytes(replacement ?? "");
        Apply(o => { var expected = state.Info.identity; fixed (byte* p = text) return viem_core_view_edit_command_line(Document.Handle, Id, &expected, replacement == null ? 0u : 1u, start, end, p, (ulong)text.Length, o); });
    }
    public CompletionState Completion()
    {
        var info = New<ViemCompletionInfoV1>(); Check(viem_core_view_completion_info(Document.Handle, Id, &info), "Read completion");
        if ((info.flags & VIEM_COMPLETION_ACTIVE) == 0) return new(info, []);
        var expected = info; var items = new ViemCompletionItemV1[checked((int)info.item_count)]; var text = new byte[checked((int)info.utf8_length)];
        ulong count = 0;
        fixed (ViemCompletionItemV1* p = items) Check(viem_core_view_copy_completion_items(Document.Handle, Id, &expected, p, (ulong)items.Length, &count), "Read completion items");
        fixed (byte* p = text) Check(viem_core_view_copy_completion_utf8(Document.Handle, Id, &expected, p, (ulong)text.Length, &count), "Read completion text");
        return new(info, items.Select(i => Abi.Text(text, i.text_offset, i.text_length)).ToArray());
    }
    public void AcceptCompletion(CompletionState state, int index)
    {
        var current = Completion();
        if (current.Info.session_id != state.Info.session_id || current.Info.generation != state.Info.generation) throw new InvalidOperationException("Completion changed. Select the item again.");
        long delta = index - current.Info.selected_index;
        for (long i = 0; i < Math.Abs(delta); i++) Key(VIEM_KEY_CONTROL_CHARACTER, delta > 0 ? 'n' : 'p');
        ulong before = Document.State.document_revision;
        byte changed = 0; Check(viem_core_view_accept_completion(Document.Handle, Id, &changed), "Accept completion");
        if (Document.State.document_revision != before) Document.NotifyChanged();
        else if (changed != 0) Changed?.Invoke();
    }
    public WhitespaceExport Whitespace(ViemLayoutSnapshotInfoV1 layout)
    {
        var viewport = Viewport;
        var bounds = new ViemLayoutRectV1 { x = viewport.left, y = viewport.top, width = layout.viewport_width, height = layout.viewport_height };
        byte[] bytes = Copy((p, n, r) => { var expected = layout.identity; var rect = bounds; return viem_core_view_copy_whitespace_markers(Document.Handle, Id, &expected, &rect, p, n, r); });
        using var json = JsonDocument.Parse(bytes);
        return new(json.RootElement.GetProperty("style").Clone(), json.RootElement.GetProperty("markers").EnumerateArray().Select(m => new WhitespaceMarker(m.GetProperty("text").GetString()!, m.GetProperty("rowIndex").GetUInt64(), m.GetProperty("x").GetSingle(), m.GetProperty("y").GetSingle(), m.GetProperty("width").GetSingle(), m.GetProperty("height").GetSingle())).ToArray(), json.RootElement.GetProperty("enabled").GetBoolean());
    }
    public byte[] ClipboardJson(ulong start, ulong end)
    {
        var state = Document.State;
        var range = New<ViemFormattedUtf8RangeV1>(); range.identity = New<ViemFormattedSnapshotIdentityV1>(); range.identity.document_id = state.document_id; range.identity.document_revision = state.document_revision; range.utf8_start = start; range.utf8_end = end;
        return Copy((p, n, r) => { var expected = range; return viem_core_copy_clipboard_json(Document.Handle, &expected, p, n, r); });
    }
    public void ReadFile(byte[] bytes, ulong document, ulong revision, ulong afterLine)
        => Apply(o => { fixed (byte* p = bytes) return viem_core_view_read_file(Document.Handle, Id, document, revision, afterLine, p, (ulong)bytes.Length, o); });
    public HostEffects? SourceLine(string line, uint depth)
        => Send((c, o, e) => { byte[] bytes = Encoding.UTF8.GetBytes(line); fixed (byte* p = bytes) return viem_core_view_source_line(Document.Handle, Id, depth, p, (ulong)bytes.Length, c, o, e); }, false);
}
