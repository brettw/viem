import AppKit

enum EVCustomCaretPresentation: Equatable {
    /// No custom caret is drawn. This is the stopped state and the hidden
    /// phase of an active blink cycle.
    case hidden

    /// Draw the active mode-specific block or Replace underline.
    case active

    /// Draw the nonblinking hollow caret used by an inactive editor view.
    case inactiveOutline

    var isVisible: Bool { self != .hidden }
}

struct EVCustomCaretAccessibilityOptions: Equatable {
    let reduceMotion: Bool
    let differentiateWithoutColor: Bool

    @MainActor
    static var current: Self {
        let workspace = NSWorkspace.shared
        return Self(
            reduceMotion: workspace.accessibilityDisplayShouldReduceMotion,
            differentiateWithoutColor: workspace.accessibilityDisplayShouldDifferentiateWithoutColor
        )
    }
}

struct EVCustomCaretBlinkTiming: Equatable {
    let visibleDuration: TimeInterval
    let hiddenDuration: TimeInterval

    /// AppKit exposes no public custom-caret blink interval. Keep this explicit
    /// fallback local to custom block/underline carets instead of consulting a
    /// private defaults key. `NSTextInsertionIndicator` retains native timing.
    static let `default` = Self(visibleDuration: 0.5, hiddenDuration: 0.5)
}

@MainActor
protocol EVCustomCaretScheduledAction: AnyObject {
    /// Implementations must also cancel their underlying work when released.
    func cancel()
}

@MainActor
protocol EVCustomCaretBlinkClock: AnyObject {
    @discardableResult
    func schedule(
        after delay: TimeInterval,
        _ action: @escaping @MainActor () -> Void
    ) -> EVCustomCaretScheduledAction
}

@MainActor
final class EVRunLoopCustomCaretBlinkClock: EVCustomCaretBlinkClock {
    @discardableResult
    func schedule(
        after delay: TimeInterval,
        _ action: @escaping @MainActor () -> Void
    ) -> EVCustomCaretScheduledAction {
        let token = EVRunLoopCustomCaretScheduledAction()
        let timer = Timer(timeInterval: max(0, delay), repeats: false) { [weak token] _ in
            MainActor.assumeIsolated {
                guard token?.isCancelled == false else { return }
                action()
            }
        }
        token.install(timer)
        RunLoop.main.add(timer, forMode: .common)
        return token
    }
}

@MainActor
private final class EVRunLoopCustomCaretScheduledAction: EVCustomCaretScheduledAction {
    private var timer: Timer?
    private(set) var isCancelled = false

    func install(_ timer: Timer) {
        guard !isCancelled else {
            timer.invalidate()
            return
        }
        self.timer = timer
    }

    func cancel() {
        isCancelled = true
        timer?.invalidate()
        timer = nil
    }

    deinit {
        timer?.invalidate()
    }
}

/// Owns timing and presentation state only for eVim's custom Normal/Visual
/// block caret and Replace underline. It must never drive the native vertical
/// `NSTextInsertionIndicator` used by Insert and command-line modes.
///
/// Host lifecycle:
///
/// - Enter a custom-caret mode with `start(active:)`.
/// - On resigning first responder, call `setActive(false)`. This cancels blink
///   scheduling while keeping the required hollow inactive outline visible.
/// - On becoming active, call `setActive(true)`. After every key event, cursor
///   move, or custom-caret mode/geometry change, call `restartAfterActivity()`
///   so the caret becomes visible and receives a fresh blink interval.
/// - On leaving custom-caret modes, view removal, or teardown, call `stop()`.
///   Insert/command-line mode should then use only `NSTextInsertionIndicator`.
///
/// `onVisibilityChange` may receive the same presentation twice when a public
/// accessibility display-option notification requires a redraw. The host can
/// read `accessibilityOptions` to adapt non-color visual treatment.
@MainActor
final class EVCustomCaretBlinkController {
    private let clock: EVCustomCaretBlinkClock
    private let timing: EVCustomCaretBlinkTiming
    private let notificationCenter: NotificationCenter
    private let accessibilityOptionsProvider: @MainActor () -> EVCustomCaretAccessibilityOptions
    private var displayOptionsObserver: NSObjectProtocol?
    private var scheduledAction: EVCustomCaretScheduledAction?
    private var cycleGeneration: UInt64 = 0

    private(set) var isStarted = false
    private(set) var isActive = false
    private(set) var presentation: EVCustomCaretPresentation = .hidden
    private(set) var accessibilityOptions: EVCustomCaretAccessibilityOptions

