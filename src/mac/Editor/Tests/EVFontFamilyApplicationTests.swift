import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemCoreTextProvider
@testable import ViemEditor

@MainActor
final class EVFontFamilyApplicationTests: XCTestCase {
    func testNativeFamilyChoiceUpdatesCodePreviewGlyphsHistoryAndPersistedSettings() async throws {
        let configuration = configuration()
        let source = Data(("// Wide WWW and narrow iii change width\n"
            + (0..<2_048).map { "fn item_\($0)() {}\n" }.joined()).utf8)
        let fixture = try makeSurface(source: source, type: EVDocument.plainTextType,
                                      configuration: configuration, filename: "family.rs")
        let original = try fixture.backend.recoverySnapshot()
        let styleSession = try EVCodeStyleSession(configuration: configuration)
        let comment = try XCTUnwrap(styleSession.snapshot().definitions.first { $0.name == "Comment" })
        let editor = EVStyleEditorViewController()
        editor.retarget(settingsSession: styleSession)
        editor.selectStyle(comment.key)

        try chooseFamily("Helvetica", in: editor)
        try await waitForFamily("Helvetica", fixture: fixture)
        let firstWidth = try firstRowWidth(fixture)
        let firstPreviewWidth = previewWidth(editor)
        let firstResource = try firstCluster(fixture).render_run.identifier

        try chooseFamily("Georgia", in: editor)
        try await waitForFamily("Georgia", fixture: fixture)
        XCTAssertEqual(editor.inspection.preview.resolvedFontFamily, "Georgia")
        XCTAssertGreaterThan(abs(previewWidth(editor) - firstPreviewWidth), 0.5,
                             "Committed font changes must rebuild the attributed preview text")
        XCTAssertGreaterThan(abs(try firstRowWidth(fixture) - firstWidth), 0.5)
        XCTAssertNotEqual(try firstCluster(fixture).render_run.identifier, firstResource)
        try assertLocalCurrentLayout(fixture)
        let committed = try XCTUnwrap(styleSession.snapshot().definition(for: comment.key)?
            .properties[.characterFontFamilies]?.declared)
        let persisted = try XCTUnwrap(configuration.codeStyleSheet())

        styleSession.undoManager.undo()
        try await waitForFamily("Helvetica", fixture: fixture)
        XCTAssertEqual(editor.inspection.preview.resolvedFontFamily, "Helvetica")
        XCTAssertEqual(try firstRowWidth(fixture), firstWidth, accuracy: 0.01)
        XCTAssertEqual(previewWidth(editor), firstPreviewWidth, accuracy: 0.01)
        styleSession.undoManager.redo()
        try await waitForFamily("Georgia", fixture: fixture)
        XCTAssertEqual(try fixture.backend.recoverySnapshot(), original)
        XCTAssertEqual(try fixture.backend.serializedSource(typeName: EVDocument.plainTextType), source)
        XCTAssertFalse(fixture.surface.canUndo, "Global font changes belong only to settings history")

        // A different settings authority forces the normal disk-load path;
        // observing the existing global snapshot alone would not prove reload.
        let reloadedConfiguration = self.configuration()
        try reloadedConfiguration.saveCodeStyleSheet(persisted)
        let reopened = try makeSurface(source: source, type: EVDocument.plainTextType,
                                       configuration: reloadedConfiguration, filename: "family.rs")
        let reloadedSession = try EVCodeStyleSession(configuration: reloadedConfiguration)
        XCTAssertEqual(try reloadedSession.snapshot().definition(for: comment.key)?
            .properties[.characterFontFamilies]?.declared, committed)
        try await waitForFamily("Georgia", fixture: reopened)
        try assertLocalCurrentLayout(reopened)
    }

