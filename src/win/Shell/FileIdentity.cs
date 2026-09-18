using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace Viem.Windows.Shell;

internal static class FileIdentity
{
    [StructLayout(LayoutKind.Sequential)] private struct Information
    { public uint Attributes; public System.Runtime.InteropServices.ComTypes.FILETIME Created, Accessed, Written; public uint Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow; }
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool GetFileInformationByHandle(SafeFileHandle file, out Information information);
    private static (uint Volume, uint High, uint Low)? Identity(string path)
    {
        try { using var file = File.OpenHandle(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete); return GetFileInformationByHandle(file, out var i) ? (i.Volume, i.IndexHigh, i.IndexLow) : null; }
        catch { return null; }
    }
    public static bool Same(string first, string second)
    {
        if (string.Equals(Path.GetFullPath(first), Path.GetFullPath(second), StringComparison.OrdinalIgnoreCase)) return true;
        var a = Identity(first); return a != null && a == Identity(second);
    }
}
