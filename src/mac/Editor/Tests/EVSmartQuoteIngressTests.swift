import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVSmartQuoteIngressTests: XCTestCase {
    private enum InputRoute: CaseIterable {
        case typing, attributedTyping, markedCommit, unmark, explicitReplacement
        case accessibility, paste, pasteAndMatchStyle
    }

    private final class Pasteboard: EVPasteboardAccess {
        var text: String?
        var fragment: Data?
        var viemGeneration: UInt64 = 1
        var viemIsWritable: Bool { true }
        func viemString() -> String? { text }
        func viemCanReadString() -> Bool { text != nil }
        func viemClearContents() -> Int { text = nil; fragment = nil; viemGeneration += 1; return Int(viemGeneration) }
        func viemSetString(_ string: String) -> Bool { text = string; viemGeneration += 1; return true }
        func viemData(forType type: NSPasteboard.PasteboardType) -> Data? {
            type == EVClipboardRepresentations.fragmentType ? fragment : nil
        }
    }

    func testEveryNativeTextIngressUsesSmartQuotesInProseAndOneUndoUnit() throws {
        for route in InputRoute.allCases {
            let (backend, surface, session) = try makeSurface("X", type: EVDocument.plainTextType)
            surface.performInput { _ = try session.sendText("i") }
            try insert("\"word\"", through: route, surface: surface)
            XCTAssertEqual(try backend.formattedText(), "“word”X", "\(route)")
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64("“word”".utf8.count), "\(route)")
            XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: "“word”".utf16.count, length: 0), "\(route)")
            XCTAssertFalse(surface.editorView.hasMarkedText())
            surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.formattedText(), "X", "\(route)")
            surface.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try backend.formattedText(), "“word”X", "\(route)")
        }
    }

    func testCodeSpansAndParagraphsKeepLiteralQuotesAcrossNativeIngress() throws {
        for (source, type) in [
            ("X", EVDocument.codeType),
            ("`X`", EVDocument.markdownType),
            ("```\nX\n```", EVDocument.markdownType),
            ("`X`", EVDocument.markdownSourceType),
            ("```\nX\n```", EVDocument.markdownSourceType),
        ] {
            for route in InputRoute.allCases {
                let (backend, surface, session) = try makeSurface(source, type: type)
                let original = try backend.formattedText()
                try startInsertAtX(surface, session: session)
                try insert("\"word\"", through: route, surface: surface)
                XCTAssertEqual(try backend.formattedText(), original.replacingOccurrences(of: "X", with: "\"word\"X"), "\(type) \(source) \(route)")
                surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
                surface.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            }
        }
    }

    func testNormalReplaceAndItsMarkedTextOperandUseTheSameQuotePolicy() throws {
        for (source, type, expected) in [
            ("X", EVDocument.plainTextType, "“"),
            ("X", EVDocument.codeType, "\""),
            ("`X`", EVDocument.markdownType, "\""),
            ("```\nX\n```", EVDocument.markdownType, "\""),
        ] {
            for marked in [false, true] {
                let (backend, surface, session) = try makeSurface(source, type: type)
                surface.performInput { _ = try session.sendText("r") }
                let view = surface.editorView
                if marked {
                    view.setMarkedText("\"", selectedRange: NSRange(location: 1, length: 0), replacementRange: missingRange)
                    XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
                }
                view.insertText("\"", replacementRange: missingRange)
                XCTAssertEqual(try backend.formattedText(), expected, "\(source) marked=\(marked)")
                XCTAssertFalse(view.hasMarkedText())
                surface.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            }
        }
    }

    func testExplicitNativeReplacementUsesTheReplacedContentCodeContext() throws {
        for (source, type, replacement) in [
            ("X", EVDocument.plainTextType, "“word”"),
            ("X", EVDocument.codeType, "\"word\""),
            ("`X`", EVDocument.markdownType, "\"word\""),
            ("`X`", EVDocument.markdownSourceType, "\"word\""),
        ] {
            let (backend, surface, session) = try makeSurface(source, type: type)
            let original = try backend.formattedText()
            try startInsertAtX(surface, session: session)
            let at = surface.editorView.selectedRange().location
            surface.editorView.insertText("\"word\"", replacementRange: NSRange(location: at, length: 1))
            XCTAssertEqual(try backend.formattedText(), original.replacingOccurrences(of: "X", with: replacement), "\(type)")
            surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        }
    }

    func testNativeReplaceModeCurvesProseAndKeepsCodeLiteral() throws {
        for (source, type, expected) in [
            ("XXXXXX", EVDocument.plainTextType, "“word”"),
            ("XXXXXX", EVDocument.codeType, "\"word\""),
            ("```\nXXXXXX\n```", EVDocument.markdownType, "\"word\""),
        ] {
            let (backend, surface, session) = try makeSurface(source, type: type)
            surface.performInput { _ = try session.sendText("R") }
            surface.editorView.insertText("\"word\"", replacementRange: missingRange)
            XCTAssertEqual(try backend.formattedText(), expected, "\(type)")
            surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        }
    }

    func testAccessibilityWholeValueReplacementUsesSmartQuotesInNormalMode() throws {
        for (source, type, expected) in [
            ("X", EVDocument.plainTextType, "“word”"),
            ("X", EVDocument.codeType, "\"word\""),
        ] {
            let (backend, surface, _) = try makeSurface(source, type: type)
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
            surface.editorView.setAccessibilityValue("\"word\"")
            XCTAssertEqual(try backend.formattedText(), expected)
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        }
    }

    func testRichPasteCurvesProseQuotesAndPreservesCodeQuotesInTheSameFragment() throws {
        let donor = EVCoreDocumentBackend()
        try donor.read(source: Data("**\"prose\"** `\"code\"`".utf8), typeName: EVDocument.markdownType)
        let text = try donor.formattedText()
        let pasteboard = Pasteboard()
        pasteboard.text = text
        pasteboard.fragment = try donor.clipboardFragmentJSON(in: 0..<text.utf8.count, snapshot: donor.formattedSnapshot())
        let (backend, surface, session) = try makeSurface("", type: EVDocument.markdownType)
        surface.pasteboard = pasteboard
        surface.performInput { _ = try session.sendText("i") }
        surface.editorView.pasteIntoDocument(nil)
        XCTAssertEqual(try backend.formattedText(), "“prose” \"code\"")
        XCTAssertEqual(pasteboard.text, text, "Pasting must not rewrite the system clipboard")
        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("".utf8))
    }

    private var missingRange: NSRange { NSRange(location: NSNotFound, length: 0) }

    private func insert(_ text: String, through route: InputRoute, surface: EVEditorSurfaceController) throws {
        let view = surface.editorView
        switch route {
        case .typing:
            view.insertText(text, replacementRange: missingRange)
        case .attributedTyping:
            view.insertText(NSAttributedString(string: text), replacementRange: missingRange)
        case .markedCommit, .unmark:
            let before = try surface.backend.formattedText()
            view.setMarkedText(text, selectedRange: NSRange(location: text.utf16.count, length: 0), replacementRange: missingRange)
            XCTAssertEqual(try surface.backend.formattedText(), before, "Marked text remains an uncommitted overlay")
            XCTAssertEqual(view.attributedSubstring(forProposedRange: view.markedRange(), actualRange: nil)?.string, text)
            if route == .unmark { view.unmarkText() }
            else { view.insertText(text, replacementRange: missingRange) }
        case .explicitReplacement:
            view.insertText(text, replacementRange: view.selectedRange())
        case .accessibility:
            view.setAccessibilitySelectedText(text)
        case .paste, .pasteAndMatchStyle:
            let pasteboard = Pasteboard()
            pasteboard.text = text
            surface.pasteboard = pasteboard
            if route == .paste { view.pasteIntoDocument(nil) }
            else { view.pastePlainTextIntoDocument(nil) }
            XCTAssertEqual(pasteboard.text, text)
        }
    }

    private func startInsertAtX(_ surface: EVEditorSurfaceController, session: EVCoreViewSession) throws {
        surface.performInput {
            _ = try session.sendText("/X")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
            _ = try session.sendText("i")
        }
    }

    private func makeSurface(_ source: String, type: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-quotes-native-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        try configuration.setSmartQuotes(true)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(source.utf8), typeName: type)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.editorView.editingPreferences = EVEditingPreferences(configuration: configuration)
        surface.view.frame = NSRect(x: 0, y: 0, width: 640, height: 240)
        surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }
}
