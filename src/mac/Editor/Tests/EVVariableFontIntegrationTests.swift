import AppKit
import CoreText
import ViemAppShell
import ViemCoreTextProvider
import XCTest
@testable import ViemEditor

@MainActor
final class EVVariableFontIntegrationTests: XCTestCase {

    func testRecursivePresetPreservesAutoAndCursiveSliderOffersAllThreeModes() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let file = root.appendingPathComponent("assets/fonts/recursive/Recursive_VF_1.085.ttf")
        if CTFontManagerRegisterFontsForURL(file as CFURL, .process, nil) {
            addTeardownBlock { CTFontManagerUnregisterFontsForURL(file as CFURL, .process, nil); EVFontCatalog.invalidate() }
        }
        EVFontCatalog.invalidate()
        let (_, backend, _, styles, editor) = try fixture()
        let original = try backend.recoverySnapshot()
        let menu = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        menu.selectItem(at: try XCTUnwrap(menu.itemTitles.lastIndex(of: "Sans Linear Light")))
        XCTAssertTrue(menu.sendAction(try XCTUnwrap(menu.action), to: menu.target))
        let slider = try control(NSSlider.self, label: "Cursive", in: editor.view)
        XCTAssertEqual(slider.doubleValue, 0.5)
        XCTAssertEqual(slider.numberOfTickMarks, 3)
        XCTAssertTrue(slider.allowsTickMarkValuesOnly)
        for (value, name) in [(0.0, "Off"), (1.0, "On"), (0.5, "Auto")] {
            slider.doubleValue = value
            XCTAssertTrue(slider.sendAction(try XCTUnwrap(slider.action), to: slider.target))
            XCTAssertEqual(slider.doubleValue, value)
            XCTAssertTrue(descendants(editor.view).compactMap { ($0 as? NSTextField)?.stringValue }
                .contains("Cursive \(name) (\(String(format: "%g", value)))"))
            let base = try XCTUnwrap(try styles.snapshot().definitions.first { $0.key == .baseParagraph })
            guard case let .string(json)? = base.properties[.characterFontAxes]?.effective else {
                return XCTFail("Cursive selection must be saved as an axis coordinate")
            }
            XCTAssertEqual(EVFontVariations.decode(json)["CRSV"], value)
        }
        XCTAssertEqual(try backend.recoverySnapshot(), original)
    }

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
        try backend.readUntitled(source: Data("Variable aegklrWim".utf8), typeName: EVDocument.plainTextType)
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