    func testNativeFamilyChoiceUpdatesRTFPreviewGlyphsUndoAndSourcePersistence() async throws {
        let configuration = configuration()
        let source = Data(("{\\rtf1{\\stylesheet{\\s0\\fs28 Paragraph;}}\\s0 " + (0..<512).map {
            "Wide WWW and narrow iii change width \($0)"
        }.joined(separator: "\\par ") + "}").utf8)
        let fixture = try makeSurface(source: source, type: EVDocument.rtfType,
                                      configuration: configuration)
        let editor = EVStyleEditorViewController()
        editor.retarget(document: fixture.surface, styleKey: .baseParagraph)

        try chooseFamily("Helvetica", in: editor)
        try await waitForFamily("Helvetica", fixture: fixture)
        let firstWidth = try firstRowWidth(fixture)
        let firstPreviewWidth = previewWidth(editor)
        let firstResource = try firstCluster(fixture).render_run.identifier

        try chooseFamily("Georgia", in: editor)
        try await waitForFamily("Georgia", fixture: fixture)
        XCTAssertEqual(editor.inspection.preview.resolvedFontFamily, "Georgia")
        // The installed font identity is checked above; geometry only needs
        // to demonstrate that the attributed preview was actually reshaped.
        XCTAssertGreaterThan(abs(previewWidth(editor) - firstPreviewWidth), 0.01)
        XCTAssertGreaterThan(abs(try firstRowWidth(fixture) - firstWidth), 0.5)
        XCTAssertNotEqual(try firstCluster(fixture).render_run.identifier, firstResource)
        try assertLocalCurrentLayout(fixture)
        let saved = try fixture.backend.serializedSource(typeName: EVDocument.rtfType)
        let committed = try fixture.backend.styleSheetSnapshot().definition(for: .baseParagraph)?
            .properties[.characterFontFamilies]?.declared

        fixture.surface.perform(menuCommand: .undo, sender: nil)
        try await waitForFamily("Helvetica", fixture: fixture)
        XCTAssertEqual(editor.inspection.preview.resolvedFontFamily, "Helvetica")
        XCTAssertEqual(try firstRowWidth(fixture), firstWidth, accuracy: 0.01)
        XCTAssertEqual(previewWidth(editor), firstPreviewWidth, accuracy: 0.01)
        fixture.surface.perform(menuCommand: .redo, sender: nil)
        try await waitForFamily("Georgia", fixture: fixture)
        XCTAssertEqual(try fixture.backend.serializedSource(typeName: EVDocument.rtfType), saved)

        let reopened = try makeSurface(source: saved, type: EVDocument.rtfType,
                                       configuration: configuration)
        XCTAssertEqual(try reopened.backend.styleSheetSnapshot().definition(for: .baseParagraph)?
            .properties[.characterFontFamilies]?.declared, committed)
        try await waitForFamily("Georgia", fixture: reopened)
        try assertLocalCurrentLayout(reopened)
    }

    private struct Fixture {
        let backend: EVCoreDocumentBackend
        let surface: EVEditorSurfaceController
    }

    private func configuration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-font-application-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory, legacyDefaults: nil)
    }

    private func makeSurface(source: Data, type: String, configuration: EVConfigurationStore,
                             filename: String? = nil) throws -> Fixture {
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: source, typeName: type, filename: filename, allowAutomaticCode: filename != nil)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 700, height: 180)
        surface.viewDidLayout()
        _ = try XCTUnwrap(surface.session).setWrap(false)
        surface.refreshPresentation()
        return Fixture(backend: backend, surface: surface)
    }

    private func chooseFamily(_ name: String, in editor: EVStyleEditorViewController) throws {
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        let combo = try XCTUnwrap(descendants(editor.view).first {
            $0 is NSComboBox && $0.accessibilityLabel() == "Font family"
        } as? NSComboBox)
        combo.stringValue = name
        XCTAssertTrue(combo.sendAction(try XCTUnwrap(combo.action), to: combo.target))
        XCTAssertEqual(editor.inspection.diagnostic, "")
        XCTAssertEqual(combo.stringValue, name)
        XCTAssertEqual(editor.inspection.preview.resolvedFontFamily, name)
    }

    private func previewWidth(_ editor: EVStyleEditorViewController) -> CGFloat {
        editor.previewInspectionForTesting(layoutSize: CGSize(width: 2_000, height: 300))
            .currentStyleLines.reduce(0) { $0 + $1.typographicWidth }
    }

    private func firstCluster(_ fixture: Fixture) throws -> ViemPositionedClusterV1 {
        try XCTUnwrap(fixture.surface.layoutSnapshot?.clusters.first { $0.text_start == 0 })
    }

    private func firstRowWidth(_ fixture: Fixture) throws -> Float {
        try XCTUnwrap(fixture.surface.layoutSnapshot?.rows.first { $0.text_start == 0 }).width
    }

    private func renderedFamily(_ fixture: Fixture) throws -> String {
        let cluster = try firstCluster(fixture)
        let registry = try XCTUnwrap(fixture.surface.session).provider.renderRegistry
        return try XCTUnwrap(registry.resolvedFontFamily(identifier: cluster.render_run.identifier,
                                                        metricsGeneration: cluster.render_run.metrics_generation))
    }

    private func waitForFamily(_ family: String, fixture: Fixture) async throws {
        for _ in 0..<300 {
            fixture.backend.pollSyntax()
            fixture.surface.refreshPresentation()
            if try renderedFamily(fixture) == family { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Rendered font never became \(family); actual \((try? renderedFamily(fixture)) ?? "missing")")
    }

    private func assertLocalCurrentLayout(_ fixture: Fixture) throws {
        let layout = try XCTUnwrap(fixture.surface.layoutSnapshot)
        let paint = try XCTUnwrap(fixture.surface.layoutPaint)
        XCTAssertTrue(layout.info.identity.isSameLayout(as: paint.info.identity))
        XCTAssertEqual(layout.info.identity.metrics_generation,
                       try XCTUnwrap(fixture.surface.session).provider.metricsGeneration)
        XCTAssertLessThan(layout.rows.count, 100)
        XCTAssertLessThan(layout.clusters.count, 6_000)
        XCTAssertLessThan(layout.info.coverage_y_end, layout.info.total_height)
        XCTAssertNil(fixture.surface.commandOutput)
    }
}
