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

    func testMarkdownCodeModeTransitionRetargetsOpenInspectorAndKeepsFollowing() async throws {
        for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
            let source = Data("# Title\n\n**bold** plain\n".utf8)
            let backend = EVCoreDocumentBackend(configuration: configuration())
            try backend.read(source: source, typeName: type, filename: "tracking.md", allowAutomaticCode: false)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            prepare(surface)
            let host = EVDocumentWindowController(document: EVDocument(), editorSurface: surface)
            let documentWindow = try XCTUnwrap(host.window)
            XCTAssertTrue(documentWindow.makeFirstResponder(surface.editorView))
            let coordinator = EVStyleEditorCoordinator.shared
            coordinator.close()
            coordinator.show(document: surface, sender: nil)
            defer { coordinator.close(); documentWindow.orderOut(nil) }
            let window = try XCTUnwrap(coordinator.styleWindow)
            XCTAssertTrue(window.title.contains("Markdown"))
            surface.selectDocumentMode(.code("markdown"), expected: try XCTUnwrap(surface.currentDocumentMode()))
            XCTAssertEqual(backend.sourceFormat, .code)
            XCTAssertTrue(window.title.contains("Code"), window.title)
            XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)
            for _ in 0..<200 {
                backend.pollSyntax()
                surface.refreshPresentation()
                if surface.currentStyleEditorKey() != .baseParagraph { break }
                try await Task.sleep(for: .milliseconds(10))
            }
            coordinator.settleSelectionFollowForTesting()
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, surface.currentStyleEditorKey(),
                           "The inspector follows the syntax style that arrives after the mode switch without a caret move")
            for (token, name) in [("bold", "Markup.strong"), ("plain", ""), ("Title", "Markup.heading.1")] {
                let text = try backend.formattedText()
                let offset = try XCTUnwrap(text.range(of: token)).lowerBound.utf16Offset(in: text)
                let expected = name.isEmpty ? EVStyleKey.baseParagraph : try XCTUnwrap(
                    EVCoreStyleBridge.copyStyleSheet(core: nil).definitions.first { $0.name == name }?.key)
                moveCaret(offset, in: surface)
                XCTAssertTrue(coordinator.selectionFollowScheduledForTesting, token)
                coordinator.settleSelectionFollowForTesting()
                XCTAssertEqual(coordinator.inspection?.selectedStyleKey, expected, token)
                XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, 0)
                XCTAssertTrue(coordinator.inspection?.mutationsEnabled == true)
            }
            let editor = try XCTUnwrap(window.contentViewController as? EVStyleEditorViewController)
            XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(27)), editor.inspection.diagnostic)
            let selected = try XCTUnwrap(coordinator.inspection?.selectedStyleKey)
            XCTAssertEqual(try EVCoreStyleBridge.copyStyleSheet(core: nil).definition(for: selected)?
                .properties[.characterSize]?.declared, .float(27))
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(backend.sourceFormat, type == EVDocument.markdownType ? .markdown : .markdownSource)
            XCTAssertTrue(window.title.contains("Markdown"), window.title)
            XCTAssertEqual(try backend.serializedSource(typeName: type), source)
            XCTAssertFalse(backend.persistenceState.isDirty)
            XCTAssertTrue(coordinator.styleWindow === window)
            coordinator.close()
        }
    }

    func testDelayedCodeSyntaxPreservesExplicitStyleChoiceUntilCaretMoves() async throws {
        let surface = try markdownSurface()
        let coordinator = EVStyleEditorCoordinator()
        coordinator.show(document: surface, sender: nil)
        defer { coordinator.close() }
        surface.selectDocumentMode(.code("markdown"), expected: try XCTUnwrap(surface.currentDocumentMode()))
        coordinator.selectStyle(EVStyleKey.baseParagraph)
        let count = coordinator.caretFollowQueryCount
        for _ in 0..<200 {
            surface.backend.pollSyntax()
            if surface.currentStyleEditorKey() != .baseParagraph { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertNotEqual(surface.currentStyleEditorKey(), .baseParagraph)
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        coordinator.settleSelectionFollowForTesting()
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
        XCTAssertEqual(coordinator.caretFollowQueryCount, count,
                       "Published syntax must not override an explicit definition choice")
        moveCaret(3, in: surface)
        coordinator.settleSelectionFollowForTesting()
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, surface.currentStyleEditorKey())
        XCTAssertNotEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
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

    func testMarkdownSemanticStylesFollowTheCaretInNormalAndInsertModes() throws {
        let source = """
        Plain

        [linked](https://example.com)

        <https://example.com/automatic>

        <!-- comment -->

        [reference][ref]

        ~~strike~~

        `code`

        [`nested`](https://example.com)

        [~~overlap~~](https://example.com)

        ![photo](https://example.com/photo.png)

        [ref]: docs/next.md
        """
        for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
            for mode in [UInt32(VIEM_MODE_NORMAL), UInt32(VIEM_MODE_INSERT)] {
                let backend = EVCoreDocumentBackend(configuration: configuration())
                try backend.read(source: Data(source.utf8), typeName: type)
                let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
                prepare(surface)
                surface.view.frame.size.height = 1_200
                surface.viewDidLayout()
                let session = try XCTUnwrap(surface.session)
                if mode == UInt32(VIEM_MODE_INSERT) {
                    surface.performInput { _ = try session.sendText("i") }
                }
                let original = try backend.recoverySnapshot()
                let coordinator = EVStyleEditorCoordinator()
                coordinator.show(document: surface, sender: nil)
                defer { coordinator.close() }
                XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
                let text = try backend.formattedText()
                let cases: [(String, EVStyleKey)] = [
                    ("linked", EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Link"))),
                    ("automatic", EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Link"))),
                    ("comment", EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Comment"))),
                    ("reference", EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Markdown reference"))),
                    ("strike", EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Strikethrough"))),
                    ("code", inlineCode),
                    ("nested", inlineCode),
                    ("overlap", .baseParagraph),
                    (type == EVDocument.markdownType ? "\u{fffc}" : "photo",
                     EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Image"))),
                    ("Plain", .baseParagraph),
                ]
                for (token, expected) in cases {
                    let range = try XCTUnwrap(text.range(of: token))
                    let offset = range.lowerBound.utf16Offset(in: text)
                    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: offset, length: 0))
                    coordinator.settleSelectionFollowForTesting()
                    XCTAssertEqual(surface.viewPresentation.mode, mode, "\(type): \(token)")
                    XCTAssertEqual(surface.currentStyleEditorKey(), expected, "\(type): \(token)")
                    XCTAssertEqual(coordinator.inspection?.selectedStyleKey, expected, "\(type): \(token)")
                }
                XCTAssertEqual(try backend.recoverySnapshot(), original)
                XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
                XCTAssertFalse(surface.canUndo)
            }
        }
    }

    func testLinkStyleSelectionAndMixedCharacterStylesUseTheSameInspectorPolicy() throws {
        for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
            let backend = EVCoreDocumentBackend(configuration: configuration())
            let source = "plain [linked](https://example.com) and `code`"
            try backend.read(source: Data(source.utf8), typeName: type)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            prepare(surface)
            let session = try XCTUnwrap(surface.session)
            let text = try backend.formattedText()
            let label = try XCTUnwrap(text.range(of: "linked"))
            let coordinator = EVStyleEditorCoordinator()
            coordinator.show(document: surface, sender: nil)
            defer { coordinator.close() }
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(label, in: text))
            coordinator.settleSelectionFollowForTesting()
            XCTAssertEqual(try session.selectedNamedStyles().character?.rawValue, "Link")
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey,
                           EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Link")))
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: text.utf16.count))
            coordinator.settleSelectionFollowForTesting()
            let selected = try session.selectedNamedStyles()
            XCTAssertTrue(selected.characterMixed)
            XCTAssertNil(selected.character)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            XCTAssertFalse(surface.canUndo)
        }
    }

    func testPendingCharacterChangesFollowWithoutMovingTheInsertCaret() throws {
        let source = "plain [linked](target.md) ~~strike~~ tail"
        for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
            let backend = EVCoreDocumentBackend(configuration: configuration())
            try backend.read(source: Data(source.utf8), typeName: type)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            prepare(surface)
            let session = try XCTUnwrap(surface.session)
            var sourceChangeCount = 0
            backend.sourceDidChange = { [weak surface] in
                sourceChangeCount += 1
                surface?.refreshPresentation()
            }
            surface.performInput { _ = try session.sendText("i") }
            let text = try backend.formattedText()
            let link = try XCTUnwrap(text.range(of: "linked"))
            moveCaret(link.lowerBound.utf16Offset(in: text) + 1, in: surface)
            let original = try backend.recoverySnapshot()
            let coordinator = EVStyleEditorCoordinator()
            coordinator.show(document: surface, sender: nil)
            defer { coordinator.close() }
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey,
                           EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Link")))
            let caret = surface.viewPresentation.cursor_utf8_offset
            let affinity = surface.viewPresentation.cursor_affinity
            surface.formattingToolbar.openLinkEditor(surface.formattingToolbar.insertLink)
            XCTAssertNil(surface.commandOutput)
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
            XCTAssertEqual(surface.viewPresentation.cursor_affinity, affinity)
            XCTAssertTrue(coordinator.selectionFollowScheduledForTesting)
            coordinator.settleSelectionFollowForTesting()
            XCTAssertEqual(surface.currentStyleEditorKey(), .baseParagraph)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)

            surface.formattingToolbar.toggleCharacterCode(surface.formattingToolbar.characterCode)
            XCTAssertNil(surface.commandOutput)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, caret)
            XCTAssertTrue(coordinator.selectionFollowScheduledForTesting)
            coordinator.settleSelectionFollowForTesting()
            XCTAssertEqual(surface.currentStyleEditorKey(), inlineCode)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
            surface.formattingToolbar.toggleCharacterCode(surface.formattingToolbar.characterCode)
            coordinator.settleSelectionFollowForTesting()
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)

            let strike = try XCTUnwrap(text.range(of: "strike"))
            moveCaret(strike.lowerBound.utf16Offset(in: text) + 1, in: surface)
            coordinator.settleSelectionFollowForTesting()
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey,
                           EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Strikethrough")))
            let strikeCaret = surface.viewPresentation.cursor_utf8_offset
            surface.perform(menuCommand: .strikethrough, sender: nil)
            XCTAssertNil(surface.commandOutput)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, strikeCaret)
            XCTAssertTrue(coordinator.selectionFollowScheduledForTesting)
            coordinator.settleSelectionFollowForTesting()
            XCTAssertEqual(surface.currentStyleEditorKey(), .baseParagraph)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph,
                           "A pending off choice suppresses the automatic source strikethrough style")
            XCTAssertEqual(try backend.recoverySnapshot(), original)
            XCTAssertEqual(sourceChangeCount, 0)
            XCTAssertFalse(surface.canUndo)

            coordinator.selectStyle(heading)
            let editor = try XCTUnwrap(coordinator.styleWindow?.contentViewController as? EVStyleEditorViewController)
            XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(29)), editor.inspection.diagnostic)
            surface.refreshPresentation()
            XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading,
                           "Editing a definition preserves the explicit inspector choice")
            let generation = session.characterContextGeneration
            let snapshot = try backend.styleSheetSnapshot()
            XCTAssertThrowsError(try session.assignStyle(
                EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Missing style")),
                identity: snapshot.identity, expected: session.listSelection()))
            surface.refreshPresentation()
            XCTAssertEqual(session.characterContextGeneration, generation)
            XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        }
    }

    func testCollapsedCaretModeChangeResumesFollowingTheInsertionStyle() throws {
        for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
            let backend = EVCoreDocumentBackend(configuration: configuration())
            try backend.read(source: Data("plain `code` tail".utf8), typeName: type)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            prepare(surface)
            let session = try XCTUnwrap(surface.session)
            var point = ViemLayoutCaretPointV1()
            point.struct_size = UInt32(MemoryLayout<ViemLayoutCaretPointV1>.size)
            point.document_revision = try backend.revision()
            point.text_offset = 6
            point.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
            surface.performInput { _ = try session.placeCursor(point, extendSelection: false) }
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
            let coordinator = EVStyleEditorCoordinator()
            coordinator.show(document: surface, sender: nil)
            defer { coordinator.close() }
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
            coordinator.selectStyle(heading)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
            XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
            let original = try backend.recoverySnapshot()
            surface.performInput { _ = try session.sendText("i") }
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, point.text_offset)
            XCTAssertEqual(surface.viewPresentation.cursor_affinity, point.affinity)
            XCTAssertTrue(coordinator.selectionFollowScheduledForTesting)
            coordinator.settleSelectionFollowForTesting()
            XCTAssertEqual(surface.currentStyleEditorKey(), inlineCode)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode,
                           "A typing mode change resumes caret following without a move or affinity change")
            XCTAssertEqual(try backend.recoverySnapshot(), original)
            XCTAssertFalse(surface.canUndo)
        }
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

    func testSelectionFollowingCoalescesQueriesAndCancelsPendingWork() throws {
        let surface = try markdownSurface()
        moveCaret(7, in: surface)
        let coordinator = EVStyleEditorCoordinator()
        coordinator.show(document: surface, sender: nil)
        defer { coordinator.close() }
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        let before = try surface.backend.recoverySnapshot()
        let count = coordinator.caretFollowQueryCount
        // Exercise the queued callback directly: AppKit scheduling latency is
        // independent of the coalescing and cancellation policy under test.
        for index in 0..<6 {
            moveCaret(index.isMultiple(of: 2) ? 1 : 7, in: surface)
            XCTAssertEqual(coordinator.caretFollowQueryCount, count)
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
        }
        moveCaret(1, in: surface)
        XCTAssertEqual(coordinator.caretFollowQueryCount, count)
        XCTAssertTrue(coordinator.selectionFollowScheduledForTesting)
        coordinator.settleSelectionFollowForTesting()
        XCTAssertEqual(coordinator.caretFollowQueryCount, count + 1)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, heading)
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        for _ in 0..<3 { surface.refreshPresentation() }
        moveCaret(1, in: surface)
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        coordinator.settleSelectionFollowForTesting()
        XCTAssertEqual(coordinator.caretFollowQueryCount, count + 1)

        moveCaret(7, in: surface)
        XCTAssertTrue(coordinator.selectionFollowScheduledForTesting)
        coordinator.selectStyle(EVStyleKey.baseParagraph)
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        coordinator.settleSelectionFollowForTesting()
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
        XCTAssertEqual(coordinator.caretFollowQueryCount, count + 1)
        moveCaret(1, in: surface)
        coordinator.close()
        XCTAssertFalse(coordinator.selectionFollowScheduledForTesting)
        coordinator.settleSelectionFollowForTesting()
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
