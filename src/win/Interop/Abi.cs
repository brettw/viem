using System.Runtime.InteropServices;
using System.Text;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Interop;

internal static unsafe class Abi
{
    public static T New<T>() where T : unmanaged
    {
        T result = default;
        *(uint*)&result = (uint)sizeof(T);
        return result;
    }

    public static void Check(uint status, string operation)
    {
        if (status == VIEM_STATUS_OK) return;
        string explanation = status switch
        {
            VIEM_STATUS_VERIFICATION_FAILED => "This edit cannot preserve the format's text and structure.",
            VIEM_STATUS_AMBIGUOUS_PROJECTION => "This edit is not supported for the selected content structure.",
            VIEM_STATUS_UNREPRESENTABLE_CHARACTER => "The document's encoding cannot represent this character.",
            VIEM_STATUS_UNSUPPORTED_OPERATION => "This operation is not supported for the current format or selection.",
            VIEM_STATUS_STYLE_READ_ONLY => "This style is read-only.",
            VIEM_STATUS_STALE_REVISION => "The document changed. Please repeat the action.",
            _ => $"{operation} failed (core status {status})."
        };
        throw new CoreException(status, explanation);
    }

    internal unsafe delegate uint ReadBytes(byte* data, ulong capacity, ulong* required);
    public static byte[] Copy(ReadBytes read)
    {
        ulong length = 0;
        uint status = read(null, 0, &length);
        if (status != VIEM_STATUS_BUFFER_TOO_SMALL) Check(status, "Read size");
        byte[] result = new byte[checked((int)length)];
        if (length == 0) return result;
        fixed (byte* data = result) Check(read(data, length, &length), "Read data");
        return result;
    }
    public static string Text(ViemUtf8Slice value) => value.length == 0 ? "" : Encoding.UTF8.GetString(value.data, checked((int)value.length));
    public static string Text(byte[] bytes, ulong offset, ulong length) => Encoding.UTF8.GetString(bytes, checked((int)offset), checked((int)length));
    public static string Text(byte[] bytes, ViemStyleStringRefV1 value) => Text(bytes, value.offset, value.length);
    public static string Text(byte[] bytes, ViemEffectBytesRefV1 value) => Text(bytes, value.offset, value.length);
}

internal sealed class CoreException(uint status, string message) : Exception(message)
{
    public uint Status { get; } = status;
}

/// <summary>Response pointers remain valid until the next provider call.</summary>
internal sealed unsafe class NativeArena : IDisposable
{
    private readonly List<nint> allocations = [];
    public T* Copy<T>(ReadOnlySpan<T> values) where T : unmanaged
    {
        if (values.IsEmpty) return null;
        T* pointer = (T*)NativeMemory.Alloc(checked((nuint)values.Length * (nuint)sizeof(T)));
        allocations.Add((nint)pointer);
        values.CopyTo(new Span<T>(pointer, values.Length));
        return pointer;
    }
    public ViemUtf8Slice Utf8(string text)
    {
        byte[] bytes = Encoding.UTF8.GetBytes(text);
        return new() { data = Copy<byte>(bytes), length = (ulong)bytes.Length };
    }
    public void Dispose()
    {
        foreach (nint pointer in allocations) NativeMemory.Free((void*)pointer);
        allocations.Clear();
    }
}
