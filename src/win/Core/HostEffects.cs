using System.Text;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed record ClipboardWrite(string Text, string Fragment);
internal sealed record FrontendRequest(ViemExFrontendRequestV1 Value, string Text);
internal sealed record HostEffects(ClipboardWrite[] Clipboard, FrontendRequest[] Requests, string[] Output, IReadOnlyDictionary<uint, string> SelectionOptions)
{
    public static unsafe HostEffects Read(ulong handle)
    {
        var info = New<ViemEffectBatchInfoV1>(); Check(viem_effect_batch_info(handle, &info), "Read effects");
        var borrowed = new ViemEffectBatchViewV1();
        Check(viem_effect_batch_read(handle, &borrowed), "Read effect arrays");
        var clipboard = new ReadOnlySpan<ViemClipboardWriteV1>(borrowed.clipboard_writes, checked((int)info.clipboard_write_count)).ToArray();
        var requests = new ReadOnlySpan<ViemExFrontendRequestV1>(borrowed.ex_requests, checked((int)info.ex_request_count)).ToArray();
        var options = new ReadOnlySpan<ViemExOptionDisplayV1>(borrowed.ex_options, checked((int)info.ex_option_count)).ToArray();
        var bytes = new ReadOnlySpan<byte>(borrowed.strings, checked((int)info.string_bytes)).ToArray();
        var writes = clipboard.Select((c, i) => new ClipboardWrite(Text(bytes, c.plain_text),
            System.Text.Encoding.UTF8.GetString(Copy((p, n, r) => viem_effect_batch_copy_clipboard_json(handle, (ulong)i, p, n, r))))).ToArray();
        string[] output = requests.Where(r => r.kind is VIEM_EX_FRONTEND_OPTIONS
            or VIEM_EX_FRONTEND_MARKS or VIEM_EX_FRONTEND_REGISTERS
            or VIEM_EX_FRONTEND_JUMPS or VIEM_EX_FRONTEND_PRINT_LINES)
            .Select(r => Text(bytes, r.display_text)).ToArray();
        var selectionOptions = new Dictionary<uint, string>();
        foreach (var option in options)
            if ((option.name is VIEM_EX_OPTION_KEYMODEL or VIEM_EX_OPTION_SELECTMODE) && option.value_kind == VIEM_EX_OPTION_VALUE_STRING)
                selectionOptions[option.name] = Text(bytes, option.text);
            else if (option.name == VIEM_EX_OPTION_AUTOSELECT && option.value_kind == VIEM_EX_OPTION_VALUE_BOOLEAN)
                selectionOptions[option.name] = option.scalar_value == 0 ? "0" : "1";
        return new(writes, requests.Select(r => new FrontendRequest(r, Text(bytes, r.text))).ToArray(), output, selectionOptions);
    }
}
