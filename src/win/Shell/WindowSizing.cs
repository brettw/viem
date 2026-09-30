using System.Runtime.InteropServices;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Windows.Graphics;
using Windows.Foundation;

namespace Viem.Windows.Shell;

internal static class WindowSizing
{
    [StructLayout(LayoutKind.Sequential)] private struct NativePoint { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] private struct MinMaxInfo
    { public NativePoint Reserved, MaxSize, MaxPosition, MinTrackSize, MaxTrackSize; }
    private delegate nint SubclassProcedure(nint hwnd, uint message, nuint wparam, nint lparam, nuint id, nuint data);
    [DllImport("comctl32.dll")] private static extern bool SetWindowSubclass(nint hwnd, SubclassProcedure procedure, nuint id, nuint data);
    [DllImport("comctl32.dll")] private static extern bool RemoveWindowSubclass(nint hwnd, SubclassProcedure procedure, nuint id);
    [DllImport("comctl32.dll")] private static extern nint DefSubclassProc(nint hwnd, uint message, nuint wparam, nint lparam);
    [DllImport("user32.dll")] private static extern uint GetDpiForWindow(nint window);
    [DllImport("dwmapi.dll")] private static extern int DwmSetWindowAttribute(nint window, uint attribute, ref int value, uint size);
    internal static void TrackMinimumSize(Window window, Func<Size> minimumClientSize)
    {
        nint hwnd = WinRT.Interop.WindowNative.GetWindowHandle(window);
        SubclassProcedure procedure = (handle, message, wparam, lparam, _, _) => {
            nint result = DefSubclassProc(handle, message, wparam, lparam);
            if (message == 0x0024) // WM_GETMINMAXINFO
            {
                var info = Marshal.PtrToStructure<MinMaxInfo>(lparam);
                var minimum = minimumClientSize();
                double scale = Math.Max(96, GetDpiForWindow(handle)) / 96d;
                info.MinTrackSize.X = Math.Max(info.MinTrackSize.X, (int)Math.Ceiling(minimum.Width * scale)
                    + window.AppWindow.Size.Width - window.AppWindow.ClientSize.Width);
                info.MinTrackSize.Y = Math.Max(info.MinTrackSize.Y, (int)Math.Ceiling(minimum.Height * scale)
                    + window.AppWindow.Size.Height - window.AppWindow.ClientSize.Height);
                Marshal.StructureToPtr(info, lparam, false);
            }
            return result;
        };
        if (!SetWindowSubclass(hwnd, procedure, 1, 0)) throw new InvalidOperationException("Cannot configure the editor window's minimum size.");
        // Keep the callback rooted until native teardown finishes.
        window.Closed += (_, _) => { RemoveWindowSubclass(hwnd, procedure, 1); GC.KeepAlive(procedure); };
    }
    public static void Appearance(Window window, bool dark)
    {
        window.AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "Viem.ico"));
        int value = dark ? 1 : 0;
        _ = DwmSetWindowAttribute(WinRT.Interop.WindowNative.GetWindowHandle(window), 20, ref value, 4);
    }
    public static void Resize(Window window, int width, int height)
    {
        nint hwnd = WinRT.Interop.WindowNative.GetWindowHandle(window);
        double scale = Math.Max(96, GetDpiForWindow(hwnd)) / 96d;
        var area = DisplayArea.GetFromWindowId(window.AppWindow.Id, DisplayAreaFallback.Nearest).WorkArea;
        window.AppWindow.Resize(new SizeInt32(Math.Min(area.Width, (int)(width * scale)), Math.Min(area.Height, (int)(height * scale))));
    }
    public static void FitClient(Window window, int width, int height)
    {
        double scale = Math.Max(96, GetDpiForWindow(WinRT.Interop.WindowNative.GetWindowHandle(window))) / 96d;
        var area = DisplayArea.GetFromWindowId(window.AppWindow.Id, DisplayAreaFallback.Nearest).WorkArea;
        var chrome = window.AppWindow.Size.Height - window.AppWindow.ClientSize.Height;
        var size = new SizeInt32(Math.Min(area.Width - 16, (int)(width * scale)), Math.Min(area.Height - chrome - 16, (int)(height * scale)));
        if (window.AppWindow.ClientSize != size) window.AppWindow.ResizeClient(size);
    }
}
