import AppKit
import XCTest

@testable import EvimEditor

final class EVCustomCaretBlinkControllerTests: XCTestCase {
    @MainActor
    func testActiveCaretBlinksWithInjectedClockAndStopCancelsIt() {
        let clock = TestCaretBlinkClock()
        let controller = makeController(clock: clock)
        var presentations: [EVCustomCaretPresentation] = []
        controller.onVisibilityChange = { presentations.append($0) }

        controller.start(active: true)

        XCTAssertTrue(controller.isStarted)
        XCTAssertTrue(controller.isActive)
        XCTAssertTrue(controller.isVisible)
        XCTAssertEqual(controller.presentation, .active)
        XCTAssertEqual(clock.activeDelays, [0.6])

        clock.fireNext()
        XCTAssertEqual(controller.presentation, .hidden)
        XCTAssertEqual(clock.activeDelays, [0.4])

        clock.fireNext()
        XCTAssertEqual(controller.presentation, .active)
        XCTAssertEqual(clock.activeDelays, [0.6])
        XCTAssertEqual(presentations, [.active, .hidden, .active])

        controller.stop()
        XCTAssertFalse(controller.isStarted)
        XCTAssertFalse(controller.isVisible)
        XCTAssertFalse(controller.isBlinkScheduled)
        XCTAssertTrue(clock.activeDelays.isEmpty)
        clock.fireNext()
        XCTAssertEqual(presentations, [.active, .hidden, .active, .hidden])
    }

    @MainActor
    func testActivityRestartsVisiblePhaseAndInactiveStateIsSteadyOutline() {
        let clock = TestCaretBlinkClock()
        let controller = makeController(clock: clock)
        var presentations: [EVCustomCaretPresentation] = []
        controller.onVisibilityChange = { presentations.append($0) }
        controller.start(active: true)
        clock.fireNext()
        XCTAssertEqual(controller.presentation, .hidden)

        controller.restartAfterActivity()

        XCTAssertEqual(controller.presentation, .active)
        XCTAssertEqual(clock.activeDelays, [0.6])

        controller.setActive(false)

        XCTAssertEqual(controller.presentation, .inactiveOutline)
        XCTAssertTrue(controller.isVisible)
        XCTAssertFalse(controller.isBlinkScheduled)
        XCTAssertTrue(clock.activeDelays.isEmpty)
        let callbacksBeforeInactiveActivity = presentations.count

        controller.restartAfterActivity()

        XCTAssertEqual(controller.presentation, .inactiveOutline)
        XCTAssertEqual(presentations.count, callbacksBeforeInactiveActivity + 1)
        XCTAssertTrue(clock.activeDelays.isEmpty)

        controller.setActive(true)

        XCTAssertEqual(controller.presentation, .active)
        XCTAssertEqual(clock.activeDelays, [0.6])
    }

    @MainActor
    func testReduceMotionMakesActiveCaretSteadyAndRestoresBlinkingWhenDisabled() {
        let clock = TestCaretBlinkClock()
        let center = NotificationCenter()
        let options = TestAccessibilityOptions(
            value: EVCustomCaretAccessibilityOptions(
                reduceMotion: false,
                differentiateWithoutColor: false
            )
        )
        let controller = makeController(
            clock: clock,
            center: center,
            options: options
        )
        var presentations: [EVCustomCaretPresentation] = []
        controller.onVisibilityChange = { presentations.append($0) }
        controller.start(active: true)
        XCTAssertTrue(controller.isBlinkScheduled)

        options.value = EVCustomCaretAccessibilityOptions(
            reduceMotion: true,
            differentiateWithoutColor: false
        )
        center.post(name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil)

        XCTAssertEqual(controller.presentation, .active)
        XCTAssertTrue(controller.isVisible)
        XCTAssertFalse(controller.isBlinkScheduled)
        XCTAssertTrue(clock.activeDelays.isEmpty)
        clock.fireNext()
        XCTAssertEqual(controller.presentation, .active)

        options.value = EVCustomCaretAccessibilityOptions(
            reduceMotion: false,
            differentiateWithoutColor: false
        )
        center.post(name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil)

        XCTAssertEqual(controller.presentation, .active)
        XCTAssertTrue(controller.isBlinkScheduled)
        XCTAssertEqual(clock.activeDelays, [0.6])
        XCTAssertEqual(presentations.last, .active)
    }

