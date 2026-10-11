using System.Runtime.InteropServices;
using Microsoft.UI.Windowing;
using Viem.Windows.Editor;

namespace Viem.Windows.Shell;

internal sealed partial class EditorWindow
{
    [DllImport("user32.dll")] private static extern uint GetDpiForWindow(nint window);

    internal void AdjustForGutter(EditorPane pane, double delta)
    {
        if (!double.IsFinite(delta) || Math.Abs(delta) < .01 || !Panes.Contains(pane)
            || Panes.Select(p => p.Document).Distinct().Count() != 1 || PaneStack.Splitters.Count != 0
            || AppWindow.Presenter is not OverlappedPresenter { State: OverlappedPresenterState.Restored }) return;
        double scale = Math.Max(96, GetDpiForWindow(Hwnd)) / 96d;
        var before = WindowFrame.From(AppWindow);
        var area = DocumentWindowPlacement.WorkArea(DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Nearest));
        var next = GutterFrame(before, area, delta * scale, PaneStack.MinimumSize.Width * scale);
        if (next != before) AppWindow.MoveAndResize(next.Native);
    }

    // Preserve the content width first, then its left screen coordinate. Grow
    // from the left where possible, using the right side when the monitor edge
    // prevents preserving both. A partial expansion consumes the remaining gap.
    internal static WindowFrame GutterFrame(WindowFrame frame, WindowFrame area, double delta, double minimumWidth = 1)
    {
        if (!frame.IsValid || !area.IsValid || !double.IsFinite(delta)) return frame;
        double width = Math.Clamp(frame.Width + delta, Math.Min(Math.Max(1, minimumWidth), area.Width), area.Width);
        double x = Math.Clamp(frame.X - delta, area.X, area.X + area.Width - width);
        return frame with { X = x, Width = width };
    }
}
