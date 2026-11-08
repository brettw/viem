using System.Runtime.InteropServices;
using System.Text.Json.Nodes;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Windows.Graphics;

namespace Viem.Windows.Shell;

// Native desktop coordinates, independent of document layout coordinates.
internal readonly record struct WindowFrame(double X, double Y, double Width, double Height)
{
    internal bool IsValid => double.IsFinite(X) && double.IsFinite(Y) && Width > 0 && Height > 0
        && double.IsFinite(Width) && double.IsFinite(Height) && double.IsFinite(X + Width) && double.IsFinite(Y + Height);
    internal static WindowFrame From(RectInt32 value) => new(value.X, value.Y, value.Width, value.Height);
    internal static WindowFrame From(AppWindow window) => new(window.Position.X, window.Position.Y, window.Size.Width, window.Size.Height);
    internal RectInt32 Native => new(checked((int)Math.Round(X)), checked((int)Math.Round(Y)),
        Math.Max(1, checked((int)Math.Round(Width))), Math.Max(1, checked((int)Math.Round(Height))));
    internal JsonObject Json => new() { ["x"] = X, ["y"] = Y, ["width"] = Width, ["height"] = Height };
    internal static WindowFrame? Read(JsonNode? node)
    {
        try {
            if (node is not JsonObject value) return null;
            var frame = new WindowFrame(value["x"]!.GetValue<double>(), value["y"]!.GetValue<double>(),
                value["width"]!.GetValue<double>(), value["height"]!.GetValue<double>());
            return frame.IsValid ? frame : null;
        }
        catch { return null; }
    }
}

