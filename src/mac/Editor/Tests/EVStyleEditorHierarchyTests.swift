import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVStyleEditorHierarchyTests: XCTestCase {
    private let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))

    func testParentButtonsTraverseFixedBaseStylesWithoutChangingDocumentOrHistory() throws {
        let (backend, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let before = try backend.recoverySnapshot()
        let identity = try backend.styleSheetSnapshot().identity
        let parent = try button("Edit based on style", in: editor)

        XCTAssertTrue(parent.isEnabled)
        editor.selectTab(.paragraph)
        parent.performClick(nil)
        XCTAssertEqual(editor.inspection.selectedStyleKey, .baseParagraph)
        XCTAssertTrue(editor.inspection.parentChoices.isEmpty,
                      "A fixed parent can be inspected even though it cannot be changed")
        XCTAssertFalse(parent.isEnabled, "Base Paragraph is the style root")
        parent.performClick(nil)
        XCTAssertEqual(editor.inspection.selectedStyleKey, .baseParagraph)

        editor.selectStyle(EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Code")))
        XCTAssertEqual(editor.inspection.parentValue, "Default Paragraph")
        XCTAssertTrue(editor.inspection.parentChoices.contains(.defaultParagraph))
        XCTAssertFalse(parent.isEnabled, "The contextual paragraph default is not an editable definition")
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertEqual(try backend.styleSheetSnapshot().identity, identity)
        XCTAssertFalse(surface.canUndo)
    }

    private func makeEditor() throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVStyleEditorViewController) {
        let configuration = isolatedConfiguration()
        try EVStyleTestFixtures.configure(configuration, declarations: [
            (.baseParagraph, .characterFontFamilies, .stringList(["system-ui"])),
            (.baseParagraph, .characterFontAxes, .string("{}")),
            (heading, .characterSize, .float(24)),
        ])
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.readUntitled(source: Data("# Heading\n\nText".utf8), typeName: "public.markdown")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(document: surface, styleKey: heading)
        return (backend, surface, editor)
    }

    private func isolatedConfiguration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-style-hierarchy-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory, legacyDefaults: nil)
    }

    private func button(_ label: String, in editor: EVStyleEditorViewController) throws -> NSButton {
        try XCTUnwrap(descendants(of: editor.view).compactMap { $0 as? NSButton }
            .first { $0.accessibilityLabel() == label })
    }

    private func descendants(of view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants(of: $0) }
    }

    private func alignmentRect(of view: NSView, in root: NSView) -> NSRect {
        root.convert(view.alignmentRect(forFrame: view.frame), from: view.superview)
    }
}
