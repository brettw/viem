import AppKit
import EvimAppShell
import XCTest
@testable import EvimEditor

final class EVRtfInteroperabilityTests: XCTestCase {
    @MainActor
    func testDirectParagraphFormattingCoversTheBodyAndTerminatorInAppKit() throws {
        for source in [
            #"{\rtf1 One\par Tail{\*\unknown Keep}}"#,
            #"{\rtf1 {\b One}\par Tail{\*\unknown Keep}}"#,
            #"{\rtf1 One{\*\unknown Keep}}"#,
        ] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data(source.utf8), typeName: EVDocument.rtfType)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            surface.perform(menuCommand: .alignCenter, sender: nil)
            XCTAssertEqual(surface.statusBarState.message, "")
            let saved = try backend.serializedSource(typeName: EVDocument.rtfType)
            XCTAssertTrue(String(decoding: saved, as: UTF8.self).contains(#"{\*\unknown Keep}"#))
            let attributed = try NSAttributedString(data: saved, options: [.documentType: NSAttributedString.DocumentType.rtf], documentAttributes: nil)
            XCTAssertTrue(attributed.string.hasPrefix("One"))
            for offset in 0..<min(4, attributed.length) {
                let paragraph = try XCTUnwrap(attributed.attribute(.paragraphStyle, at: offset, effectiveRange: nil) as? NSParagraphStyle)
                XCTAssertEqual(paragraph.alignment, .center, "\(source), offset \(offset)")
            }
            if attributed.length > 4 {
                XCTAssertNotEqual((attributed.attribute(.paragraphStyle, at: 4, effectiveRange: nil) as? NSParagraphStyle)?.alignment, .center)
            }
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.rtfType), Data(source.utf8))
        }
    }

    @MainActor
    func testListIndentationCoversTheBodyAndTerminatorInAppKit() throws {
        let source = #"{\rtf1 {\b One}\par Tail{\*\unknown Keep}}"#
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.rtfType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.perform(menuCommand: .bulletedList, sender: nil)
        XCTAssertEqual(surface.statusBarState.message, "")
        let saved = try backend.serializedSource(typeName: EVDocument.rtfType)
        XCTAssertTrue(String(decoding: saved, as: UTF8.self).contains(#"\ls0"#))
        let attributed = try NSAttributedString(data: saved, options: [.documentType: NSAttributedString.DocumentType.rtf], documentAttributes: nil)
        XCTAssertEqual(attributed.string, "One\nTail")
        for offset in 0..<4 {
            let paragraph = try XCTUnwrap(attributed.attribute(.paragraphStyle, at: offset, effectiveRange: nil) as? NSParagraphStyle)
            XCTAssertEqual(paragraph.headIndent, 20, "offset \(offset)")
            XCTAssertEqual(paragraph.firstLineHeadIndent, 10, "offset \(offset)")
        }
        XCTAssertEqual((attributed.attribute(.paragraphStyle, at: 4, effectiveRange: nil) as? NSParagraphStyle)?.headIndent ?? 0, 0)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.rtfType), Data(source.utf8))
    }
}
