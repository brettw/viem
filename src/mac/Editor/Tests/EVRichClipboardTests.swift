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
            XCTAssertEqual(view.viewPresentation.mode, UInt32(command == nil ? VIEM_MODE_VISUAL_CHARACTER : VIEM_MODE_NORMAL))
            XCTAssertNil(view.commandOutput)
        }
    }

    func testPlatformCopyRetainsVimCharacterLineAndBlockSelectionsInBothDirections() throws {
        let source = "**one two** three\n\nfour five"
        for sequence in ["vll", "vllo", "Vj", "Vjo", "\u{16}lj", "\u{16}ljo"] {
            for useSelector in [false, true] {
                let pasteboard = NSPasteboard.withUniqueName()
                defer { pasteboard.releaseGlobally() }
                let (backend, view, session) = try surface(source, type: EVDocument.markdownType, pasteboard: pasteboard)
                for scalar in sequence.unicodeScalars {
                    _ = try session.sendKey(kind: UInt32(scalar.value == 22 ? VIEM_KEY_CONTROL_CHARACTER : VIEM_KEY_CHARACTER), codepoint: scalar.value == 22 ? 118 : scalar.value)
                }
                view.refreshPresentation()
                let before = view.viewPresentation
                let ranges = view.selectedUTF8Ranges()
                let selected = view.selectionText()
                XCTAssertFalse(ranges.isEmpty)
                if useSelector { view.editorView.copyDocumentSelection(nil) }
                else { view.perform(menuCommand: .copy, sender: nil) }
                XCTAssertEqual(view.viewPresentation.mode, before.mode, sequence)
                XCTAssertEqual(view.viewPresentation.cursor_utf8_offset, before.cursor_utf8_offset, sequence)
                XCTAssertEqual(view.viewPresentation.cursor_affinity, before.cursor_affinity, sequence)
                XCTAssertEqual(view.selectedUTF8Ranges(), ranges, sequence)
                XCTAssertEqual(pasteboard.string(forType: .string), selected, sequence)
                XCTAssertNotNil(pasteboard.data(forType: fragmentType))
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
                XCTAssertNil(view.commandOutput)
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: 121)
                view.refreshPresentation()
                XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
            }
        }
    }

    func testPlatformCopyLeavesPendingMappingAndTemporaryVisualCommandUntouched() throws {
        let source = "**abcdef**"
        for temporaryVisual in [false, true] {
            let pasteboard = NSPasteboard.withUniqueName()
            defer { pasteboard.releaseGlobally() }
            let (backend, view, session) = try surface(source, type: EVDocument.markdownType, pasteboard: pasteboard)
            if temporaryVisual {
                _ = try session.selectAll()
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 111)
            } else {
                try keys(":vmap dd y", session: session, surface: view)
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
                try keys("vld", session: session, surface: view)
            }
            view.refreshPresentation()
            let before = view.viewPresentation
            let ranges = view.selectedUTF8Ranges()
            if temporaryVisual { view.editorView.copyDocumentSelection(nil) }
            else { view.perform(menuCommand: .copy, sender: nil) }
            XCTAssertEqual(view.viewPresentation.mode, before.mode)
            XCTAssertEqual(view.viewPresentation.cursor_utf8_offset, before.cursor_utf8_offset)
            XCTAssertEqual(view.selectedUTF8Ranges(), ranges)
            _ = try richText(pasteboard, expected: temporaryVisual ? "abcdef" : "ab")
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
            try keys(temporaryVisual ? "y" : "d", session: session, surface: view)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
            XCTAssertNil(view.commandOutput)
            if !temporaryVisual { XCTAssertEqual(view.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL)) }
        }
    }

    func testSourceViewCopyAndCopySourceExportOnlyPlainSource() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        for (source, sourceType, formattedType) in [
            ("let café = \"👩‍💻\";", EVDocument.codeType, EVDocument.codeType),
            ("# Title\n\n__café 👩‍💻__", EVDocument.markdownSourceType, EVDocument.markdownType),
        ] {
            let (_, sourceView, sourceSession) = try surface(source, type: sourceType, pasteboard: pasteboard)
            try selectAll(session: sourceSession, surface: sourceView)
            let beforeSourceCopy = sourceView.selectedUTF8Ranges()
            let beforeSourceMode = sourceView.viewPresentation.mode
            sourceView.editorView.copyDocumentSelection(nil)
            XCTAssertEqual(sourceView.selectedUTF8Ranges(), beforeSourceCopy)
            XCTAssertEqual(sourceView.viewPresentation.mode, beforeSourceMode)
            XCTAssertEqual(pasteboard.string(forType: .string), source)
            XCTAssertNil(pasteboard.data(forType: .rtf))
            XCTAssertNil(pasteboard.data(forType: fragmentType))
            for type in [sourceType, formattedType] {
                let (_, view, session) = try surface(source, type: type, pasteboard: pasteboard)
                for useSelector in [false, true] {
                    try selectAll(session: session, surface: view)
                    let beforeRanges = view.selectedUTF8Ranges()
                    let beforeMode = view.viewPresentation.mode
                    if useSelector { view.editorView.copyDocumentSource(nil) }
                    else { view.perform(menuCommand: .copySource, sender: nil) }
                    XCTAssertEqual(view.selectedUTF8Ranges(), beforeRanges)
                    XCTAssertEqual(view.viewPresentation.mode, beforeMode)
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

    func testCodePastesRichClipboardAsExactPlainQuotesAndCopiesWithoutTypography() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        defer { pasteboard.releaseGlobally() }
        let (_, donor, donorSession) = try surface("**\"quoted\"** & *'literal'*", type: EVDocument.markdownType, pasteboard: pasteboard)
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
