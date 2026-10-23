using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.UI.Input;

namespace Viem.Windows.Editor;

/// Owns no shared system handles. The returned WinUI cursor owns a copy of our
/// temporary HCURSOR; its caller disposes it when the pane closes.
internal static unsafe class GutterCursor
{
    [StructLayout(LayoutKind.Sequential)] private struct IconInfo
    { public int IsIcon; public uint HotX, HotY; public nint Mask, Color; }
    [StructLayout(LayoutKind.Sequential)] private struct Bitmap
    { public int Type, Width, Height, WidthBytes; public ushort Planes, BitsPixel; public nint Bits; }
    [StructLayout(LayoutKind.Sequential)] private struct BitmapInfo
    {
        public uint Size; public int Width, Height; public ushort Planes, BitCount;
        public uint Compression, ImageSize; public int XPixels, YPixels; public uint Used, Important;
        public uint Black, White;
    }
    [DllImport("user32.dll", EntryPoint = "LoadCursorW")] private static extern nint LoadCursor(nint module, nint name);
    [DllImport("user32.dll")] private static extern bool GetIconInfo(nint cursor, out IconInfo info);
    [DllImport("user32.dll")] private static extern nint CreateIconIndirect(ref IconInfo info);
    [DllImport("user32.dll")] private static extern bool DestroyCursor(nint cursor);
    [DllImport("user32.dll")] private static extern nint GetDC(nint window);
    [DllImport("user32.dll")] private static extern int ReleaseDC(nint window, nint dc);
    [DllImport("gdi32.dll", EntryPoint = "GetObjectW")] private static extern int GetObject(nint bitmap, int size, out Bitmap value);
    [DllImport("gdi32.dll")] private static extern int GetDIBits(nint dc, nint bitmap, uint start, uint lines, [Out] byte[] bytes, ref BitmapInfo info, uint usage);
    [DllImport("gdi32.dll")] private static extern nint CreateDIBSection(nint dc, ref BitmapInfo info, uint usage, out nint pixels, nint section, uint offset);
    [DllImport("gdi32.dll")] private static extern bool DeleteObject(nint value);
    [DllImport("combase.dll")] private static extern int WindowsCreateString([MarshalAs(UnmanagedType.LPWStr)] string value, uint length, out nint text);
    [DllImport("combase.dll")] private static extern int WindowsDeleteString(nint text);
    [DllImport("combase.dll")] private static extern int RoGetActivationFactory(nint text, in Guid iid, out nint factory);

    internal static InputCursor Create() => Owned(TransformSystemCursor(32512 /* IDC_ARROW */, vertical: false), FallbackArrow);

    /// The system up arrow turned to point down, for selecting table columns.
    internal static InputCursor CreateColumnSelection() => Owned(TransformSystemCursor(32516 /* IDC_UPARROW */, vertical: true), FallbackDownArrow);

    private static InputCursor Owned(nint cursor, Func<nint> fallback)
    {
        if (cursor == 0) cursor = fallback();
        try { return FromHandle(cursor); }
        catch (Exception error) when (error is Win32Exception or COMException) {
            System.Diagnostics.Trace.TraceWarning("Custom cursor unavailable: {0}", error.Message);
            return InputSystemCursor.Create(InputSystemCursorShape.Arrow);
        }
        finally { if (cursor != 0) DestroyCursor(cursor); }
    }

    /// Mirrors a shared system cursor horizontally, or flips it vertically.
    private static nint TransformSystemCursor(nint id, bool vertical)
    {
        nint source = LoadCursor(0, id /* shared, never destroy */);
        if (source == 0 || !GetIconInfo(source, out var original)) return 0;
        nint mask = 0, color = 0, dc = GetDC(0);
        try {
            if (dc == 0 || GetObject(original.Mask, Marshal.SizeOf<Bitmap>(), out var size) == 0
                || size.Width <= 0 || size.Width > 512 || size.Height <= 0 || size.Height > 1024
                || original.HotX >= size.Width) return 0;
            // A monochrome cursor's mask stacks its AND and XOR images.
            int halves = original.Color == 0 ? 2 : 1, height = size.Height / halves;
            if (original.HotY >= height) return 0;
            mask = TransformBitmap(dc, original.Mask, monochrome: true, vertical, halves);
            if (original.Color != 0) color = TransformBitmap(dc, original.Color, monochrome: false, vertical, 1);
            if (mask == 0 || (original.Color != 0 && color == 0)) return 0;
            var transformed = new IconInfo { Mask = mask, Color = color,
                HotX = vertical ? original.HotX : (uint)size.Width - 1 - original.HotX,
                HotY = vertical ? (uint)height - 1 - original.HotY : original.HotY };
            return CreateIconIndirect(ref transformed);
        }
        finally {
            if (mask != 0) DeleteObject(mask);
            if (color != 0) DeleteObject(color);
            if (original.Mask != 0) DeleteObject(original.Mask);
            if (original.Color != 0) DeleteObject(original.Color);
            if (dc != 0) ReleaseDC(0, dc);
        }
    }

    private static nint TransformBitmap(nint dc, nint bitmap, bool monochrome, bool vertical, int halves)
    {
        if (GetObject(bitmap, Marshal.SizeOf<Bitmap>(), out var size) == 0
            || size.Width <= 0 || size.Width > 512 || size.Height <= 0 || size.Height > 1024
            || size.Height % halves != 0) return 0;
        var info = Info(size.Width, size.Height, monochrome);
        int stride = ((size.Width * info.BitCount + 31) / 32) * 4;
        byte[] source = new byte[checked(stride * size.Height)], target = new byte[source.Length];
        if (GetDIBits(dc, bitmap, 0, (uint)size.Height, source, ref info, 0) != size.Height) return 0;
        if (vertical) FlipRows(source, target, size.Height, stride, halves);
        else MirrorRows(source, target, size.Width, size.Height, stride, monochrome);
        return MakeBitmap(dc, ref info, target);
    }

