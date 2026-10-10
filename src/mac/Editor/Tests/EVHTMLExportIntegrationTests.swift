import AppKit
import CViemCore
import UniformTypeIdentifiers
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVHTMLExportIntegrationTests: XCTestCase {
    private func surface(_ source: String, type: String, filename: String = "") throws -> EVEditorSurfaceController {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-html-export-core-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        try EVStyleTestFixtures.configure(configuration, declarations: [
            (.baseParagraph, .characterWeight, .unsigned(400)),
        ])
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(source.utf8), typeName: type, filename: filename, allowAutomaticCode: false)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        return surface
    }

    func testMarkdownSourceExportsFormattedContentAndPreservesEditingState() async throws {
        let source = "# Heading\r\n\r\n**bold** & <word>\r\n"
        let surface = try surface(source, type: EVDocument.markdownSourceType)
        let backend = surface.backend
        let session = try XCTUnwrap(surface.session)
        _ = try session.sendText("A tail")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        let before = try backend.recoverySnapshot()
        let canUndo = surface.canUndo
        let canRedo = surface.canRedo
        let cursor = surface.viewPresentation.cursor_utf8_offset

        let data = try await surface.htmlExportData()
        let html = try XCTUnwrap(String(data: data, encoding: .utf8))

        XCTAssertTrue(html.contains("<!DOCTYPE html>"))
        XCTAssertTrue(html.contains("<h1"))
        XCTAssertTrue(html.contains("font-weight: 700"))
        XCTAssertFalse(html.contains("**bold**"))
        XCTAssertFalse(html.contains("# Heading"))
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertEqual(surface.canUndo, canUndo)
        XCTAssertEqual(surface.canRedo, canRedo)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, cursor)
        XCTAssertEqual(backend.sourceFormat, .markdownSource)
    }

    func testCodeExportsEscapedLiteralTextWithSyntaxStyles() async throws {
        let source = "fn main() {\n    let text = \"<script>&\";\n}\n"
        let surface = try surface(source, type: EVDocument.codeType, filename: "main.rs")
        let before = try surface.backend.recoverySnapshot()
        let data = try await surface.htmlExportData()
        let html = try XCTUnwrap(String(data: data, encoding: .utf8))
        XCTAssertTrue(html.contains("<pre"))
        XCTAssertTrue(html.contains("&lt;script&gt;&amp;"))
        XCTAssertFalse(html.contains("<script>"))
        XCTAssertTrue(html.contains("color:"))
        XCTAssertTrue(html.contains("font-weight:"))
        XCTAssertTrue(html.contains(">fn</span>"), "The Rust keyword must have its own syntax-styled span")
        XCTAssertGreaterThan(html.components(separatedBy: "<span ").count, 3)
        XCTAssertEqual(try surface.backend.recoverySnapshot(), before)
        XCTAssertFalse(surface.canUndo)
    }

    func testNativeExportCommandUsesTheActivePaneAndHTMLPanel() throws {
        let surface = try surface("# Export me", type: EVDocument.markdownSourceType)
        let document = EVDocument(editorBackend: surface.backend)
        let controller = EVDocumentWindowController(document: document, editorSurface: surface)
        defer { controller.window?.orderOut(nil); document.close() }
        var shown = 0
        document.htmlExportPanelHandler = { panel, complete in
            shown += 1
            XCTAssertEqual(panel.allowedContentTypes, [.html])
            XCTAssertEqual(panel.prompt, "Export")
            complete(nil)
        }
        let item = NSMenuItem(title: "Export…", action: #selector(EVEditorCommandRouting.performEditorMenuCommand(_:)), keyEquivalent: "")
        item.tag = EVMenuCommand.exportHTML.rawValue
        XCTAssertTrue(controller.documentContentController.validateMenuItem(item))
        controller.documentContentController.performEditorMenuCommand(item)
        XCTAssertEqual(shown, 1)
        XCTAssertFalse(surface.backend.persistenceState.isDirty)
    }
}
