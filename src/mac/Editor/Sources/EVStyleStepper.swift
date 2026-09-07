import AppKit

/// Native autorepeat actions share one core style gesture while the mouse is
/// held. Keyboard and accessibility actions remain independent undo units.
@MainActor
final class EVStyleStepper: NSStepper {
    var onGestureBegan: (() -> Void)?
    var onGestureEnded: (() -> Void)?
    private var trackingGesture = false

    override var acceptsFirstResponder: Bool { true }

    override func mouseDown(with event: NSEvent) {
        guard isEnabled else { return }
        window?.makeFirstResponder(self)
        performTrackingGesture { super.mouseDown(with: event) }
    }

    func performTrackingGesture(_ action: () -> Void) {
        guard !trackingGesture else { action(); return }
        trackingGesture = true
        onGestureBegan?()
        defer {
            onGestureEnded?()
            trackingGesture = false
        }
        action()
    }

    override func sendAction(_ action: Selector?, to target: Any?) -> Bool {
        guard isEnabled else { return false }
        var sent = false
        performTrackingGesture { sent = super.sendAction(action, to: target) }
        return sent
    }
}