    /// Called on a presentation transition and again when display options make
    /// the existing custom caret need repainting.
    var onVisibilityChange: ((EVCustomCaretPresentation) -> Void)?

    var isVisible: Bool { presentation.isVisible }
    var isBlinkScheduled: Bool { scheduledAction != nil }

    init(
        clock: EVCustomCaretBlinkClock? = nil,
        timing: EVCustomCaretBlinkTiming = .default,
        notificationCenter: NotificationCenter = NSWorkspace.shared.notificationCenter,
        accessibilityOptionsProvider: @escaping @MainActor () -> EVCustomCaretAccessibilityOptions = {
            .current
        }
    ) {
        precondition(timing.visibleDuration > 0 && timing.hiddenDuration > 0)
        self.clock = clock ?? EVRunLoopCustomCaretBlinkClock()
        self.timing = timing
        self.notificationCenter = notificationCenter
        self.accessibilityOptionsProvider = accessibilityOptionsProvider
        accessibilityOptions = accessibilityOptionsProvider()
        displayOptionsObserver = notificationCenter.addObserver(
            forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.accessibilityDisplayOptionsDidChange() }
        }
    }

    deinit {
        if let displayOptionsObserver {
            notificationCenter.removeObserver(displayOptionsObserver)
        }
    }

    /// Enables custom-caret presentation. Active carets become visible and
    /// begin a fresh blink cycle; inactive carets remain a steady outline.
    func start(active: Bool = true) {
        isStarted = true
        isActive = active
        restartCycle(forceCallback: true)
    }

    /// Removes custom-caret presentation and cancels all scheduled work.
    func stop() {
        isStarted = false
        cancelScheduledAction()
        publish(.hidden)
    }

    /// Changes first-responder activity without losing the started mode.
    func setActive(_ active: Bool) {
        guard isActive != active else { return }
        isActive = active
        guard isStarted else {
            cancelScheduledAction()
            publish(.hidden)
            return
        }
        restartCycle(forceCallback: true)
    }

    /// Makes an active custom caret immediately visible and gives it a full new
    /// visible interval. Safe to call while inactive or stopped.
    func restartAfterActivity() {
        guard isStarted else { return }
        restartCycle(forceCallback: true)
    }

    private func restartCycle(forceCallback: Bool) {
        cancelScheduledAction()
        guard isStarted else {
            publish(.hidden, force: forceCallback)
            return
        }
        guard isActive else {
            publish(.inactiveOutline, force: forceCallback)
            return
        }
        publish(.active, force: forceCallback)
        guard isStarted, isActive, presentation == .active,
              !accessibilityOptions.reduceMotion
        else { return }
        scheduleNext(after: timing.visibleDuration)
    }

    private func scheduleNext(after delay: TimeInterval) {
        cycleGeneration &+= 1
        let generation = cycleGeneration
        scheduledAction = clock.schedule(after: delay) { [weak self] in
            guard let self, self.cycleGeneration == generation else { return }
            self.scheduledAction = nil
            self.advanceBlinkCycle()
        }
    }

    private func advanceBlinkCycle() {
        guard isStarted, isActive, !accessibilityOptions.reduceMotion else {
            restartCycle(forceCallback: false)
            return
        }
        if presentation == .active {
            publish(.hidden)
            guard isStarted, isActive, presentation == .hidden,
                  !accessibilityOptions.reduceMotion
            else { return }
            scheduleNext(after: timing.hiddenDuration)
        } else {
            publish(.active)
            guard isStarted, isActive, presentation == .active,
                  !accessibilityOptions.reduceMotion
            else { return }
            scheduleNext(after: timing.visibleDuration)
        }
    }

    private func cancelScheduledAction() {
        cycleGeneration &+= 1
        scheduledAction?.cancel()
        scheduledAction = nil
    }

    private func accessibilityDisplayOptionsDidChange() {
        let previous = accessibilityOptions
        accessibilityOptions = accessibilityOptionsProvider()
        if previous.reduceMotion != accessibilityOptions.reduceMotion {
            restartCycle(forceCallback: true)
        } else {
            // Differentiate-without-color and contrast/appearance changes do
            // not alter cadence, but the host must repaint its custom shape.
            publish(presentation, force: true)
        }
    }

    private func publish(_ next: EVCustomCaretPresentation, force: Bool = false) {
        guard force || next != presentation else { return }
        presentation = next
        onVisibilityChange?(next)
    }
}
