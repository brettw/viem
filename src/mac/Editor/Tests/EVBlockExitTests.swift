import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVBlockExitTests: XCTestCase {
    private let codeBackground = EVStyleColor(red: 0.25, green: 0.5, blue: 0.75, alpha: 1)
    private let codeBottomPadding: CGFloat = 12

    private func makeSurface(_ source: String) throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-block-exit-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        try EVStyleTestFixtures.configure(configuration, declarations: [
            (.baseParagraph, .paragraphFirstLineIndent, .float(0)),
            (.baseParagraph, .paragraphLeadingIndent, .float(0)),
            (EVStyleTestFixtures.block("BulletedList1"), .blockPaddingLeft, .float(36)),
            (EVStyleTestFixtures.block("Code Block"), .blockPaddingLeft, .float(24)),
            (EVStyleTestFixtures.block("Code Block"), .blockPaddingTop, .float(8)),
            (EVStyleTestFixtures.block("Code Block"), .blockPaddingBottom, .float(Float(codeBottomPadding))),
            (EVStyleTestFixtures.block("Code Block"), .blockMarginBottom, .float(0)),
            (EVStyleTestFixtures.block("Code Block"), .blockBorderTopWidth, .float(0)),
            (EVStyleTestFixtures.block("Code Block"), .blockBorderBottomWidth, .float(0)),
            (EVStyleTestFixtures.block("Code Block"), .blockBackground, .color(codeBackground)),
            (EVStyleTestFixtures.block("Code Block"), .characterSize, .float(14)),
            (EVStyleTestFixtures.block("Code Block"), .characterFontFamilies, .stringList(["system-ui"])),
        ])
        try EVStyleTestFixtures.configure(configuration, format: .code, declarations: [
            (.baseParagraph, .characterSize, .float(14)),
            (.baseParagraph, .characterFontFamilies, .stringList(["system-ui"])),
        ])
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 400)
        surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }

    private func key(_ code: UInt16 = 36, modifiers: NSEvent.ModifierFlags = []) throws -> NSEvent {
        let character = code == 51 ? "\u{7f}" : code == 76 ? "\u{3}" : "\r"
        return try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: modifiers, timestamp: 0, windowNumber: 0, context: nil,
            characters: character, charactersIgnoringModifiers: character,
            isARepeat: false, keyCode: code))
    }

    private func insertAtEnd(_ backend: EVCoreDocumentBackend, _ surface: EVEditorSurfaceController,
                             _ session: EVCoreViewSession) throws {
        surface.performInput { _ = try session.sendText("i") }
        let text = try backend.formattedText()
        // The end of a final nonempty paragraph is an upstream boundary;
        // AX's downstream-only placement can decline that endpoint. Use the
        // native document-end command to reach the intended insertion point.
        surface.editorView.doCommand(by: #selector(NSResponder.moveToEndOfDocument(_:)))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64(text.utf8.count))
    }

    private func caretRect(_ surface: EVEditorSurfaceController) throws -> NSRect {
        // Inspect the snapshot published by the native input path immediately,
        // before typing or an explicit refresh can repair stale indentation.
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        return try XCTUnwrap(surface.editorView.caretItemGeometry(snapshot).rect)
    }

    private func proseX(_ surface: EVEditorSurfaceController) throws -> CGFloat {
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        return try XCTUnwrap(surface.editorView.caretItemGeometry(snapshot,
            target: .boundary(offset: 0, affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM))).rect).minX
    }

    private func codeFills(_ snapshot: EVLayoutExport) -> [ViemLayoutDecorationV1] {
        snapshot.decorations.filter {
            $0.flags & UInt32(VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND) != 0
                && abs($0.paint.foreground.red - codeBackground.red) < 0.001
                && abs($0.paint.foreground.green - codeBackground.green) < 0.001
                && abs($0.paint.foreground.blue - codeBackground.blue) < 0.001
        }
    }

    private struct CodeBoxGeometry {
        let bounds: NSRect
        let bottomPadding: CGFloat
        let baselineToBottom: CGFloat
    }

    private func nativeCaret(_ surface: EVEditorSurfaceController, in snapshot: EVLayoutExport,
                             file: StaticString = #filePath, line: UInt = #line) throws -> (rect: NSRect, rowIndex: UInt64) {
        let item = surface.editorView.caretItemGeometry(snapshot)
        let rect = try XCTUnwrap(item.rect, file: file, line: line)
        if let cluster = item.cluster { return (rect, cluster.row_index) }
        // Use the actual native insertion caret, which resolves either side of
        // an EOF boundary. Find its row by exported geometry instead of
        // independently choosing a cursor offset or affinity.
        let rows = snapshot.rows.filter {
            abs(surface.editorView.viewPoint(fromLayoutPoint: CGPoint(x: 0, y: CGFloat($0.y))).y - rect.minY) < 0.01
        }
        XCTAssertEqual(rows.count, 1, "The drawn insertion caret must identify one visual row", file: file, line: line)
        return (rect, try XCTUnwrap(rows.first, file: file, line: line).row_index)
    }

    private func codeBoxGeometry(_ surface: EVEditorSurfaceController, caretInside: Bool = true,
                                 file: StaticString = #filePath, line: UInt = #line) throws -> CodeBoxGeometry {
        let snapshot = try XCTUnwrap(surface.layoutSnapshot, file: file, line: line)
        let fills = codeFills(snapshot)
        let lastFill = try XCTUnwrap(fills.max { $0.row_index < $1.row_index },
            "Code must retain its painted container", file: file, line: line)
        let lastRow = try XCTUnwrap(snapshot.rows.first { $0.row_index == lastFill.row_index }, file: file, line: line)
        let bounds = fills.map { surface.editorView.viewRect($0.typographic_bounds) }
            .reduce(NSRect.null) { $0.union($1) }
        let rowBottom = surface.editorView.viewPoint(fromLayoutPoint:
            CGPoint(x: 0, y: CGFloat(lastRow.y + lastRow.line_advance))).y
        let baseline = surface.editorView.viewPoint(fromLayoutPoint:
            CGPoint(x: 0, y: CGFloat(lastRow.baseline))).y
        if caretInside {
            let caret = try nativeCaret(surface, in: snapshot, file: file, line: line)
            XCTAssertTrue(fills.contains { $0.row_index == caret.rowIndex },
                "The blank caret row must itself have code background before typing", file: file, line: line)
            let rect = caret.rect
            XCTAssertLessThanOrEqual(bounds.minY, rect.minY + 0.01, file: file, line: line)
            XCTAssertGreaterThanOrEqual(bounds.maxY, rect.maxY - 0.01, file: file, line: line)
            XCTAssertGreaterThan(rect.minX, bounds.minX, file: file, line: line)
        }
        XCTAssertEqual(bounds.maxY - rowBottom, codeBottomPadding, accuracy: 0.01,
            "The final code row must retain its bottom padding before typing", file: file, line: line)
        return CodeBoxGeometry(bounds: bounds, bottomPadding: bounds.maxY - rowBottom,
            baselineToBottom: bounds.maxY - baseline)
    }

    private func assertCaretOutsideCode(_ surface: EVEditorSurfaceController,
                                        file: StaticString = #filePath, line: UInt = #line) throws {
        let snapshot = try XCTUnwrap(surface.layoutSnapshot, file: file, line: line)
        let caret = try nativeCaret(surface, in: snapshot, file: file, line: line)
        let fills = codeFills(snapshot)
        XCTAssertFalse(fills.contains { $0.row_index == caret.rowIndex }, file: file, line: line)
        let rect = caret.rect
        XCTAssertEqual(rect.minX, CGFloat(snapshot.info.content_insets.left), accuracy: 0.01,
            "An exited paragraph uses the base paragraph's left margin", file: file, line: line)
        if !fills.isEmpty {
            let bottom = fills.map { surface.editorView.viewRect($0.typographic_bounds).maxY }.max()!
            XCTAssertGreaterThanOrEqual(rect.minY, bottom - 0.01, file: file, line: line)
        }
    }

    private func bitmapContext(size: NSSize) throws -> CGContext {
        try XCTUnwrap(CGContext(data: nil, width: Int(size.width), height: Int(size.height),
            bitsPerComponent: 8, bytesPerRow: Int(size.width) * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
    }

    private func bitmap(_ view: NSView) throws -> NSBitmapImageRep {
        let context = try bitmapContext(size: view.bounds.size)
        context.translateBy(x: 0, y: view.bounds.height)
        context.scaleBy(x: 1, y: -1)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
        view.draw(view.bounds)
        NSGraphicsContext.restoreGraphicsState()
        return NSBitmapImageRep(cgImage: try XCTUnwrap(context.makeImage()))
    }

    private func renderedCodeBackground() throws -> NSColor {
        // NSBitmapImageRep's color reads pass through AppKit color management.
        // Paint an independent solid swatch through the same bitmap round trip,
        // rather than comparing its converted components to raw style numbers.
        let context = try bitmapContext(size: NSSize(width: 2, height: 2))
        context.setFillColor(codeBackground.appKitColor.cgColor)
        context.fill(CGRect(x: 0, y: 0, width: 2, height: 2))
        let image = NSBitmapImageRep(cgImage: try XCTUnwrap(context.makeImage()))
        return try XCTUnwrap(image.colorAt(x: 0, y: 0)?.usingColorSpace(.sRGB))
    }

    private func assertCodeBoxMatchesReopenedSource(_ backend: EVCoreDocumentBackend,
                                                    _ surface: EVEditorSurfaceController,
                                                    file: StaticString = #filePath, line: UInt = #line) throws {
        let source = try backend.serializedSource(typeName: EVDocument.markdownType)
        let (_, reopened, _) = try makeSurface(String(decoding: source, as: UTF8.self))
        let current = try codeBoxGeometry(surface)
        let fresh = try codeBoxGeometry(reopened, caretInside: false)
        XCTAssertEqual(current.bounds, fresh.bounds, file: file, line: line)
        XCTAssertEqual(current.bottomPadding, fresh.bottomPadding, accuracy: 0.01, file: file, line: line)
        let rendered = try bitmap(surface.editorView)
        let reference = try bitmap(reopened.editorView)
        let background = try renderedCodeBackground()
        // This stripe is wholly inside the left padding, away from syntax
        // glyphs and the language label. It includes the final empty row and
        // bottom padding, so cached drawing must match a fresh projection.
        let x = Int(ceil(current.bounds.minX + 5))
        for y in Int(ceil(current.bounds.minY + 1))..<Int(floor(current.bounds.maxY)) {
            let pixel = try XCTUnwrap(rendered.colorAt(x: x, y: y)?.usingColorSpace(.sRGB), file: file, line: line)
            let expected = try XCTUnwrap(reference.colorAt(x: x, y: y)?.usingColorSpace(.sRGB), file: file, line: line)
            XCTAssertEqual(pixel.redComponent, background.redComponent, accuracy: 0.01, file: file, line: line)
            XCTAssertEqual(pixel.greenComponent, background.greenComponent, accuracy: 0.01, file: file, line: line)
            XCTAssertEqual(pixel.blueComponent, background.blueComponent, accuracy: 0.01, file: file, line: line)
            XCTAssertEqual(pixel.alphaComponent, 1, accuracy: 0.01, file: file, line: line)
            XCTAssertEqual(pixel.redComponent, expected.redComponent, accuracy: 0.001, file: file, line: line)
            XCTAssertEqual(pixel.greenComponent, expected.greenComponent, accuracy: 0.001, file: file, line: line)
            XCTAssertEqual(pixel.blueComponent, expected.blueComponent, accuracy: 0.001, file: file, line: line)
        }
    }

    private func assertHistory(_ backend: EVCoreDocumentBackend, _ surface: EVEditorSurfaceController,
                               original: String) throws {
        let saved = try backend.serializedSource(typeName: EVDocument.markdownType)
        let text = try backend.formattedText()
        let (reopened, _, _) = try makeSurface(String(decoding: saved, as: UTF8.self))
        XCTAssertEqual(try reopened.formattedText(), text)
        surface.editorView.cancelOperation(nil)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(original.utf8))
        surface.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), saved)
        XCTAssertEqual(try backend.formattedText(), text)
        XCTAssertNil(surface.commandOutput)
    }

    func testReturnAndBackspaceImmediatelyUnindentTheCaretAfterAnEmptyBullet() throws {
        for exitKey in [UInt16(36), 76, 51] {
            let source = "Before\n\n- Item"
            let (backend, surface, session) = try makeSurface(source)
            try insertAtEnd(backend, surface, session)
            surface.editorView.keyDown(with: try key())
            XCTAssertEqual(try backend.formattedText(), "Before\nItem\n")
            XCTAssertEqual(try session.layoutExport().decorations.count, 2)
            XCTAssertGreaterThan(try caretRect(surface).minX, try proseX(surface))

            surface.editorView.keyDown(with: try key(exitKey))
            XCTAssertEqual(try backend.formattedText(), "Before\nItem\n")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
            XCTAssertEqual(try caretRect(surface).minX, try proseX(surface), accuracy: 0.01,
                "exiting with key \(exitKey) must move the empty caret before any text is typed")
            XCTAssertEqual(try session.layoutExport().decorations.count, 1)
            let emptyStart = surface.viewPresentation.cursor_utf8_offset
            surface.editorView.insertText("After", replacementRange: NSRange(location: NSNotFound, length: 0))
            XCTAssertEqual(try backend.formattedText(), "Before\nItem\nAfter")
            let snapshot = try XCTUnwrap(surface.layoutSnapshot)
            let firstTyped = try XCTUnwrap(snapshot.clusters.first { $0.text_start == emptyStart })
            XCTAssertEqual(surface.editorView.viewRect(firstTyped.typographic_bounds).minX,
                try proseX(surface), accuracy: 0.01)
            try assertHistory(backend, surface, original: source)
        }
    }

    func testOptionReturnKeepsTheEmptyContinuationParagraphIndentedInsideItsListItem() throws {
        for returnKey in [UInt16(36), 76] {
            let source = "Before\n\n- Item"
            let (backend, surface, session) = try makeSurface(source)
            try insertAtEnd(backend, surface, session)
            surface.editorView.keyDown(with: try key(returnKey, modifiers: [.option]))
            XCTAssertEqual(try backend.formattedText(), "Before\nItem\n")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertGreaterThan(try caretRect(surface).minX, try proseX(surface),
                "Option-Return creates a paragraph that still belongs to the list item")
            XCTAssertEqual(try session.layoutExport().decorations.count, 1)
            surface.editorView.insertText("Continuation", replacementRange: NSRange(location: NSNotFound, length: 0))
            XCTAssertEqual(try backend.formattedText(), "Before\nItem\nContinuation")
            XCTAssertEqual(String(decoding: try backend.serializedSource(typeName: EVDocument.markdownType), as: UTF8.self),
                "Before\n\n- Item\n\n  Continuation<span></span>")
            XCTAssertEqual(try session.layoutExport().decorations.count, 1)
            try assertHistory(backend, surface, original: source)
        }
    }

    func testThirdReturnExitsTheFinalCodeBlockWithAnImmediatelyUnindentedCaret() throws {
        for source in ["Before\n\n```rust\nbody\n```", "Before\n\n```rust\nbody"] {
            for returnKey in [UInt16(36), 76] {
                let (backend, surface, session) = try makeSurface(source)
                try insertAtEnd(backend, surface, session)
                let filled = try codeBoxGeometry(surface)
                for count in 1...2 {
                    surface.editorView.keyDown(with: try key(returnKey))
                    XCTAssertEqual(try backend.formattedText(), "Before\nbody" + String(repeating: "\n", count: count))
                    XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Code Block",
                        "the first two Returns must keep their blank rows inside code")
                    XCTAssertGreaterThan(try caretRect(surface).minX, try proseX(surface))
                    let empty = try codeBoxGeometry(surface)
                    XCTAssertEqual(empty.baselineToBottom, filled.baselineToBottom, accuracy: 0.01,
                        "Return \(count) must keep the filled row's spacing below the baseline")
                    try assertCodeBoxMatchesReopenedSource(backend, surface)
                }
                surface.editorView.keyDown(with: try key(returnKey))
                XCTAssertEqual(try backend.formattedText(), "Before\nbody\n")
                XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
                XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
                XCTAssertEqual(try caretRect(surface).minX, try proseX(surface), accuracy: 0.01)
                try assertCaretOutsideCode(surface)
                surface.editorView.insertText("After", replacementRange: NSRange(location: NSNotFound, length: 0))
                XCTAssertEqual(try backend.formattedText(), "Before\nbody\nAfter")
                XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
                try assertHistory(backend, surface, original: source)
            }
        }
    }

    func testTwoReturnsAllowTypingAfterABlankCodeLine() throws {
        let source = "Before\n\n```rust\nbody\n```"
        let (backend, surface, session) = try makeSurface(source)
        try insertAtEnd(backend, surface, session)
        surface.editorView.keyDown(with: try key())
        surface.editorView.keyDown(with: try key())
        surface.editorView.insertText("next", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(try backend.formattedText(), "Before\nbody\n\nnext")
        XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Code Block")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("Before\n\n```rust\nbody\n\nnext\n```".utf8))
        try assertHistory(backend, surface, original: source)
    }

    func testSecondReturnExitsFrontMatterAfterKeepingTheFirstBlankRowInsideItsBox() throws {
        for returnKey in [UInt16(36), 76] {
            let source = "---\nbody\n---"
            let (backend, surface, session) = try makeSurface(source)
            try insertAtEnd(backend, surface, session)
            let filled = try codeBoxGeometry(surface)
            surface.editorView.keyDown(with: try key(returnKey))
            XCTAssertEqual(try backend.formattedText(), "body\n")
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Code Block")
            let empty = try codeBoxGeometry(surface)
            XCTAssertEqual(empty.baselineToBottom, filled.baselineToBottom, accuracy: 0.01,
                "The first Return must preserve front matter's spacing below the baseline")
            try assertCodeBoxMatchesReopenedSource(backend, surface)

            surface.editorView.keyDown(with: try key(returnKey))
            XCTAssertEqual(try backend.formattedText(), "body\n")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
            try assertCaretOutsideCode(surface)
            surface.editorView.insertText("After", replacementRange: NSRange(location: NSNotFound, length: 0))
            XCTAssertEqual(try backend.formattedText(), "body\nAfter")
            XCTAssertEqual(String(decoding: try backend.serializedSource(typeName: EVDocument.markdownType), as: UTF8.self),
                "---\nbody\n---\n\nAfter")
            try assertHistory(backend, surface, original: source)
        }
    }

    func testBackspaceRemovesAnEmptyCodeOrFrontMatterEnvelopeAndItsPaintedBox() throws {
        for source in ["```\n```", "```\n\n```", "---\n---", "---\n\n---"] {
            let (backend, surface, session) = try makeSurface(source)
            try insertAtEnd(backend, surface, session)
            XCTAssertEqual(try backend.formattedText(), "")
            _ = try codeBoxGeometry(surface)
            surface.editorView.keyDown(with: try key(51))
            XCTAssertEqual(String(decoding: try backend.serializedSource(typeName: EVDocument.markdownType), as: UTF8.self),
                "", source)
            XCTAssertEqual(try backend.formattedText(), "")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Paragraph")
            XCTAssertTrue(codeFills(try XCTUnwrap(surface.layoutSnapshot)).isEmpty)
            try assertCaretOutsideCode(surface)
            try assertHistory(backend, surface, original: source)
        }
    }

    func testOpenAboveAndBelowKeepBlankCodeAndFrontMatterRowsInsideTheirPaintedBox() throws {
        for delimiter in ["```", "---"] {
            for command in ["o", "O"] {
                let source = "\(delimiter)\nbody\n\(delimiter)"
                let (backend, surface, session) = try makeSurface(source)
                let filled = try codeBoxGeometry(surface)
                XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
                surface.editorView.insertText(command, replacementRange: NSRange(location: NSNotFound, length: 0))
                XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
                XCTAssertEqual(try backend.formattedText(), command == "o" ? "body\n" : "\nbody")
                XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "Code Block")
                let empty = try codeBoxGeometry(surface)
                XCTAssertEqual(empty.baselineToBottom, filled.baselineToBottom, accuracy: 0.01,
                    "\(command) must keep the final row's bottom padding")
                try assertCodeBoxMatchesReopenedSource(backend, surface)
                surface.editorView.insertText("next", replacementRange: NSRange(location: NSNotFound, length: 0))
                let body = command == "o" ? "body\nnext" : "next\nbody"
                XCTAssertEqual(try backend.formattedText(), body)
                XCTAssertEqual(String(decoding: try backend.serializedSource(typeName: EVDocument.markdownType), as: UTF8.self),
                    "\(delimiter)\n\(body)\n\(delimiter)")
                try assertHistory(backend, surface, original: source)
            }
        }
    }
}
