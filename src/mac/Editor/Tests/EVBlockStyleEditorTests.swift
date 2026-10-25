import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVBlockStyleEditorTests: XCTestCase {
    private let quote = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Block quote"))

    func testBlockPropertiesUseInheritedControlsAndOneUndoGesture() throws {
        let (backend, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let session = try XCTUnwrap(surface.session)
        let before = try backend.styleSheetSnapshot().definition(for: quote)
        let originalSource = try backend.serializedSource(typeName: EVDocument.markdownType)
        editor.selectTab(.block)
        editor.beginContinuousStyleEditForTesting()
        XCTAssertTrue(editor.setPropertyForTesting(.blockMarginLeft, value: .float(-4)))
        XCTAssertTrue(editor.setPropertyForTesting(.blockPaddingLeft, value: .float(11)))
        XCTAssertTrue(editor.setPropertyForTesting(.blockBorderRightWidth, value: .float(3)))
        let purple = EVStyleColor(red: 0.7, green: 0.5, blue: 0.9, alpha: 0.6)
        XCTAssertTrue(editor.setPropertyForTesting(.blockBackground, value: .color(purple)))
        editor.endContinuousStyleEditForTesting()
        let changed = try backend.styleSheetSnapshot().definition(for: quote)
        XCTAssertEqual(changed?.properties[.blockPaddingLeft]?.declared, .float(11))
        XCTAssertEqual(changed?.properties[.blockBackground]?.declared, .color(purple))
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), originalSource)
        _ = try session.undo()
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote), before)
        _ = try session.redo()
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: quote), changed)
        let toggle = try control(NSButton.self, "Override padding left", editor)
        XCTAssertEqual(toggle.state, .on)
        toggle.performClick(nil)
        XCTAssertNil(try backend.styleSheetSnapshot().definition(for: quote)?.properties[.blockPaddingLeft]?.declared)
    }

    func testNestedContainerPreviewUsesCoreBoxesAndRespondsToPaddingAndBorders() throws {
        let (_, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let background = EVStyleColor(red: 0.9, green: 0.8, blue: 1, alpha: 1)
        XCTAssertTrue(editor.setPropertyForTesting(.blockBackground, value: .color(background)))
        XCTAssertTrue(editor.setPropertyForTesting(.blockPaddingLeft, value: .float(9)))
        XCTAssertTrue(editor.setPropertyForTesting(.blockPaddingTop, value: .float(4)))
        XCTAssertTrue(editor.setPropertyForTesting(.blockBorderLeftWidth, value: .float(2)))
        let initial = editor.previewInspectionForTesting(layoutSize: CGSize(width: 560, height: 400))
        XCTAssertNil(initial.blockPreviewError)
        XCTAssertGreaterThanOrEqual(initial.currentStyleLines.count, 3)
        let backgrounds = initial.boxes.filter(\.isBackground)
        XCTAssertGreaterThanOrEqual(backgrounds.count, 2, "Both nested quotation containers have a background")
        XCTAssertTrue(backgrounds.allSatisfy { $0.color == background })
        XCTAssertTrue(initial.boxes.contains { !$0.isBackground && abs($0.rect.width - 2) < 0.01 })
        let before = try XCTUnwrap(initial.currentStyleLines.last).origin.x
        XCTAssertTrue(editor.setPropertyForTesting(.blockPaddingLeft, value: .float(19)))
        let padded = editor.previewInspectionForTesting(layoutSize: CGSize(width: 560, height: 400))
        XCTAssertNil(padded.blockPreviewError)
        XCTAssertEqual(try XCTUnwrap(padded.currentStyleLines.last).origin.x - before, 20, accuracy: 0.1,
            "The nested paragraph receives ten additional points from each quote level")
    }

    private func makeEditor() throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVStyleEditorViewController) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-block-editor-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.readUntitled(source: Data("> First paragraph\n>\n> Second paragraph\n>\n> > Nested quote".utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(document: surface, styleKey: quote)
        return (backend, surface, editor)
    }

    private func control<T: NSView>(_ type: T.Type, _ label: String, _ editor: EVStyleEditorViewController) throws -> T {
        func descendants(_ view: NSView) -> [NSView] { view.subviews.flatMap { [$0] + descendants($0) } }
        return try XCTUnwrap(descendants(editor.view).first { $0 is T && $0.accessibilityLabel() == label } as? T,
            "Missing \(label)")
    }
}
