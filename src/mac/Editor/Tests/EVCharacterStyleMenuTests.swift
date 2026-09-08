import AppKit
import CEvimCore
import EvimAppShell
import XCTest
@testable import EvimEditor

@MainActor final class EVCharacterStyleMenuTests: XCTestCase {
    private func surface(_ source: String, type: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: type)
        let view = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        view.loadViewIfNeeded()
        view.view.frame = NSRect(x: 0, y: 0, width: 520, height: 240)
        view.viewDidLayout()
        return (backend, view, try XCTUnwrap(view.session))
    }

    private func keys(_ text: String, view: EVEditorSurfaceController, session: EVCoreViewSession) throws {
        for character in text {
            _ = try session.sendKey(kind: UInt32(EVIM_KEY_CHARACTER), codepoint: UInt32(try XCTUnwrap(character.asciiValue)))
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

    private func customStyle(backend: EVCoreDocumentBackend, session: EVCoreViewSession) throws -> EVStyleKey {
        _ = try session.setIncludeStyleDefinitionsInFile(true, expected: backend.documentState())
        let key = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Accent"))
        _ = try session.createStyle(key, name: "Accent", identity: backend.styleSheetSnapshot().identity)
        _ = try session.editStyle(key: key, expected: backend.styleSheetSnapshot().identity,
            mutation: .setDeclaration(.characterSize, .float(22)))
        return key
    }

    private func assertReopenedStyle(_ saved: Data, type: String, range: NSRange, id: String) throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: saved, typeName: type)
        let view = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        view.loadViewIfNeeded()
        let session = try XCTUnwrap(view.session)
        XCTAssertEqual(try selectedStyle(range, view: view, session: session)?.rawValue, id)
    }

    func testBuiltinStyleAtCaretSurvivesInsertEntryAndRoundTripsTypedText() throws {
        for (source, type) in [("base", EVDocument.markdownType), ("<p data-keep='yes'>base</p><!--keep-->", EVDocument.htmlType)] {
            for (before, after, insertion) in [("i", "", 0), ("", "i", 0), ("", "a", 1)] {
                let (backend, view, session) = try surface(source, type: type)
                try keys(before, view: view, session: session)
                let revision = try backend.revision()
                let cursor = try session.listSelection().text_start
                let mode = view.viewPresentation.mode
                try choose("Code", in: view)
                XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
                XCTAssertEqual(try backend.revision(), revision)
                XCTAssertEqual(try session.listSelection().text_start, cursor)
                XCTAssertEqual(view.viewPresentation.mode, mode)
                XCTAssertFalse(backend.persistenceState.isDirty)
                try keys(after, view: view, session: session)
                view.editorView.insertText("XY", replacementRange: NSRange(location: NSNotFound, length: 0))
                _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
                view.refreshPresentation()
                XCTAssertEqual(try backend.formattedText(), insertion == 0 ? "XYbase" : "bXYase")
                let range = NSRange(location: insertion, length: 2)
                XCTAssertEqual(try selectedStyle(range, view: view, session: session)?.rawValue, "Code")
                let saved = try backend.serializedSource(typeName: type)
                try assertReopenedStyle(saved, type: type, range: range, id: "Code")
                _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
                view.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
                view.perform(menuCommand: .redo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: type), saved)
                XCTAssertNil(view.commandOutput)
            }
        }
    }

    func testCustomStyleAtInsertCaretPersistsDefinitionAndTypedAssignment() throws {
        let type = EVDocument.htmlType
        let (backend, view, session) = try surface("<p>base</p>", type: type)
        let key = try customStyle(backend: backend, session: session)
        try keys("A", view: view, session: session)
        let before = try backend.serializedSource(typeName: type)
        let revision = try backend.revision()
        try choose(key.id.rawValue, in: view)
        XCTAssertEqual(try backend.serializedSource(typeName: type), before)
        XCTAssertEqual(try backend.revision(), revision)
        view.editorView.insertText("é", replacementRange: NSRange(location: NSNotFound, length: 0))
        _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
        view.refreshPresentation()
        XCTAssertEqual(try backend.formattedText(), "baseé")
        let range = NSRange(location: 4, length: 1)
        XCTAssertEqual(try selectedStyle(range, view: view, session: session), key.id)
        let saved = try backend.serializedSource(typeName: type)
        try assertReopenedStyle(saved, type: type, range: range, id: key.id.rawValue)
        let reopened = EVCoreDocumentBackend()
        try reopened.read(source: saved, typeName: type)
        XCTAssertEqual(try reopened.styleSheetSnapshot().definition(for: key)?.properties[.characterSize]?.declared, .float(22))
        _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
        view.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: type), before)
        view.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: type), saved)
    }

    func testChoosingBaseCharacterStopsPendingNamedStyle() throws {
        for type in [EVDocument.markdownType, EVDocument.htmlType] {
            let (backend, view, session) = try surface("", type: type)
            try keys("i", view: view, session: session)
            try choose("Code", in: view)
            view.editorView.insertText("X", replacementRange: NSRange(location: NSNotFound, length: 0))
            let beforeBaseChoice = try backend.serializedSource(typeName: type)
            let revision = try backend.revision()
            try choose("Character", in: view)
            XCTAssertEqual(try backend.serializedSource(typeName: type), beforeBaseChoice)
            XCTAssertEqual(try backend.revision(), revision)
            view.editorView.insertText("Y", replacementRange: NSRange(location: NSNotFound, length: 0))
            _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
            view.refreshPresentation()
            XCTAssertEqual(try backend.formattedText(), "XY")
            XCTAssertEqual(try selectedStyle(NSRange(location: 0, length: 1), view: view, session: session)?.rawValue, "Code")
            XCTAssertEqual(try selectedStyle(NSRange(location: 1, length: 1), view: view, session: session)?.rawValue, "Character")
            let saved = try backend.serializedSource(typeName: type)
            try assertReopenedStyle(saved, type: type, range: NSRange(location: 0, length: 1), id: "Code")
            try assertReopenedStyle(saved, type: type, range: NSRange(location: 1, length: 1), id: "Character")
            XCTAssertNil(view.commandOutput)
        }
    }

    func testBuiltinCharacterStyleAppliesToSelectedText() throws {
        for (source, type) in [("one two", EVDocument.markdownType), ("<p data-keep='x'>one two</p><!--keep-->", EVDocument.htmlType)] {
            let (backend, view, session) = try surface(source, type: type)
            let range = NSRange(location: 0, length: 3)
            _ = try selectedStyle(range, view: view, session: session)
            try choose("Code", in: view)
            XCTAssertEqual(try backend.formattedText(), "one two")
            XCTAssertEqual(try session.selectedNamedStyles().character?.rawValue, "Code")
            let saved = try backend.serializedSource(typeName: type)
            try assertReopenedStyle(saved, type: type, range: range, id: "Code")
            XCTAssertEqual(try selectedStyle(NSRange(location: 4, length: 3), view: view, session: session)?.rawValue, "Character")
            _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
            view.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            view.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), saved)
        }
    }

    func testCharacterStyleSpansParagraphsAndLeavesOutsideTextUnassigned() throws {
        for (source, type, id) in [
            ("one\n\ntwo\n\nthree", EVDocument.markdownType, "Code"),
            ("<p data-keep='a'>one</p><!--between--><p data-keep='b'>two</p><p>three</p><!--keep-->", EVDocument.htmlType, "Accent"),
        ] {
            let (backend, view, session) = try surface(source, type: type)
            if id == "Accent" { _ = try customStyle(backend: backend, session: session) }
            let before = try backend.serializedSource(typeName: type)
            let range = NSRange(location: 1, length: 8)
            _ = try selectedStyle(range, view: view, session: session)
            try choose(id, in: view)
            XCTAssertEqual(try backend.formattedText(), "one\ntwo\nthree")
            XCTAssertEqual(try session.selectedNamedStyles().character?.rawValue, id)
            let saved = try backend.serializedSource(typeName: type)
            try assertReopenedStyle(saved, type: type, range: range, id: id)
            XCTAssertEqual(try selectedStyle(NSRange(location: 0, length: 1), view: view, session: session)?.rawValue, "Character")
            XCTAssertEqual(try selectedStyle(NSRange(location: 9, length: 4), view: view, session: session)?.rawValue, "Character")
            _ = try selectedStyle(NSRange(location: 0, length: 13), view: view, session: session)
            XCTAssertTrue(try session.selectedNamedStyles().characterMixed)
            XCTAssertTrue(try XCTUnwrap(view.currentStyleMenuCatalogue()).entries.filter { $0.role == .character }.allSatisfy { $0.presentation.state == .off })
            if type == EVDocument.htmlType {
                XCTAssertTrue(String(decoding: saved, as: UTF8.self).contains("<!--between-->"))
                XCTAssertTrue(String(decoding: saved, as: UTF8.self).hasSuffix("<!--keep-->"))
            }
            _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
            view.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), before)
            view.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), saved)
        }
    }

}
