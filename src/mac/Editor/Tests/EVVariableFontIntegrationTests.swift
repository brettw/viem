import AppKit
import CoreText
import ViemAppShell
import ViemCoreTextProvider
import XCTest
@testable import ViemEditor

@MainActor
final class EVVariableFontIntegrationTests: XCTestCase {
    func testSavedStaticFlightlineRequestExposesVariableControlsWithoutAnEdit() throws {
        try registerFlightline()
        let (configuration, backend, surface, _, editor) = try fixture(family: "FlightlineCode-LightItalic", weight: 300)
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

    func testFlightlineVariablePresetsSlidersAndSeparateItalicDesign() throws {
        try registerFlightline()
        let (configuration, backend, surface, styles, editor) = try fixture(family: "Flightline Code")
        let original = try backend.recoverySnapshot()
        let catalog = EVFontCatalog.faces(for: "Flightline Code")
        XCTAssertEqual(catalog.count, 12)
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        for italic in [false, true] {
            let member = try XCTUnwrap(catalog.first { $0.italic == italic && $0.weight == 400 })
            face.selectItem(at: try XCTUnwrap(catalog.firstIndex(of: member)))
            XCTAssertTrue(face.sendAction(try XCTUnwrap(face.action), to: face.target))
            let info = EVFontVariations.info(for: member.postScriptName)
            XCTAssertEqual(info.axes.map(\.tag), ["wght"])
            XCTAssertEqual(info.instances.count, 7)
            let presets = Array(face.itemArray.dropLast().suffix(info.instances.count))
            XCTAssertEqual(presets.map(\.title), info.instances.map(\.name))
            let slider = try control(NSSlider.self, label: "Weight", in: editor.view)
            XCTAssertEqual(slider.minValue, 200)
            XCTAssertEqual(slider.maxValue, 700)
            for (instance, item) in zip(info.instances, presets) {
                face.select(item)
                XCTAssertTrue(face.sendAction(try XCTUnwrap(face.action), to: face.target))
                let font = try renderedFont(surface)
                assertAxes(font, values: instance.values)
                XCTAssertEqual(editor.inspection.preview.resolvedFontAxes, instance.values)
                assertFlightlineFile(font, italic: italic)
                XCTAssertEqual(slider.doubleValue, instance.values["wght"]!, accuracy: 0.001)
            }
            var paths: [CGPath] = []
            var identities: [UInt64] = []
            for weight in [200.0, 437.0, 700.0] {
                slider.doubleValue = weight
                XCTAssertTrue(slider.sendAction(try XCTUnwrap(slider.action), to: slider.target))
                let font = try renderedFont(surface)
                assertAxes(font, values: ["wght": weight])
                assertFlightlineFile(font, italic: italic)
                XCTAssertEqual(editor.inspection.preview.resolvedFontAxes["wght"], weight)
                if weight == 437 { XCTAssertEqual(face.titleOfSelectedItem, "Custom") }
                var glyph: CGGlyph = 0
                var character: UniChar = 77
                XCTAssertTrue(CTFontGetGlyphsForCharacters(font, &character, &glyph, 1))
                paths.append(try XCTUnwrap(CTFontCreatePathForGlyph(font, glyph, nil)))
                identities.append(try XCTUnwrap(surface.layoutSnapshot?.clusters.first).render_run.identifier)
            }
            XCTAssertFalse(paths[0] == paths[1])
            XCTAssertFalse(paths[1] == paths[2])
            XCTAssertEqual(Set(identities).count, 3)
        }
        let upright = try XCTUnwrap(catalog.first { !$0.italic && $0.weight == 400 })
        face.selectItem(at: try XCTUnwrap(catalog.firstIndex(of: upright)))
        XCTAssertTrue(face.sendAction(try XCTUnwrap(face.action), to: face.target))
        let slider = try control(NSSlider.self, label: "Weight", in: editor.view)
        slider.doubleValue = 437
        XCTAssertTrue(slider.sendAction(try XCTUnwrap(slider.action), to: slider.target))
        XCTAssertTrue(editor.setPropertyForTesting(.characterBold, value: .boolean(true)))
        assertAxes(try renderedFont(surface), values: ["wght": 700])
        XCTAssertEqual(slider.doubleValue, 437)
        XCTAssertTrue(editor.setPropertyForTesting(.characterSlant, value: .fontSlant(1)))
        assertFlightlineFile(try renderedFont(surface), italic: true)
        assertAxes(try renderedFont(surface), values: ["wght": 700])
        XCTAssertTrue(editor.setPropertyForTesting(.characterBold, value: .boolean(false)))
        assertAxes(try renderedFont(surface), values: ["wght": 437])
        assertFlightlineFile(try renderedFont(surface), italic: true)
        XCTAssertTrue(editor.setPropertyForTesting(.characterSlant, value: .fontSlant(0)))
        assertFlightlineFile(try renderedFont(surface), italic: false)
        styles.undoManager.undo()
        assertFlightlineFile(try renderedFont(surface), italic: true)
        styles.undoManager.redo()
        assertFlightlineFile(try renderedFont(surface), italic: false)
        let reopened = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: configuration.directory, legacyDefaults: nil))
        try reopened.read(source: Data("Variable aegklrWim".utf8), typeName: EVDocument.plainTextType)
        let restored = try XCTUnwrap(reopened.styleSheetSnapshot().definition(for: .baseParagraph)?
            .properties[.characterFontAxes]?.effective)
        guard case let .string(json) = restored else { return XCTFail("Missing saved Flightline axes") }
        XCTAssertEqual(EVFontVariations.decode(json), ["wght": 437])
        XCTAssertEqual(try backend.recoverySnapshot(), original)
        XCTAssertFalse(surface.canUndo)
        XCTAssertNil(surface.commandOutput)
    }

    private func assertFlightlineFile(_ font: CTFont, italic: Bool, file: StaticString = #filePath, line: UInt = #line) {
        let url = CTFontDescriptorCopyAttribute(CTFontCopyFontDescriptor(font), kCTFontURLAttribute) as? URL
        XCTAssertEqual(url?.lastPathComponent, "FlightlineCode-\(italic ? "Italic" : "Regular")-VF.ttf", file: file, line: line)
        XCTAssertEqual(CTFontGetSymbolicTraits(font).contains(.traitItalic), italic, file: file, line: line)
    }

    func testAxisSliderKeepsBaseWeightWhenNoCoordinatesWereSaved() throws {
        try registerRecursive()
        let (_, _, surface, _, editor) = try fixture()
        let weight = try control(NSSlider.self, label: "Weight", in: editor.view)
        XCTAssertEqual(weight.doubleValue, 400)
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        XCTAssertEqual(face.titleOfSelectedItem, "Sans Linear")
        let mono = try control(NSSlider.self, label: "Monospace", in: editor.view)
        mono.doubleValue = 1
        XCTAssertTrue(mono.sendAction(try XCTUnwrap(mono.action), to: mono.target))
        XCTAssertEqual(weight.doubleValue, 400)
        XCTAssertEqual(editor.inspection.preview.resolvedFontAxes["wght"], 400)
        XCTAssertEqual(editor.inspection.preview.resolvedFontAxes["MONO"], 1)
        let actual = CTFontCopyVariation(try renderedFont(surface)) as? [NSNumber: NSNumber] ?? [:]
        XCTAssertEqual(actual[NSNumber(value: EVFontVariations.identifier("wght"))]?.doubleValue, 400)
        XCTAssertEqual(actual[NSNumber(value: EVFontVariations.identifier("MONO"))]?.doubleValue, 1)
    }

    func testRecursivePresetsAndEveryAxisReachRenderedFontsWithoutSourceEdits() throws {
        try registerRecursive()
        let (configuration, backend, surface, styles, editor) = try fixture()
        let original = try backend.recoverySnapshot()
        let info = EVFontVariations.info(for: "Recursive")
        XCTAssertEqual(Set(info.axes.map(\.tag)), ["MONO", "CASL", "wght", "slnt", "CRSV"])
        XCTAssertEqual(info.instances.count, 65)
        let face = try control(NSPopUpButton.self, label: "Font face", in: editor.view)
        let presets = Array(face.itemArray.dropLast().suffix(info.instances.count))
        XCTAssertEqual(presets.map(\.title), info.instances.map(\.name))
        for (instance, item) in zip(info.instances, presets) {
            face.select(item)
            XCTAssertTrue(face.sendAction(try XCTUnwrap(face.action), to: face.target))
            let font = try renderedFont(surface)
            assertAxes(font, values: instance.values)
            XCTAssertEqual(editor.inspection.preview.resolvedFontAxes, instance.values)
            let file = CTFontDescriptorCopyAttribute(CTFontCopyFontDescriptor(font), kCTFontURLAttribute) as? URL
            XCTAssertEqual(file?.lastPathComponent, "Recursive_VF_1.085.ttf")
            for axis in info.axes where !axis.hidden {
                let slider = try control(NSSlider.self, label: axis.name, in: editor.view)
                XCTAssertEqual(slider.doubleValue, instance.values[axis.tag]!, accuracy: 0.001)
            }
        }
        for axis in info.axes {
            var outlines: [CGPath] = []
            var identities: [UInt64] = []
            for value in [axis.minimum, axis.maximum] {
                var coordinates = info.defaults
                coordinates[axis.tag] = value
                // The cursive switch affects the slanted forms as well.
                if axis.tag == "CRSV" { coordinates["slnt"] = -15 }
                XCTAssertTrue(editor.setPropertyForTesting(.characterFontAxes,
                    value: .string(EVFontVariations.encode(coordinates))))
                let font = try renderedFont(surface)
                assertAxes(font, values: coordinates)
                XCTAssertEqual(editor.inspection.preview.resolvedFontAxes, coordinates)
                let line = CTLineCreateWithAttributedString(NSAttributedString(string: "aegklrWim", attributes: [.font: font]))
                let path = CGMutablePath()
                for run in CTLineGetGlyphRuns(line) as? [CTRun] ?? [] {
                    var glyphs = [CGGlyph](repeating: 0, count: CTRunGetGlyphCount(run))
                    CTRunGetGlyphs(run, CFRange(location: 0, length: 0), &glyphs)
                    for glyph in glyphs { path.addPath(try XCTUnwrap(CTFontCreatePathForGlyph(font, glyph, nil))) }
                }
                outlines.append(path)
                identities.append(try XCTUnwrap(surface.layoutSnapshot?.clusters.first).render_run.identifier)
            }
            XCTAssertFalse(outlines[0] == outlines[1], "\(axis.tag) must change the actual outlines")
            XCTAssertNotEqual(identities[0], identities[1], "Axis changes must invalidate cached render resources")
            // Exercise the native slider action as well as the preset menu.
            let slider = try control(NSSlider.self, label: axis.name, in: editor.view)
            slider.doubleValue = axis.minimum
            XCTAssertTrue(slider.sendAction(try XCTUnwrap(slider.action), to: slider.target))
            XCTAssertEqual(editor.inspection.preview.resolvedFontAxes[axis.tag], axis.minimum)
            let actual = CTFontCopyVariation(try renderedFont(surface)) as? [NSNumber: NSNumber] ?? [:]
            XCTAssertEqual(actual[NSNumber(value: EVFontVariations.identifier(axis.tag))]?.doubleValue ?? axis.defaultValue,
                axis.minimum, accuracy: 0.001)
        }
        var saved = info.defaults
        saved["wght"] = 450; saved["slnt"] = -5; saved["CASL"] = 0.625; saved["MONO"] = 0.375
        XCTAssertTrue(editor.setPropertyForTesting(.characterFontAxes, value: .string(EVFontVariations.encode(saved))))
        XCTAssertTrue(editor.setPropertyForTesting(.characterBold, value: .boolean(true)))
        XCTAssertTrue(editor.setPropertyForTesting(.characterSlant, value: .fontSlant(1)))
        assertAxes(try renderedFont(surface), values: EVFontVariations.effective(info, saved: saved, weight: 750, bold: true, slant: 1))
        XCTAssertTrue(editor.setPropertyForTesting(.characterBold, value: .boolean(false)))
        XCTAssertTrue(editor.setPropertyForTesting(.characterSlant, value: .fontSlant(0)))
        assertAxes(try renderedFont(surface), values: saved)
        styles.undoManager.undo()
        assertAxes(try renderedFont(surface), values: EVFontVariations.effective(info, saved: saved, weight: 450, bold: false, slant: 1))
        styles.undoManager.redo()
        assertAxes(try renderedFont(surface), values: saved)
        let reopened = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: configuration.directory, legacyDefaults: nil))
        try reopened.read(source: Data("Variable aegklrWim".utf8), typeName: EVDocument.plainTextType)
        let restored = try XCTUnwrap(reopened.styleSheetSnapshot().definition(for: .baseParagraph)?
            .properties[.characterFontAxes]?.effective)
        guard case let .string(json) = restored else { return XCTFail("Missing saved font axes") }
        XCTAssertEqual(EVFontVariations.decode(json), saved)
        XCTAssertEqual(try backend.recoverySnapshot(), original)
        XCTAssertFalse(surface.canUndo)
        XCTAssertNil(surface.commandOutput)
    }

    func testAllAxisLabelsFitAtMinimumWindowSizeAndStaticFontsRemoveRows() throws {
        try registerRecursive()
        let (_, _, surface, _, editor) = try fixture()
        let panel = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 700, height: 620),
            styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        panel.appearance = NSAppearance(named: .darkAqua)
        panel.contentViewController = editor
        defer { panel.orderOut(nil); withExtendedLifetime(surface) {} }
        panel.orderFront(nil)
        panel.contentView?.layoutSubtreeIfNeeded()
        let sliders = descendants(editor.view).compactMap { $0 as? NSSlider }
        XCTAssertEqual(sliders.count, 5)
        for slider in sliders {
            let column = try XCTUnwrap(slider.superview)
            let label = try XCTUnwrap(column.subviews.compactMap { $0 as? NSTextField }.first)
            if slider.accessibilityLabel() == "Cursive" { XCTAssertEqual(label.stringValue, "Cursive 0.5") }
            XCTAssertGreaterThanOrEqual(label.frame.height, label.intrinsicContentSize.height)
            XCTAssertGreaterThanOrEqual(label.frame.width, label.intrinsicContentSize.width)
            XCTAssertFalse(label.visibleRect.isEmpty)
            XCTAssertGreaterThanOrEqual(label.visibleRect.height, label.bounds.height)
            XCTAssertGreaterThanOrEqual(slider.visibleRect.height, slider.bounds.height)
        }
        if let path = ProcessInfo.processInfo.environment["VIEM_STYLE_CAPTURE_PATH"] {
            let bitmap = try XCTUnwrap(editor.view.bitmapImageRepForCachingDisplay(in: editor.view.bounds))
            editor.view.cacheDisplay(in: editor.view.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: path))
        }
        let recursiveHeight = panel.contentMinSize.height
        XCTAssertTrue(editor.setPropertyForTesting(.characterFontFamilies, value: .stringList(["Helvetica"])))
        panel.contentView?.layoutSubtreeIfNeeded()
        XCTAssertTrue(descendants(editor.view).compactMap { $0 as? NSSlider }.isEmpty)
        XCTAssertLessThan(panel.contentMinSize.height, recursiveHeight)
    }

    private func fixture(family: String = "Recursive", weight: UInt32 = 400) throws -> (EVConfigurationStore, EVCoreDocumentBackend, EVEditorSurfaceController, EVThemeStyleSession, EVStyleEditorViewController) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-variable-font-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let config = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        try config.createTheme(named: "Variable")
        let styles = try EVThemeStyleSession(configuration: config, format: .plainText)
        try styles.edit(key: .baseParagraph, expected: styles.snapshot().identity,
            mutation: .setDeclaration(.characterFontFamilies, .stringList([family])))
        if weight != 400 {
            try styles.edit(key: .baseParagraph, expected: styles.snapshot().identity,
                mutation: .setDeclaration(.characterWeight, .unsigned(weight)))
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

    private func registerRecursive() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let file = root.appendingPathComponent("assets/fonts/recursive/Recursive_VF_1.085.ttf")
        let owned = CTFontManagerRegisterFontsForURL(file as CFURL, .process, nil)
        EVFontCatalog.invalidate()
        if owned { addTeardownBlock { CTFontManagerUnregisterFontsForURL(file as CFURL, .process, nil); EVFontCatalog.invalidate() } }
        XCTAssertFalse(EVFontVariations.info(for: "Recursive").axes.isEmpty)
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

    private func assertAxes(_ font: CTFont, values: [String: Double], file: StaticString = #filePath, line: UInt = #line) {
        let axes = CTFontCopyVariation(font) as? [NSNumber: NSNumber] ?? [:]
        let defaults = EVFontVariations.info(font: font).defaults
        for (tag, value) in values {
            XCTAssertEqual(axes[NSNumber(value: EVFontVariations.identifier(tag))]?.doubleValue ?? defaults[tag] ?? .nan,
                value, accuracy: 0.001, tag, file: file, line: line)
        }
    }

    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    private func control<T: NSView>(_ type: T.Type, label: String, in root: NSView) throws -> T {
        try XCTUnwrap(descendants(root).first { $0 is T && $0.accessibilityLabel() == label } as? T)
    }
}
