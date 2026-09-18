#if DEBUG
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Editor;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class PresentationTests
{
    internal static async Task Run(EditorPane pane, MenuBar menu, Preferences preferences)
    {
        void Check(bool value, string name)
        { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
        var view = pane.View!;
        var edit = menu.Items.Single(m => m.Title == "Edit");
        var peer = new MenuBarItemAutomationPeer(edit);
        for (int pass = 0; pass < 2; pass++)
        {
            if (pass == 0) view.SelectAll(); else view.Key(VIEM_KEY_ESCAPE);
            peer.Expand(); await Task.Delay(80);
            Check(edit.Items.OfType<MenuFlyoutItem>().Single(i => i.Text == "Copy").IsEnabled == (pass == 0),
                pass == 0 ? "opening native menus validates the current selection" : "reopening native menus discards stale selection state");
            peer.Collapse(); await Task.Delay(80);
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Escape);
        }
        pane.FocusEditor();
        using var surface = new CanvasRenderTarget(pane.Canvas.Device, (float)pane.Canvas.ActualWidth, (float)pane.Canvas.ActualHeight, pane.Canvas.Dpi);
        byte[] Draw() { using (var drawing = surface.CreateDrawingSession()) pane.Draw(drawing); return surface.GetPixelBytes(); }
        view.Place(45f, 40f);
        byte[] normal = Draw(); int initial = pane.DrawingCacheBuilds;
        view.Place(160f, 40f, true);
        byte[] selected = Draw();
        Check(pane.DrawingCacheBuilds == initial && !selected.AsSpan().SequenceEqual(normal), "selection redraw changes pixels without rebuilding text commands");
        pane.InvalidateDrawingCache();
        Check(Draw().AsSpan().SequenceEqual(selected), "cached and rebuilt selection frames are pixel-identical");
        void Invalidates(Action action, string name)
        {
            int before = pane.DrawingCacheBuilds;
            action(); Draw(); Check(pane.DrawingCacheBuilds > before, name);
        }
        view.Key(VIEM_KEY_ESCAPE);
        Invalidates(() => { view.Command("ggi"); view.Text("Test "); view.Key(VIEM_KEY_ESCAPE); }, "editing invalidates cached drawing commands");
        Invalidates(view.Undo, "undo invalidates cached drawing commands");
        Invalidates(() => view.Resize((float)pane.Canvas.ActualWidth + 35, (float)pane.Canvas.ActualHeight), "resizing invalidates cached drawing commands");
        view.Resize((float)pane.Canvas.ActualWidth, (float)pane.Canvas.ActualHeight);
        Draw();
        Invalidates(() => view.Zoom(1.25f), "zoom invalidates cached drawing commands");
        view.Zoom(1); Draw();
        Invalidates(() => view.VisibleWhitespace(!pane.WhitespaceEnabled), "whitespace visibility invalidates cached drawing commands");
        var theme = preferences.Theme;
        Invalidates(() => preferences.Set("theme", "foreground", Preferences.ColorJson(Microsoft.UI.Colors.Red)), "theme invalidates cached drawing commands");
        preferences.Set("theme", "foreground", Preferences.ColorJson(theme.Foreground));
        if (pane.LastError != null) throw pane.LastError;
    }
}
#endif
