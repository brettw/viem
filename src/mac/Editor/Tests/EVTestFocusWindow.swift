import AppKit

/// SwiftPM's test process need not own the foreground application. Model window
/// focus deterministically while delivering the same notifications as AppKit;
/// production code still reads NSWindow.isKeyWindow and the real first responder.
@MainActor
final class EVTestFocusWindow: NSWindow {
    private var simulatedKeyWindow = false
    override var isKeyWindow: Bool { simulatedKeyWindow }

    func setKeyWindowForTesting(_ value: Bool) {
        guard simulatedKeyWindow != value else { return }
        simulatedKeyWindow = value
        NotificationCenter.default.post(
            name: value ? NSWindow.didBecomeKeyNotification : NSWindow.didResignKeyNotification,
            object: self)
    }
}
