import AppKit
import CoreText
import ViemAppShell
import ViemCoreTextProvider
import XCTest
@testable import ViemEditor

@MainActor
final class EVVariableFontIntegrationTests: XCTestCase {

    func testSavedPortableFlightlineRequestExposesVariableControlsWithoutAnEdit() throws {
        try registerFlightline()
        let (configuration, backend, surface, _, editor) = try fixture(family: "Flightline Code", face: "ExtraLight Italic", weight: 300)
        let original = try backend.recoverySnapshot()
        let settingsURL = configuration.themesDirectory.appendingPathComponent("Variable.json")
        let settings = try Data(contentsOf: settingsURL)
        let slider = try control(NSSlider.self, label: "Weight", in: editor.view)
        XCTAssertEqual(slider.doubleValue, 300)
        XCTAssertEqual(try control(NSComboBox.self, label: "Font family", in: editor.view).stringValue, "Flightline Code")
        assertFlightlineFile(try renderedFont(surface), italic: true)
        XCTAssertEqual(try backend.recoverySnapshot(), original)
        XCTAssertEqual(try Data(contentsOf: settingsURL), settings)
        XCTAssertFalse(surface.canUndo)
    }

    private func assertFlightlineFile(_ font: CTFont, italic: Bool, file: StaticString = #filePath, line: UInt = #line) {
        let url = CTFontDescriptorCopyAttribute(CTFontCopyFontDescriptor(font), kCTFontURLAttribute) as? URL
        XCTAssertEqual(url?.lastPathComponent, "FlightlineCode-\(italic ? "Italic" : "Regular")-VF.ttf", file: file, line: line)
        XCTAssertEqual(CTFontGetSymbolicTraits(font).contains(.traitItalic), italic, file: file, line: line)
    }

    private func fixture(family: String = "Recursive", face: String = "", weight: UInt32 = 400) throws -> (EVConfigurationStore, EVCoreDocumentBackend, EVEditorSurfaceController, EVThemeStyleSession, EVStyleEditorViewController) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-variable-font-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let config = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        try config.createTheme(named: "Variable")
        let styles = try EVThemeStyleSession(configuration: config, format: .plainText)
        try styles.edit(key: .baseParagraph, expected: styles.snapshot().identity,
            mutation: .setDeclaration(.characterFontFamilies, .stringList([family])))
        for mutation in [
            EVStyleMutation.setDeclaration(.characterFontFace, .string(face)),
            .setDeclaration(.characterFontAxes, .string("{}")),
            .setDeclaration(.characterWeight, .unsigned(weight)),
        ] {
            try styles.edit(key: .baseParagraph, expected: styles.snapshot().identity, mutation: mutation)
        }
        let backend = EVCoreDocumentBackend(configuration: config)
        try backend.read(source: Data("Variable aegklrWim".utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        _ = try XCTUnwrap(surface.session).resize(width: 700, height: 180)
        let editor = EVStyleEditorViewController()
        editor.retarget(settingsSession: styles)
        return (config, backend, surface, styles, editor)
    }

    private func registerFlightline() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        for name in ["Regular", "Italic"] {
            let file = root.appendingPathComponent("assets/fonts/flightline/FlightlineCode-\(name)-VF.ttf")
            let owned = CTFontManagerRegisterFontsForURL(file as CFURL, .process, nil)
            if owned { addTeardownBlock { CTFontManagerUnregisterFontsForURL(file as CFURL, .process, nil); EVFontCatalog.invalidate() } }
        }
        EVFontCatalog.invalidate()
        XCTAssertFalse(EVFontVariations.info(for: "Flightline Code").axes.isEmpty)
    }

    private func renderedFont(_ surface: EVEditorSurfaceController) throws -> CTFont {
        surface.refreshPresentation()
        let cluster = try XCTUnwrap(surface.layoutSnapshot?.clusters.first)
        return try XCTUnwrap(surface.session?.provider.renderRegistry.resolvedFont(
            identifier: cluster.render_run.identifier, metricsGeneration: cluster.render_run.metrics_generation))
    }

    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    private func control<T: NSView>(_ type: T.Type, label: String, in root: NSView) throws -> T {
        try XCTUnwrap(descendants(root).first { $0 is T && $0.accessibilityLabel() == label } as? T)
    }
}
