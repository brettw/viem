using System.Runtime.CompilerServices;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;

namespace Viem.Windows.Shell;

/// <summary>Select a measurement on its first click, after native caret placement finishes.</summary>
internal sealed class MeasurementTextSelection
{
    private static readonly ConditionalWeakTable<TextBox, MeasurementTextSelection> attached = new();
    private readonly TextBox input;
    private readonly NumberBox owner;
    private bool ownerFocusAttached;
    private sealed class PointerGesture(string text)
    {
        internal readonly string Text = text;
        internal bool Released, Moved, Cancelled, CaptureQueued, NativeFocusReset;
        internal (int Start, int Length)? DragSelection;
    }
    private PointerGesture? gesture;
    private bool focused;
    private object? focusBeforePress;
    private bool selectOnRelease;
    private uint? pointer;
    private global::Windows.Foundation.Point press;
    private object? pendingSelection;
#if DEBUG
    private int focusEvents, ownerFocusEvents, blurEvents, pressEvents, releaseEvents;
    internal static string StateForTesting(TextBox input) => attached.TryGetValue(input, out var state)
        ? $"focusEvents={state.focusEvents}, ownerFocusEvents={state.ownerFocusEvents}, blurs={state.blurEvents}, presses={state.pressEvents}, releases={state.releaseEvents}, selectOnRelease={state.selectOnRelease}, pointer={state.pointer}, pending={state.pendingSelection != null}, focused={state.focused}, focusBeforePress={state.focusBeforePress != null}, gestureReleased={state.gesture?.Released}, moved={state.gesture?.Moved}, cancelled={state.gesture?.Cancelled}, dragSelection={state.gesture?.DragSelection}"
        : "not attached";
#endif

    // Collapsed inspector tabs can load their templates repeatedly.
    internal static void Attach(TextBox input, NumberBox owner) => attached.GetValue(input, value => new(value, owner));

