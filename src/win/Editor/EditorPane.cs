using System.Numerics;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Viem.Windows.Core;
using Viem.Windows.Input;
using Viem.Windows.Interop;
using Viem.Windows.Shell;
using Windows.ApplicationModel.DataTransfer;
using Windows.Foundation;
using Windows.System;
using Windows.UI;
using Windows.UI.Core;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane : Grid, IDisposable
{
    public CoreDocument Document { get; private set; }
    public CoreView? View { get; private set; }
    internal ulong RememberedArgument { get; set; } = ulong.MaxValue;
    internal ulong? InitialHeightLines { get; set; }
    internal bool InitialVerticalSplit { get; set; }
    internal Point WindowFocusPoint { get { try { if (View != null) { var caret = View.CaretGeometry(); var viewport = View.Viewport; return new(caret.rect.x - viewport.left, caret.rect.y - viewport.top); } } catch { } return new(ActualWidth / 2, ActualHeight / 2); } }
    private readonly EditorWindow window;
    private readonly Preferences preferences;
    internal CanvasControl Canvas { get; } = new();
    private readonly TextBox input = new() { Width = 2, Height = 24, MinWidth = 0, MinHeight = 0, Opacity = 0.01, AcceptsReturn = true, IsSpellCheckEnabled = false, IsTextPredictionEnabled = false, Padding = new(0), BorderThickness = new(0) };
    private readonly Canvas inputLayer = new() { IsHitTestVisible = false };
    private readonly ScrollBar vertical = new() { Orientation = Orientation.Vertical, Width = 14, SmallChange = 30 };
    private readonly ScrollBar horizontal = new() { Orientation = Orientation.Horizontal, Height = 14, SmallChange = 30, Visibility = Visibility.Collapsed };
    private readonly DragCursorGrid status = new() { Height = 28, ColumnSpacing = 5 };
    private readonly TextBlock mode = new() { VerticalAlignment = VerticalAlignment.Center, FontSize = 12 };
    private readonly TextBlock location = new() { VerticalAlignment = VerticalAlignment.Center, FontSize = 12 };
    private readonly TextBlock message = new() { VerticalAlignment = VerticalAlignment.Center, FontSize = 12, TextTrimming = TextTrimming.CharacterEllipsis };
    private readonly CommandPrompt prompt;
    private readonly ListView completionList = new() { MaxHeight = 220, Width = 280, IsItemClickEnabled = true, IsTabStop = false };
    private readonly Border completionBorder = new() { Visibility = Visibility.Collapsed, BorderThickness = new(1), CornerRadius = new(4) };
    private CompletionState? completion;
    private WhitespaceExport? whitespace;
    internal bool WhitespaceEnabled => whitespace?.Enabled ?? true;
    private (string Text, string Fragment)? clipboardOverride;
    private readonly DispatcherTimer blink = new();
    private readonly DispatcherTimer mapping = new() { Interval = TimeSpan.FromSeconds(1) };
    private bool disposed, refreshing, scrollUpdating, inputUpdating, textCaptureQueued, composing, compositionRejected, dragging, caretVisible = true;
    private bool active;
    public bool IsActive { get => active; set { active = value; Canvas.Invalidate(); } }
    private LayoutSnapshot? snapshot;
    private ViemViewPresentationV1 presentation;
    private ViemViewportStateV1 viewport;
    private Rect caretRect;
    private Point dragPoint, pointerPress;
    private uint? pressedPointer;
    private Task inputQueue = Task.CompletedTask;
    public event Action<EditorPane>? Focused;
    internal Exception? LastError { get; private set; }
    private string lastLayoutWarning = "";
    private string lastConfigurationWarning = "";
    private readonly HashSet<string> optionalPresentationWarnings = [];
    private readonly TaskCompletionSource<CoreView> ready = new(TaskCreationOptions.RunContinuationsAsynchronously);
    internal Task<CoreView> Ready => View is { } current ? Task.FromResult(current) : ready.Task;
    [DllImport("user32.dll")] private static extern uint GetCaretBlinkTime();

    public EditorPane(EditorWindow window, CoreDocument document, Preferences preferences)
    {
        using var startup = Diagnostics.StartupPerformance.Measure("pane.initialize");
        this.window = window; Document = document; this.preferences = preferences;
        prompt = new(this, preferences) { Visibility = Visibility.Collapsed };
        RowDefinitions.Add(new() { Height = new(1, GridUnitType.Star) }); RowDefinitions.Add(new() { Height = GridLength.Auto }); RowDefinitions.Add(new() { Height = GridLength.Auto });
        ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        Children.Add(Canvas); Children.Add(inputLayer); inputLayer.Children.Add(input);
        completionBorder.Child = completionList; inputLayer.Children.Add(completionBorder);
        completionList.ItemClick += (_, e) => { if (View != null && completion != null) { int index = Array.IndexOf(completion.Items, (string)e.ClickedItem); Run(() => View.AcceptCompletion(completion, index)); FocusEditor(); } };
        Children.Add(vertical); SetColumn(vertical, 1);
        Children.Add(horizontal); SetRow(horizontal, 1);
        Children.Add(status); SetRow(status, 2); SetColumnSpan(status, 2);
        BuildStatus();
        AutomationProperties.SetName(input, "Viem document input"); AutomationProperties.SetName(Canvas, "Document");
        AutomationProperties.SetName(vertical, "Vertical document scroll"); AutomationProperties.SetName(horizontal, "Horizontal document scroll");
        inputLayer.IsHitTestVisible = true; input.IsHitTestVisible = false;
        Canvas.CreateResources += (_, _) => { try { Attach(); } catch (Exception error) { ready.TrySetException(error); Report(error); } };
        Canvas.Draw += (_, args) => Run(() => {
            using (Diagnostics.StartupPerformance.Measure("editor.draw")) Draw(args.DrawingSession);
            if (snapshot != null && Diagnostics.StartupPerformance.Enabled)
                Diagnostics.StartupPerformance.FirstDraw(DispatcherQueue, Document.FilePath, new() {
                    width = Canvas.ActualWidth, height = Canvas.ActualHeight, top = viewport.top,
                    firstBaseline = snapshot.Rows.FirstOrDefault().baseline - viewport.top,
                    canvasY = Canvas.TransformToVisual(null).TransformPoint(new Point()).Y,
                    revision = viewport.layout_revision, configuration = viewport.configuration_generation,
                    shaped = View!.Provider.ShapedCharacters,
                    fontMetadataReads = View.Provider.FontMetadataReads,
                    glyphBoundsQueries = View.Provider.GlyphBoundsQueries, glyphBoundsHits = View.Provider.GlyphBoundsHits });
        });
        Canvas.SizeChanged += (_, _) => { if (View != null) Run(() => { using var timing = Diagnostics.StartupPerformance.Measure("editor.resize"); View.Resize((float)Canvas.ActualWidth, (float)Canvas.ActualHeight); }); };
        Canvas.PointerPressed += OnPointerPressed;
        Canvas.PointerMoved += OnPointerMoved;
        Canvas.PointerReleased += (_, e) => {
            if (pressedPointer != e.Pointer.PointerId) return;
            pressedPointer = null; dragging = false; Canvas.ReleasePointerCapture(e.Pointer);
        };
        Canvas.PointerCaptureLost += (_, e) => {
            if (pressedPointer == e.Pointer.PointerId) { pressedPointer = null; dragging = false; }
        };
        Canvas.DoubleTapped += (_, e) => Run(() => {
            uint returnMode = View?.Presentation.mode ?? VIEM_MODE_NORMAL;
            var p = e.GetPosition(Canvas); View?.Key(VIEM_KEY_ESCAPE); View?.Place((float)p.X, (float)p.Y);
            View?.SelectFromCommand("viw", VIEM_SELECTION_ORIGIN_MOUSE, returnMode); e.Handled = true;
        });
        Canvas.PointerWheelChanged += (_, e) => {
            if (View == null) return; var p = e.GetCurrentPoint(Canvas); bool ctrl = Down(VirtualKey.Control);
            Run(() => { if (ctrl) View.StepZoom(p.Properties.MouseWheelDelta > 0); else { var v = View.Viewport; float delta = -p.Properties.MouseWheelDelta / 120f * 60; View.Scroll(v.left + ((p.Properties.IsHorizontalMouseWheel || Down(VirtualKey.Shift)) ? delta : 0), v.top + ((!p.Properties.IsHorizontalMouseWheel && !Down(VirtualKey.Shift)) ? delta : 0)); } }); e.Handled = true;
        };
        Canvas.AllowDrop = true;
        Canvas.DragOver += (_, e) => { if (e.DataView.Contains(StandardDataFormats.StorageItems)) e.AcceptedOperation = DataPackageOperation.Copy; };
        Canvas.Drop += async (_, e) => { try { if (e.DataView.Contains(StandardDataFormats.StorageItems)) { var items = await e.DataView.GetStorageItemsAsync(); foreach (var item in items) if (File.Exists(item.Path)) await window.OpenNative(item.Path); } } catch (Exception error) { Report(error); } };
        input.GotFocus += (_, _) => { outputHadFocus = false; Focused?.Invoke(this); ResetBlink(); _ = RefreshClipboard(false); };
        input.LostFocus += (_, _) => { caretVisible = true; Canvas.Invalidate(); };
        input.PreviewKeyDown += OnKey;
        // TextChanging reliably signals input even for this nearly invisible
        // IME host. Drain after the native callback, coalescing a burst of text
        // without clearing the TextBox inside its own edit notification.
        input.TextChanging += (_, _) => {
            if (inputUpdating || composing || View == null || input.Text.Length == 0 || textCaptureQueued) return;
            textCaptureQueued = true;
            DispatcherQueue.TryEnqueue(CaptureCommittedText);
        };
        input.TextCompositionStarted += (_, _) => {
            composing = true; compositionRejected = false;
            if (View is { } view && (view.Presentation.mode is VIEM_MODE_INSERT or VIEM_MODE_REPLACE || view.IsTextSelection))
                Run(() => { try { view.BeginComposition(); } catch { compositionRejected = true; throw; } });
        };
        input.TextCompositionChanged += (_, _) => { if (View?.Composing == true) Run(() => View.UpdateComposition(input.Text, input.SelectionStart, input.SelectionLength)); };
        input.TextCompositionEnded += (_, _) => {
            string text = input.Text; bool rejected = compositionRejected;
            composing = false; compositionRejected = false; ClearInput();
            if (rejected) return;
            Enqueue(() => { if (View?.Composing == true) View.CommitComposition(text); else DeliverText(text); return Task.CompletedTask; });
        };
        vertical.Scroll += (_, e) => ScrollVerticallyFromScrollbar(e.NewValue);
        horizontal.Scroll += (_, _) => { if (!scrollUpdating && View != null) Run(() => View.Scroll((float)horizontal.Value, View.Viewport.top)); };
        uint blinkMs = GetCaretBlinkTime();
        blink.Interval = TimeSpan.FromMilliseconds(blinkMs is 0 or uint.MaxValue ? 530 : Math.Max(100, blinkMs));
        blink.Tick += (_, _) => { if (active && input.FocusState != FocusState.Unfocused && blinkMs != uint.MaxValue) { caretVisible = !caretVisible; Canvas.Invalidate(); prompt.Blink(caretVisible); } };
        blink.Start();
        mapping.Tick += (_, _) => { mapping.Stop(); if (View?.HasPendingMapping == true) Run(() => View.FlushMapping()); };
        preferences.Changed += PreferencesChanged; Document.Changed += DocumentChanged;
        Clipboard.ContentChanged += ClipboardChanged;
        var context = new MenuFlyout();
        foreach (var (title, action) in new (string, Func<Task>)[] { ("Cut", () => Copy(true)), ("Copy", () => Copy(false)), ("Copy Source", CopySource), ("Paste", () => Paste()), ("Paste and Match Style", () => Paste(true)), ("Select All", () => { SelectAll(); return Task.CompletedTask; }) })
        { var item = new MenuFlyoutItem { Text = title }; item.Click += (_, _) => Enqueue(action); context.Items.Add(item); }
        Canvas.ContextFlyout = context;
        ApplyTheme();
    }
    private void Attach()
    {
        using var startup = Diagnostics.StartupPerformance.Measure("editor.attach");
        InvalidateDrawingCache();
        if (View != null) { View.Provider.ResetDevice(Canvas.Device); View.Resize((float)Canvas.ActualWidth, (float)Canvas.ActualHeight); return; }
        var padding = new ViemLayoutInsetsV1 { top = preferences.Margin("top"), left = preferences.Margin("left"), bottom = preferences.Margin("bottom"), right = preferences.Margin("right") };
        using (Diagnostics.StartupPerformance.Measure("editor.createView")) View = new(Document, Canvas.Device, DispatcherQueue, (float)Canvas.ActualWidth, (float)Canvas.ActualHeight, padding);
        View.Changed += Refresh; View.Effects += ApplyEffects;
        ApplyPreferences(); Refresh(); if (IsActive) FocusEditor();
        ready.TrySetResult(View);
        window.PaneReady(this);
    }
    internal CoreView? PrepareReplacement(CoreDocument document)
    {
        if (View == null) return null;
        var padding = new ViemLayoutInsetsV1 { top = preferences.Margin("top"), left = preferences.Margin("left"), bottom = preferences.Margin("bottom"), right = preferences.Margin("right") };
        var prepared = new CoreView(document, Canvas.Device, DispatcherQueue, (float)Canvas.ActualWidth, (float)Canvas.ActualHeight, padding);
        try
        {
            var viewport = View.Viewport;
            prepared.Wrap((viewport.flags & VIEM_VIEWPORT_STATE_WRAP) != 0);
            prepared.Zoom(viewport.scale);
            prepared.LineMode(View.CurrentLineMode);
            if (document.State.format == VIEM_FORMAT_MARKDOWN_SOURCE) prepared.ParagraphFlow(View.ParagraphFlowEnabled);
            prepared.MarkdownAutodetect(preferences.MarkdownAutodetect);
            prepared.SmartQuotes(preferences.SmartQuotes);
            prepared.ClipboardText = View.ClipboardText;
            prepared.ClipboardFragment = View.ClipboardFragment;
            prepared.ClipboardGeneration = View.ClipboardGeneration;
            prepared.RestorePosition(View);
            return prepared;
        }
        catch { prepared.Dispose(); throw; }
    }
    internal void InstallReplacement(CoreDocument document, CoreView? prepared)
    {
        Document.Changed -= DocumentChanged;
        if (View != null) { View.Changed -= Refresh; View.Effects -= ApplyEffects; View.Dispose(); }
        Document = document; View = prepared;
        Document.Changed += DocumentChanged;
        if (View != null) { View.Changed += Refresh; View.Effects += ApplyEffects; }
        mapping.Stop(); compositionRejected = composing; composing = false; ClearInput();
        snapshot = null; InvalidateDrawingCache(); Refresh();
    }
    private void DocumentChanged() { if (View != null) Run(() => { View.Refresh(); }); }
    private void PreferencesChanged() { ApplyTheme(); if (View != null) Run(ApplyPreferences); }
    private void ApplyPreferences()
    {
        Document.ConfigureEditingDefaults(preferences.Indentation, preferences.Whitespace, preferences.TextWidth, preferences.Associations);
        View!.Padding(preferences.Margin("top"), preferences.Margin("left"), preferences.Margin("bottom"), preferences.Margin("right")); View.SmartQuotes(preferences.SmartQuotes); View.MarkdownAutodetect(preferences.MarkdownAutodetect);
    }
    private void ApplyTheme()
    {
        InvalidateDrawingCache();
        var theme = preferences.Theme;
        Background = new SolidColorBrush(theme.Background); status.Background = new SolidColorBrush(theme.StatusBackground);
        status.BorderBrush = PaneStackPanel.SeparatorBrush(preferences); status.BorderThickness = new(0, 1, 0, 0);
        mode.Foreground = message.Foreground = location.Foreground = new SolidColorBrush(theme.StatusForeground);
        foreach (var label in new[] { mode, message, location }) { label.FontFamily = new FontFamily(preferences.StatusFontFamily); label.FontSize = preferences.StatusFontSize; }
        status.Height = Math.Max(28, preferences.StatusFontSize + 12);
        ApplyStatusTheme();
        Canvas.Invalidate();
    }
    public void FocusEditor() { if (!disposed) input.Focus(FocusState.Programmatic); }
    public void Report(Exception error) { LastError = error; Diagnostics.StartupPerformance.Failed(error); ShowCommandOutput(
        error is CoreException core && core.Status == VIEM_STATUS_PROVIDER_FAILURE && View?.Provider.LastError is string detail
            ? error.Message + Environment.NewLine + detail : error.Message); }
    public void Run(Action action) { try { action(); } catch (Exception e) { Report(e); } }
    public void SetMessage(string text) { ShowCommandOutput(text); }
    private void ResetBlink() { caretVisible = true; blink.Stop(); blink.Start(); Canvas.Invalidate(); prompt.Blink(true); }
    private void ClearInput() { inputUpdating = true; input.Text = ""; inputUpdating = false; }
    private void CaptureCommittedText()
    {
        textCaptureQueued = false;
        if (disposed || inputUpdating || composing || View == null || input.Text.Length == 0) return;
        string text = input.Text;
        // WM_CHAR delivers supplementary characters as two UTF-16 messages.
        // Keep an unfinished pair in the input host until its low surrogate.
        if (char.IsHighSurrogate(text[^1])) return;
        ClearInput();
        Enqueue(() => { DeliverText(text); return Task.CompletedTask; });
    }
    private static bool Down(VirtualKey key) => (InputKeyboardSource.GetKeyStateForCurrentThread(key) & CoreVirtualKeyStates.Down) != 0;
    private void DeliverText(string text)
    {
        if (View == null) return;
        uint mode = View.Presentation.mode;
        if (mode is VIEM_MODE_INSERT or VIEM_MODE_REPLACE or VIEM_MODE_COMMAND_LINE || CoreView.IsTextSelectionMode(mode)) View.Text(text);
        else foreach (var rune in text.EnumerateRunes()) View.Key(VIEM_KEY_CHARACTER, (uint)rune.Value);
    }
    private void OnKey(object sender, KeyRoutedEventArgs e)
    {
        if (View == null) return;
        var key = e.Key; bool control = Down(VirtualKey.Control), shift = Down(VirtualKey.Shift), alt = Down(VirtualKey.Menu);
        var route = KeyPolicy.Route(key, control, shift, alt);
        if (composing && route.Action == NativeAction.None) return; // Native IME owns composition navigation/commit.
        if (route.Kind == 0 && route.Action == NativeAction.None) return;
        e.Handled = true;
        // Commit earlier characters before Escape, Backspace, Enter, etc.
        CaptureCommittedText();
        Enqueue(async () => {
            // A preceding queued key may have entered literal-next input.
            route = KeyPolicy.Route(key, control, shift, alt, (View.Presentation.flags & VIEM_VIEW_PRESENTATION_LITERAL_INPUT_PENDING) != 0);
            switch (route.Action)
            {
                case NativeAction.Copy: await Copy(false); break;
                case NativeAction.Cut: await Copy(true); break;
                case NativeAction.Paste: case NativeAction.PastePlain: await Paste(route.Action == NativeAction.PastePlain); break;
                case NativeAction.Undo: View.Undo(); break;
                case NativeAction.Redo: View.Redo(); break;
                case NativeAction.Styles: window.ShowStyles(); break;
                case NativeAction.Save: await window.Save(this); break;
                case NativeAction.SaveAs: await window.Save(this, true); break;
                case NativeAction.Heading: View.SetParagraph(route.Codepoint); break;
                default: if (route.Kind != 0) View.Key(route.Kind, route.Codepoint, route.Modifiers); break;
            }
        });
    }
    private void Enqueue(Func<Task> action)
    {
        async Task Next(Task previous)
        {
            await previous;
            if (disposed) return;
            try { message.Text = ""; DismissCommandOutput(false); await action(); ResetBlink(); if (View?.HasPendingMapping == true) { mapping.Stop(); mapping.Start(); } }
            catch (Exception e) { Report(e); }
        }
        inputQueue = Next(inputQueue);
    }
    private async void ClipboardChanged(object? sender, object e) { await RefreshClipboard(false); }
    private async Task<bool> RefreshClipboard(bool reportFailure = true)
    {
        if (View == null) return false;
        try {
            var data = await ClipboardFormats.Read();
            if (View == null) return false; View.ClipboardText = data.Text; View.ClipboardFragment = data.Fragment; View.ClipboardGeneration++;
            return true;
        } catch (Exception e) { if (reportFailure) SetMessage("Clipboard unavailable: " + e.Message); return false; }
    }
    public Task Copy(bool cut)
    {
        if (OutputIsActionTarget) { if (!cut && outputText.SelectedText.Length > 0) ClipboardFormats.Write(outputText.SelectedText, ""); return Task.CompletedTask; }
        if (View == null) return Task.CompletedTask;
        if (View.Presentation.mode == VIEM_MODE_COMMAND_LINE)
        {
            var p = View.Prompt(); ulong start = Math.Min(p.Anchor, p.Active), end = Math.Max(p.Anchor, p.Active);
            if (start != end) { byte[] bytes = Encoding.UTF8.GetBytes(p.Text); ClipboardFormats.Write(Encoding.UTF8.GetString(bytes, (int)start, (int)(end - start)), ""); if (cut) View.EditPrompt(p, start, end, ""); }
        }
        else if (View.HasSelection)
        {
            var selection = View.Selection();
            if (selection.Segments.Length == 1)
            {
                var segment = selection.Segments[0]; string fragment = Encoding.UTF8.GetString(View.ClipboardJson(segment.text_start, segment.text_end));
                using var json = System.Text.Json.JsonDocument.Parse(fragment);
                clipboardOverride = (json.RootElement.GetProperty("plain_text").GetString()!, fragment);
            }
            try { View.CopyOrCut(cut); } finally { clipboardOverride = null; }
        }
        return Task.CompletedTask;
    }
    public bool CanCopy => OutputIsActionTarget ? outputText.SelectedText.Length > 0 : View?.HasSelection == true || View?.Presentation.mode == VIEM_MODE_COMMAND_LINE && View.Prompt() is var p && p.Anchor != p.Active;
    public bool CanCut => !OutputIsActionTarget && CanCopy;
    public void SelectAll()
    {
        if (OutputIsActionTarget) { outputText.Focus(FocusState.Programmatic); outputText.SelectAll(); return; }
        if (View?.Presentation.mode == VIEM_MODE_COMMAND_LINE) { var p = View.Prompt(); View.EditPrompt(p, 0, (ulong)Encoding.UTF8.GetByteCount(p.Text)); }
        else View?.SelectAll();
        FocusEditor();
    }
    public Task CopySource()
    {
        if (OutputIsActionTarget) return Copy(false);
        if (View?.Presentation.mode == VIEM_MODE_COMMAND_LINE) return Copy(false);
        if (View?.HasSelection != true) return Task.CompletedTask;
        var fragments = new List<string>(); var selection = View.Selection();
        foreach (var range in selection.Segments)
        {
            using var value = System.Text.Json.JsonDocument.Parse(View.ClipboardJson(range.text_start, range.text_end));
            var root = value.RootElement; string source = root.GetProperty("source_text").GetString()!;
            if (source.Length == 0 && root.GetProperty("plain_text").GetString()!.Length > 0) throw new InvalidOperationException("The selection has no editable source fragment.");
            fragments.Add(source);
        }
        ClipboardFormats.Write(string.Join(selection.Info.identity.kind == VIEM_VISUAL_SELECTION_KIND_BLOCK ? "\n" : "", fragments), "");
        return Task.CompletedTask;
    }
    public async Task Paste(bool plain = false) { if (await RefreshClipboard()) View?.Paste(plain); }
    private void ApplyEffects(HostEffects effects)
    {
        foreach (var write in effects.Clipboard)
        {
            var data = clipboardOverride ?? (write.Text, write.Fragment);
            try {
                ClipboardFormats.Write(data.Text, data.Fragment);
                View!.ClipboardText = data.Text; View.ClipboardFragment = data.Fragment; View.ClipboardGeneration++;
            } catch (Exception error) {
                // Core may already have committed the command. Keep its undo
                // and finish publishing the new frame; never replay the input.
                Report(error);
            }
        }
        _ = window.ApplyEffects(this, effects);
    }
    private void OnPointerPressed(object sender, PointerRoutedEventArgs e)
    {
        if (View == null || !e.GetCurrentPoint(Canvas).Properties.IsLeftButtonPressed) return;
        DismissCommandOutput(false);
        FocusEditor(); var point = e.GetCurrentPoint(Canvas).Position;
        Run(() => View.Place((float)point.X, (float)point.Y, Down(VirtualKey.Shift)));
        pointerPress = dragPoint = point; pressedPointer = e.Pointer.PointerId; dragging = false;
        if (!Canvas.CapturePointer(e.Pointer)) pressedPointer = null;
        e.Handled = true; ResetBlink();
    }
    private void OnPointerMoved(object sender, PointerRoutedEventArgs e)
    {
        if (pressedPointer != e.Pointer.PointerId || View == null) return;
        var point = e.GetCurrentPoint(Canvas);
        if (!point.Properties.IsLeftButtonPressed)
        {
            pressedPointer = null; dragging = false; Canvas.ReleasePointerCapture(e.Pointer); return;
        }
        dragPoint = point.Position;
        if (!dragging)
        {
            // WinUI can report movement at the press point. Match the pane-bar
            // drag threshold so a click or small jitter keeps the editing mode.
            if (Math.Abs(dragPoint.X - pointerPress.X) < 4 && Math.Abs(dragPoint.Y - pointerPress.Y) < 4) return;
            dragging = true;
        }
        Run(() => View.Place((float)Math.Clamp(dragPoint.X, 0, Canvas.ActualWidth), (float)Math.Clamp(dragPoint.Y, 0, Canvas.ActualHeight), true));
        e.Handled = true;
    }
    internal ScrollBar VerticalScrollControl => vertical;
    internal ScrollBar HorizontalScrollControl => horizontal;
    internal Rect CaretRectangle => caretRect;

    internal void ScrollVerticallyFromScrollbar(double value)
    {
        if (scrollUpdating || View == null) return;
        // Estimated document heights cannot identify the actual last row.
        // Let core materialize and clamp at the real end when the thumb reaches it.
        float top = vertical.Maximum > 0 && value >= vertical.Maximum ? float.MaxValue : (float)value;
        Run(() => View.Scroll(View.Viewport.left, top));
    }

    public void Refresh()
    {
        using var measurement = Diagnostics.InputPerformance.Measure("refresh");
        if (View == null || refreshing || disposed) return;
        refreshing = true;
        try
        {
            // The command/status surface must update even when document
            // geometry is temporarily unavailable (for example after scrolling).
            presentation = View.Presentation;
            message.Text = View.SubstitutePrompt();
            UpdateStatusPresentation();
            try { using var layoutMeasurement = Diagnostics.InputPerformance.Measure("layout.export"); snapshot = View.Layout(); }
            catch (CoreException e) when (e.Status == VIEM_STATUS_LAYOUT_UNAVAILABLE) { View.Resize((float)Canvas.ActualWidth, (float)Canvas.ActualHeight); snapshot = View.Layout(); }
            presentation = View.Presentation; viewport = View.Viewport;
            mode.Text = presentation.mode switch {
                VIEM_MODE_INSERT => "INSERT", VIEM_MODE_REPLACE => "REPLACE",
                VIEM_MODE_VISUAL_CHARACTER => "VISUAL", VIEM_MODE_VISUAL_LINE => "V-LINE", VIEM_MODE_VISUAL_BLOCK => "V-BLOCK",
                VIEM_MODE_SELECTION_CHARACTER or VIEM_MODE_SELECTION_LINE or VIEM_MODE_SELECTION_BLOCK => "SELECTION",
                VIEM_MODE_SELECT_CHARACTER => "SELECT", VIEM_MODE_SELECT_LINE => "S-LINE", VIEM_MODE_SELECT_BLOCK => "S-BLOCK",
                VIEM_MODE_COMMAND_LINE => "COMMAND", _ => "NORMAL"
            };
            UpdateLocation();
            scrollUpdating = true;
            vertical.Maximum = viewport.maximum_top; vertical.ViewportSize = Math.Max(1, snapshot.Info.viewport_height); vertical.LargeChange = Math.Max(1, snapshot.Info.viewport_height * .9); vertical.Value = viewport.top;
            horizontal.Maximum = viewport.maximum_left; horizontal.ViewportSize = Math.Max(1, Canvas.ActualWidth); horizontal.Value = viewport.left;
            horizontal.Visibility = viewport.maximum_left > 0 ? Visibility.Visible : Visibility.Collapsed;
            scrollUpdating = false;
            caretRect = CalculateCaret();
            try { using (Diagnostics.InputPerformance.Measure("whitespace.export")) whitespace = View.Whitespace(snapshot.Info); }
            catch (CoreException error) { whitespace = null; PresentationWarning("whitespace markers", error); }
            try { RefreshCompletion(); }
            catch (CoreException error) { completion = null; completionBorder.Visibility = Visibility.Collapsed; PresentationWarning("completion popup", error); }
            Microsoft.UI.Xaml.Controls.Canvas.SetLeft(input, Math.Clamp(caretRect.X, 0, Math.Max(0, Canvas.ActualWidth - 2)));
            Microsoft.UI.Xaml.Controls.Canvas.SetTop(input, Math.Clamp(caretRect.Y, 0, Math.Max(0, Canvas.ActualHeight - 24)));
            input.Height = Math.Max(16, caretRect.Height);
            Canvas.Invalidate(); window.UpdateTitle();
            if (snapshot.Diagnostics != lastLayoutWarning) {
                lastLayoutWarning = snapshot.Diagnostics;
                if (lastLayoutWarning.Length > 0) SetMessage(lastLayoutWarning);
            }
            if (Document.ConfigurationDiagnostics != lastConfigurationWarning) {
                lastConfigurationWarning = Document.ConfigurationDiagnostics;
                if (lastConfigurationWarning.Length > 0) SetMessage(lastConfigurationWarning);
            }
        }
        catch (Exception e) {
            // A previous source/device snapshot cannot become the new frame
            // merely because exporting its replacement failed.
            snapshot = null; whitespace = null; caretRect = new();
            InvalidateDrawingCache(); Canvas.Invalidate(); Report(e);
        }
        finally { refreshing = false; scrollUpdating = false; }
    }
    private void PresentationWarning(string name, CoreException error)
    {
        string warning = $"Could not display {name}. {error.Message}";
        if (optionalPresentationWarnings.Count >= 8) optionalPresentationWarnings.Clear();
        if (optionalPresentationWarnings.Add(warning)) SetMessage(warning);
    }
    private void RefreshCompletion()
    {
        var next = View!.Completion();
        completionBorder.Visibility = (next.Info.flags & (VIEM_COMPLETION_ACTIVE | VIEM_COMPLETION_HAS_ANCHOR)) == (VIEM_COMPLETION_ACTIVE | VIEM_COMPLETION_HAS_ANCHOR) ? Visibility.Visible : Visibility.Collapsed;
        if (completionBorder.Visibility == Visibility.Collapsed) { completion = null; return; }
        if (completion == null || completion.Info.generation != next.Info.generation) completionList.ItemsSource = next.Items;
        completionList.SelectedIndex = (int)next.Info.selected_index;
        completion = next;
        var anchor = next.Info.anchor_rect;
        completionBorder.Background = new SolidColorBrush(preferences.Theme.StatusBackground); completionBorder.BorderBrush = new SolidColorBrush(preferences.Theme.StatusForeground);
        Microsoft.UI.Xaml.Controls.Canvas.SetLeft(completionBorder, Math.Clamp(anchor.x - viewport.left, 0, Math.Max(0, Canvas.ActualWidth - 280)));
        double y = anchor.y + anchor.height - viewport.top;
        if (y + 220 > Canvas.ActualHeight) y = Math.Max(0, anchor.y - viewport.top - 220);
        Microsoft.UI.Xaml.Controls.Canvas.SetTop(completionBorder, y);
    }
    private Rect CalculateCaret()
    {
        if (View == null || snapshot == null) return new();
        if (!View.Composing && presentation.caret_shape == VIEM_CARET_SHAPE_CELL)
        {
            var cluster = snapshot.Clusters.FirstOrDefault(c => c.text_start <= presentation.caret_utf8_start && c.text_end > presentation.caret_utf8_start);
            if (cluster.text_end > cluster.text_start) return OffsetRect(cluster.typographic_bounds, viewport);
        }
        try
        {
            var caret = View.CaretGeometry(); var rect = OffsetRect(caret.rect, viewport);
            bool boundary = View.Composing || presentation.caret_shape == VIEM_CARET_SHAPE_BOUNDARY;
            return new(rect.X, rect.Y, boundary ? 1.5 : Math.Max(6, rect.Height * .45), Math.Max(1, rect.Height));
        }
        catch (CoreException error) when (error.Status == VIEM_STATUS_OUTSIDE_LAYOUT_COVERAGE)
        { return new(); } // A manually scrolled viewport need not contain the caret.
    }
    private static Color Color(ViemRgbaV1 c) => global::Windows.UI.Color.FromArgb((byte)Math.Clamp(c.alpha * 255, 0, 255), (byte)Math.Clamp(c.red * 255, 0, 255), (byte)Math.Clamp(c.green * 255, 0, 255), (byte)Math.Clamp(c.blue * 255, 0, 255));
    private static Rect OffsetRect(ViemLayoutRectV1 r, ViemViewportStateV1 v) => new(r.x - v.left, r.y - v.top, Math.Max(0, r.width), Math.Max(0, r.height));
    internal void Draw(CanvasDrawingSession drawing)
    {
        using var measurement = Diagnostics.InputPerformance.Measure("draw");
        var theme = preferences.Theme;
        drawing.Clear(snapshot != null && (snapshot.Paint.flags & VIEM_LAYOUT_PAINT_DEFAULT_CANVAS) == 0 ? Color(snapshot.Paint.canvas_background) : theme.Background);
        if (snapshot == null || View == null) return;
        EnsureDrawingCache();
        // Source highlights, selection, then text: preserve the original layering.
        var scrollOffset = new System.Numerics.Vector2(drawnViewport.left - viewport.left, drawnViewport.top - viewport.top);
        drawing.DrawImage(cachedBackground, scrollOffset);
        foreach (var rectangle in snapshot.Selection) drawing.FillRectangle(OffsetRect(rectangle.rect, viewport), theme.Selection);
        drawing.DrawImage(cachedText, scrollOffset);
        bool focused = active && window.IsWindowActive && input.FocusState != FocusState.Unfocused;
        if (caretRect.Height > 0 && presentation.mode != VIEM_MODE_COMMAND_LINE && (!focused || caretVisible))
        {
            Rect caret = caretRect;
            if (presentation.mode == VIEM_MODE_REPLACE && !View.Composing) caret = new(caret.X, caret.Bottom - 2, Math.Max(6, caret.Width), 2);
            if (!focused) { var color = theme.Caret; color.A = 191; drawing.DrawRectangle(caret, color, 1); }
            else if (View.Composing || presentation.caret_shape == VIEM_CARET_SHAPE_BOUNDARY || presentation.mode == VIEM_MODE_REPLACE) drawing.FillRectangle(caret, theme.Caret);
            else if (snapshot.Clusters.Any(c => c.text_start < presentation.caret_utf8_end && c.text_end > presentation.caret_utf8_start && View.Provider.IsColorGlyph(c.render_run)))
            {
                var translucent = theme.Caret; translucent.A = 85; drawing.FillRectangle(caret, translucent); drawing.DrawRectangle(caret, theme.Caret, 1);
            }
            else
            {
                drawing.FillRectangle(caret, theme.Caret);
                using var clip = drawing.CreateLayer(1, caret);
                double L(byte b) { double v = b / 255.0; return v <= .04045 ? v / 12.92 : Math.Pow((v + .055) / 1.055, 2.4); }
                double luminance = .2126 * L(theme.Caret.R) + .7152 * L(theme.Caret.G) + .0722 * L(theme.Caret.B);
                var contrast = (luminance + .05) / .05 >= 1.05 / (luminance + .05) ? Microsoft.UI.Colors.Black : Microsoft.UI.Colors.White;
                foreach (var c in snapshot.Clusters)
                {
                    var ink = OffsetRect(c.ink_bounds, viewport);
                    if (ink.Right < caret.X - 1 || ink.X > caret.Right + 1 || ink.Bottom < caret.Y - 1 || ink.Y > caret.Bottom + 1) continue;
                    var row = snapshot.Rows.First(r => r.row_index == c.row_index);
                    View.Provider.Draw(drawing, c.render_run, new(c.x - viewport.left, row.baseline - viewport.top), contrast);
                }
            }
        }
    }
    private ViemTextPaintV1 PaintFor(ulong offset)
    {
        var runs = snapshot!.PaintRuns;
        int lo = 0, hi = runs.Length;
        while (lo < hi) { int mid = lo + (hi - lo) / 2; if (runs[mid].text_start <= offset) lo = mid + 1; else hi = mid; }
        return lo > 0 && runs[lo - 1].text_end > offset ? runs[lo - 1].paint : snapshot.Paint.default_paint;
    }
    public void Poll()
    {
        if (View == null) return;
        Run(() => {
            View.Poll();
            if (!dragging) return;
            double dy = dragPoint.Y < 0 ? Math.Max(-60, dragPoint.Y) : dragPoint.Y > Canvas.ActualHeight ? Math.Min(60, dragPoint.Y - Canvas.ActualHeight) : 0;
            double dx = dragPoint.X < 0 ? Math.Max(-60, dragPoint.X) : dragPoint.X > Canvas.ActualWidth ? Math.Min(60, dragPoint.X - Canvas.ActualWidth) : 0;
            if (dx == 0 && dy == 0) return;
            View.Scroll(View.Viewport.left + (float)dx, View.Viewport.top + (float)dy);
            View.Place((float)Math.Clamp(dragPoint.X, 0, Canvas.ActualWidth), (float)Math.Clamp(dragPoint.Y, 0, Canvas.ActualHeight), true);
        });
    }
    public void Dispose()
    {
        if (disposed) return; disposed = true; blink.Stop(); mapping.Stop(); outputTimer.Stop();
        preferences.Changed -= PreferencesChanged; Document.Changed -= DocumentChanged; Clipboard.ContentChanged -= ClipboardChanged;
        InvalidateDrawingCache();
        View?.Dispose(); View = null; prompt.Dispose(); status.Dispose(); Canvas.RemoveFromVisualTree();
    }
}