    @MainActor
    func testDifferentiateWithoutColorRefreshesPresentationWithoutChangingCadence() {
        let clock = TestCaretBlinkClock()
        let center = NotificationCenter()
        let options = TestAccessibilityOptions(
            value: EVCustomCaretAccessibilityOptions(
                reduceMotion: false,
                differentiateWithoutColor: false
            )
        )
        let controller = makeController(
            clock: clock,
            center: center,
            options: options
        )
        var presentations: [EVCustomCaretPresentation] = []
        controller.onVisibilityChange = { presentations.append($0) }
        controller.start(active: true)
        let callbackCountBeforeChange = presentations.count

        options.value = EVCustomCaretAccessibilityOptions(
            reduceMotion: false,
            differentiateWithoutColor: true
        )
        center.post(name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil)

        XCTAssertTrue(controller.accessibilityOptions.differentiateWithoutColor)
        XCTAssertEqual(controller.presentation, .active)
        XCTAssertEqual(presentations.count, callbackCountBeforeChange + 1)
        XCTAssertEqual(presentations.last, .active)
        XCTAssertEqual(clock.activeDelays, [0.6])

        clock.fireNext()
        XCTAssertEqual(controller.presentation, .hidden)
        XCTAssertEqual(clock.activeDelays, [0.4])
    }

    @MainActor
    func testStoppedControllerIgnoresActivityAndActiveStateUntilStarted() {
        let clock = TestCaretBlinkClock()
        let controller = makeController(clock: clock)
        var presentations: [EVCustomCaretPresentation] = []
        controller.onVisibilityChange = { presentations.append($0) }

        controller.setActive(true)
        controller.restartAfterActivity()

        XCTAssertEqual(controller.presentation, .hidden)
        XCTAssertTrue(presentations.isEmpty)
        XCTAssertTrue(clock.activeDelays.isEmpty)

        controller.start(active: false)

        XCTAssertEqual(controller.presentation, .inactiveOutline)
        XCTAssertTrue(clock.activeDelays.isEmpty)
    }

    @MainActor
    private func makeController(
        clock: TestCaretBlinkClock,
        center: NotificationCenter = NotificationCenter(),
        options: TestAccessibilityOptions? = nil
    ) -> EVCustomCaretBlinkController {
        let resolvedOptions = options ?? TestAccessibilityOptions(
            value: EVCustomCaretAccessibilityOptions(
                reduceMotion: false,
                differentiateWithoutColor: false
            )
        )
        return EVCustomCaretBlinkController(
            clock: clock,
            timing: EVCustomCaretBlinkTiming(visibleDuration: 0.6, hiddenDuration: 0.4),
            notificationCenter: center,
            accessibilityOptionsProvider: { resolvedOptions.value }
        )
    }
}

@MainActor
private final class TestAccessibilityOptions {
    var value: EVCustomCaretAccessibilityOptions

    init(value: EVCustomCaretAccessibilityOptions) {
        self.value = value
    }
}

@MainActor
private final class TestCaretBlinkClock: EVCustomCaretBlinkClock {
    private struct Entry {
        let delay: TimeInterval
        let token: TestCaretScheduledAction
        let action: @MainActor () -> Void
    }

    private var entries: [Entry] = []

    var activeDelays: [TimeInterval] {
        entries.filter { !$0.token.isCancelled }.map(\.delay)
    }

    @discardableResult
    func schedule(
        after delay: TimeInterval,
        _ action: @escaping @MainActor () -> Void
    ) -> EVCustomCaretScheduledAction {
        let token = TestCaretScheduledAction()
        entries.append(Entry(delay: delay, token: token, action: action))
        return token
    }

    func fireNext() {
        while !entries.isEmpty {
            let entry = entries.removeFirst()
            guard !entry.token.isCancelled else { continue }
            entry.action()
            return
        }
    }
}

@MainActor
private final class TestCaretScheduledAction: EVCustomCaretScheduledAction {
    private(set) var isCancelled = false

    func cancel() {
        isCancelled = true
    }
}