    /// Reverses row order within each of `halves` stacked images.
    internal static void FlipRows(ReadOnlySpan<byte> source, Span<byte> target, int height, int stride, int halves)
    {
        int band = height / halves;
        for (int y = 0; y < height; y++) {
            int from = y / band * band + band - 1 - y % band;
            source.Slice(from * stride, stride).CopyTo(target.Slice(y * stride, stride));
        }
    }

    internal static void MirrorRows(ReadOnlySpan<byte> source, Span<byte> target,
        int width, int height, int stride, bool monochrome)
    {
        target.Clear();
        for (int y = 0; y < height; y++)
            for (int x = 0; x < width; x++) {
                int from = width - 1 - x;
                if (monochrome) {
                    if ((source[y * stride + from / 8] & (0x80 >> (from % 8))) != 0)
                        target[y * stride + x / 8] |= (byte)(0x80 >> (x % 8));
                }
                else source.Slice(y * stride + from * 4, 4).CopyTo(target.Slice(y * stride + x * 4, 4));
            }
    }

    private static BitmapInfo Info(int width, int height, bool monochrome) => new() {
        Size = 40, Width = width, Height = -height, Planes = 1,
        BitCount = (ushort)(monochrome ? 1 : 32), White = 0x00ffffff,
        Used = monochrome ? 2u : 0u
    };
    private static nint MakeBitmap(nint dc, ref BitmapInfo info, byte[] bytes)
    {
        nint bitmap = CreateDIBSection(dc, ref info, 0, out nint pixels, 0, 0);
        if (bitmap != 0) Marshal.Copy(bytes, 0, pixels, bytes.Length);
        return bitmap;
    }

    // A white arrow with a black outline if the current system image cannot
    // be read. The tip and hotspot are at the upper right of the arrow.
    private static nint FallbackArrow() =>
        PolygonCursor([(27, 2), (27, 23), (22, 18), (18, 27), (14, 25), (18, 16), (10, 16)], 26, 3);

    // A centered white down arrow with a black outline; the tip is the hotspot.
    private static nint FallbackDownArrow() =>
        PolygonCursor([(13, 3), (19, 3), (19, 15), (25, 15), (16, 27), (7, 15), (13, 15)], 16, 26);

    private static nint PolygonCursor((double X, double Y)[] polygon, uint hotX, uint hotY)
    {
        const int size = 32;
        bool Inside(double x, double y) {
            bool inside = false;
            for (int i = 0, j = polygon.Length - 1; i < polygon.Length; j = i++) {
                var a = polygon[i]; var b = polygon[j];
                if ((a.Y > y) != (b.Y > y) && x < (b.X - a.X) * (y - a.Y) / (b.Y - a.Y) + a.X) inside = !inside;
            }
            return inside;
        }
        byte[] colors = new byte[size * size * 4], maskBytes = new byte[size * 4];
        Array.Fill(maskBytes, (byte)255);
        for (int y = 0; y < size; y++) for (int x = 0; x < size; x++) {
            if (!Inside(x + .5, y + .5)) continue;
            bool edge = !Inside(x - .5, y + .5) || !Inside(x + 1.5, y + .5)
                || !Inside(x + .5, y - .5) || !Inside(x + .5, y + 1.5);
            int at = (y * size + x) * 4;
            colors[at] = colors[at + 1] = colors[at + 2] = edge ? (byte)0 : (byte)255;
            colors[at + 3] = 255;
            maskBytes[y * 4 + x / 8] &= (byte)~(0x80 >> (x % 8));
        }
        var colorInfo = Info(size, size, false); var maskInfo = Info(size, size, true);
        nint color = 0, mask = 0;
        try {
            color = MakeBitmap(0, ref colorInfo, colors); mask = MakeBitmap(0, ref maskInfo, maskBytes);
            if (color == 0 || mask == 0) return 0;
            var info = new IconInfo { HotX = hotX, HotY = hotY, Color = color, Mask = mask };
            return CreateIconIndirect(ref info);
        }
        finally { if (color != 0) DeleteObject(color); if (mask != 0) DeleteObject(mask); }
    }

    private static InputCursor FromHandle(nint cursor)
    {
        if (cursor == 0) throw new Win32Exception("Cannot create a custom selection cursor.");
        nint name = 0, factory = 0, result = 0;
        try {
            const string runtimeClass = "Microsoft.UI.Input.InputCursor";
            Marshal.ThrowExceptionForHR(WindowsCreateString(runtimeClass, (uint)runtimeClass.Length, out name));
            Guid iid = new("ac6f5065-90c4-46ce-beb7-05e138e54117"); // IInputCursorStaticsInterop
            Marshal.ThrowExceptionForHR(RoGetActivationFactory(name, in iid, out factory));
            // IInspectable occupies slots 0–5. CreateFromHCursor copies custom
            // cursors, so the temporary Win32 handle can be destroyed immediately.
            // https://learn.microsoft.com/windows/windows-app-sdk/api/win32/microsoft.ui.input.inputcursor.interop/nf-microsoft-ui-input-inputcursor-interop-iinputcursorstaticsinterop-createfromhcursor
            Marshal.ThrowExceptionForHR(((delegate* unmanaged[Stdcall]<nint, nint, nint*, int>)(*(nint**)factory)[6])(factory, cursor, &result));
            return WinRT.MarshalInspectable<InputCursor>.FromAbi(result);
        }
        finally {
            if (result != 0) Marshal.Release(result);
            if (factory != 0) Marshal.Release(factory);
            if (name != 0) WindowsDeleteString(name);
        }
    }
}
