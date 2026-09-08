import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVDocumentCloseReviewTests: XCTestCase {
    private final class Surface: EVEditorSurface {
        let viewController: NSViewController = {
            let value = NSViewController()
            value.view = NSView(frame: NSRect(x: 0, y: 0, width: 920, height: 655))
            return value
        }()
        var statusBarState = EVStatusBarState()
        var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
        func perform(menuCommand: EVMenuCommand, sender: Any?) {}
        func presentation(for: EVMenuCommand) -> EVMenuItemPresentation { .enabled }
    }
    private final class Backend: EVDocumentBackend {
        var sourceDidChange: (() -> Void)?
        var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)?
        var persistenceState = EVDocumentPersistenceState(isDirty: true, documentID: 1, documentRevision: 1)
        var sourceFormat: EVSourceFormat = .plainText
        var bytes = Data("Unsaved words".utf8)
        var acknowledgements = 0
        func makeEditorSurface() -> any EVEditorSurface { Surface() }
        func read(source: Data, typeName: String) throws { bytes = source }
        func serializedSource(typeName: String) throws -> Data { bytes }
        func nativeSaveSnapshot(typeName: String) throws -> EVDocumentSaveSnapshot {
            EVDocumentSaveSnapshot(data: bytes, documentID: 1, documentRevision: 1)
        }
        func acknowledgeNativeSave(_ snapshot: EVDocumentSaveSnapshot) throws {
            acknowledgements += 1
            persistenceState.isDirty = false
            persistenceStateDidChange?(persistenceState)
        }
    }
    private enum Decision { case discard, cancel, save }
    private var reviewDecisions: [Decision] = []
    private var reviewCount = 0
    private var reviewCancelled = false

    private func installDecisions(on document: EVDocument) {
        // Untitled reviews embed an out-of-process NSSavePanel. Inject only
        // that dialog's answer; performClose still traverses real NSDocument
        // preflight and NSWindow delegate callbacks, exposing duplicate review.
        document.closeReviewDecisionHandler = { [weak self] reply in
            guard let self else { reply(false); return }
            let decision = self.reviewDecisions[min(self.reviewCount, self.reviewDecisions.count - 1)]
            self.reviewCount += 1
            self.reviewCancelled = decision == .cancel
            reply(decision != .cancel)
        }
    }

    private func makeDocument() throws -> (EVDocument, Backend, EVDocumentWindowController) {
        _ = NSApplication.shared
        let backend = Backend()
        let document = EVDocument(editorBackend: backend)
        installDecisions(on: document)
        NSDocumentController.shared.addDocument(document)
        document.makeWindowControllers()
        let controller = try XCTUnwrap(document.windowControllers.first as? EVDocumentWindowController)
        controller.showWindow(nil)
        return (document, backend, controller)
    }
    private func buttons(_ view: NSView) -> [NSButton] {
        (view as? NSButton).map { [$0] } ?? view.subviews.flatMap(buttons)
    }
    private func pump() {
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.01))
    }
    /// Use the native performClose path, driving an actual named-file Save
    /// sheet or the injected untitled dialog response as appropriate.
    private func close(_ window: NSWindow, decisions: [Decision]) throws -> Int {
        reviewCount = 0
        reviewCancelled = false
        reviewDecisions = decisions
        window.performClose(nil)
        let deadline = Date(timeIntervalSinceNow: 5)
        var seen = Set<ObjectIdentifier>()
        var count = 0
        var cancelled = false
        while Date() < deadline {
            if let sheet = window.attachedSheet, seen.insert(ObjectIdentifier(sheet)).inserted {
                let decision = decisions[min(count, decisions.count - 1)]
                count += 1
                let choices = buttons(try XCTUnwrap(sheet.contentView))
                let selected = choices.first { button in
                    switch decision {
                    case .discard: button.title == "Delete" || button.title == "Don’t Save" || button.title == "Don't Save"
                    case .cancel: button.title == "Cancel"
                    case .save: button.title == "Save" || button.title == "Save…"
                    }
                }
                let button = try XCTUnwrap(selected, "Review buttons: \(choices.map(\.title))")
                button.performClick(nil)
                cancelled = decision == .cancel
            }
            pump()
            if !window.isVisible || ((cancelled || reviewCancelled) && window.attachedSheet == nil) { break }
        }
        if !cancelled && !reviewCancelled { XCTAssertFalse(window.isVisible, "Approved close must finish") }
        return count + reviewCount
    }

    func testUntitledDeleteReviewsOnceWithoutSavingOrClearingTheCoreState() throws {
        let (document, backend, controller) = try makeDocument()
        defer { document.close() }
        XCTAssertEqual(try close(XCTUnwrap(controller.window), decisions: [.discard]), 1)
        XCTAssertEqual(backend.acknowledgements, 0)
        XCTAssertTrue(backend.persistenceState.isDirty)
        XCTAssertEqual(backend.bytes, Data("Unsaved words".utf8))
        XCTAssertFalse(NSDocumentController.shared.documents.contains { $0 === document })
    }

    func testCancelKeepsTheWindowAndASecondCloseReviewsAgain() throws {
        let (document, backend, controller) = try makeDocument()
        defer { document.close() }
        let window = try XCTUnwrap(controller.window)
        XCTAssertEqual(try close(window, decisions: [.cancel]), 1)
        XCTAssertTrue(window.isVisible)
        XCTAssertTrue(backend.persistenceState.isDirty)
        XCTAssertEqual(backend.acknowledgements, 0)
        XCTAssertEqual(try close(window, decisions: [.discard]), 1)
    }

    func testSaveDuringCloseWritesAndAcknowledgesBeforeClosingWithOneReview() throws {
        let (document, backend, controller) = try makeDocument()
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { document.close(); try? FileManager.default.removeItem(at: directory) }
        let target = directory.appendingPathComponent("saved.txt")
        try Data("Original".utf8).write(to: target)
        document.fileURL = target
        document.closeReviewDecisionHandler = nil
        document.recordFileBaseline(Data("Original".utf8), at: target)
        XCTAssertEqual(try close(XCTUnwrap(controller.window), decisions: [.save]), 1)
        XCTAssertEqual(try Data(contentsOf: target), backend.bytes)
        XCTAssertEqual(backend.acknowledgements, 1)
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    func testAnotherWindowKeepsDirtyDocumentAliveAndOnlyFinalWindowReviews() throws {
        let (document, backend, first) = try makeDocument()
        defer { document.close() }
        document.showAdditionalWindow()
        let second = try XCTUnwrap(document.windowControllers.last as? EVDocumentWindowController)
        XCTAssertFalse(first === second)
        XCTAssertEqual(try close(XCTUnwrap(first.window), decisions: [.discard]), 0)
        XCTAssertTrue(try XCTUnwrap(second.window).isVisible)
        XCTAssertTrue(backend.persistenceState.isDirty)
        XCTAssertEqual(document.windowControllers.count, 1)
        XCTAssertEqual(try close(XCTUnwrap(second.window), decisions: [.discard]), 1)
    }

    func testTwoPanesOfTheSameDirtyDocumentReviewOnlyOnce() throws {
        let (document, backend, controller) = try makeDocument()
        defer { document.close() }
        var split: Result<String?, Error>?
        controller.perform(documentHostRequests: [.init(kind: .split, documentID: 1, documentRevision: 1)]) { split = $0 }
        _ = try XCTUnwrap(split).get()
        XCTAssertEqual(controller.paneCount, 2)
        XCTAssertEqual(try close(XCTUnwrap(controller.window), decisions: [.discard]), 1)
        XCTAssertEqual(backend.acknowledgements, 0)
    }

    func testDifferentDirtyPaneDocumentsEachReviewAndCancelPreservesBoth() throws {
        let (first, firstBackend, controller) = try makeDocument()
        let secondBackend = Backend()
        let second = EVDocument(editorBackend: secondBackend)
        installDecisions(on: second)
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("viem-close-\(UUID().uuidString).txt")
        try Data().write(to: url)
        second.fileURL = url
        NSDocumentController.shared.addDocument(second)
        defer { first.close(); second.close(); try? FileManager.default.removeItem(at: url) }
        var split: Result<String?, Error>?
        controller.perform(documentHostRequests: [.init(kind: .split, documentID: 1, documentRevision: 1, path: url.path)]) { split = $0 }
        _ = try XCTUnwrap(split).get()
        XCTAssertEqual(controller.paneCount, 2)
        let window = try XCTUnwrap(controller.window)
        XCTAssertEqual(try close(window, decisions: [.discard, .cancel]), 2)
        XCTAssertTrue(window.isVisible)
        XCTAssertTrue(firstBackend.persistenceState.isDirty)
        XCTAssertTrue(secondBackend.persistenceState.isDirty)
        XCTAssertEqual(try close(window, decisions: [.discard]), 2)
        XCTAssertEqual(firstBackend.acknowledgements + secondBackend.acknowledgements, 0)
    }
}
