using DispatcherQueuePriority = Microsoft.UI.Dispatching.DispatcherQueuePriority;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;

namespace Viem.Windows.Editor;

internal sealed partial class EditorPane
{
    private sealed class PopupFocusRequest(ButtonBase origin, FrameworkElement popup, UIElement keyRoot)
    {
        internal readonly ButtonBase Origin = origin;
        internal readonly FrameworkElement Popup = popup;
        internal readonly UIElement KeyRoot = keyRoot;
        internal KeyEventHandler? KeyReleased;
        internal KeyEventHandler? NextKey;
        internal PointerEventHandler? PointerPressed;
        internal RoutedEventHandler? FocusLost;
        internal RoutedEventHandler? Unloaded;
    }
    private PopupFocusRequest? popupFocus;
#if DEBUG
    internal string PopupFocusDiagnosticForTesting { get; private set; } = "none";
#endif
    [System.Diagnostics.Conditional("DEBUG")]
    private void TracePopupFocus(string value)
    {
#if DEBUG
        PopupFocusDiagnosticForTesting = value;
#endif
    }
    [System.Diagnostics.Conditional("DEBUG")]
    private void TracePopupFieldFocus(string requested, bool accepted)
    {
#if DEBUG
        var current = XamlRoot is { } root ? FocusManager.GetFocusedElement(root) : null;
        PopupFocusDiagnosticForTesting += $"; requested={requested}, FocusResult={accepted}, immediate={current?.GetType().Name}/"
            + (current is DependencyObject element ? AutomationProperties.GetName(element) : "none");
#endif
    }

    private bool HasPendingPopupFocus(FrameworkElement? popup) => popup != null
        && popupFocus is { } pending && ReferenceEquals(pending.Popup, popup)
        && IsLoaded && popup.IsLoaded && pending.Origin.IsLoaded && XamlRoot is { } root
        && ReferenceEquals(popup.XamlRoot, root) && ReferenceEquals(pending.Origin.XamlRoot, root)
        && ReferenceEquals(FocusManager.GetFocusedElement(XamlRoot), pending.Origin);

    private void CancelPopupFocus(FrameworkElement? popup)
    {
        if (popupFocus is not { } pending || !ReferenceEquals(pending.Popup, popup)) return;
        popupFocus = null;
        if (pending.KeyReleased != null) pending.KeyRoot.RemoveHandler(UIElement.KeyUpEvent, pending.KeyReleased);
        if (pending.NextKey != null) {
            pending.Origin.PreviewKeyDown -= pending.NextKey;
            pending.Popup.PreviewKeyDown -= pending.NextKey;
        }
        if (pending.PointerPressed != null) pending.Popup.RemoveHandler(UIElement.PointerPressedEvent, pending.PointerPressed);
        if (pending.FocusLost != null) {
            pending.Origin.LostFocus -= pending.FocusLost;
            pending.Popup.LostFocus -= pending.FocusLost;
        }
        if (pending.Unloaded != null) {
            pending.Origin.Unloaded -= pending.Unloaded;
            pending.Popup.Unloaded -= pending.Unloaded;
            Unloaded -= pending.Unloaded;
        }
    }

