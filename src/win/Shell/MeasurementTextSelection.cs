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
    private bool selectOnRelease;
    private uint? pointer;
    private global::Windows.Foundation.Point press;
    private object? pendingSelection;

    // Collapsed inspector tabs can load their templates repeatedly.
    internal static void Attach(TextBox input) => attached.GetValue(input, value => new(value));

    private MeasurementTextSelection(TextBox input)
    {
        this.input = input;
        input.GotFocus += (_, _) => selectOnRelease = input.FocusState == FocusState.Pointer;
        input.LostFocus += (_, _) => Reset();
        input.Unloaded += (_, _) => Reset();
        input.TextChanged += (_, _) => Reset();
        input.AddHandler(UIElement.KeyDownEvent, new KeyEventHandler((_, _) => Reset()), true);
        input.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler(Pressed), true);
        input.AddHandler(UIElement.PointerMovedEvent, new PointerEventHandler(Moved), true);
        input.AddHandler(UIElement.PointerReleasedEvent, new PointerEventHandler(Released), true);
        input.AddHandler(UIElement.PointerCanceledEvent, new PointerEventHandler((_, _) => Reset()), true);
    }

    private void Reset()
    {
        selectOnRelease = false;
        pointer = null;
        pendingSelection = null;
    }

    private void Pressed(object sender, PointerRoutedEventArgs args)
    {
        pendingSelection = null;
        var point = args.GetCurrentPoint(input);
        if (!point.Properties.IsLeftButtonPressed) { Reset(); return; }
        pointer = args.Pointer.PointerId;
        press = point.Position;
    }

    private void Moved(object sender, PointerRoutedEventArgs args)
    {
        if (pointer != args.Pointer.PointerId || !selectOnRelease) return;
        var point = args.GetCurrentPoint(input).Position;
        // A deliberate drag keeps the TextBox's native selection, even on entry.
        if (Math.Abs(point.X - press.X) >= 4 || Math.Abs(point.Y - press.Y) >= 4)
            selectOnRelease = false;
    }

    private void Released(object sender, PointerRoutedEventArgs args)
    {
        bool select = selectOnRelease && pointer == args.Pointer.PointerId;
        Reset();
        if (!select) return;
        object request = new();
        pendingSelection = request;
        string value = input.Text;
        // GotFocus occurs during pointer press. Selecting there races WinUI's
        // release handling, which can place the caret again. Do this only once
        // the completed click has returned, without touching later clicks/keys.
        input.DispatcherQueue.TryEnqueue(() => {
            if (!ReferenceEquals(pendingSelection, request)) return;
            pendingSelection = null;
            if (input.IsLoaded && input.FocusState == FocusState.Pointer && input.Text == value)
                input.SelectAll();
        });
    }
}
