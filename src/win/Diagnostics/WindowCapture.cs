#if DEBUG
using System.Runtime.InteropServices;
using Microsoft.Graphics.Canvas;
using Windows.Graphics.DirectX;
using System.Runtime.InteropServices.WindowsRuntime;

namespace Viem.Windows.Diagnostics;

/// <summary>Captures only this test application's HWND, including Win2D content.</summary>
internal static class WindowCapture
{
    public static async Task SaveElement(Microsoft.UI.Xaml.FrameworkElement element, CanvasDevice device, string path)
    {
        var target = new Microsoft.UI.Xaml.Media.Imaging.RenderTargetBitmap();
        await target.RenderAsync(element);
        if (target.PixelWidth == 0 || target.PixelHeight == 0) throw new InvalidOperationException("The test dialog was not laid out.");
        var pixels = await target.GetPixelsAsync();
        using var image = CanvasBitmap.CreateFromBytes(device, pixels.ToArray(), target.PixelWidth, target.PixelHeight, DirectXPixelFormat.B8G8R8A8UIntNormalized);
        await image.SaveAsync(path, CanvasBitmapFileFormat.Png);
    }
    [StructLayout(LayoutKind.Sequential)] private struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] private struct BitmapInfo
    { public uint Size; public int Width, Height; public ushort Planes, BitCount; public uint Compression, ImageSize; public int XPixels, YPixels; public uint Used, Important; }
    [DllImport("user32.dll")] private static extern bool GetWindowRect(nint hwnd, out Rect rect);
    private delegate bool EnumerateWindow(nint hwnd, nint parameter);
    [DllImport("user32.dll")] private static extern bool EnumThreadWindows(uint thread, EnumerateWindow callback, nint parameter);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(nint hwnd, out uint process);
    [DllImport("user32.dll")] private static extern nint GetWindow(nint hwnd, uint command);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(nint hwnd);
    [DllImport("user32.dll")] private static extern nint GetForegroundWindow();
    [DllImport("user32.dll")] private static extern nint GetDC(nint hwnd);
    [DllImport("user32.dll")] private static extern nint GetWindowDC(nint hwnd);
    [DllImport("user32.dll")] private static extern int ReleaseDC(nint hwnd, nint dc);
    [DllImport("user32.dll")] private static extern bool PrintWindow(nint hwnd, nint dc, uint flags);
    [DllImport("gdi32.dll")] private static extern nint CreateCompatibleDC(nint dc);
    [DllImport("gdi32.dll")] private static extern nint CreateCompatibleBitmap(nint dc, int width, int height);
    [DllImport("gdi32.dll")] private static extern nint SelectObject(nint dc, nint value);
    [DllImport("gdi32.dll")] private static extern bool DeleteObject(nint value);
    [DllImport("gdi32.dll")] private static extern bool DeleteDC(nint dc);
    [DllImport("gdi32.dll")] private static extern bool BitBlt(nint target, int x, int y, int width, int height, nint source, int sourceX, int sourceY, uint operation);
    [DllImport("gdi32.dll")] private static extern int GetDIBits(nint dc, nint bitmap, uint start, uint lines, byte[] bytes, ref BitmapInfo info, uint usage);
    public static async Task<bool> SavePopup(nint owner, CanvasDevice device, string path)
    {
        uint thread = GetWindowThreadProcessId(owner, out uint process);
        nint popup = 0;
        EnumThreadWindows(thread, (candidate, _) => {
            GetWindowThreadProcessId(candidate, out uint candidateProcess);
            if (candidateProcess == process && GetWindow(candidate, 4) == owner && IsWindowVisible(candidate)) popup = candidate;
            return true;
        }, 0);
        if (popup == 0) throw new InvalidOperationException("The native color popup has no visible owned window.");
        GetWindowRect(owner, out var parent); GetWindowRect(popup, out var bounds);
        // PrintWindow returns black for WinUI's separate composition popup.
        // Capture only its on-screen bounds while our own test app is foreground.
        GetWindowThreadProcessId(GetForegroundWindow(), out uint foregroundProcess);
        if (foregroundProcess != process) throw new InvalidOperationException("The test popup lost foreground before its capture.");
        await Save(popup, device, path, screen: true);
        return bounds.Left < parent.Left || bounds.Top < parent.Top || bounds.Right > parent.Right || bounds.Bottom > parent.Bottom;
    }
    public static async Task Save(nint hwnd, CanvasDevice device, string path, bool screen = false)
    {
        if (!GetWindowRect(hwnd, out var rect)) throw new InvalidOperationException("Cannot measure test window.");
        int width = rect.Right - rect.Left, height = rect.Bottom - rect.Top;
        nint dc = screen ? GetDC(0) : GetWindowDC(hwnd), memory = CreateCompatibleDC(dc), bitmap = CreateCompatibleBitmap(dc, width, height), previous = SelectObject(memory, bitmap);
        byte[] bytes = new byte[checked(width * height * 4)];
        try
        {
            bool captured = screen ? BitBlt(memory, 0, 0, width, height, dc, rect.Left, rect.Top, 0x40CC0020) : PrintWindow(hwnd, memory, 2);
            if (!captured) throw new InvalidOperationException("Cannot capture test window.");
            SelectObject(memory, previous);
            var info = new BitmapInfo { Size = (uint)Marshal.SizeOf<BitmapInfo>(), Width = width, Height = -height, Planes = 1, BitCount = 32 };
            if (GetDIBits(memory, bitmap, 0, (uint)height, bytes, ref info, 0) == 0) throw new InvalidOperationException("Cannot read test-window pixels.");
        }
        finally { SelectObject(memory, previous); DeleteObject(bitmap); DeleteDC(memory); ReleaseDC(screen ? 0 : hwnd, dc); }
        using var image = CanvasBitmap.CreateFromBytes(device, bytes, width, height, DirectXPixelFormat.B8G8R8A8UIntNormalized, 96, CanvasAlphaMode.Ignore);
        await image.SaveAsync(path, CanvasBitmapFileFormat.Png);
    }
}
#endif