    private void FocusPopupField(FrameworkElement popup, Func<bool> isCurrent, Action focus)
    {
        CancelPopupFocus(popupFocus?.Popup);
        if (XamlRoot is not { } root) return;
        if (FocusManager.GetFocusedElement(root) is not ButtonBase origin) {
            TracePopupFocus("immediate (origin is not a button)");
            if (isCurrent()) focus();
            return;
        }
        // Keep the activation's character messages on its native button. Enter
        // may invoke on key-down; Space normally invokes on key-up. Drain the
        // matching release before moving focus to an editable popup field.
        if (root.Content is not { } keyRoot) return;
        var pending = new PopupFocusRequest(origin, popup, keyRoot);
        popupFocus = pending;
        bool FocusIsOwned()
        {
            if (disposed || !IsLoaded || !origin.IsLoaded || !popup.IsLoaded || !ReferenceEquals(XamlRoot, root)
                || !ReferenceEquals(origin.XamlRoot, root) || !ReferenceEquals(popup.XamlRoot, root)) return false;
            var focused = FocusManager.GetFocusedElement(root) as DependencyObject;
            if (ReferenceEquals(focused, origin)) return true;
            for (var element = focused; element != null; element = VisualTreeHelper.GetParent(element))
                if (ReferenceEquals(element, popup)) return true;
            return false;
        }
        void Complete()
        {
            if (!ReferenceEquals(popupFocus, pending)) return;
            bool sameRoots = ReferenceEquals(XamlRoot, root) && ReferenceEquals(origin.XamlRoot, root) && ReferenceEquals(popup.XamlRoot, root);
            bool current = isCurrent();
            bool ownedFocus = sameRoots && FocusIsOwned();
            bool valid = !disposed && IsLoaded && origin.IsLoaded && popup.IsLoaded && sameRoots
                && window.IsWindowActive && current && ownedFocus;
            TracePopupFocus($"complete valid={valid}, disposed={disposed}, loaded pane/origin/popup={IsLoaded}/{origin.IsLoaded}/{popup.IsLoaded}, "
                + $"roots={sameRoots}, active={window.IsWindowActive}, current={current}, ownedFocus={ownedFocus}");
            CancelPopupFocus(popup);
            if (valid) Run(focus);
        }
        void EnqueueFocus()
        {
            TracePopupFocus("queued");
            if (!DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Normal, Complete)) CancelPopupFocus(popup);
        }
        pending.FocusLost = (_, _) => {
            // Editable ComboBox setup can restore focus within this same form.
            // Observe its completed focus move before deciding ownership was lost.
            DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () => {
                if (ReferenceEquals(popupFocus, pending) && !FocusIsOwned()) {
                    TracePopupFocus("focus left origin and popup"); CancelPopupFocus(popup);
                }
            });
        };
        pending.NextKey = (_, _) => { if (ReferenceEquals(popupFocus, pending)) { TracePopupFocus("subsequent keyboard input"); CancelPopupFocus(popup); } };
        pending.PointerPressed = (_, _) => { if (ReferenceEquals(popupFocus, pending)) { TracePopupFocus("explicit pointer focus in popup"); CancelPopupFocus(popup); } };
        pending.Unloaded = (_, _) => { if (ReferenceEquals(popupFocus, pending)) { TracePopupFocus("pane, origin or popup unloaded"); CancelPopupFocus(popup); } };
        origin.LostFocus += pending.FocusLost;
        popup.LostFocus += pending.FocusLost;
        origin.PreviewKeyDown += pending.NextKey;
        popup.PreviewKeyDown += pending.NextKey;
        popup.AddHandler(UIElement.PointerPressedEvent, pending.PointerPressed, true);
        origin.Unloaded += pending.Unloaded;
        popup.Unloaded += pending.Unloaded;
        Unloaded += pending.Unloaded;
        VirtualKey? held = Down(VirtualKey.Space) ? VirtualKey.Space : Down(VirtualKey.Enter) ? VirtualKey.Enter : null;
        if (held is { } activation) {
            TracePopupFocus("awaiting release of " + activation);
            void Released()
            {
                if (!ReferenceEquals(popupFocus, pending) || pending.KeyReleased == null) return;
                pending.KeyRoot.RemoveHandler(UIElement.KeyUpEvent, pending.KeyReleased);
                pending.KeyReleased = null;
                EnqueueFocus();
            }
            pending.KeyReleased = (_, args) => {
                if (args.Key != activation || !ReferenceEquals(popupFocus, pending)) return;
                Released();
            };
            pending.KeyRoot.AddHandler(UIElement.KeyUpEvent, pending.KeyReleased, true);
            // Click can run inside the release whose routed phase has already
            // passed. Recheck once after that native activation has unwound.
            if (!DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Normal, () => { if (!Down(activation)) Released(); }))
                CancelPopupFocus(popup);
        } else EnqueueFocus();
    }
}
