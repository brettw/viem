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

    private func markdownSurface() throws -> EVEditorSurfaceController {
        let backend = EVCoreDocumentBackend(configuration: configuration())
        try backend.readUntitled(source: Data("# Title `code` tail\n\nBody".utf8), typeName: EVDocument.markdownType)
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
