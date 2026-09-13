import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVCommandQuitApplicationTests: XCTestCase {
    private func drainCommandClose() {
        let done = expectation(description: "deferred close processed")
        DispatchQueue.main.async { done.fulfill() }
        wait(for: [done], timeout: 1)
    }

    func testQuitChecksRemainingDocumentWindowsAndDoesNotLatchOrdinaryClose() {
        let delegate = EVApplicationDelegate()
        var documentOpen = true
        var terminations = 0
        delegate.hasOpenDocumentWindows = { documentOpen }
        delegate.applicationWindows = { [] }
        delegate.terminateApplication = { terminations += 1 }
        delegate.terminateAfterCommandClose()
        documentOpen = false
        drainCommandClose()
        XCTAssertEqual(terminations, 0, "Even an immediate ordinary close must not inherit the command's intent")
        XCTAssertFalse(delegate.applicationShouldTerminateAfterLastWindowClosed(NSApplication.shared))
        drainCommandClose()
        XCTAssertEqual(terminations, 0, "A later stoplight close must not inherit the command's intent")
        delegate.terminateAfterCommandClose()
        XCTAssertEqual(terminations, 0, "Finish the command and window close before quitting")
        drainCommandClose()
        XCTAssertEqual(terminations, 1)
    }

    func testOtherOrdinaryWindowsAndNewlyOpenedDocumentsPreventTermination() {
        let delegate = EVApplicationDelegate()
        let settings = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 200, height: 100),
            styleMask: [.titled], backing: .buffered, defer: false)
        settings.isReleasedWhenClosed = false
        settings.animationBehavior = .none
        settings.orderFront(nil)
        defer { settings.close() }
        var documentOpen = false
        var terminations = 0
        delegate.hasOpenDocumentWindows = { documentOpen }
        delegate.applicationWindows = { [settings] }
        delegate.terminateApplication = { terminations += 1 }
        delegate.terminateAfterCommandClose()
        drainCommandClose()
        XCTAssertEqual(terminations, 0)
        settings.orderOut(nil)
        delegate.terminateAfterCommandClose()
        documentOpen = true
        drainCommandClose()
        XCTAssertEqual(terminations, 0, "Check current windows when the deferred action runs")
    }
}