    private MeasurementTextSelection(TextBox input, NumberBox owner)
    {
        this.input = input; this.owner = owner;
        void AttachOwnerFocus() { if (!ownerFocusAttached) { owner.GotFocus += OwnerFocused; ownerFocusAttached = true; } }
        if (input.IsLoaded) AttachOwnerFocus();
        input.Loaded += (_, _) => AttachOwnerFocus();
        input.GotFocus += (_, _) => {
#if DEBUG
            focusEvents++;
#endif
            if (input.XamlRoot is not { } root || !ReferenceEquals(FocusManager.GetFocusedElement(root), input)) return;
            if (focused) return;
            focused = true;
            if (input.FocusState != FocusState.Pointer) return;
            if (gesture == null) {
                object entry = new(); focusBeforePress = entry;
                // Focus may precede the routed press in this dispatch. If no
                // owned press follows, it must not arm a later click.
                input.DispatcherQueue.TryEnqueue(() => { if (ReferenceEquals(focusBeforePress, entry)) focusBeforePress = null; });
                return;
            }
            if (gesture.Cancelled || gesture.Moved) return;
            // WinUI can route entry focus after the pointer release. Finish
            // that click without arming the next click's release handler.
            if (gesture.Released) SelectAfterGesture(gesture);
            else if (pointer != null) selectOnRelease = true;
        };
        input.LostFocus += (_, _) => {
#if DEBUG
            blurEvents++;
#endif
            Reset(); focused = false; gesture = null;
        };
        input.Unloaded += (_, _) => {
            Reset(); focused = false; gesture = null;
            if (ownerFocusAttached) { owner.GotFocus -= OwnerFocused; ownerFocusAttached = false; }
        };
        input.TextChanged += (_, _) => Reset();
        input.AddHandler(UIElement.KeyDownEvent, new KeyEventHandler((_, _) => Reset()), true);
        input.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler(Pressed), true);
        input.AddHandler(UIElement.PointerMovedEvent, new PointerEventHandler(Moved), true);
        input.AddHandler(UIElement.PointerReleasedEvent, new PointerEventHandler(Released), true);
        input.AddHandler(UIElement.PointerCanceledEvent, new PointerEventHandler((_, _) => Reset()), true);
    }

    private void Reset()
    {
        if (gesture != null) gesture.Cancelled = true;
        focusBeforePress = null;
        selectOnRelease = false;
        pointer = null;
        pendingSelection = null;
    }

    private void Pressed(object sender, PointerRoutedEventArgs args)
    {
#if DEBUG
        pressEvents++;
#endif
        pendingSelection = null;
        var point = args.GetCurrentPoint(input);
        if (!point.Properties.IsLeftButtonPressed) { Reset(); return; }
        gesture = new(input.Text);
        selectOnRelease = focusBeforePress != null;
        focusBeforePress = null;
        pointer = args.Pointer.PointerId;
        press = point.Position;
    }

    private void Moved(object sender, PointerRoutedEventArgs args)
    {
        if (pointer != args.Pointer.PointerId || gesture == null) return;
        pendingSelection = null;
        var point = args.GetCurrentPoint(input).Position;
        // A deliberate drag keeps the TextBox's native selection, even on entry.
        if (Math.Abs(point.X - press.X) >= 4 || Math.Abs(point.Y - press.Y) >= 4) {
            gesture.Moved = true;
            selectOnRelease = false;
        }
        if (gesture.Moved) {
            var current = gesture;
            current.NativeFocusReset = false;
            CaptureDragSelection(current);
            // Read after native routed-move handling too, but never sample a
            // later focus reset after this pointer has already been released.
            if (!current.CaptureQueued) {
                current.CaptureQueued = true;
                input.DispatcherQueue.TryEnqueue(() => {
                    current.CaptureQueued = false;
                    if (ReferenceEquals(gesture, current) && !current.Released && !current.NativeFocusReset) CaptureDragSelection(current);
                });
            }
        }
    }

    private void CaptureDragSelection(PointerGesture current)
    {
        if (current.Cancelled || input.Text != current.Text) return;
        int start = input.SelectionStart, length = input.SelectionLength;
        current.DragSelection = length > 0 && length < input.Text.Length ? (start, length) : null;
    }

    private void OwnerFocused(object sender, RoutedEventArgs args)
    {
#if DEBUG
        ownerFocusEvents++;
#endif
        // NumberBox's built-in GotFocus handler selects all, and runs after
        // the inner TextBox event. Preserve a real entry drag after that native
        // initialization, rather than guessing its anchor from character hits.
        if (gesture is not { Moved: true, Cancelled: false } current) return;
        current.NativeFocusReset = true;
        if (current.DragSelection == null) return;
        object request = new(); pendingSelection = request;
        input.DispatcherQueue.TryEnqueue(() => {
            if (!ReferenceEquals(pendingSelection, request)) return;
            pendingSelection = null;
            if (ReferenceEquals(gesture, current) && !current.Cancelled && input.Text == current.Text
                && input.IsLoaded && input.FocusState == FocusState.Pointer
                && ReferenceEquals(FocusManager.GetFocusedElement(input.XamlRoot), input)
                && current.DragSelection is { } selection)
                input.Select(selection.Start, selection.Length);
        });
    }

    private void Released(object sender, PointerRoutedEventArgs args)
    {
#if DEBUG
        releaseEvents++;
#endif
        if (pointer != args.Pointer.PointerId || gesture == null) return;
        bool select = selectOnRelease && !gesture.Cancelled && !gesture.Moved;
        pointer = null; selectOnRelease = false; focusBeforePress = null;
        gesture.Released = true;
        if (select) SelectAfterGesture(gesture);
    }

    private void SelectAfterGesture(PointerGesture completed)
    {
        object request = new();
        pendingSelection = request;
        string value = input.Text;
        // Selecting during pointer focus can race native caret placement on
        // release. Finish only this completed click, never later clicks/keys.
        input.DispatcherQueue.TryEnqueue(() => {
            if (!ReferenceEquals(pendingSelection, request)) return;
            pendingSelection = null;
            if (ReferenceEquals(gesture, completed) && !completed.Cancelled && !completed.Moved
                && input.IsLoaded && input.FocusState == FocusState.Pointer && input.Text == value)
                input.SelectAll();
        });
    }
}
