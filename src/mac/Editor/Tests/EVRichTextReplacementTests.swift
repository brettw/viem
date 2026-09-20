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
        try backend.read(source: Data(source.utf8), typeName: EVDocument.htmlType)
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
        let source = "<p data-keep='yes'>baseparagraph <b>bold</b> baseparagraph</p><!--keep-->"
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
                let replacedSource = try backend.serializedSource(typeName: EVDocument.htmlType)
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
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8),
                               "\(selected), IME=\(composition)")
            }
        }
    }

    func testVisualLineDeleteOfFinalWrappedHTMLRowPreservesEarlierTextAndUndo() throws {
        for body in ["one two three four five six seven eight nine ten eleven twelve",
                     "one two three <b>four five six seven</b> eight nine ten eleven twelve",
                     "one two three four five six seven eight nine ten eleven twelve<br>"] {
            let source = "<p data-keep='yes'>\(body)</p><!--keep-->"
            let (backend, surface, session, window) = try fixture(source, width: 240)
            defer { window.close() }
            let original = try backend.formattedText()
            let layout = try session.layoutExport()
            let row = try XCTUnwrap(layout.rows.last { $0.text_end > $0.text_start })
            XCTAssertGreaterThan(row.text_start, 0)
            let geometry = try session.caretGeometry(offset: row.text_start,
                affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM), in: layout.info)
            _ = try session.placeCursor(geometry.point, extendSelection: false)
            _ = try session.sendText("V")
            surface.refreshPresentation()
            let selection = try XCTUnwrap(surface.selectedUTF8Range())
            XCTAssertEqual(selection.lowerBound, Int(row.text_start))
            var expected = Array(original.utf8)
            expected.removeSubrange(selection)
            _ = try session.sendText("d")
            surface.refreshPresentation()
            // HTML protects a retained space at the new paragraph edge as NBSP.
            XCTAssertEqual(try backend.formattedText().replacingOccurrences(of: "\u{a0}", with: " "),
                           String(decoding: expected, as: UTF8.self))
            XCTAssertNil(surface.commandOutput)
            _ = try session.undo()
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8))
        }
    }

    func testWholeDocumentReplacementRetainsParagraphStyleWithNativeAndIMEInput() throws {
        let source = "<h2 style='text-align:center'>Heading</h2>"
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
            let saved = try backend.serializedSource(typeName: EVDocument.htmlType)
            XCTAssertTrue(String(decoding: saved, as: UTF8.self).contains("<h2"))
            XCTAssertNil(surface.commandOutput)
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.undo()
            if composition { _ = try session.undo() }
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8))
        }
    }

    func testWhitespaceReplacementPreservesStyleAndUndoWithNativeAndIMEInput() throws {
        let source = "<p>A <b>B</b> C</p>\r\n<p>D</p>"
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
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8))
        }
    }

    func testReplacementLatencyAcrossLargeHTMLDocuments() throws {
        // Work-count gates live in the portable suite. These timings include
        // AppKit selection, native/IME input, and presentation refresh, and are
        // supplementary diagnostics rather than machine-dependent thresholds.
        let paragraph = "<p>alpha <b>bold</b> omega</p>\n"
        let visible = "alpha bold omega"
        for count in [1_000, 10_000] {
            for composition in [false, true] {
                let (backend, surface, session, window) = try fixture(String(repeating: paragraph, count: count))
                defer { window.close() }
                for line in [0, count / 2, count - 1] {
                    let range = NSRange(location: line * (visible.utf8.count + 1) + 6, length: 4)
                    // Accessibility selection requires visible layout coverage.
                    // Navigation is setup, outside the measured replacement.
                    _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                    _ = try session.sendText("\(line + 1)G")
                    surface.refreshPresentation()
                    let start = ProcessInfo.processInfo.systemUptime
                    surface.editorView.setAccessibilitySelectedTextRange(range)
                    XCTAssertEqual(surface.selectedUTF8Range(), range.location..<NSMaxRange(range))
                    if composition {
                        surface.editorView.setMarkedText("X", selectedRange: NSRange(location: 1, length: 0),
                                                         replacementRange: noRange)
                    }
                    surface.editorView.insertText("X", replacementRange: noRange)
                    surface.editorView.insertText("Y", replacementRange: noRange)
                    let milliseconds = (ProcessInfo.processInfo.systemUptime - start) * 1_000
                    print("HTML replacement native paragraphs=\(count) line=\(line) ime=\(composition) milliseconds=\(milliseconds)")
                    let result = try backend.formattedText() as NSString
                    XCTAssertEqual(result.substring(with: NSRange(location: range.location, length: 2)), "XY")
                    XCTAssertEqual(try session.selectedFormatting()[.characterBold], .boolean(true))
                    XCTAssertNil(surface.commandOutput)
                    _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                    _ = try session.undo()
                    if composition { _ = try session.undo() }
                    surface.refreshPresentation()
                }
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType),
                               Data(String(repeating: paragraph, count: count).utf8))
            }
        }
    }
}
