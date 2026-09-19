#if DEBUG
using Microsoft.UI.Windowing;
using Viem.Windows.Shell;

namespace Viem.Windows.Diagnostics;

internal static class WindowPlacementTests
{
    private static void Check(bool value, string name)
    { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
    internal static async Task Run(EditorWindow owner, string profile)
    {
        var desktop = new WindowFrame(0, 30, 1600, 1000);
        var secondary = new WindowFrame(-1500, 50, 1400, 900);
        var saved = new WindowFrame(-1400, 250, 900, 600);
        Check(DocumentWindowPlacement.Fitting(saved, [desktop, secondary]) == saved,
            "window restoration preserves geometry on a monitor with negative coordinates");
        Check(DocumentWindowPlacement.Fitting(saved, [desktop]) == new WindowFrame(0, 250, 900, 600),
            "disconnected-monitor restoration translates before shrinking");
        Check(DocumentWindowPlacement.Fitting(new(1500, -2000, 1900, 700), [desktop]) == new WindowFrame(0, 30, 1600, 700),
            "window restoration shrinks only dimensions larger than the work area");
        Check(DocumentWindowPlacement.Fitting(new(4000, 200, 900, 600), [desktop, new(2000, 0, 1400, 1000)]) == new WindowFrame(2500, 200, 900, 600),
            "offscreen restoration selects the nearest remaining monitor");
        Check(DocumentWindowPlacement.DesktopWorkArea(new(-1600, -900, 1600, 900), new(40, 30, 1560, 870)) == new WindowFrame(-1560, -870, 1560, 870),
            "monitor-relative work areas convert to desktop coordinates including taskbar offsets");
        Check(DocumentWindowPlacement.Fitting(saved, []) == saved, "empty display inventories preserve valid placement candidates");

        string directory = Path.Combine(profile, "window-placement");
        Directory.CreateDirectory(directory);
        string config = Path.Combine(directory, "config.json");
        File.WriteAllText(config, "{\"version\":1,\"future\":42,\"windows\":{\"showMenu\":false,\"future\":true}}");
        var preferences = new Preferences(directory);
        int notifications = 0; preferences.Changed += () => notifications++;
        var area = DocumentWindowPlacement.WorkArea(DisplayArea.Primary);
        var initial = new WindowFrame(area.X + 30, area.Y + 50, Math.Min(760, area.Width / 2), Math.Min(500, area.Height / 2));
        preferences.SetDocumentWindowFrame(initial);
        Check(notifications == 0 && !preferences.ShowMenu && preferences.Get("windows", "future", false)
            && System.Text.Json.Nodes.JsonNode.Parse(File.ReadAllText(config))!["future"]!.GetValue<int>() == 42,
            "placement writes preserve unknown preferences without triggering editor refreshes");
        byte[] valid = File.ReadAllBytes(config);
        foreach (var invalid in new[] { new WindowFrame(0, 0, 0, 200), new WindowFrame(double.PositiveInfinity, 0, 800, 600) }) {
            bool rejected = false;
            try { preferences.SetDocumentWindowFrame(invalid); } catch (InvalidDataException) { rejected = true; }
            Check(rejected && File.ReadAllBytes(config).AsSpan().SequenceEqual(valid), "invalid placement writes preserve the prior configuration");
        }
        var placement = new DocumentWindowPlacement(preferences);
        EditorWindow? first = null, second = null, reopened = null;
        EditorWindow Create(Preferences settings, DocumentWindowPlacement history) {
            var window = new EditorWindow(settings, placement: history);
            App.Instance.Windows.Add(window); window.Closed += (_, _) => App.Instance.Windows.Remove(window);
            return window;
        }
        try {
            first = Create(preferences, placement);
            Check(WindowFrame.From(first.AppWindow) == initial, "saved document placement is applied before first activation");
            first.Activate(); await first.ActivePane!.Ready; await Task.Delay(100);
            var moved = initial with { X = initial.X + 50, Y = initial.Y + 40, Width = initial.Width + 30, Height = initial.Height + 40 };
            first.AppWindow.MoveAndResize(moved.Native);
            Check(preferences.DocumentWindowFrame == initial, "native move/resize does not synchronously write preferences");
            await Task.Delay(650);
            Check(preferences.DocumentWindowFrame == moved && new Preferences(directory).DocumentWindowFrame == moved && placement.LastError == null,
                "native document move/resize persists the final frame after idle");
            first.Activate(); await Task.Delay(100);
            Check(WindowFrame.From(first.AppWindow) == moved, "reactivating an existing document window preserves its frame");
            var presenter = (OverlappedPresenter)first.AppWindow.Presenter;
            presenter.Maximize(); await Task.Delay(150); placement.Flush();
            Check(preferences.DocumentWindowFrame == moved, "maximizing does not replace saved normal geometry");
            presenter.Minimize(); await Task.Delay(150); placement.Flush();
            Check(preferences.DocumentWindowFrame == moved, "minimized coordinates are never saved as normal geometry");
            presenter.Restore(); await Task.Delay(150);
            // Restoring a minimized window returns to its previous maximized
            // state. Restore again before testing normal-window placement.
            if (presenter.State == OverlappedPresenterState.Maximized) { presenter.Restore(); await Task.Delay(150); }
            Check(presenter.State == OverlappedPresenterState.Restored && WindowFrame.From(first.AppWindow) == moved,
                "restoring from minimized/maximized recovers the normal native frame");
            first.AppWindow.SetPresenter(AppWindowPresenterKind.FullScreen); await Task.Delay(150); placement.Flush();
            Check(preferences.DocumentWindowFrame == moved, "fullscreen bounds are never saved as normal geometry");
            first.AppWindow.SetPresenter(AppWindowPresenterKind.Overlapped); await Task.Delay(150);
            Check(first.AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Restored } && WindowFrame.From(first.AppWindow) == moved,
                "leaving fullscreen restores the prior normal native frame");
            second = Create(preferences, placement);
            var cascaded = WindowFrame.From(second.AppWindow);
            Check(cascaded.Width == moved.Width && cascaded.Height == moved.Height && cascaded.X > moved.X && cascaded.Y > moved.Y,
                "subsequent document windows cascade down and right at the saved size");
            second.Activate(); await second.ActivePane!.Ready; await Task.Delay(100);
            second.Close(); second = null;
            var final = moved with { X = moved.X + 10, Width = moved.Width + 20 };
            first.AppWindow.MoveAndResize(final.Native);
            var nativeFinal = WindowFrame.From(first.AppWindow);
            first.Close(); first = null;
            var persisted = new Preferences(directory).DocumentWindowFrame;
            if (persisted != final) throw new InvalidOperationException($"Final placement: expected {final}; native {nativeFinal}; saved {persisted}; error {placement.LastError}");
            Check(true, "closing flushes final placement without waiting for the idle timer");
            var reload = new Preferences(directory);
            reopened = Create(reload, new(reload));
            Check(WindowFrame.From(reopened.AppWindow) == final, "a new placement session restores size and position from disk");
            reopened.Activate(); await reopened.ActivePane!.Ready; await Task.Delay(100);
            Check(WindowFrame.From(reopened.AppWindow) == final, "activation retains the restored native window frame");
        }
        finally { second?.Close(); first?.Close(); reopened?.Close(); owner.Activate(); owner.ActivePane?.FocusEditor(); }

        byte[] malformed = "{\"version\":1,\"windows\":{\"documentFrame\":{\"x\":true,\"y\":0,\"width\":800,\"height\":600}}}"u8.ToArray();
        File.WriteAllBytes(config, malformed);
        var broken = new Preferences(directory); bool refused = false;
        try { broken.SetDocumentWindowFrame(initial); } catch (InvalidOperationException) { refused = true; }
        Check(broken.Error != null && broken.DocumentWindowFrame == null && refused && File.ReadAllBytes(config).AsSpan().SequenceEqual(malformed),
            "malformed saved geometry is reported and never overwritten");
    }
}
#endif
