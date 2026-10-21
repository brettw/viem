using System.Text;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;
internal readonly record struct FileMarkTarget(uint Name, ulong Generation, ulong DocumentId, string Path);
internal sealed unsafe partial class CoreView
{
    internal void DropText(string text)
    {
        using var arena = new NativeArena();
        var value = arena.Utf8(text);
        Send((context, outcome, effects) => viem_core_view_drop_text(Document.Handle, Id, value.data, value.length, context, outcome, effects));
    }
    internal void SetHostRegister(uint name, string text)
    {
        byte[] bytes = Encoding.UTF8.GetBytes(text);
        fixed (byte* input = bytes) Check(viem_core_view_set_host_register(Document.Handle, Id, name, input, (ulong)bytes.Length), "Set register context");
    }
    internal static FileMarkTarget ResolveFileMark(uint name)
    {
        ViemFileMarkTargetV1 info = default;
        Check(viem_file_mark_target(name, 0, &info, null, 0), "Find file mark");
        byte[] bytes = new byte[checked((int)info.path_length)];
        fixed (byte* path = bytes) Check(viem_file_mark_target(name, info.generation, &info, path, (ulong)bytes.Length), "Read file mark");
        return new(name, info.generation, info.document_id, Encoding.UTF8.GetString(bytes, 0, checked((int)info.path_length)));
    }
    internal void JumpToFileMark(FileMarkTarget target, bool linewise) => Apply(o => viem_core_view_jump_file_mark(Document.Handle, Id, target.Name, target.Generation, linewise ? 1u : 0u, o));
}
