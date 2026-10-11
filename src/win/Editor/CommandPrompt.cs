using System.Numerics;
using System.Text;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Input;
using Viem.Windows.Core;
using Viem.Windows.Rendering;
using Viem.Windows.Shell;

namespace Viem.Windows.Editor;

internal sealed partial class CommandPrompt : Grid, IDisposable
{
    private readonly CanvasControl canvas = new();
    private readonly EditorPane pane;
    private readonly Preferences preferences;
    private CanvasTextLayout? layout;
    private PromptState? state;
    private float scroll;
    private ChromeStyle? fontStyle;
    private string displayText = "";
    private bool resourcesReady;
    private bool dragging, caret = true;
    private ulong anchor;
    public CommandPrompt(EditorPane pane, Preferences preferences)
    {
        this.pane = pane; this.preferences = preferences; Children.Add(canvas);
        AutomationProperties.SetName(this, "Command line");
        canvas.CreateResources += (_, _) => { resourcesReady = true; layout?.Dispose(); layout = null; pane.Run(Refresh); };
        canvas.Draw += (_, e) => pane.Run(() => {
            var style = pane.CommandStyle;
            e.DrawingSession.Clear(style.Background);
            if (layout == null || state == null) return;
            float top = ((float)ActualHeight - (float)layout.LayoutBounds.Height) / 2;
            float left = pane.CommandInset - scroll;
            var map = new Utf8IndexMap(state.Text);
            int prefixLength = Prefix(state).Length;
            int start = map.Utf16(checked((int)Math.Min(state.Anchor, state.Active))) + prefixLength, end = map.Utf16(checked((int)Math.Max(state.Anchor, state.Active))) + prefixLength;
            if (start != end)
                foreach (var r in layout.GetCharacterRegions(start, end - start))
                { var bounds = r.LayoutBounds; bounds.X += left; bounds.Y += top; e.DrawingSession.FillRectangle(bounds, preferences.Theme.Selection); }
            e.DrawingSession.DrawTextLayout(layout, left, top, style.Foreground);
            int active = map.Utf16(checked((int)state.Active)) + prefixLength; var p = layout.GetCaretPosition(active, false);
            if (caret) e.DrawingSession.DrawLine(p.X + left, top, p.X + left, top + (float)layout.LayoutBounds.Height, style.Foreground, 2);
#if DEBUG
            LastDrawnText = displayText;
#endif
        });
        canvas.PointerPressed += (_, e) => {
            if (pane.View == null || layout == null) return;
            pane.FocusEditor(); state = pane.View.Prompt(); anchor = Hit(e.GetCurrentPoint(canvas).Position); dragging = true; canvas.CapturePointer(e.Pointer);
            pane.Run(() => pane.View.EditPrompt(state, anchor, anchor)); e.Handled = true;
        };
        canvas.PointerMoved += (_, e) => { if (dragging && pane.View != null) { ulong active = Hit(e.GetCurrentPoint(canvas).Position); pane.Run(() => pane.View.EditPrompt(pane.View.Prompt(), anchor, active)); } };
        canvas.PointerReleased += (_, e) => { dragging = false; canvas.ReleasePointerCapture(e.Pointer); };
        canvas.PointerCaptureLost += (_, _) => dragging = false;
        SizeChanged += (_, _) => pane.Run(Refresh);
    }
    private ulong Hit(global::Windows.Foundation.Point point)
    {
        if (layout == null || state == null) return 0;
        float top = ((float)ActualHeight - (float)layout.LayoutBounds.Height) / 2;
        layout.HitTest(new Vector2((float)(point.X - pane.CommandInset) + scroll, (float)point.Y - top), out var region, out bool trailing);
        int position = Math.Clamp(region.CharacterIndex + (trailing ? region.CharacterCount : 0) - Prefix(state).Length, 0, state.Text.Length);
        return (ulong)Encoding.UTF8.GetByteCount(state.Text.AsSpan(0, position));
    }
    public void Refresh()
    {
        if (pane.View == null) return;
        state = pane.View.Prompt();
        string prefix = Prefix(state);
        if (prefix.Length == 0) { layout?.Dispose(); layout = null; displayText = ""; scroll = 0; return; }
        // A newly shown CanvasControl may not have a device or width yet.
        // CreateResources/SizeChanged will render the latest core prompt when ready.
        if (!resourcesReady || ActualWidth <= 0) return;
        string text = prefix + state.Text;
        var style = pane.CommandStyle;
        if (layout == null || displayText != text || fontStyle?.FontIdentity != style.FontIdentity)
        {
            layout?.Dispose();
            layout = style.Layout(canvas.Device, text);
            displayText = text; fontStyle = style;
        }
        float x = layout.GetCaretPosition(new Utf8IndexMap(state.Text).Utf16(checked((int)state.Active)) + prefix.Length, false).X;
        float available = Math.Max(1, (float)(ActualWidth - pane.CommandInset - pane.CommandRightInset));
        scroll = Math.Min(scroll, Math.Max(0, (float)layout.LayoutBounds.Width - available));
        if (x - scroll > available) scroll = x - available;
        if (x - scroll < 0) scroll = x;
        canvas.Invalidate();
    }
    private static string Prefix(PromptState value) => value.Info.identity.kind switch { 1 => ":", 2 => "/", 3 => "?", _ => "" };
#if DEBUG
    internal string LastDrawnText { get; private set; } = "";
    internal float ScrollOffset => scroll;
#endif
    public void Blink(bool visible) { caret = visible; canvas.Invalidate(); }
    public void Dispose() { layout?.Dispose(); canvas.RemoveFromVisualTree(); }
}
