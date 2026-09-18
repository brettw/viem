using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed record ClipboardWrite(string Text, string Fragment);
internal sealed record FrontendRequest(ViemExFrontendRequestV1 Value, string Text);
internal sealed record HostEffects(ClipboardWrite[] Clipboard, FrontendRequest[] Requests, string[] Output)
{
    public static unsafe HostEffects Read(ulong handle)
    {
        var info = New<ViemEffectBatchInfoV1>(); Check(viem_effect_batch_info(handle, &info), "Read effects");
        var clipboard = new ViemClipboardWriteV1[checked((int)info.clipboard_write_count)];
        var requests = new ViemExFrontendRequestV1[checked((int)info.ex_request_count)];
        var options = new ViemExOptionDisplayV1[checked((int)info.ex_option_count)];
        var marks = new ViemExMarkV1[checked((int)info.ex_mark_count)];
        var registers = new ViemExRegisterV1[checked((int)info.ex_register_count)];
        var jumps = new ViemExJumpV1[checked((int)info.ex_jump_count)];
        var lines = new ViemExTextLineV1[checked((int)info.ex_text_line_count)];
        var formats = new uint[checked((int)info.file_format_count)]; var breaks = new ulong[checked((int)info.hard_break_count)];
        var bytes = new byte[checked((int)info.string_bytes)];
        fixed (ViemClipboardWriteV1* c = clipboard) fixed (ViemExFrontendRequestV1* r = requests)
        fixed (ViemExOptionDisplayV1* o = options) fixed (ViemExMarkV1* m = marks) fixed (ViemExRegisterV1* g = registers)
        fixed (ViemExJumpV1* j = jumps) fixed (ViemExTextLineV1* l = lines) fixed (uint* f = formats) fixed (ulong* b = breaks) fixed (byte* s = bytes)
            Check(viem_effect_batch_copy(handle, c, (ulong)clipboard.Length, r, (ulong)requests.Length, o, (ulong)options.Length,
                m, (ulong)marks.Length, g, (ulong)registers.Length, j, (ulong)jumps.Length, l, (ulong)lines.Length,
                f, (ulong)formats.Length, b, (ulong)breaks.Length, s, (ulong)bytes.Length, &info), "Copy effects");
        var writes = clipboard.Select((c, i) => new ClipboardWrite(Text(bytes, c.plain_text),
            System.Text.Encoding.UTF8.GetString(Copy((p, n, r) => viem_effect_batch_copy_clipboard_json(handle, (ulong)i, p, n, r))))).ToArray();
        string[] output = lines.Select(l => Text(bytes, l.text))
            .Concat(marks.Select(m => $"{(char)m.name}  {m.hard_line_index + 1}:{m.grapheme_column + 1}  {Text(bytes, m.line_text)}"))
            .Concat(registers.Select(r => $"\"{(char)r.name}  {Text(bytes, r.text)}"))
            .Concat(jumps.Select(j => $"{j.list_index + 1}  {j.hard_line_index + 1}:{j.grapheme_column + 1}  {Text(bytes, j.line_text)}"))
            .Concat(options.Select(o => $"{OptionName(o.name)}={((o.value_kind == VIEM_EX_OPTION_VALUE_STRING) ? Text(bytes, o.text) : o.scalar_value.ToString())}" )).ToArray();
        return new(writes, requests.Select(r => new FrontendRequest(r, Text(bytes, r.text))).ToArray(), output);
    }
    private static string OptionName(uint name) => name switch { 1 => "wrap", 2 => "linebreak", 3 => "fileformat", 5 => "ignorecase", 6 => "smartcase", 7 => "wrapscan", 8 => "textwidth", 9 => "autoindent", 10 => "tabstop", 11 => "shiftwidth", 12 => "softtabstop", 13 => "expandtab", 14 => "smarttab", 17 => "list", 18 => "listchars", 19 => "hlsearch", 20 => "incsearch", _ => $"option {name}" };
}
