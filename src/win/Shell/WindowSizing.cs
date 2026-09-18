using System.Runtime.InteropServices;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Windows.Graphics;

namespace Viem.Windows.Shell;

internal static class WindowSizing
{
    [DllImport("user32.dll")] private static extern uint GetDpiForWindow(nint window);
    [DllImport("dwmapi.dll")] private static extern int DwmSetWindowAttribute(nint window, uint attribute, ref int value, uint size);
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
}
