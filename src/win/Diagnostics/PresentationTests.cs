#if DEBUG
using Microsoft.Graphics.Canvas;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Viem.Windows.Editor;
using Viem.Windows.Core;
using Viem.Windows.Interop;
using Viem.Windows.Shell;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Diagnostics;

internal static class PresentationTests
{
    internal static async Task Run(EditorPane pane, MenuBar menu, Preferences preferences, EditorWindow window)
    {
        void Check(bool value, string name)
        { if (!value) throw new InvalidOperationException(name); FrontendSmokeTests.UiChecks.Add(name); }
        var view = pane.View!;
        var edit = menu.Items.Single(m => m.Title == "Edit");
        var peer = new MenuBarItemAutomationPeer(edit);
        var copy = edit.Items.OfType<MenuFlyoutItem>().Single(i => i.Text == "Copy");
        for (int pass = 0; pass < 2; pass++)
        {
            if (pass == 0) view.SelectAll(); else view.Key(VIEM_KEY_ESCAPE);
            await MenuTestHelpers.Transition(true, () => { peer.Expand(); return Task.CompletedTask; }, copy);
            Check(copy.IsEnabled == (pass == 0),
                pass == 0 ? "opening native menus validates the current selection" : "reopening native menus discards stale selection state");
            await MenuTestHelpers.Transition(false, () => { peer.Collapse(); return Task.CompletedTask; }, copy);
            await InputRoutingTests.Key(global::Windows.System.VirtualKey.Escape);
        }
        pane.FocusEditor();
        var windowSize = window.AppWindow.Size;
        var revision = pane.Document.State.document_revision;
        try
        {
            foreach (int delta in new[] { -120, -200, 40 })
            {
                window.AppWindow.Resize(new(windowSize.Width + delta, windowSize.Height + delta / 2));
                await Task.Delay(100);
                var resized = view.Layout().Info;
                Check(resized.viewport_width == (float)pane.Canvas.ActualWidth
                    && resized.viewport_height == (float)pane.Canvas.ActualHeight
                    && resized.identity.document_revision == revision,
                    $"native window resize installs exact editor geometry ({delta})");
            }
        }
        finally { window.AppWindow.Resize(windowSize); }
        await Task.Delay(100);
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
        var unchangedRevision = pane.Document.State.document_revision;
        foreach (string export in new[] { "paint", "selection", "paragraph decorations", "table controls" }) {
            try {
                view.BeforePresentationExport = name => {
                    if (name == export) throw new CoreException(VIEM_STATUS_CORE_FAILURE, $"Injected {name} failure");
                };
                view.Zoom(1.1f); pane.Refresh();
                var degraded = view.Layout();
                Check(degraded.Rows.Length > 0 && CoreView.SameLayout(degraded.Info.identity, view.LayoutInfo().identity),
                    $"failed {export} export preserves current geometry");
                view.Provider.InvalidateMetrics();
                pane.ScrollVerticallyFromScrollbar(0);
                Check(view.LayoutInfo().identity.metrics_generation == view.Provider.Generation,
                    $"failed {export} export permits fresh scrollbar geometry after metrics invalidation");
                view.Place(45f, 40f); Draw();
                Check(pane.Document.State.document_revision == unchangedRevision,
                    $"failed {export} export leaves pointer placement available without source edits");
            } finally { view.BeforePresentationExport = null; view.Zoom(1); pane.Refresh(); }
            byte[] recovered = Draw(); pane.InvalidateDrawingCache();
            Check(Draw().AsSpan().SequenceEqual(recovered), $"recovered {export} export matches fresh pixels");
        }
        if (pane.LastError != null) throw pane.LastError;
    }
}
#endif
