import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

final class EVEditingReliabilityTests: XCTestCase {
    @MainActor
    func testIndentedMarkdownCodeEnterAndTypingRoundTrip() throws {
        let source = "    code\n\nTail"
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        XCTAssertEqual(surface.formattedText, "code\nTail")
        let client: NSTextInputClient = surface.editorView
        let implicit = NSRange(location: NSNotFound, length: 0)
        client.insertText("A", replacementRange: implicit)
        surface.performInput { _ = try surface.session?.sendKey(kind: UInt32(VIEM_KEY_ENTER)) }
        client.insertText("more", replacementRange: implicit)
        surface.performInput { _ = try surface.session?.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        XCTAssertEqual(surface.statusBarState.message, "")
        XCTAssertEqual(surface.formattedText, "code\nmore\nTail")
        let saved = try backend.serializedSource(typeName: EVDocument.markdownType)
        let reopened = EVCoreDocumentBackend()
        try reopened.read(source: saved, typeName: EVDocument.markdownType)
        let reopenedSurface = try XCTUnwrap(reopened.makeEditorSurface() as? EVEditorSurfaceController)
        reopenedSurface.loadViewIfNeeded()
        XCTAssertEqual(reopenedSurface.formattedText, surface.formattedText)
        client.insertText("u", replacementRange: implicit)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    @MainActor
    func testMarkdownSourceBulletEnterAndContinuedTypingPreserveTheNewItem() throws {
        for (prefix, tail) in [
            ("- ", ""), ("- ", "\n- Last"),
            ("> - ", ""), ("> - ", "\n> - Last"),
            ("  - ", ""), ("  - ", "\n  - Last"),
        ] {
            let source = "\(prefix)First\(tail)"
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownSourceType)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            let client: NSTextInputClient = surface.editorView
            let implicit = NSRange(location: NSNotFound, length: 0)
            client.insertText("A", replacementRange: implicit)
            surface.performInput { _ = try surface.session?.sendKey(kind: UInt32(VIEM_KEY_ENTER)) }
            XCTAssertEqual(surface.statusBarState.message, "")
            client.insertText("Added", replacementRange: implicit)
            surface.performInput { _ = try surface.session?.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
            XCTAssertEqual(surface.statusBarState.message, "")
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType),
                           Data("\(prefix)First\n\(prefix)Added\(tail)".utf8))
            client.insertText("u", replacementRange: implicit)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data(source.utf8))
        }
    }

    @MainActor
    func testMarkdownSourceListEnterAndOpenContinueNumbering() throws {
        for command in ["A", "o"] {
            let source = "1.\tFirst\n2.\tLast"
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownSourceType)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            let client: NSTextInputClient = surface.editorView
            let implicit = NSRange(location: NSNotFound, length: 0)
            client.insertText(command, replacementRange: implicit)
            if command == "A" {
                surface.performInput { _ = try surface.session?.sendKey(kind: UInt32(VIEM_KEY_ENTER)) }
            }
            XCTAssertEqual(surface.statusBarState.message, "")
            client.insertText("Added", replacementRange: implicit)
            surface.performInput { _ = try surface.session?.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType),
                           Data("1.\tFirst\n2.\tAdded\n3.\tLast".utf8))
            client.insertText("u", replacementRange: implicit)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data(source.utf8))
        }
    }

    @MainActor
    func testMarkdownSourceBareEmptyNumberedItemEnterBecomesProse() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("1.".utf8), typeName: EVDocument.markdownSourceType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let client: NSTextInputClient = surface.editorView
        let implicit = NSRange(location: NSNotFound, length: 0)
        client.insertText("A", replacementRange: implicit)
        surface.performInput { _ = try surface.session?.sendKey(kind: UInt32(VIEM_KEY_ENTER)) }
        XCTAssertEqual(surface.statusBarState.message, "")
        client.insertText("Plain", replacementRange: implicit)
        surface.performInput { _ = try surface.session?.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data("Plain".utf8))
        client.insertText("u", replacementRange: implicit)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data("1.".utf8))
    }

    @MainActor
    func testMarkdownFormattingMenusUseEffectiveStateAndPreserveSourceOnUndo() throws {
        for (type, source) in [(EVDocument.markdownType, "**Words**")] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data(source.utf8), typeName: type)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            try backend.acknowledgeNativeSave(backend.nativeSaveSnapshot(typeName: type))
            surface.perform(menuCommand: .selectAll, sender: nil)
            XCTAssertEqual(surface.presentation(for: .bold).state, .on, type)
            surface.perform(menuCommand: .bold, sender: nil)
            XCTAssertEqual(surface.presentation(for: .bold).state, .off, type)
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            XCTAssertEqual(try backend.documentState().flags & UInt32(VIEM_DOCUMENT_STATE_IS_DIRTY), 0)
            for command in [EVMenuCommand.strikethrough] {
                surface.perform(menuCommand: .selectAll, sender: nil)
                XCTAssertTrue(surface.presentation(for: command).isEnabled)
                surface.perform(menuCommand: command, sender: nil)
                XCTAssertEqual(surface.presentation(for: command).state, .on, type)
                surface.perform(menuCommand: command, sender: nil)
                XCTAssertEqual(surface.presentation(for: command).state, .off, type)
                surface.perform(menuCommand: .undo, sender: nil)
                surface.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            }
        }
    }

    @MainActor
    func testNativeMultilingualInputAcrossFormatsSurvivesResizeAndMetricsChanges() throws {
        for (type, source) in [
            (EVDocument.plainTextType, "first line\nsecond line"),
            (EVDocument.markdownType, "**first** line\nsecond line"),
            (EVDocument.markdownSourceType, "**first** line\nsecond line"),
        ] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data(source.utf8), typeName: type)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            let session = try XCTUnwrap(surface.session)
            let client: NSTextInputClient = surface.editorView
            let implicit = NSRange(location: NSNotFound, length: 0)
            let original = surface.formattedText
            for iteration in 0..<20 {
                client.insertText("ggi", replacementRange: implicit)
                client.insertText("مَرْحَبًا 👩🏽‍💻 café ", replacementRange: implicit)
                _ = session.provider.invalidateMetrics()
                surface.refreshPresentation()
                _ = try session.resize(width: CGFloat(150 + iteration * 7), height: 240)
                surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
                XCTAssertEqual(surface.statusBarState.message, "", "\(type), iteration \(iteration)")
                XCTAssertTrue(surface.formattedText.contains("مَرْحَبًا 👩🏽‍💻 café "), type)
                surface.perform(menuCommand: .undo, sender: nil)
                XCTAssertEqual(surface.formattedText, original, type)
                XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
            }
        }
    }

    @MainActor
    func testFontInvalidationRefreshesHitTestingWithoutReplayingText() throws {
        let backend = EVCoreDocumentBackend()
        let original = "WWW iii ffi café e\u{301} 👩🏽‍💻\nsecond line\n"
        try backend.read(source: Data(original.utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)

        for iteration in 0..<40 {
            _ = session.provider.invalidateMetrics()
            surface.refreshPresentation()
            XCTAssertEqual(surface.statusBarState.message, "")
            var layout = try XCTUnwrap(surface.layoutSnapshot)
            XCTAssertEqual(layout.info.identity.metrics_generation, session.provider.metricsGeneration)
            let point = try session.hitTest(CGPoint(x: 0, y: CGFloat(layout.rows[0].y)), in: layout.info)
            _ = try session.placeCursor(point, extendSelection: false)
            _ = try session.sendText("i")
            _ = try session.sendText("é")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.resize(width: CGFloat(120 + iteration * 3), height: 180)
            _ = try session.undo()
            surface.refreshPresentation()
            layout = try XCTUnwrap(surface.layoutSnapshot)
            XCTAssertEqual(layout.info.identity.document_revision, backend.currentDocumentState.document_revision)
            XCTAssertEqual(try backend.formattedText(), original)
            XCTAssertEqual(surface.statusBarState.message, "")
        }
    }

    @MainActor
    func testMetricRecoveryInLargeDocumentKeepsVisibleExportBounded() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(String(repeating: "Local text and emoji 👩🏽‍💻\n", count: 20_000).utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        _ = try session.resize(width: 300, height: 160)
        _ = session.provider.invalidateMetrics()
        surface.refreshPresentation()
        let layout = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertLessThan(layout.rows.count, 100)
        XCTAssertEqual(layout.info.identity.metrics_generation, session.provider.metricsGeneration)
        XCTAssertEqual(surface.statusBarState.message, "")
    }

    @MainActor
    func testBlankCanvasHitTestsDocumentEdgesWithoutGuessingUnmaterializedContent() throws {
        for text in ["", "short document"] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data(text.utf8), typeName: EVDocument.plainTextType)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            let session = try XCTUnwrap(surface.session)
            let layout = try session.layoutExport()
            let below = try session.hitTest(CGPoint(x: 10_000, y: 10_000), in: layout.info)
            XCTAssertEqual(below.text_offset, UInt64(text.utf8.count))
            let above = try session.hitTest(CGPoint(x: -100, y: -100), in: layout.info)
            XCTAssertEqual(above.text_offset, 0)
        }
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(String(repeating: "more text\n", count: 20_000).utf8), typeName: EVDocument.plainTextType)
        let session = try EVCoreViewSession(document: backend, width: 300, height: 100)
        let layout = try session.layoutExport()
        XCTAssertThrowsError(try session.hitTest(CGPoint(x: 0, y: 1_000_000), in: layout.info))
    }

    @MainActor
    func testFormatPickerAndEncodingMenuPreserveSourceAndHistory() throws {
        let source = "## Heading\n__bold__ café\n"
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownSourceType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.perform(statusOption: .format(.markdownSource))
        XCTAssertEqual(surface.statusBarState.format, "Markdown Source")
        XCTAssertEqual(surface.formattedText, source)
        surface.perform(statusOption: .format(.markdown))
        XCTAssertEqual(surface.statusBarState.format, "Markdown WYSIWYG")
        XCTAssertEqual(surface.formattedText, "Heading\nbold café")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        surface.perform(menuCommand: .encodingUTF16LE, sender: nil)
        XCTAssertEqual(surface.presentation(for: .encodingUTF16LE).state, .on)
        XCTAssertEqual(surface.formattedText, "Heading\nbold café")
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(surface.statusBarState.format, "Markdown Source")
        XCTAssertEqual(surface.formattedText, source)
        XCTAssertEqual(surface.statusBarState.message, "")
    }

    @MainActor
    func testFormatPickerKeepsInteriorUnicodeCursorAtTheSameRepeatedOccurrence() throws {
        let source = (0..<180).map { "## Heading \($0)\n\nText **café العربية** α\($0) end." }.joined(separator: "\n\n")
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 420, height: 180)
        surface.viewDidLayout()
        let session = try XCTUnwrap(surface.session)
        surface.perform(statusOption: .format(.markdownSource))
        surface.performInput {
            _ = try session.sendText("/العربية")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
            _ = try session.sendText("75nli")
        }
        func occurrenceOffsets(_ text: String) -> [Int] {
            var start = text.startIndex
            var offsets: [Int] = []
            while let range = text.range(of: "العربية", range: start..<text.endIndex) {
                offsets.append(text[..<range.lowerBound].utf8.count)
                start = range.upperBound
            }
            return offsets
        }
        func firstVisibleOccurrence() throws -> Int {
            let text = try backend.formattedText()
            let snapshot = try XCTUnwrap(surface.layoutSnapshot)
            let row = try XCTUnwrap(snapshot.rows.first { $0.y + $0.line_advance > surface.viewportState.top })
            return occurrenceOffsets(text).filter { $0 < Int(row.text_start) }.count
        }
        let topOccurrence = try firstVisibleOccurrence()
        XCTAssertGreaterThan(topOccurrence, 70)
        for format: EVSourceFormat in [.markdown, .markdownSource, .markdown] {
            surface.perform(statusOption: .format(format))
            let text = try backend.formattedText()
            let offsets = occurrenceOffsets(text)
            XCTAssertEqual(offsets.count, 180)
            XCTAssertEqual(surface.statusBarState.message, "", "\(format)")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            let expectedCursor = try XCTUnwrap(offsets.dropFirst(75).first) + "ا".utf8.count
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64(expectedCursor), "\(format)")
            XCTAssertLessThanOrEqual(abs(try firstVisibleOccurrence() - topOccurrence), 2, "\(format)")
            XCTAssertGreaterThan(surface.viewportState.top, 0)
        }
    }

    @MainActor
    func testNativePrintableKeysMoveAndEditVisualBlockUsingLayout() throws {
        let original = "WWW iii ffi\nsecond row\nthird row"
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(original.utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        surface.performInput {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 113)
        }
        let client: NSTextInputClient = surface.editorView
        let implicit = NSRange(location: NSNotFound, length: 0)
        client.insertText("lljj", replacementRange: implicit)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_BLOCK))
        let selection = try XCTUnwrap(surface.visualSelection)
        XCTAssertEqual(selection.segments.count, 3)
        XCTAssertGreaterThan(surface.viewPresentation.cursor_utf8_offset, 20)
        client.insertText("d", replacementRange: implicit)
        XCTAssertNotEqual(surface.formattedText, original)
        client.insertText("u", replacementRange: implicit)
        XCTAssertEqual(surface.formattedText, original)
        XCTAssertEqual(surface.statusBarState.message, "")
    }

    @MainActor
    func testListMenuUpdatesMarkersAndIsUndoable() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("first\nsecond\n".utf8), typeName: EVDocument.markdownSourceType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.perform(menuCommand: .selectAll, sender: nil)
        surface.perform(menuCommand: .bulletedList, sender: nil)
        XCTAssertTrue(surface.formattedText.hasPrefix("- first\n- second"))
        XCTAssertEqual(surface.statusBarState.message, "")
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(surface.formattedText, "first\nsecond\n")
    }
}
