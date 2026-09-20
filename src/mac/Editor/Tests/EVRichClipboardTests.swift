import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor final class EVRichClipboardTests: XCTestCase {
    private let fragmentType = NSPasteboard.PasteboardType("com.viem.clipboard.fragment.v1")

    private func surface(_ source: String, type: String, pasteboard: NSPasteboard) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: type)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.pasteboard = EVAppKitPasteboardAccess(pasteboard)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 520, height: 180)
        surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }

    private func keys(_ text: String, session: EVCoreViewSession, surface: EVEditorSurfaceController) throws {
        for character in text {
            let scalar = try XCTUnwrap(character.unicodeScalars.first)
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: scalar.value)
        }
        surface.refreshPresentation()
    }

    private func selectAll(session: EVCoreViewSession, surface: EVEditorSurfaceController) throws {
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        // Characterwise selection retains an absent final document newline.
        try keys("ggvG$", session: session, surface: surface)
    }

    private func richText(_ pasteboard: NSPasteboard, expected: String) throws -> NSAttributedString {
        XCTAssertEqual(pasteboard.string(forType: .string), expected)
        XCTAssertNotNil(pasteboard.data(forType: fragmentType))
        let data = try XCTUnwrap(pasteboard.data(forType: .rtf))
        let attributed = try NSAttributedString(data: data, options: [.documentType: NSAttributedString.DocumentType.rtf], documentAttributes: nil)
        XCTAssertEqual(attributed.string.replacingOccurrences(of: "\u{2028}", with: "\n"), expected)
        return attributed
    }

    private func assertFontTrait(_ trait: NSFontDescriptor.SymbolicTraits, in text: NSAttributedString, at substring: String) throws {
        let range = (text.string as NSString).range(of: substring)
        XCTAssertNotEqual(range.location, NSNotFound)
        let font = try XCTUnwrap(text.attribute(.font, at: range.location, effectiveRange: nil) as? NSFont)
        XCTAssertTrue(font.fontDescriptor.symbolicTraits.contains(trait), "Missing \(trait) at \(substring)")
    }

    func testNativeRTFCopyRetainsSuperscriptAndSubscriptSemantics() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        let source = "<p>x<sup>2</sup> H<sub>2</sub>O</p>"
        let (backend, view, session) = try surface(source, type: EVDocument.htmlType, pasteboard: pasteboard)
        try selectAll(session: session, surface: view)
        view.editorView.copyDocumentSelection(nil)
        let attributed = try richText(pasteboard, expected: "x2 H2O")
        XCTAssertEqual((attributed.attribute(.superscript, at: 1, effectiveRange: nil) as? NSNumber)?.intValue, 1)
        XCTAssertEqual((attributed.attribute(.superscript, at: 4, effectiveRange: nil) as? NSNumber)?.intValue, -1)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8))
    }

    func testNativeCopyExportsVisibleUnicodeWithCharacterAndParagraphFormatting() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        let source = "<p style='text-align:end;margin-block-end:12pt'><b>Bold 👩‍💻</b> <i>café العربية</i><br><span style='color:#336699'>中</span></p>"
        let (backend, view, session) = try surface(source, type: EVDocument.htmlType, pasteboard: pasteboard)
        try selectAll(session: session, surface: view)
        view.editorView.copyDocumentSelection(nil)
        let attributed = try richText(pasteboard, expected: "Bold 👩‍💻 café العربية\n中")
        try assertFontTrait(.bold, in: attributed, at: "Bold")
        try assertFontTrait(.italic, in: attributed, at: "café")
        let chinese = (attributed.string as NSString).range(of: "中").location
        let color = try XCTUnwrap((attributed.attribute(.foregroundColor, at: chinese, effectiveRange: nil) as? NSColor)?.usingColorSpace(.deviceRGB))
        XCTAssertEqual(color.redComponent, 0.2, accuracy: 0.015)
        XCTAssertEqual(color.greenComponent, 0.4, accuracy: 0.015)
        XCTAssertEqual(color.blueComponent, 0.6, accuracy: 0.015)
        let paragraph = try XCTUnwrap(attributed.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSParagraphStyle)
        XCTAssertEqual(paragraph.alignment, .right)
        XCTAssertEqual(paragraph.paragraphSpacing, 12, accuracy: 0.1)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8))
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertNil(view.commandOutput)
    }

    func testMenuCopyAndExplicitPrimaryCopyExportMarkdownFormatting() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        let source = "__Bold__ and *café 👩‍💻*"
        let (backend, view, session) = try surface(source, type: EVDocument.markdownType, pasteboard: pasteboard)
        for command in [nil, "\"*y", "\"*c"] {
            try selectAll(session: session, surface: view)
            if let command {
                try keys(command, session: session, surface: view)
            } else {
                view.perform(menuCommand: .copy, sender: nil)
            }
            let attributed = try richText(pasteboard, expected: "Bold and café 👩‍💻")
            try assertFontTrait(.bold, in: attributed, at: "Bold")
            try assertFontTrait(.italic, in: attributed, at: "café")
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
            XCTAssertFalse(backend.persistenceState.isDirty)
            XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
            XCTAssertNil(view.commandOutput)
        }
    }

    func testPartialCopyRetainsOriginalRTLParagraphDirectionForEndAlignment() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        let source = "<p style='text-align:end'>العربية English</p>"
        let (_, view, session) = try surface(source, type: EVDocument.htmlType, pasteboard: pasteboard)
        // Only Latin text is selected. Its paragraph direction still comes
        // from the original paragraph's unselected first strong character.
        try keys("fEv$", session: session, surface: view)
        view.perform(menuCommand: .copy, sender: nil)
        let attributed = try richText(pasteboard, expected: "English")
        let paragraph = try XCTUnwrap(attributed.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSParagraphStyle)
        XCTAssertEqual(paragraph.baseWritingDirection, .rightToLeft)
        XCTAssertEqual(paragraph.alignment, .left)
        XCTAssertNil(view.commandOutput)
    }

    func testHTMLHardBreakRemainsInsideOneRTFParagraph() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        let source = "<p style='margin-block-end:12pt'>a<br>b</p><p>tail</p>"
        let (_, view, session) = try surface(source, type: EVDocument.htmlType, pasteboard: pasteboard)
        try selectAll(session: session, surface: view)
        view.perform(menuCommand: .copy, sender: nil)
        let attributed = try richText(pasteboard, expected: "a\nb\ntail")
        XCTAssertEqual(attributed.string, "a\u{2028}b\ntail")
        let text = attributed.string as NSString
        XCTAssertEqual(text.paragraphRange(for: NSRange(location: 0, length: 1)), NSRange(location: 0, length: 4))
        XCTAssertEqual(text.paragraphRange(for: NSRange(location: 2, length: 1)), NSRange(location: 0, length: 4))
        XCTAssertEqual(text.paragraphRange(for: NSRange(location: 4, length: 1)), NSRange(location: 4, length: 4))
        let paragraph = try XCTUnwrap(attributed.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSParagraphStyle)
        XCTAssertEqual(paragraph.paragraphSpacing, 12, accuracy: 0.1)
        XCTAssertNil(view.commandOutput)
    }

    func testSourceViewCopyAndCopySourceExportOnlyPlainSource() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        for (source, sourceType, formattedType) in [
            ("let café = \"👩‍💻\";", EVDocument.codeType, EVDocument.codeType),
            ("<p><b>café 👩‍💻</b></p>", EVDocument.htmlSourceType, EVDocument.htmlType),
            ("# Title\n\n__café 👩‍💻__", EVDocument.markdownSourceType, EVDocument.markdownType),
        ] {
            let (_, sourceView, sourceSession) = try surface(source, type: sourceType, pasteboard: pasteboard)
            try selectAll(session: sourceSession, surface: sourceView)
            sourceView.editorView.copyDocumentSelection(nil)
            XCTAssertEqual(pasteboard.string(forType: .string), source)
            XCTAssertNil(pasteboard.data(forType: .rtf))
            XCTAssertNil(pasteboard.data(forType: fragmentType))
            for type in [sourceType, formattedType] {
                let (_, view, session) = try surface(source, type: type, pasteboard: pasteboard)
                for useSelector in [false, true] {
                    try selectAll(session: session, surface: view)
                    if useSelector { view.editorView.copyDocumentSource(nil) }
                    else { view.perform(menuCommand: .copySource, sender: nil) }
                    XCTAssertEqual(pasteboard.string(forType: .string), source)
                    XCTAssertNil(pasteboard.data(forType: .rtf))
                    XCTAssertNil(pasteboard.data(forType: fragmentType))
                    XCTAssertNil(view.commandOutput)
                }
            }
        }
    }

    func testSameFormatPastePreservesExactSourceAndUndoRedo() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        for (source, type) in [
            ("__Bold__ café\n\nSecond *italic*", EVDocument.markdownType),
            ("<p data-keep='x'><b>Bold</b> &amp; <i>café</i></p><!--keep-->", EVDocument.htmlType),
        ] {
            for useSelectAllMenu in [false, true] {
                let (original, sourceView, sourceSession) = try surface(source, type: type, pasteboard: pasteboard)
                if useSelectAllMenu { sourceView.perform(menuCommand: .selectAll, sender: nil) }
                else { try selectAll(session: sourceSession, surface: sourceView) }
                sourceView.perform(menuCommand: .copy, sender: nil)
                // Native Select All uses a line selection internally, but Copy
                // must retain this document's absent final newline.
                _ = try richText(pasteboard, expected: try original.formattedText())
                let (target, targetView, _) = try surface("", type: type, pasteboard: pasteboard)
                targetView.perform(menuCommand: .paste, sender: nil)
                XCTAssertEqual(try target.serializedSource(typeName: type), Data(source.utf8))
                XCTAssertEqual(try target.formattedText(), try original.formattedText())
                XCTAssertNil(targetView.commandOutput)
                targetView.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try target.serializedSource(typeName: type), Data())
                targetView.perform(menuCommand: .redo, sender: nil)
                XCTAssertEqual(try target.serializedSource(typeName: type), Data(source.utf8))
            }
        }
    }

    func testPasteAndMatchStyleIgnoresPrivateSourceAndRichFormatting() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        for (source, type, markup) in [
            ("__Bold__", EVDocument.markdownType, "__"),
            ("<p><b>Bold</b></p>", EVDocument.htmlType, "<b>"),
        ] {
            let (_, sourceView, sourceSession) = try surface(source, type: type, pasteboard: pasteboard)
            try selectAll(session: sourceSession, surface: sourceView)
            sourceView.perform(menuCommand: .copy, sender: nil)
            XCTAssertNotNil(pasteboard.data(forType: fragmentType))
            let (target, targetView, _) = try surface("", type: type, pasteboard: pasteboard)
            targetView.perform(menuCommand: .pasteAndMatchStyle, sender: nil)
            XCTAssertEqual(try target.formattedText(), "Bold")
            let saved = try target.serializedSource(typeName: type)
            XCTAssertFalse(String(decoding: saved, as: UTF8.self).contains(markup))
            XCTAssertNotEqual(saved, Data(source.utf8))
            XCTAssertNil(targetView.commandOutput)
        }
    }

    func testNativePasteNormalizesLineEndingsAndShowsUnsupportedNullWithoutPrompt() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        let input = "é\0\r\nx\0y\rz"
        let source = "<p>AB</p>"
        for matchStyle in [false, true] {
            pasteboard.clearContents()
            XCTAssertTrue(pasteboard.setString(input, forType: .string))
            let (backend, view, session) = try surface(source, type: EVDocument.htmlType, pasteboard: pasteboard)
            try keys("a", session: session, surface: view)
            view.perform(menuCommand: matchStyle ? .pasteAndMatchStyle : .paste, sender: nil)
            XCTAssertEqual(try backend.formattedText(), "Aé␀\nx␀y\nzB")
            XCTAssertEqual(pasteboard.string(forType: .string), input)
            XCTAssertNil(view.commandOutput)
            XCTAssertNil(NSApp.modalWindow)
            let saved = try backend.serializedSource(typeName: EVDocument.htmlType)
            let reopened = EVCoreDocumentBackend()
            try reopened.read(source: saved, typeName: EVDocument.htmlType)
            XCTAssertEqual(try reopened.formattedText(), "Aé␀\nx␀y\nzB")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.undo()
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8))
        }
    }

    func testCodePastesRichClipboardAsExactPlainQuotesAndCopiesWithoutTypography() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        let (_, donor, donorSession) = try surface("<p><b>\"quoted\"</b> &amp; <i>'literal'</i></p>", type: EVDocument.htmlType, pasteboard: pasteboard)
        try selectAll(session: donorSession, surface: donor)
        donor.perform(menuCommand: .copy, sender: nil)
        XCTAssertNotNil(pasteboard.data(forType: fragmentType))
        let (backend, target, session) = try surface("", type: EVDocument.codeType, pasteboard: pasteboard)
        target.perform(menuCommand: .paste, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data("\"quoted\" & 'literal'".utf8))
        try selectAll(session: session, surface: target)
        target.perform(menuCommand: .copy, sender: nil)
        XCTAssertEqual(pasteboard.string(forType: .string), "\"quoted\" & 'literal'")
        XCTAssertNil(pasteboard.data(forType: fragmentType))
        XCTAssertNil(pasteboard.data(forType: .rtf))
    }

    func testSelectAllCutThenPastePreservesRichSourceAndHistory() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        for (source, type) in [
            ("<p data-keep='x'><b>Bold</b> &amp; <i>café</i></p><!--keep-->", EVDocument.htmlType),
        ] {
            let (original, sourceView, _) = try surface(source, type: type, pasteboard: pasteboard)
            let visible = try original.formattedText()
            sourceView.perform(menuCommand: .selectAll, sender: nil)
            sourceView.perform(menuCommand: .cut, sender: nil)
            XCTAssertNil(sourceView.commandOutput)
            XCTAssertEqual(try original.formattedText(), "")
            _ = try richText(pasteboard, expected: visible)
            let cutSource = try original.serializedSource(typeName: type)
            XCTAssertNil(sourceView.commandOutput)

            let (target, targetView, _) = try surface("", type: type, pasteboard: pasteboard)
            targetView.perform(menuCommand: .paste, sender: nil)
            XCTAssertEqual(try target.serializedSource(typeName: type), Data(source.utf8))
            XCTAssertEqual(try target.formattedText(), visible)
            XCTAssertNil(targetView.commandOutput)
            targetView.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try target.serializedSource(typeName: type), Data())
            targetView.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try target.serializedSource(typeName: type), Data(source.utf8))
            sourceView.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try original.serializedSource(typeName: type), Data(source.utf8))
            sourceView.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try original.serializedSource(typeName: type), cutSource)
        }
    }

    func testPartialCopyExcludesUnselectedSourceAndFullCopyIncludesOffscreenText() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        let source = "<p><b>selected</b></p>" + (0..<80).map { "<p>outside-marker-\($0)</p>" }.joined()
        let (backend, view, session) = try surface(source, type: EVDocument.htmlType, pasteboard: pasteboard)
        try keys("v7l", session: session, surface: view)
        view.perform(menuCommand: .copy, sender: nil)
        _ = try richText(pasteboard, expected: "selected")
        let fragment = try XCTUnwrap(pasteboard.data(forType: fragmentType))
        let object = try JSONSerialization.jsonObject(with: fragment)
        XCTAssertFalse(containsMarker("outside-marker", in: object))
        try selectAll(session: session, surface: view)
        view.perform(menuCommand: .copy, sender: nil)
        _ = try richText(pasteboard, expected: try backend.formattedText())
        XCTAssertTrue(pasteboard.string(forType: .string)?.hasSuffix("outside-marker-79") == true)
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    func testVisualBlockCopyExportsStyledRowsAndOnlySelectedSourceSegments() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        let source = "<p><b>ab</b> outside-marker-one<br><b>ab</b> outside-marker-two</p><!--outside-marker-whole-->"
        let (backend, view, session) = try surface(source, type: EVDocument.htmlType, pasteboard: pasteboard)
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: UInt32(Character("v").asciiValue!))
        try keys("lj", session: session, surface: view)
        let secondRow = "ab outside-marker-one\n".utf8.count
        XCTAssertEqual(view.selectedUTF8Ranges(), [0..<2, secondRow..<(secondRow + 2)])
        view.perform(menuCommand: .copy, sender: nil)
        let attributed = try richText(pasteboard, expected: "ab\nab")
        for position in [0, 3] {
            let font = try XCTUnwrap(attributed.attribute(.font, at: position, effectiveRange: nil) as? NSFont)
            XCTAssertTrue(font.fontDescriptor.symbolicTraits.contains(.bold))
        }
        let json = try XCTUnwrap(pasteboard.data(forType: fragmentType))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: json) as? [String: Any])
        XCTAssertFalse(containsMarker("outside-marker", in: object))
        let segments = try XCTUnwrap(object["source_segments"] as? [[String: Any]])
        XCTAssertEqual(segments.count, 2)
        for (row, segment) in segments.enumerated() {
            XCTAssertEqual(segment["row"] as? Int, row)
            XCTAssertEqual(segment["start"] as? Int, row * 3)
            XCTAssertEqual(segment["end"] as? Int, row * 3 + 2)
            let fragment = try XCTUnwrap(segment["fragment"] as? [String: Any])
            XCTAssertEqual(fragment["plain_text"] as? String, "ab")
            XCTAssertTrue((fragment["source_text"] as? String)?.contains("<b>ab</b>") == true)
        }
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8))
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertNil(view.commandOutput)
    }

    func testInvalidPrivateDataAllowsTypingAndFallsBackToPlainPaste() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        let (_, sourceView, sourceSession) = try surface("__Bold__", type: EVDocument.markdownType, pasteboard: pasteboard)
        try selectAll(session: sourceSession, surface: sourceView)
        sourceView.perform(menuCommand: .copy, sender: nil)
        let validFragment = try XCTUnwrap(pasteboard.data(forType: fragmentType))

        // A valid fragment for different plain text is stale just as malformed
        // private JSON is unusable. Neither may make unrelated input fail.
        for fragment in [Data("{malformed".utf8), validFragment] {
            let item = NSPasteboardItem()
            XCTAssertTrue(item.setString("fallback", forType: .string))
            XCTAssertTrue(item.setData(fragment, forType: fragmentType))
            pasteboard.clearContents()
            XCTAssertTrue(pasteboard.writeObjects([item]))
            let (target, view, session) = try surface("", type: EVDocument.markdownType, pasteboard: pasteboard)
            try keys("i", session: session, surface: view)
            view.editorView.insertText("x", replacementRange: NSRange(location: NSNotFound, length: 0))
            XCTAssertEqual(try target.formattedText(), "x")
            XCTAssertNil(view.commandOutput)
            view.perform(menuCommand: .paste, sender: nil)
            XCTAssertEqual(try target.formattedText(), "xfallback")
            XCTAssertEqual(try target.serializedSource(typeName: EVDocument.markdownType), Data("xfallback".utf8))
            XCTAssertNil(view.commandOutput)
        }
    }

    private func containsMarker(_ marker: String, in value: Any) -> Bool {
        if let string = value as? String { return string.contains(marker) }
        if let object = value as? [String: Any] { return object.values.contains { containsMarker(marker, in: $0) } }
        if let array = value as? [Any] {
            if let bytes = array as? [UInt8], String(decoding: bytes, as: UTF8.self).contains(marker) { return true }
            return array.contains { containsMarker(marker, in: $0) }
        }
        return false
    }
}
