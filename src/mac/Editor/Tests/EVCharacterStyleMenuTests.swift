import AppKit
import CViemCore
import CoreText
import ViemAppShell
import XCTest
@testable import ViemEditor
@testable import ViemCoreTextProvider

@MainActor final class EVCharacterStyleMenuTests: XCTestCase {
    private func surface(_ source: String, type: String, configuration: EVConfigurationStore? = nil) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let backend = EVCoreDocumentBackend(configuration: configuration ?? EVConfigurationStore())
        try backend.read(source: Data(source.utf8), typeName: type)
        let view = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        view.loadViewIfNeeded()
        view.view.frame = NSRect(x: 0, y: 0, width: 520, height: 240)
        view.viewDidLayout()
        return (backend, view, try XCTUnwrap(view.session))
    }

    private func keys(_ text: String, view: EVEditorSurfaceController, session: EVCoreViewSession) throws {
        for character in text {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: UInt32(try XCTUnwrap(character.asciiValue)))
        }
        view.refreshPresentation()
    }

    private func choose(_ id: String, in view: EVEditorSurfaceController) throws {
        let catalogue = try XCTUnwrap(view.currentStyleMenuCatalogue())
        let entry = try XCTUnwrap(catalogue.entries.first { $0.role == .character && $0.stableID == id })
        XCTAssertTrue(entry.presentation.isEnabled, "Character style \(id) should be assignable")
        let item = NSMenuItem(title: entry.displayName, action: #selector(EVStyleMenuActionRouting.performEditorStyleMenuAction(_:)), keyEquivalent: "")
        item.representedObject = EVStyleMenuAction(kind: .assign, role: .character, stableID: id,
            documentID: catalogue.documentID, documentRevision: catalogue.documentRevision,
            styleSheetRevision: catalogue.styleSheetRevision)
        XCTAssertTrue(view.editorView.validateMenuItem(item))
        view.editorView.performEditorStyleMenuAction(item)
        XCTAssertNil(view.commandOutput)
        XCTAssertEqual(view.currentStyleMenuCatalogue()?.entries.first { $0.role == .character && $0.stableID == id }?.presentation.state, .on)
    }

    private func selectedStyle(_ range: NSRange, view: EVEditorSurfaceController, session: EVCoreViewSession) throws -> EVStyleID? {
        view.editorView.setAccessibilitySelectedTextRange(range)
        return try session.selectedNamedStyles().character
    }

    private func assertReopenedStyle(_ saved: Data, type: String, range: NSRange, id: String?) throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: saved, typeName: type)
        let view = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        view.loadViewIfNeeded()
        let session = try XCTUnwrap(view.session)
        XCTAssertEqual(try selectedStyle(range, view: view, session: session)?.rawValue, id)
    }

    func testBuiltinStyleAtNormalCaretEntersInsertAndRoundTripsTypedText() throws {
        for (source, type) in [("base", EVDocument.markdownType)] {
            for (before, insertion) in [("i", 0), ("", 0), ("l", 1), ("R", 0)] {
                let (backend, view, session) = try surface(source, type: type)
                try keys(before, view: view, session: session)
                let revision = try backend.revision()
                let cursor = try session.listSelection().text_start
                let mode = view.viewPresentation.mode
                try choose("Code", in: view)
                XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
                XCTAssertEqual(try backend.revision(), revision)
                XCTAssertEqual(try session.listSelection().text_start, cursor)
                XCTAssertEqual(view.viewPresentation.mode,
                               mode == UInt32(VIEM_MODE_REPLACE) ? mode : UInt32(VIEM_MODE_INSERT))
                XCTAssertFalse(backend.persistenceState.isDirty)
                view.editorView.insertText("XY", replacementRange: NSRange(location: NSNotFound, length: 0))
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                view.refreshPresentation()
                XCTAssertEqual(try backend.formattedText(), before == "R" ? "XYse" : insertion == 0 ? "XYbase" : "bXYase")
                let range = NSRange(location: insertion, length: 2)
                XCTAssertEqual(try selectedStyle(range, view: view, session: session)?.rawValue, "Code")
                let saved = try backend.serializedSource(typeName: type)
                try assertReopenedStyle(saved, type: type, range: range, id: "Code")
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                view.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
                view.perform(menuCommand: .redo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: type), saved)
                XCTAssertNil(view.commandOutput)
            }
        }
    }

    func testChoosingDefaultParagraphStopsPendingNamedStyle() throws {
        for type in [EVDocument.markdownType] {
            let (backend, view, session) = try surface("", type: type)
            try keys("i", view: view, session: session)
            try choose("Code", in: view)
            view.editorView.insertText("X", replacementRange: NSRange(location: NSNotFound, length: 0))
            let beforeBaseChoice = try backend.serializedSource(typeName: type)
            let revision = try backend.revision()
            try choose("", in: view)
            XCTAssertEqual(try backend.serializedSource(typeName: type), beforeBaseChoice)
            XCTAssertEqual(try backend.revision(), revision)
            view.editorView.insertText("Y", replacementRange: NSRange(location: NSNotFound, length: 0))
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            view.refreshPresentation()
            XCTAssertEqual(try backend.formattedText(), "XY")
            XCTAssertEqual(try selectedStyle(NSRange(location: 0, length: 1), view: view, session: session)?.rawValue, "Code")
            XCTAssertEqual(try selectedStyle(NSRange(location: 1, length: 1), view: view, session: session)?.rawValue, nil)
            let saved = try backend.serializedSource(typeName: type)
            try assertReopenedStyle(saved, type: type, range: NSRange(location: 0, length: 1), id: "Code")
            try assertReopenedStyle(saved, type: type, range: NSRange(location: 1, length: 1), id: nil)
            XCTAssertNil(view.commandOutput)
        }
    }

    func testDefaultParagraphClearsSelectedCharacterStylesAndUndoRestoresThem() throws {
        for (source, type) in [("one two", EVDocument.markdownType)] {
            let (backend, view, session) = try surface(source, type: type)
            _ = try selectedStyle(NSRange(location: 0, length: 3), view: view, session: session)
            try choose("Code", in: view)
            let styled = try backend.serializedSource(typeName: type)
            try choose("", in: view)
            XCTAssertNil(try session.selectedNamedStyles().character)
            XCTAssertEqual(view.currentStyleEditorKey(), .baseParagraph)
            XCTAssertEqual(try backend.formattedText(), "one two")
            let cleared = try backend.serializedSource(typeName: type)
            XCTAssertFalse(String(decoding: cleared, as: UTF8.self).contains("Default Paragraph"))
            try assertReopenedStyle(cleared, type: type, range: NSRange(location: 0, length: 3), id: nil)
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            view.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), styled)
            view.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), cleared)
        }
    }

    func testBuiltinCharacterStyleAppliesToSelectedText() throws {
        for (source, type) in [("one two", EVDocument.markdownType)] {
            let (backend, view, session) = try surface(source, type: type)
            let range = NSRange(location: 0, length: 3)
            _ = try selectedStyle(range, view: view, session: session)
            try choose("Code", in: view)
            XCTAssertEqual(try backend.formattedText(), "one two")
            XCTAssertEqual(try session.selectedNamedStyles().character?.rawValue, "Code")
            let saved = try backend.serializedSource(typeName: type)
            try assertReopenedStyle(saved, type: type, range: range, id: "Code")
            XCTAssertEqual(try selectedStyle(NSRange(location: 4, length: 3), view: view, session: session)?.rawValue, nil)
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            view.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            view.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), saved)
        }
    }

    func testCharacterStyleSpansParagraphsAndLeavesOutsideTextUnassigned() throws {
        for (source, type, id) in [
            ("one\n\ntwo\n\nthree", EVDocument.markdownType, "Code"),
        ] {
            let (backend, view, session) = try surface(source, type: type)
            let before = try backend.serializedSource(typeName: type)
            let range = NSRange(location: 1, length: 8)
            _ = try selectedStyle(range, view: view, session: session)
            try choose(id, in: view)
            XCTAssertEqual(try backend.formattedText(), "one\ntwo\nthree")
            XCTAssertEqual(try session.selectedNamedStyles().character?.rawValue, id)
            let saved = try backend.serializedSource(typeName: type)
            try assertReopenedStyle(saved, type: type, range: range, id: id)
            XCTAssertEqual(try selectedStyle(NSRange(location: 0, length: 1), view: view, session: session)?.rawValue, nil)
            XCTAssertEqual(try selectedStyle(NSRange(location: 9, length: 4), view: view, session: session)?.rawValue, nil)
            _ = try selectedStyle(NSRange(location: 0, length: 13), view: view, session: session)
            XCTAssertTrue(try session.selectedNamedStyles().characterMixed)
            XCTAssertTrue(try XCTUnwrap(view.currentStyleMenuCatalogue()).entries.filter { $0.role == .character }.allSatisfy { $0.presentation.state == .off })
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            view.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), before)
            view.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), saved)
        }
    }

    func testCommentMenuWrapsSelectionAndReopensWithExactUndo() throws {
        let source = "before word after"
        let (backend, view, session) = try surface(source, type: EVDocument.markdownType)
        _ = try selectedStyle(NSRange(location: 7, length: 4), view: view, session: session)
        try choose("Comment", in: view)
        XCTAssertEqual(try backend.formattedText(), "before <!-- word --> after")
        let saved = try backend.serializedSource(typeName: EVDocument.markdownType)
        try assertReopenedStyle(saved, type: EVDocument.markdownType,
            range: NSRange(location: 12, length: 4), id: "Comment")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        view.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        view.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), saved)
    }

    func testCommentMenuAtCaretImmediatelyInsertsDelimiters() throws {
        let (backend, view, session) = try surface("", type: EVDocument.markdownType)
        try choose("Comment", in: view)
        XCTAssertTrue(backend.persistenceState.isDirty)
        XCTAssertEqual(try backend.formattedText(), "<!--  -->")
        XCTAssertEqual(try session.listSelection().text_start, 5)
        XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        for text in ["he", "llo"] {
            view.editorView.insertText(text, replacementRange: NSRange(location: NSNotFound, length: 0))
            XCTAssertNil(view.commandOutput)
        }
        XCTAssertEqual(try backend.formattedText(), "<!-- hello -->")
        XCTAssertEqual(try session.listSelection().text_start, 10)
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        view.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.formattedText(), "<!--  -->")
        view.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data())
    }

    func testUnfinishedCommentMenuErrorExplainsHowToRecover() throws {
        for source in ["<!-- hello --", "before <!-- hello -- after", "before <!-- hello --\\> after", "<span><!-- hello --</span>tail"] {
            let (backend, view, session) = try surface(source, type: EVDocument.markdownType)
            let formatted = try backend.formattedText()
            _ = try selectedStyle((formatted as NSString).range(of: "hello"), view: view, session: session)
            let catalogue = try XCTUnwrap(view.currentStyleMenuCatalogue())
            let item = NSMenuItem(title: "Comment", action: #selector(EVStyleMenuActionRouting.performEditorStyleMenuAction(_:)), keyEquivalent: "")
            item.representedObject = EVStyleMenuAction(kind: .assign, role: .character, stableID: "Comment",
                documentID: catalogue.documentID, documentRevision: catalogue.documentRevision,
                styleSheetRevision: catalogue.styleSheetRevision)
            view.editorView.performEditorStyleMenuAction(item)
            XCTAssertTrue(view.commandOutput?.contains("Can’t create a comment inside an unfinished comment, please fix the source.") == true,
                view.commandOutput ?? "Missing error")
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        }
    }

}
