import AppKit
import CViemCore
import XCTest

@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVStyleEditorTrackingTests: XCTestCase {
    private let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
    private let inlineCode = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Code"))

    func testNativeEditStylesFootersAndCommandsOpenTheCurrentRequestedStyle() throws {
        let surface = try markdownSurface()
        let original = try surface.backend.recoverySnapshot()
        let coordinator = EVStyleEditorCoordinator.shared
        coordinator.close()
        defer { coordinator.close() }
        moveCaret(7, in: surface)
        XCTAssertEqual(try XCTUnwrap(surface.session).selectedNamedStyles().character?.rawValue, "Code")

        let owner = EVApplicationDelegate(configuration: surface.backend.configuration)
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { surface })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        for (title, expected) in [("Character", inlineCode), ("Paragraph", heading)] {
            let menu = try XCTUnwrap(main.item(withTitle: title)?.submenu)
            builder.menuNeedsUpdate(menu)
            let item = try XCTUnwrap(menu.item(withTitle: "Edit Styles…"))
            XCTAssertTrue(surface.editorView.validateMenuItem(item))
            surface.editorView.performEditorStyleMenuAction(item)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, expected)
        }

        for (command, expected) in [(EVMenuCommand.editCharacterStyles, inlineCode),
                                     (.editParagraphStyles, heading)] {
            surface.perform(menuCommand: command, sender: nil)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, expected)
        }
        moveCaret(1, in: surface)
        surface.perform(menuCommand: .editCharacterStyles, sender: nil)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading,
                       "With no named character style, Character opens the current paragraph style")
        surface.perform(menuCommand: .editParagraphStyles, sender: nil)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        XCTAssertEqual(try surface.backend.recoverySnapshot(), original)
        XCTAssertFalse(surface.canUndo)
    }

    func testCaretFollowingPrefersNamedCharactersAndManualPickerSurvivesRefreshAndStyleEdits() throws {
        let surface = try markdownSurface()
        moveCaret(7, in: surface)
        let coordinator = EVStyleEditorCoordinator { _ in nil }
        coordinator.show(document: surface, preferredStyle: .character, sender: nil)
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

        moveCaret(1, in: surface)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        moveCaret(7, in: surface)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
        moveCaret(17, in: surface)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
        XCTAssertEqual(try surface.backend.recoverySnapshot(), afterStyleEdit,
                       "Following presentation state must not write source, styles, or undo history")
        XCTAssertTrue(surface.canUndo, "Only the deliberate style edit contributes history")
    }

    func testTrackingIsBoundToTheTargetSurfaceAndStopsWhenTargetOrPanelCloses() throws {
        let first = try markdownSurface()
        let otherView = try XCTUnwrap(first.backend.makeEditorSurface() as? EVEditorSurfaceController)
        prepare(otherView)
        let unrelated = try markdownSurface()
        moveCaret(7, in: first)
        let coordinator = EVStyleEditorCoordinator { _ in nil }
        coordinator.show(document: first, preferredStyle: .character, sender: nil)
        defer { coordinator.close() }
        moveCaret(17, in: otherView)
        moveCaret(1, in: unrelated)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
        XCTAssertEqual(coordinator.inspection?.targetDocumentIdentity, ObjectIdentifier(first.backend))
        moveCaret(1, in: first)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)

        // Retargeting detaches the original view observer even when that
        // original document still has a second live view.
        coordinator.show(document: unrelated, preferredStyle: .paragraph, sender: nil)
        moveCaret(7, in: first)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        XCTAssertEqual(coordinator.inspection?.targetDocumentIdentity, ObjectIdentifier(unrelated.backend))
        coordinator.documentDidClose(unrelated)
        XCTAssertFalse(coordinator.inspection?.hasDocument ?? true)
        XCTAssertNil(coordinator.inspection?.selectedStyleKey)
        moveCaret(7, in: unrelated)
        XCTAssertNil(coordinator.inspection?.selectedStyleKey)
        coordinator.close()
        moveCaret(17, in: unrelated)
        XCTAssertNil(coordinator.styleWindow)
        XCTAssertNil(coordinator.inspection)
    }

    func testMixedSelectionsFollowTheirUniformParagraphOrBaseAndCharacterFooterUsesBase() throws {
        let surface = try markdownSurface()
        let session = try XCTUnwrap(surface.session)
        let original = try surface.backend.recoverySnapshot()
        moveCaret(7, in: surface)
        let coordinator = EVStyleEditorCoordinator.shared
        coordinator.close()
        defer { coordinator.close() }
        surface.perform(menuCommand: .editCharacterStyles, sender: nil)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)

        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 10))
        let headingSelection = try session.selectedNamedStyles()
        XCTAssertTrue(headingSelection.characterMixed)
        XCTAssertNil(headingSelection.character)
        XCTAssertFalse(headingSelection.paragraphMixed)
        XCTAssertEqual(headingSelection.paragraph, heading.id)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)

        let owner = EVApplicationDelegate(configuration: surface.backend.configuration)
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { surface })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let character = try XCTUnwrap(main.item(withTitle: "Character")?.submenu)
        builder.menuNeedsUpdate(character)
        let footer = try XCTUnwrap(character.item(withTitle: "Edit Styles…"))
        XCTAssertTrue(surface.editorView.validateMenuItem(footer))
        surface.editorView.performEditorStyleMenuAction(footer)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading,
                       "Mixed character assignments open the uniform current paragraph style")

        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 20))
        let mixedParagraphs = try session.selectedNamedStyles()
        XCTAssertTrue(mixedParagraphs.paragraphMixed)
        XCTAssertNil(mixedParagraphs.paragraph)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
        XCTAssertEqual(try surface.backend.recoverySnapshot(), original)
        XCTAssertFalse(surface.canUndo)
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
        coordinator.close()
        defer { coordinator.close() }
        surface.perform(menuCommand: .editCharacterStyles, sender: nil)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, captureKey)
        XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)
        XCTAssertNil(coordinator.inspection?.targetDocumentIdentity)

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
                       "Standalone Code settings have no source caret to follow")
        XCTAssertTrue(coordinator.inspection?.mutationsEnabled == true)
        XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)

        moveCaret(0, in: surface)
        surface.perform(menuCommand: .editCharacterStyles, sender: nil)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, captureKey)
        coordinator.documentDidClose(surface)
        XCTAssertNotNil(coordinator.styleWindow)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, captureKey)
        XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)
        XCTAssertTrue(coordinator.inspection?.mutationsEnabled == true,
                      "Closing the followed view must retain the global settings editor")
        moveCaret(2, in: surface)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, captureKey,
                       "A closed source view must no longer drive global style selection")
        XCTAssertEqual(try backend.recoverySnapshot(), original)
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
    }

    private func control<T: NSView>(_ type: T.Type, label: String, in root: NSView) throws -> T {
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        return try XCTUnwrap(descendants(root).first {
            $0 is T && $0.accessibilityLabel() == label
        } as? T)
    }
}