/// <summary>One placement history per application profile, shared by document windows.</summary>
internal sealed class DocumentWindowPlacement(Preferences preferences)
{
    [DllImport("user32.dll")] private static extern uint GetDpiForWindow(nint window);
    [DllImport("user32.dll")] private static extern int GetSystemMetricsForDpi(int index, uint dpi);
    [StructLayout(LayoutKind.Sequential)] private struct NativeRect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] private static extern bool GetWindowRect(nint window, out NativeRect rect);
    private readonly DispatcherTimer saveTimer = new() { Interval = TimeSpan.FromMilliseconds(400) };
    private WindowFrame? lastFrame, pending, lastDiffFrame, pendingDiff;
    private bool hasPlacedWindow, timerAttached;
    internal Exception? LastError { get; private set; }

    internal Action Track(Window window, Func<double>? gutterWidth = null, Func<bool>? diffMode = null)
    {
        var appWindow = window.AppWindow;
        double GutterPixels() => Math.Max(0, gutterWidth?.Invoke() ?? 0) * Math.Max(96, GetDpiForWindow(WinRT.Interop.WindowNative.GetWindowHandle(window))) / 96d;
        nint hwnd = WinRT.Interop.WindowNative.GetWindowHandle(window);
        var displays = DisplayArea.FindAll();
        var screens = new List<WindowFrame>(displays.Count);
        // The native vector view supports indexed access, but not IEnumerable
        // on every supported Windows App SDK/runtime combination.
        for (int i = 0; i < displays.Count; i++) screens.Add(WorkArea(displays[i]));
        var primary = WorkArea(DisplayArea.Primary);
        if (screens.Count == 0) screens.Add(primary);
        var frame = lastFrame ?? preferences.DocumentWindowFrame
            ?? new WindowFrame(primary.X + (primary.Width - 1100) / 2, primary.Y + (primary.Height - 780) / 2, 1100, 780);
        frame = Fitting(IncludingGutter(frame, GutterPixels()), screens);
        appWindow.MoveAndResize(frame.Native);
        if (hasPlacedWindow) {
            // Moving first gives the caption metric the destination monitor's DPI.
            uint dpi = Math.Max(96, GetDpiForWindow(hwnd));
            int offset = Math.Max(1, GetSystemMetricsForDpi(4 /* SM_CYCAPTION */, dpi));
            frame = Fitting(frame with { X = frame.X + offset, Y = frame.Y + offset }, screens);
            appWindow.MoveAndResize(frame.Native);
        }
        lastFrame = ExcludingGutter(frame, GutterPixels()); hasPlacedWindow = true;
        WindowFrame normalFrame = ExcludingGutter(WindowFrame.From(appWindow), GutterPixels());
        bool shown = false, closed = false, captureQueued = false, recordedDiff = false;
        void RecordNormal(bool requireVisible = true)
        {
            if (closed || !shown || !GetWindowRect(hwnd, out var bounds) || (requireVisible && !appWindow.IsVisible)
                || appWindow.Presenter is not OverlappedPresenter { State: OverlappedPresenterState.Restored }) return;
            // Use desktop bounds only while the native window still exists.
            var current = new WindowFrame(bounds.Left, bounds.Top, (double)bounds.Right - bounds.Left, (double)bounds.Bottom - bounds.Top);
            if (!current.IsValid) return;
            normalFrame = ExcludingGutter(current, GutterPixels());
            recordedDiff = diffMode?.Invoke() == true;
            Record(normalFrame, recordedDiff);
        }
        window.Activated += (_, args) => {
            if (args.WindowActivationState == WindowActivationState.Deactivated) return;
            // Initial activation may precede the AppWindow visibility update.
            if (!shown) { shown = true; RecordNormal(); }
        };
        appWindow.Changed += (_, args) => {
            if (closed || captureQueued || !(args.DidPositionChange || args.DidSizeChange || args.DidVisibilityChange || args.DidPresenterChange)) return;
            // WinUI can send size changes before updating Presenter during a
            // fullscreen transition. Sample once that transition has finished.
            captureQueued = window.DispatcherQueue.TryEnqueue(() => {
                captureQueued = false;
                if (closed) return;
                shown |= appWindow.IsVisible;
                RecordNormal();
            });
        };
        window.Closed += (_, _) => {
            // Capture a final move even when its queued notification has not run.
            // If native teardown already occurred, keep the last normal frame.
            RecordNormal(requireVisible: false);
            closed = true;
            if (shown) Record(normalFrame, recordedDiff);
            Flush();
        };
        if (!timerAttached) { saveTimer.Tick += (_, _) => Flush(); timerAttached = true; }
        return () => { RecordNormal(requireVisible: false); Flush(); };
    }

    internal static WindowFrame ExcludingGutter(WindowFrame frame, double width)
    {
        width = double.IsFinite(width) ? Math.Clamp(width, 0, Math.Max(0, frame.Width - 1)) : 0;
        return frame with { X = frame.X + width, Width = frame.Width - width };
    }
    internal static WindowFrame IncludingGutter(WindowFrame frame, double width)
    {
        width = double.IsFinite(width) ? Math.Max(0, width) : 0;
        return frame with { X = frame.X - width, Width = frame.Width + width };
    }

    internal static WindowFrame WorkArea(DisplayArea screen) => DesktopWorkArea(screen.OuterBounds, screen.WorkArea);
    // WinUI reports work-area offsets relative to the monitor, while AppWindow
    // positions are desktop coordinates. Include the monitor's origin once.
    internal static WindowFrame DesktopWorkArea(RectInt32 bounds, RectInt32 work) =>
        new((double)bounds.X + work.X, (double)bounds.Y + work.Y, work.Width, work.Height);

    internal void ApplyDiffFrame(Window window, bool cascade)
    {
        var current = WindowFrame.From(window.AppWindow);
        var previous = lastDiffFrame ?? preferences.DiffWindowFrame;
        var frame = previous ?? (current with { Width = current.Width * 2 });
        if (!cascade) frame = frame with { X = current.X, Y = current.Y };
        else if (lastDiffFrame.HasValue) {
            int offset = Math.Max(1, GetSystemMetricsForDpi(4, GetDpiForWindow(WinRT.Interop.WindowNative.GetWindowHandle(window))));
            frame = frame with { X = frame.X + offset, Y = frame.Y + offset };
        }
        var displays = DisplayArea.FindAll();
        var screens = new List<WindowFrame>();
        for (int i = 0; i < displays.Count; ++i) screens.Add(WorkArea(displays[i]));
        window.AppWindow.MoveAndResize(Fitting(frame, screens).Native);
    }

    private void Record(WindowFrame frame, bool diff)
    {
        if (diff) lastDiffFrame = pendingDiff = frame;
        else lastFrame = pending = frame;
        saveTimer.Stop(); saveTimer.Start();
    }
    internal void Flush()
    {
        saveTimer.Stop();
        if (pending == null && pendingDiff == null) return;
        try {
            if (pending is { } frame && preferences.DocumentWindowFrame != frame) preferences.SetDocumentWindowFrame(frame);
            if (pendingDiff is { } diff && preferences.DiffWindowFrame != diff) preferences.SetDocumentWindowFrame(diff, diff: true);
            pending = pendingDiff = null; LastError = null;
        }
        // A bad/read-only configuration must not interrupt moving or closing.
        catch (Exception error) { LastError = error; }
    }

    // Translate onto the best remaining work area before shrinking dimensions.
    // Negative coordinates are valid on monitors above/left of the primary.
    internal static WindowFrame Fitting(WindowFrame frame, IEnumerable<WindowFrame> screens)
    {
        if (!frame.IsValid) return frame;
        double Intersection(WindowFrame screen) => Math.Max(0, Math.Min(frame.X + frame.Width, screen.X + screen.Width) - Math.Max(frame.X, screen.X))
            * Math.Max(0, Math.Min(frame.Y + frame.Height, screen.Y + screen.Height) - Math.Max(frame.Y, screen.Y));
        double Distance(WindowFrame screen) {
            double x = frame.X + frame.Width / 2, y = frame.Y + frame.Height / 2;
            return Math.Pow(x - Math.Clamp(x, screen.X, screen.X + screen.Width), 2) + Math.Pow(y - Math.Clamp(y, screen.Y, screen.Y + screen.Height), 2);
        }
        var available = screens.Where(s => s.IsValid).OrderByDescending(Intersection).ThenBy(Distance).ToArray();
        if (available.Length == 0) return frame;
        var area = available[0];
        double width = Math.Min(frame.Width, area.Width), height = Math.Min(frame.Height, area.Height);
        return new(Math.Clamp(frame.X, area.X, area.X + area.Width - width), Math.Clamp(frame.Y, area.Y, area.Y + area.Height - height), width, height);
    }
}
