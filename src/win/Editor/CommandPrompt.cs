using System.Numerics;
using System.Text;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Viem.Windows.Core;
using Viem.Windows.Rendering;
using Viem.Windows.Shell;

namespace Viem.Windows.Editor;

internal sealed class CommandPrompt : Grid, IDisposable
{
    private readonly CanvasControl canvas = new();
    private readonly EditorPane pane;
    private readonly Preferences preferences;
    private CanvasTextLayout? layout;
    private PromptState? state;
    private float scroll;
    private bool dragging, caret = true;
    private ulong anchor;
    public CommandPrompt(EditorPane pane, Preferences preferences)
    {
        this.pane = pane; this.preferences = preferences; Children.Add(canvas);
        canvas.Draw += (_, e) => {
            e.DrawingSession.Clear(preferences.Theme.StatusForeground);
            if (layout == null || state == null) return;
            var map = new Utf8IndexMap(state.Text);
            int start = map.Utf16(checked((int)Math.Min(state.Anchor, state.Active))) + 1, end = map.Utf16(checked((int)Math.Max(state.Anchor, state.Active))) + 1;
            foreach (var r in layout.GetCharacterRegions(start, end - start))
            { var bounds = r.LayoutBounds; bounds.X -= scroll; bounds.Y += 5; e.DrawingSession.FillRectangle(bounds, preferences.Theme.Selection); }
            e.DrawingSession.DrawTextLayout(layout, -scroll, 5, preferences.Theme.StatusBackground);
            int active = map.Utf16(checked((int)state.Active)) + 1; var p = layout.GetCaretPosition(active, false);
            if (caret) e.DrawingSession.DrawLine(p.X - scroll, 5, p.X - scroll, (float)ActualHeight - 5, preferences.Theme.StatusBackground, 1.5f);
        };
        canvas.PointerPressed += (_, e) => {
            if (pane.View == null || layout == null) return;
            pane.FocusEditor(); state = pane.View.Prompt(); anchor = Hit(e.GetCurrentPoint(canvas).Position); dragging = true; canvas.CapturePointer(e.Pointer);
            pane.Run(() => pane.View.EditPrompt(state, anchor, anchor)); e.Handled = true;
        };
        canvas.PointerMoved += (_, e) => { if (dragging && pane.View != null) { ulong active = Hit(e.GetCurrentPoint(canvas).Position); pane.Run(() => pane.View.EditPrompt(pane.View.Prompt(), anchor, active)); } };
        canvas.PointerReleased += (_, e) => { dragging = false; canvas.ReleasePointerCapture(e.Pointer); };
        canvas.PointerCaptureLost += (_, _) => dragging = false;
        SizeChanged += (_, _) => Refresh();
    }
    private ulong Hit(global::Windows.Foundation.Point point)
    {
        if (layout == null || state == null) return 0;
        layout.HitTest(new Vector2((float)point.X + scroll, (float)point.Y - 5), out var region, out bool trailing);
        int position = Math.Clamp(region.CharacterIndex + (trailing ? region.CharacterCount : 0) - 1, 0, state.Text.Length);
        return (ulong)Encoding.UTF8.GetByteCount(state.Text.AsSpan(0, position));
    }
    public void Refresh()
    {
        if (pane.View == null || ActualWidth <= 0) return;
        state = pane.View.Prompt();
        layout?.Dispose();
        using var format = new CanvasTextFormat { FontFamily = "Consolas", FontSize = 13, WordWrapping = CanvasWordWrapping.NoWrap };
        string prefix = state.Info.identity.kind switch { 1 => ":", 2 => "/", 3 => "?", _ => "" };
        layout = new(canvas.Device, prefix + state.Text, format, 1_000_000, 100);
        float x = layout.GetCaretPosition(new Utf8IndexMap(state.Text).Utf16(checked((int)state.Active)) + prefix.Length, false).X;
        if (x - scroll > ActualWidth - 8) scroll = x - (float)ActualWidth + 8;
        if (x - scroll < 0) scroll = x;
        canvas.Invalidate();
    }
    public void Blink(bool visible) { caret = visible; canvas.Invalidate(); }
    public void Dispose() { layout?.Dispose(); canvas.RemoveFromVisualTree(); }
}
