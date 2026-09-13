import AppKit
import CoreText
import CViemCore
import ViemAppShell
import ViemCoreTextProvider
import XCTest
@testable import ViemEditor

@MainActor
final class EVWhitespaceIntegrationTests: XCTestCase {
    private func fixture(_ source: String, type: String = "public.plain-text") throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVEditingPreferences) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-whitespace-native-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory)
        let preferences = EVEditingPreferences(configuration: configuration)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(source.utf8), typeName: type)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.editorView.editingPreferences = preferences
        surface.view.frame = NSRect(x: 0, y: 0, width: 500, height: 240)
        surface.viewDidLayout()
        _ = try XCTUnwrap(surface.session).resize(width: 500, height: 240)
        surface.refreshPresentation()
        return (backend, surface, preferences)
    }

    private func ex(_ command: String, surface: EVEditorSurfaceController) throws {
        let session = try XCTUnwrap(surface.session)
        _ = try session.sendText(":" + command)
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        surface.refreshPresentation()
    }

    private func geometry(_ snapshot: EVLayoutExport) -> [Float] {
        snapshot.rows.flatMap { [$0.y, $0.baseline, $0.ascent, $0.descent, $0.width, $0.line_advance] }
            + snapshot.clusters.flatMap { [$0.x, $0.advance, $0.typographic_bounds.x, $0.typographic_bounds.y, $0.typographic_bounds.width, $0.typographic_bounds.height] }
    }

    func testExactMarkersAndExListTogglesPreserveSourceGeometryAndHistory() throws {
        let source = "\tone  \nlast\t "
        let (backend, surface, _) = try fixture(source)
        let before = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertFalse(before.whitespace.markers.isEmpty)
        XCTAssertTrue(before.whitespace.markers.contains { $0.text.contains(">") })
        XCTAssertTrue(before.whitespace.markers.contains { $0.text == "*" })
        let state = backend.persistenceState
        XCTAssertFalse(surface.canUndo)
        try ex("set nolist", surface: surface)
        let hidden = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertTrue(hidden.whitespace.markers.isEmpty)
        XCTAssertEqual(geometry(hidden), geometry(before))
        XCTAssertEqual(backend.persistenceState, state)
        XCTAssertFalse(surface.canUndo)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
        try ex("set list", surface: surface)
        let restored = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertEqual(restored.whitespace.markers, before.whitespace.markers)
        XCTAssertEqual(geometry(restored), geometry(before))
        XCTAssertThrowsError(try XCTUnwrap(surface.session).whitespaceMarkersExport(identity: before.info.identity))
    }

    func testDefaultChangesPropagateToMultipleViewsAndRespectExplicitListOverride() throws {
        let (backend, first, preferences) = try fixture("\tone  ")
        let second = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        second.loadViewIfNeeded()
        second.editorView.editingPreferences = preferences
        _ = try XCTUnwrap(second.session).resize(width: 500, height: 240)
        second.refreshPresentation()
        try ex("set nolist", surface: second)
        let source = try backend.serializedSource(typeName: EVDocument.plainTextType)
        var presentation = preferences.whitespacePresentation
        presentation.visibleWhitespace.enabled = false
        XCTAssertTrue(preferences.setWhitespacePresentation(presentation))
        XCTAssertTrue(try XCTUnwrap(first.layoutSnapshot).whitespace.markers.isEmpty)
        XCTAssertTrue(try XCTUnwrap(second.layoutSnapshot).whitespace.markers.isEmpty)
        presentation.visibleWhitespace.enabled = true
        presentation.visibleWhitespace.listchars = "tab:>-^,trail:!"
        XCTAssertTrue(preferences.setWhitespacePresentation(presentation))
        XCTAssertTrue(try XCTUnwrap(first.layoutSnapshot).whitespace.markers.contains { $0.text == "!" })
        XCTAssertTrue(try XCTUnwrap(second.layoutSnapshot).whitespace.markers.isEmpty)
        let width = try XCTUnwrap(first.layoutSnapshot?.clusters.first).advance
        var indentation = preferences.indentation
        indentation.tabstop = 8
        XCTAssertTrue(preferences.setIndentation(indentation))
        let changedWidth = try XCTUnwrap(first.layoutSnapshot?.clusters.first).advance
        XCTAssertGreaterThan(changedWidth, width)
        XCTAssertEqual(try XCTUnwrap(second.layoutSnapshot?.clusters.first).advance, changedWidth)
        try ex("set tabstop=4", surface: first)
        let explicitWidth = try XCTUnwrap(first.layoutSnapshot?.clusters.first).advance
        XCTAssertLessThan(explicitWidth, changedWidth)
        XCTAssertEqual(try XCTUnwrap(second.layoutSnapshot?.clusters.first).advance, explicitWidth,
            "An Ex change refreshes the other native pane immediately")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), source)
        XCTAssertFalse(first.canUndo || second.canUndo)
    }

    func testWhitespaceMarkersAreSuppressedInWYSIWYGAndShownInSourceViews() throws {
        let fixtures = [
            (EVDocument.markdownType, "A  \n\nB", false),
            (EVDocument.htmlType, "<p>A </p><p>B</p>", false),
            (EVDocument.rtfType, #"{\rtf1 A \par B}"#, false),
            (EVDocument.markdownSourceType, "A  \n\nB", true),
            (EVDocument.htmlSourceType, "<p>A</p>  \n", true),
            (EVDocument.codeType, "A  \nB", true),
        ]
        for (type, source, visible) in fixtures {
            let (backend, surface, _) = try fixture(source, type: type)
            let snapshot = try XCTUnwrap(surface.layoutSnapshot)
            XCTAssertEqual(!snapshot.whitespace.markers.isEmpty, visible, type)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8), type)
        }
    }

    func testCompositionMarkersUseTheOverlaySnapshotAndCancellationRetainsSource() throws {
        let (backend, surface, _) = try fixture("base ")
        let session = try XCTUnwrap(surface.session)
        _ = try session.sendText("A")
        surface.refreshPresentation()
        let before = try XCTUnwrap(surface.layoutSnapshot)
        let revision = try backend.revision()
        surface.editorView.setMarkedText("\t  ", selectedRange: NSRange(location: 3, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(session.hasActiveComposition)
        let overlay = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertFalse(overlay.info.identity.isSameLayout(as: before.info.identity))
        XCTAssertGreaterThan(overlay.whitespace.markers.count, before.whitespace.markers.count)
        XCTAssertEqual(try session.whitespaceMarkersExport(identity: overlay.info.identity), overlay.whitespace)
        XCTAssertThrowsError(try session.whitespaceMarkersExport(identity: before.info.identity))
        XCTAssertEqual(try backend.formattedText(), "base ")
        XCTAssertEqual(try backend.revision(), revision)
        surface.editorView.cancelOperation(nil)
        XCTAssertFalse(session.hasActiveComposition)
        XCTAssertEqual(try backend.formattedText(), "base ")
        XCTAssertEqual(try XCTUnwrap(surface.layoutSnapshot).whitespace.markers, before.whitespace.markers)
    }

    func testNativeInvisibleCharactersMenuWorksInInsertModeAndFollowsExState() throws {
        let (backend, surface, _) = try fixture("word ")
        XCTAssertEqual(surface.presentation(for: .showInvisibleCharacters).state, .on)
        let session = try XCTUnwrap(surface.session)
        _ = try session.sendText("A")
        surface.refreshPresentation()
        surface.perform(menuCommand: .showInvisibleCharacters, sender: nil)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(try backend.formattedText(), "word ")
        XCTAssertEqual(surface.presentation(for: .showInvisibleCharacters).state, .off)
        XCTAssertTrue(try XCTUnwrap(surface.layoutSnapshot).whitespace.markers.isEmpty)
        surface.perform(menuCommand: .showInvisibleCharacters, sender: nil)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(surface.presentation(for: .showInvisibleCharacters).state, .on)
        XCTAssertFalse(surface.canUndo)
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        try ex("set nolist", surface: surface)
        XCTAssertEqual(surface.presentation(for: .showInvisibleCharacters).state, .off)
        let (_, rich, _) = try fixture("<p>word </p>", type: EVDocument.htmlType)
        XCTAssertFalse(rich.presentation(for: .showInvisibleCharacters).isEnabled)
        XCTAssertTrue(try XCTUnwrap(rich.layoutSnapshot).whitespace.markers.isEmpty)
    }

    func testMalformedFFISettingsAreRejectedAtomically() throws {
        let (backend, surface, _) = try fixture("\tword ")
        let before = try XCTUnwrap(surface.layoutSnapshot)
        let source = try backend.serializedSource(typeName: EVDocument.plainTextType)
        for text in [#"{"visibleWhitespace":{"listchars":"tab:>"}}"#,
                     #"{"visibleWhitespace":{"listchars":"space:界"}}"#,
                     #"{"visibleWhitespace":{"style":{"weight":0}}}"#,
                     #"{"codeWhitespace":"unknown"}"#] {
            let status = Data(text.utf8).withUnsafeBytes {
                viem_core_set_whitespace_presentation_defaults(backend.core, $0.bindMemory(to: UInt8.self).baseAddress, UInt64($0.count))
            }
            XCTAssertNotEqual(status, UInt32(VIEM_STATUS_OK), text)
        }
        let invalidIndent = Data(#"{"tabstop":0}"#.utf8).withUnsafeBytes {
            viem_core_set_indentation_defaults(backend.core, $0.bindMemory(to: UInt8.self).baseAddress, UInt64($0.count))
        }
        XCTAssertNotEqual(invalidIndent, UInt32(VIEM_STATUS_OK))
        surface.refreshPresentation()
        XCTAssertTrue(try XCTUnwrap(surface.layoutSnapshot).info.identity.isSameLayout(as: before.info.identity))
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), source)
        XCTAssertFalse(surface.canUndo)
    }

    func testMarkerFontOverridesFitOriginalSlotAndDoNotChangeLayout() throws {
        let (_, surface, preferences) = try fixture("\tword ")
        let before = try XCTUnwrap(surface.layoutSnapshot)
        let inherited = CTFontCreateWithName("Georgia" as CFString, 17, nil)
        let same = EVEditorView.whitespaceFont(EVVisibleWhitespaceStyle(), inherited: inherited, scale: 1)
        XCTAssertEqual(CTFontCopyPostScriptName(same) as String, CTFontCopyPostScriptName(inherited) as String)
        XCTAssertEqual(CTFontGetSize(same), 17)
        var style = EVVisibleWhitespaceStyle.defaultStyle
        style.fontFamilies = ["Viem nonexistent marker font", "Courier"]
        style.size = 60
        style.bold = true
        let override = EVEditorView.whitespaceFont(style, inherited: inherited, scale: 2)
        XCTAssertEqual(CTFontGetSize(override), 120)
        XCTAssertTrue((CTFontCopyFamilyName(override) as String).contains("Courier"))
        XCTAssertTrue(CTFontGetSymbolicTraits(override).contains(.traitBold))
        var unbold = EVVisibleWhitespaceStyle()
        unbold.bold = false
        unbold.slant = .upright
        let heavy = CTFontCreateWithName("Georgia-BoldItalic" as CFString, 17, nil)
        let regular = EVEditorView.whitespaceFont(unbold, inherited: heavy, scale: 1)
        XCTAssertFalse(CTFontGetSymbolicTraits(regular).contains(.traitBold))
        XCTAssertFalse(CTFontGetSymbolicTraits(regular).contains(.traitItalic))
        let marker = try XCTUnwrap(before.whitespace.markers.first)
        let line = CTLineCreateWithAttributedString(NSAttributedString(string: marker.text, attributes: [NSAttributedString.Key(kCTFontAttributeName as String): override]))
        var ascent: CGFloat = 0
        var descent: CGFloat = 0
        let width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, nil))
        let fit = EVEditorView.whitespaceInkScale(width: width, ascent: ascent, descent: descent, slot: CGSize(width: marker.width, height: marker.height))
        XCTAssertLessThanOrEqual(width * fit, marker.width + 0.001)
        XCTAssertLessThanOrEqual((ascent + descent) * fit, marker.height + 0.001)
        var options = preferences.whitespacePresentation
        options.visibleWhitespace.style = style
        XCTAssertTrue(preferences.setWhitespacePresentation(options))
        let after = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertEqual(geometry(after), geometry(before))
        XCTAssertEqual(after.whitespace.style, style)
        XCTAssertEqual(after.whitespace.markers, before.whitespace.markers)
    }

    func testClearedMarkerForegroundInheritsExactTextPaintAndRendersDecoration() throws {
        let (_, surface, preferences) = try fixture("a ")
        var options = preferences.whitespacePresentation
        options.visibleWhitespace.listchars = "trail:*"
        options.visibleWhitespace.style.foreground = nil
        options.visibleWhitespace.style.strikethrough = true
        XCTAssertTrue(preferences.setWhitespacePresentation(options))
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let marker = try XCTUnwrap(snapshot.whitespace.markers.first)
        var paint = try XCTUnwrap(surface.layoutPaint)
        paint.info.default_paint.flags = 0
        paint.info.default_paint.foreground.red = 0
        paint.info.default_paint.foreground.green = 0.8
        paint.info.default_paint.foreground.blue = 0
        paint.info.default_paint.foreground.alpha = 1
        paint.runs = []
        surface.layoutPaint = paint
        let image = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 500, pixelsHigh: 240,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        let graphics = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: image))
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = graphics
        defer { NSGraphicsContext.restoreGraphicsState() }
        let context = graphics.cgContext
        context.translateBy(x: 0, y: 240)
        context.scaleBy(x: 1, y: -1)
        surface.editorView.drawWhitespaceMarkers(snapshot, in: context)
        context.flush()
        var greenPixels = 0
        for y in 0..<240 { for x in 0..<500 {
            if let color = image.colorAt(x: x, y: y)?.usingColorSpace(.sRGB), color.alphaComponent > 0.1,
               color.greenComponent > 0.4 && color.redComponent < 0.3 && color.blueComponent < 0.3 { greenPixels += 1 }
        }}
        XCTAssertGreaterThan(greenPixels, 0)
        XCTAssertGreaterThan(marker.width, 0)
    }

    func testMarkerInheritsNonFontRunAttributesAndExplicitOverridesPreserveGeometry() throws {
        let (backend, surface, preferences) = try fixture("x ", type: EVDocument.codeType)
        let codeStyle = try EVCodeStyleSession(configuration: backend.configuration)
        let declarations: [(EVStyleProperty, EVStyleValue)] = [
            (.characterBaselineShift, .float(4)), (.characterLetterSpacing, .float(3)),
            (.characterLanguage, .string("ar")), (.characterDirection, .writingDirection(2))
        ]
        for (property, value) in declarations {
            try codeStyle.edit(key: .baseParagraph, expected: codeStyle.snapshot().identity,
                mutation: .setDeclaration(property, value))
        }
        surface.refreshPresentation()
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let marker = try XCTUnwrap(snapshot.whitespace.markers.first)
        let clusters = snapshot.clusters.filter { Int($0.row_index) == marker.rowIndex }.sorted { $0.x < $1.x }
        let cluster = try XCTUnwrap(EVEditorView.whitespaceCluster(at: marker.x, in: clusters))
        let registry = try XCTUnwrap(surface.session).provider.renderRegistry
        let inherited = try XCTUnwrap(registry.textAttributes(identifier: cluster.render_run.identifier,
            metricsGeneration: cluster.render_run.metrics_generation))
        XCTAssertEqual(inherited.baselineShift, 4)
        XCTAssertEqual(inherited.letterSpacing, 3)
        XCTAssertEqual(inherited.language, "ar")
        XCTAssertEqual(inherited.writingDirection, .rightToLeft)
        XCTAssertEqual(EVEditorView.whitespaceTextAttributes(snapshot.whitespace.style,
            inherited: inherited, scale: 2), inherited, "Inherited attributes are already scaled")

        var explicit = EVVisibleWhitespaceStyle.defaultStyle
        explicit.baselineShift = 0
        explicit.letterSpacing = 0
        explicit.language = "en"
        explicit.direction = .natural
        let resolved = EVEditorView.whitespaceTextAttributes(explicit, inherited: inherited, scale: 2)
        XCTAssertEqual(resolved, CoreTextRenderAttributes(language: "en"))
        let inheritedInk = try markerInkCentroid(snapshot, surface: surface)
        var options = preferences.whitespacePresentation
        options.visibleWhitespace.style = explicit
        XCTAssertTrue(preferences.setWhitespacePresentation(options))
        let overridden = try XCTUnwrap(surface.layoutSnapshot)
        let overriddenInk = try markerInkCentroid(overridden, surface: surface)
        XCTAssertGreaterThan(abs(inheritedInk.y - overriddenInk.y), 1,
            "Inherited baseline shift must move the marker ink")
        XCTAssertEqual(geometry(overridden), geometry(snapshot))
        XCTAssertEqual(overridden.whitespace.markers, snapshot.whitespace.markers)
        XCTAssertEqual(try backend.formattedText(), "x ")
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(surface.canUndo)
    }

    private func markerInkCentroid(_ snapshot: EVLayoutExport, surface: EVEditorSurfaceController) throws -> CGPoint {
        let image = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 500, pixelsHigh: 240,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        let graphics = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: image))
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = graphics
        defer { NSGraphicsContext.restoreGraphicsState() }
        graphics.cgContext.translateBy(x: 0, y: 240)
        graphics.cgContext.scaleBy(x: 1, y: -1)
        surface.editorView.drawWhitespaceMarkers(snapshot, in: graphics.cgContext)
        graphics.cgContext.flush()
        var xTotal: CGFloat = 0, yTotal: CGFloat = 0, total: CGFloat = 0
        for y in 0..<240 { for x in 0..<500 {
            let alpha = image.colorAt(x: x, y: y)?.alphaComponent ?? 0
            xTotal += CGFloat(x) * alpha
            yTotal += CGFloat(y) * alpha
            total += alpha
        }}
        XCTAssertGreaterThan(total, 0)
        return CGPoint(x: xTotal / max(total, 1), y: yTotal / max(total, 1))
    }

    func testLongWhitespaceRowExportsOnlyViewportMarkersAndLookupUsesContainingSlot() throws {
        let (_, surface, _) = try fixture(String(repeating: " ", count: 100_000))
        try ex("set nowrap", surface: surface)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertFalse(snapshot.whitespace.markers.isEmpty)
        XCTAssertLessThan(snapshot.whitespace.markers.count, 1000)
        let clusters = (0..<10_000).map { index in
            var cluster = ViemPositionedClusterV1()
            cluster.x = Float(index * 2)
            cluster.advance = 2
            cluster.text_start = UInt64(index)
            return cluster
        }
        for index in stride(from: 0, to: 10_000, by: 79) {
            XCTAssertEqual(EVEditorView.whitespaceCluster(at: CGFloat(index * 2) + 1.75, in: clusters)?.text_start, UInt64(index))
        }
    }
}
