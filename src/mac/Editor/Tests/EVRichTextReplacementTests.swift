import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVRichTextReplacementTests: XCTestCase {
    private let noRange = NSRange(location: NSNotFound, length: 0)

    private func fixture(_ source: String, width: CGFloat = 600) throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession, NSWindow) {
        let configuration = EVConfigurationStore(directory: FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-replacement-\(UUID().uuidString)"), legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(source.utf8), typeName: EVDocument.rtfType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 400),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = surface
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: width, height: 400)
        surface.viewDidLayout()
        window.makeFirstResponder(surface.editorView)
        return (backend, surface, try XCTUnwrap(surface.session), window)
    }

    func testNativeReplacementAndIMEUseFirstSelectedCharacterStyle() throws {
        let source = #"{\rtf1{\pard baseparagraph {\b bold} baseparagraph}{\*\comment keep}}"#
        let original = "baseparagraph bold baseparagraph"
        for (selected, bold) in [("aph bold base", false), ("bold base", true),
                                 ("bold", true), ("ol", true), ("aph bold", false)] {
            for composition in [false, true] {
                let (backend, surface, session, window) = try fixture(source)
                defer { window.close() }
                let range = (original as NSString).range(of: selected)
                surface.editorView.setAccessibilitySelectedTextRange(range)
                if composition {
                    surface.editorView.setMarkedText("X", selectedRange: NSRange(location: 1, length: 0),
                                                     replacementRange: noRange)
                }
                surface.editorView.insertText("X", replacementRange: noRange)
                surface.editorView.insertText("Y", replacementRange: noRange)
                XCTAssertEqual(try backend.formattedText(), (original as NSString).replacingCharacters(in: range, with: "XY"))
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                surface.refreshPresentation()
                surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: range.location, length: 2))
                let formatting = try session.selectedFormatting()
                let replacedSource = try backend.serializedSource(typeName: EVDocument.rtfType)
                XCTAssertEqual(formatting[.characterBold], .boolean(bold), "\(selected), IME=\(composition)")
                XCTAssertFalse(formatting.mixed.contains(.characterBold),
                               "\(selected), IME=\(composition): \(String(decoding: replacedSource, as: UTF8.self))")
                XCTAssertNil(surface.commandOutput)
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                _ = try session.undo()
                if composition {
                    // IME commits have their own undo unit; later typing is separate.
                    XCTAssertEqual(try backend.formattedText(),
                                   (original as NSString).replacingCharacters(in: range, with: "X"))
                    _ = try session.undo()
                }
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.rtfType), Data(source.utf8),
                               "\(selected), IME=\(composition)")
            }
        }
    }

    func testWholeDocumentReplacementRetainsParagraphStyleWithNativeAndIMEInput() throws {
        let source = #"{\rtf1{\stylesheet{\s0 Paragraph;}{\s1\sbasedon0\snext0\b\fs44 Heading1;}{\s2\sbasedon0\snext0\b\fs40 Heading2;}{\s3\sbasedon0\snext0\b\fs36 Heading3;}{\s4\sbasedon0\snext0\b\fs32 Heading4;}{\s5\sbasedon0\snext0\b\fs28 Heading5;}{\s6\sbasedon0\snext0\b\fs24 Heading6;}}{\pard \s2 \qc Heading}}"#
        for composition in [false, true] {
            let (backend, surface, session, window) = try fixture(source)
            defer { window.close() }
            surface.perform(menuCommand: .selectAll, sender: nil)
            if composition {
                surface.editorView.setMarkedText("X", selectedRange: NSRange(location: 1, length: 0),
                                                 replacementRange: noRange)
            }
            surface.editorView.insertText("X", replacementRange: noRange)
            surface.editorView.insertText("Y", replacementRange: noRange)
            XCTAssertEqual(try backend.formattedText(), "XY")
            let formatting = try session.selectedFormatting()
            XCTAssertEqual(formatting[.paragraphAlignment],
                           .paragraphAlignment(UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER)))
            let saved = try backend.serializedSource(typeName: EVDocument.rtfType)
            XCTAssertTrue(String(decoding: saved, as: UTF8.self).contains("\\s2"))
            XCTAssertNil(surface.commandOutput)
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.undo()
            if composition { _ = try session.undo() }
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.rtfType), Data(source.utf8))
        }
    }

    func testWhitespaceReplacementPreservesStyleAndUndoWithNativeAndIMEInput() throws {
        let source = #"{\rtf1{\pard A {\b B} C}\par {\pard D}}"#
        for composition in [false, true] {
            let (backend, surface, session, window) = try fixture(source)
            defer { window.close() }
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 2, length: 3))
            if composition {
                surface.editorView.setMarkedText(" ", selectedRange: NSRange(location: 1, length: 0),
                                                 replacementRange: noRange)
            }
            surface.editorView.insertText(" ", replacementRange: noRange)
            XCTAssertEqual(try backend.formattedText().replacingOccurrences(of: "\u{a0}", with: " "),
                           "A  \nD")
            XCTAssertNil(surface.commandOutput)
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            surface.refreshPresentation()
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 1, length: 1))
            XCTAssertEqual(try session.selectedFormatting()[.characterBold], .boolean(false))
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 2, length: 1))
            XCTAssertEqual(try session.selectedFormatting()[.characterBold], .boolean(true))
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.undo()
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.rtfType), Data(source.utf8))
        }
    }

}
