import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVCodeStyleMenuTests: XCTestCase {
    private func surface() throws -> EVEditorSurfaceController {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-style-menu-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.read(source: Data("fn main() { let count = 42; }\n".utf8),
                         typeName: EVDocument.codeType, filename: "main.rs", allowAutomaticCode: false)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        return surface
    }

    func testCodeMenuChecksOneUniformSyntaxSelectionAndClearsCharacterChecksForMixedText() async throws {
        let surface = try surface()
        try await waitForRustHighlighting(surface)
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 2))
        var catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        XCTAssertEqual(catalogue.entries.filter { $0.role == .character && $0.presentation.state == .on }
            .map(\.displayName), ["Keyword.function"])
        XCTAssertEqual(catalogue.entries.first { $0.role == .paragraph && $0.stableID == "Paragraph" }?
            .presentation.state, .on)

        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 3))
        catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        XCTAssertTrue(try XCTUnwrap(surface.session).selectedNamedStyles().characterMixed)
        XCTAssertTrue(catalogue.entries.filter { $0.role == .character }
            .allSatisfy { $0.presentation.state == .off }, "A mixed syntax/default selection has no unique character style")
        XCTAssertEqual(catalogue.entries.first { $0.role == .paragraph && $0.stableID == "Paragraph" }?
            .presentation.state, .on)
        XCTAssertFalse(surface.canUndo)
    }

    func testChoosingAnUnmaterializedSyntaxNameGeneratesItsAncestry() throws {
        // Implicit definitions are not exported, so restoring the export
        // removes them from the process-wide authority.
        let saved = try EVCodeStyleSession.exportGlobalJSON()
        defer {
            _ = saved.withUnsafeBytes { raw in
                viem_code_replace_style_json(raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count))
            }
        }
        try EVCoreStyleBridge.materializeCodeStyle(name: "Keyword.directive.define")
        let snapshot = try EVCoreStyleBridge.copyStyleSheet(core: nil)
        let leaf = try XCTUnwrap(snapshot.definitions.first { $0.name == "Keyword.directive.define" })
        let middle = try XCTUnwrap(snapshot.definitions.first { $0.name == "Keyword.directive" })
        let root = try XCTUnwrap(snapshot.definitions.first { $0.name == "Keyword" })
        XCTAssertTrue(leaf.flags.contains(.implicit))
        XCTAssertTrue(middle.flags.contains(.implicit))
        XCTAssertEqual(leaf.parentID, middle.key.id)
        XCTAssertEqual(middle.parentID, root.key.id)
        XCTAssertEqual(leaf.name, "Keyword.directive.define")
    }

    private func waitForRustHighlighting(_ surface: EVEditorSurfaceController) async throws {
        for _ in 0..<200 {
            surface.backend.pollSyntax()
            surface.refreshPresentation()
            if surface.layoutPaint?.runs.contains(where: {
                $0.text_start == 0 && $0.text_end >= 2
                    && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0
            }) == true { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Rust fn did not receive syntax highlighting: \(surface.statusBarState.message)")
    }
}
