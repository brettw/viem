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
    [DllImport("user32.dll")] private static extern nint GetWindowDC(nint hwnd);
    [DllImport("user32.dll")] private static extern int ReleaseDC(nint hwnd, nint dc);
    [DllImport("user32.dll")] private static extern bool PrintWindow(nint hwnd, nint dc, uint flags);
    [DllImport("gdi32.dll")] private static extern nint CreateCompatibleDC(nint dc);
    [DllImport("gdi32.dll")] private static extern nint CreateCompatibleBitmap(nint dc, int width, int height);
    [DllImport("gdi32.dll")] private static extern nint SelectObject(nint dc, nint value);
    [DllImport("gdi32.dll")] private static extern bool DeleteObject(nint value);
    [DllImport("gdi32.dll")] private static extern bool DeleteDC(nint dc);
    [DllImport("gdi32.dll")] private static extern int GetDIBits(nint dc, nint bitmap, uint start, uint lines, byte[] bytes, ref BitmapInfo info, uint usage);
    public static async Task Save(nint hwnd, CanvasDevice device, string path)
    {
        if (!GetWindowRect(hwnd, out var rect)) throw new InvalidOperationException("Cannot measure test window.");
        int width = rect.Right - rect.Left, height = rect.Bottom - rect.Top;
        nint dc = GetWindowDC(hwnd), memory = CreateCompatibleDC(dc), bitmap = CreateCompatibleBitmap(dc, width, height), previous = SelectObject(memory, bitmap);
        byte[] bytes = new byte[checked(width * height * 4)];
        try
        {
            if (!PrintWindow(hwnd, memory, 2)) throw new InvalidOperationException("Cannot capture test window.");
            SelectObject(memory, previous);
            var info = new BitmapInfo { Size = (uint)Marshal.SizeOf<BitmapInfo>(), Width = width, Height = -height, Planes = 1, BitCount = 32 };
            if (GetDIBits(memory, bitmap, 0, (uint)height, bytes, ref info, 0) == 0) throw new InvalidOperationException("Cannot read test-window pixels.");
        }
        finally { SelectObject(memory, previous); DeleteObject(bitmap); DeleteDC(memory); ReleaseDC(hwnd, dc); }
        using var image = CanvasBitmap.CreateFromBytes(device, bytes, width, height, DirectXPixelFormat.B8G8R8A8UIntNormalized, 96, CanvasAlphaMode.Ignore);
        await image.SaveAsync(path, CanvasBitmapFileFormat.Png);
    }
}
#endif
