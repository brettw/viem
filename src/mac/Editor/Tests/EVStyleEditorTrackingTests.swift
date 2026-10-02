import AppKit
import CViemCore
import XCTest

@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVStyleEditorTrackingTests: XCTestCase {
    private var coordinatorToSettle: EVStyleEditorCoordinator?
    private let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
    private let inlineCode = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Code"))

    func testDocumentActivationRetargetsCurrentStyleWithoutStealingFocusAndCancelsOldFollowing() throws {
        let first = try markdownSurface()
        let other = try markdownSurface()
        let firstHost = EVDocumentWindowController(document: EVDocument(), editorSurface: first)
        let otherHost = EVDocumentWindowController(document: EVDocument(), editorSurface: other)
        let firstWindow = try XCTUnwrap(firstHost.window)
        let otherWindow = try XCTUnwrap(otherHost.window)
        let coordinator = EVStyleEditorCoordinator.shared
        coordinator.close()
        defer { coordinator.close(); firstWindow.orderOut(nil); otherWindow.orderOut(nil) }
        moveCaret(7, in: first)
        moveCaret(1, in: other)
        coordinator.show(document: first, sender: nil)
        let inspector = try XCTUnwrap(coordinator.styleWindow)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
        moveCaret(17, in: first)
        XCTAssertTrue(coordinator.selectionFollowScheduledForTesting)
        XCTAssertTrue(otherWindow.makeFirstResponder(other.editorView))
        XCTAssertTrue(coordinator.styleWindow === inspector)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        XCTAssertTrue(otherWindow.firstResponder === other.editorView)
        coordinator.selectStyle(EVStyleKey.baseParagraph)
        NotificationCenter.default.post(name: NSWindow.didBecomeKeyNotification, object: otherWindow)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph,
                       "Returning from the inspector to the same document preserves an explicit choice")
        moveCaret(7, in: first)
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        // Returning to a document whose first responder was already its editor
        // must retarget on window activation, without another mouse/key event.
        XCTAssertTrue(firstWindow.makeFirstResponder(first.editorView))
        coordinator.documentDidBecomeActive(other)
        XCTAssertTrue(firstWindow.makeFirstResponder(nil))
        NotificationCenter.default.post(name: NSWindow.didBecomeKeyNotification, object: firstWindow)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
        coordinator.documentDidClose(first)
        NotificationCenter.default.post(name: .viemActiveEditorSurfaceDidChange, object: other)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        XCTAssertFalse(first.canUndo)
        XCTAssertFalse(other.canUndo)
    }

    func testActivationSwitchesStyleFamiliesAndLeavesStandaloneCodeSettingsIndependent() throws {
        let markdown = try markdownSurface()
        moveCaret(1, in: markdown)
        let backend = EVCoreDocumentBackend(configuration: markdown.backend.configuration)
        try backend.read(source: Data("Text".utf8), typeName: EVDocument.plainTextType)
        let text = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        prepare(text)
        let coordinator = EVStyleEditorCoordinator()
        coordinator.show(document: markdown, sender: nil)
        defer { coordinator.close() }
        coordinator.documentDidBecomeActive(text)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
        XCTAssertTrue(coordinator.styleWindow?.title.contains("Plain Text") == true)
        coordinator.documentDidBecomeActive(markdown)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        XCTAssertTrue(coordinator.styleWindow?.title.contains("Markdown") == true)
        coordinator.showCode(configuration: backend.configuration, sender: nil)
        coordinator.documentDidBecomeActive(markdown)
        XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)
        XCTAssertTrue(coordinator.styleWindow?.title.contains("Code") == true)
    }

    func testNativeEditStylesMenuAndCommandOpenTheCurrentCaretStyle() throws {
        let surface = try markdownSurface()
        let controller = EVDocumentWindowController(document: EVDocument(), editorSurface: surface)
        let window = try XCTUnwrap(controller.window)
        defer { window.orderOut(nil) }
        let original = try surface.backend.recoverySnapshot()
        let coordinator = EVStyleEditorCoordinator.shared
        coordinatorToSettle = coordinator
        coordinator.close()
        defer { coordinator.close() }
        moveCaret(7, in: surface)
        XCTAssertEqual(try XCTUnwrap(surface.session).selectedNamedStyles().character?.rawValue, "Code")

        let owner = EVApplicationDelegate(configuration: surface.backend.configuration)
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { surface })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let styles = try XCTUnwrap(main.item(withTitle: "Style")?.submenu)
        let item = try XCTUnwrap(styles.item(withTitle: "Edit Styles…"))
        XCTAssertEqual(item.action, #selector(EVEditorCommandRouting.performEditorMenuCommand(_:)))
        XCTAssertTrue(controller.documentContentController.validateMenuItem(item))
        controller.documentContentController.performEditorMenuCommand(item)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)

        let commands: [EVMenuCommand] = [.editStyles]
        for command in commands {
            coordinator.close()
            surface.perform(menuCommand: command, sender: nil)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
        }
        moveCaret(1, in: surface)
        for command in commands {
            coordinator.close()
            surface.perform(menuCommand: command, sender: nil)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading,
                           "With no named character style, Edit Styles opens the current paragraph style")
        }
        XCTAssertEqual(try surface.backend.recoverySnapshot(), original)
        XCTAssertFalse(surface.canUndo)
        withExtendedLifetime((builder, owner, controller)) {}
    }

    func testCaretFollowingPrefersNamedCharactersAndManualPickerSurvivesRefreshAndStyleEdits() throws {
        let surface = try markdownSurface()
        moveCaret(7, in: surface)
        let coordinator = EVStyleEditorCoordinator()
        coordinatorToSettle = coordinator
        coordinator.show(document: surface, sender: nil)
        defer { coordinator.close() }
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
        let editor = try XCTUnwrap(coordinator.styleWindow?.contentViewController as? EVStyleEditorViewController)
        let explicitKey = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading2"))
        let name = try XCTUnwrap(surface.backend.styleSheetSnapshot().definition(for: explicitKey)?.name)
        let popup = try control(NSPopUpButton.self, label: "Style", in: editor.view)
        popup.select(try XCTUnwrap(popup.itemArray.first { $0.title == name }))
        XCTAssertTrue(popup.sendAction(try XCTUnwrap(popup.action), to: popup.target))
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, explicitKey)

        surface.refreshPresentation()
        surface.refreshPresentation()
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, explicitKey)
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(29)), editor.inspection.diagnostic)
        surface.refreshPresentation()
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, explicitKey,
                       "A stylesheet revision and layout rebuild are not a caret move")
        moveCaret(7, in: surface)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, explicitKey,
                       "Reporting the same caret again must retain an explicit picker choice")
        let afterStyleEdit = try surface.backend.recoverySnapshot()

        let originalWindow = try XCTUnwrap(coordinator.styleWindow)
        coordinator.show(document: surface, sender: nil)
        XCTAssertTrue(coordinator.styleWindow === originalWindow)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode,
                       "Invoking Edit Styles again selects the current caret style without requiring a move")

        moveCaret(1, in: surface)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        moveCaret(7, in: surface)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
        moveCaret(17, in: surface)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
        XCTAssertEqual(try surface.backend.recoverySnapshot(), afterStyleEdit,
                       "Following presentation state must not write source, styles, or undo history")
        XCTAssertFalse(surface.canUndo, "Theme style edits never enter document history")
        XCTAssertEqual(coordinator.styleWindow?.undoManager?.canUndo, true,
                       "Reopening the same theme family preserves its independent settings history")
    }

    func testTrackingIsBoundToTheTargetSurfaceAndStopsWhenTargetOrPanelCloses() throws {
        let first = try markdownSurface()
        let otherView = try XCTUnwrap(first.backend.makeEditorSurface() as? EVEditorSurfaceController)
        prepare(otherView)
        let unrelated = try markdownSurface()
        moveCaret(7, in: first)
        let coordinator = EVStyleEditorCoordinator()
        coordinatorToSettle = coordinator
        coordinator.show(document: first, sender: nil)
        defer { coordinator.close() }
        moveCaret(17, in: otherView)
        moveCaret(1, in: unrelated)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
        XCTAssertNil(coordinator.inspection?.targetDocumentIdentity)
        moveCaret(1, in: first)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)

        // Retargeting detaches the original view observer even when that
        // original document still has a second live view.
        coordinator.show(document: unrelated, sender: nil)
        moveCaret(7, in: first)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        XCTAssertNil(coordinator.inspection?.targetDocumentIdentity)
        coordinator.documentDidClose(unrelated)
        XCTAssertFalse(coordinator.inspection?.hasDocument ?? true)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        moveCaret(7, in: unrelated)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        coordinator.close()
        moveCaret(17, in: unrelated)
        XCTAssertNil(coordinator.styleWindow)
        XCTAssertNil(coordinator.inspection)
    }

    func testMixedSelectionsFollowTheirUniformParagraphOrBaseAndEditStylesUsesBase() throws {
        let surface = try markdownSurface()
        let controller = EVDocumentWindowController(document: EVDocument(), editorSurface: surface)
        let window = try XCTUnwrap(controller.window)
        defer { window.orderOut(nil) }
        let session = try XCTUnwrap(surface.session)
        let original = try surface.backend.recoverySnapshot()
        moveCaret(7, in: surface)
        let coordinator = EVStyleEditorCoordinator.shared
        coordinatorToSettle = coordinator
        coordinator.close()
        defer { coordinator.close() }
        surface.perform(menuCommand: .editStyles, sender: nil)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)

        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 10))
        coordinator.settleSelectionFollowForTesting()
        let headingSelection = try session.selectedNamedStyles()
        XCTAssertTrue(headingSelection.characterMixed)
        XCTAssertNil(headingSelection.character)
        XCTAssertFalse(headingSelection.paragraphMixed)
        XCTAssertEqual(headingSelection.paragraph, heading.id)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)

        let owner = EVApplicationDelegate(configuration: surface.backend.configuration)
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { surface })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let styles = try XCTUnwrap(main.item(withTitle: "Style")?.submenu)
        builder.menuNeedsUpdate(styles)
        let item = try XCTUnwrap(styles.item(withTitle: "Edit Styles…"))
        XCTAssertEqual(item.action, #selector(EVEditorCommandRouting.performEditorMenuCommand(_:)))
        XCTAssertTrue(controller.documentContentController.validateMenuItem(item))
        controller.documentContentController.performEditorMenuCommand(item)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading,
                       "Mixed character assignments open the uniform current paragraph style")

        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 20))
        coordinator.settleSelectionFollowForTesting()
        let mixedParagraphs = try session.selectedNamedStyles()
        XCTAssertTrue(mixedParagraphs.paragraphMixed)
        XCTAssertNil(mixedParagraphs.paragraph)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
        for command in [EVMenuCommand.editStyles] {
            coordinator.close()
            surface.perform(menuCommand: command, sender: nil)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph,
                           "Opening with mixed character and paragraph styles uses the same fallback as following")
        }
        XCTAssertEqual(try surface.backend.recoverySnapshot(), original)
        XCTAssertFalse(surface.canUndo)
        withExtendedLifetime((builder, owner, controller)) {}
    }

    func testCodeTrackingKeepsTheGlobalSettingsTargetAndStandaloneSettingsStopFollowing() async throws {
        let configuration = configuration()
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data("fn main() {}\n".utf8), typeName: EVDocument.codeType,
                         filename: "tracking.rs", allowAutomaticCode: false)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        prepare(surface)
        let session = try XCTUnwrap(surface.session)
        for _ in 0..<200 {
            backend.pollSyntax()
            surface.refreshPresentation()
            if try session.selectedNamedStyles().character != nil { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        let capture = try XCTUnwrap(session.selectedNamedStyles().character)
        XCTAssertFalse(capture.rawValue.isEmpty)
        let captureKey = EVStyleKey(namespace: .character, id: capture)
        let original = try backend.recoverySnapshot()
        let coordinator = EVStyleEditorCoordinator.shared
        coordinatorToSettle = coordinator
        coordinator.close()
        defer { coordinator.close() }
        for command in [EVMenuCommand.editStyles] {
            coordinator.close()
            surface.perform(menuCommand: command, sender: nil)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, captureKey)
            XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)
            XCTAssertNil(coordinator.inspection?.targetDocumentIdentity)
        }

        moveCaret(2, in: surface)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
        moveCaret(0, in: surface)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, captureKey)
        XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)
        XCTAssertEqual(try backend.recoverySnapshot(), original)
        XCTAssertFalse(surface.canUndo)

        coordinator.showCode(configuration: configuration, preferredStyle: .baseParagraph, sender: nil)
        moveCaret(2, in: surface)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph,
                       "A standalone global Code inspector has no source caret to follow")
        XCTAssertTrue(coordinator.inspection?.mutationsEnabled == true)
        XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)

        moveCaret(0, in: surface)
        surface.perform(menuCommand: .editStyles, sender: nil)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, captureKey)
        coordinator.documentDidClose(surface)
        XCTAssertNotNil(coordinator.styleWindow)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, captureKey)
        XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)
        XCTAssertTrue(coordinator.inspection?.mutationsEnabled == true,
                      "Closing the followed view must retain the global style editor")
        moveCaret(2, in: surface)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, captureKey,
                       "A closed source view must no longer drive global style selection")
        XCTAssertEqual(try backend.recoverySnapshot(), original)
    }

    func testRapidSelectionChangesWaitForHalfAnIdleSecondAndDoNotPollWhileUnchanged() async throws {
        let surface = try markdownSurface()
        moveCaret(7, in: surface)
        let coordinator = EVStyleEditorCoordinator()
        coordinator.show(document: surface, sender: nil)
        defer { coordinator.close() }
        // Let AppKit finish the first panel presentation before measuring idle
        // time: its initial drawing can otherwise delay the first 40 ms task
        // resumption long enough for a correctly scheduled timer to fire.
        try await Task.sleep(for: .milliseconds(700))
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        let before = try surface.backend.recoverySnapshot()
        let count = coordinator.caretFollowQueryCount
        for index in 0..<30 {
            moveCaret(index.isMultiple(of: 2) ? 1 : 7, in: surface)
            try await Task.sleep(for: .milliseconds(40))
            XCTAssertEqual(coordinator.caretFollowQueryCount, count)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
        }
        moveCaret(1, in: surface)
        try await Task.sleep(for: .milliseconds(250))
        XCTAssertEqual(coordinator.caretFollowQueryCount, count)
        XCTAssertTrue(coordinator.selectionFollowScheduledForTesting)
        try await Task.sleep(for: .milliseconds(450))
        XCTAssertEqual(coordinator.caretFollowQueryCount, count + 1)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        for _ in 0..<30 { surface.refreshPresentation() }
        moveCaret(1, in: surface)
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        try await Task.sleep(for: .milliseconds(1100))
        XCTAssertEqual(coordinator.caretFollowQueryCount, count + 1)

        moveCaret(7, in: surface)
        XCTAssertTrue(coordinator.selectionFollowScheduledForTesting)
        coordinator.selectStyle(EVStyleKey.baseParagraph)
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        try await Task.sleep(for: .milliseconds(1100))
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
        XCTAssertEqual(coordinator.caretFollowQueryCount, count + 1)
        moveCaret(1, in: surface)
        coordinator.close()
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        try await Task.sleep(for: .milliseconds(1100))
        XCTAssertEqual(coordinator.caretFollowQueryCount, count + 1)
        XCTAssertEqual(try surface.backend.recoverySnapshot(), before)
        XCTAssertFalse(surface.canUndo)
    }

    private func markdownSurface() throws -> EVEditorSurfaceController {
        let backend = EVCoreDocumentBackend(configuration: configuration())
        try backend.read(source: Data("# Title `code` tail\n\nBody".utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        prepare(surface)
        XCTAssertEqual(try backend.formattedText(), "Title code tail\nBody")
        return surface
    }

    private func configuration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-style-tracking-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory, legacyDefaults: nil)
    }

    private func prepare(_ surface: EVEditorSurfaceController) {
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 240)
        surface.viewDidLayout()
    }

    private func moveCaret(_ offset: Int, in surface: EVEditorSurfaceController) {
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: offset, length: 0))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64(offset))
        // These tests exercise selection policy independently of wall-clock timing.
        coordinatorToSettle?.settleSelectionFollowForTesting()
    }

    private func control<T: NSView>(_ type: T.Type, label: String, in root: NSView) throws -> T {
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        return try XCTUnwrap(descendants(root).first {
            $0 is T && $0.accessibilityLabel() == label
        } as? T)
    }
}
