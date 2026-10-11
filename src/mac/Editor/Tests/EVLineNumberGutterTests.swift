import AppKit
import CoreText
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVLineNumberGutterTests: XCTestCase {
    private func makeSurface(_ source: String, format: EVSourceFormat = .plainText,
        declarations: [(EVStyleKey, EVStyleProperty, EVStyleValue?)] = []) throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-line-numbers-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        try EVStyleTestFixtures.configure(configuration, format: format, declarations: [
            (.baseParagraph, .characterSize, .float(14)),
            (.baseParagraph, .characterFontFamilies, .stringList(["system-ui"])),
            (.baseParagraph, .blockMarginTop, .float(0)),
            (.baseParagraph, .blockMarginBottom, .float(0)),
            (EVStyleTestFixtures.block("Builtin line number"), .characterFontFamilies, .stringList(["Times"])),
            (EVStyleTestFixtures.block("Builtin line number"), .characterSize, .float(11)),
            (EVStyleTestFixtures.block("Builtin line number"), .blockPaddingLeft, .float(4)),
            (EVStyleTestFixtures.block("Builtin line number"), .blockPaddingRight, .float(4)),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderRightWidth, .float(1)),
        ] + declarations)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(source.utf8), typeName: format == .plainText ? EVDocument.plainTextType : EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 330, height: 220)
        surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }

    func testGutterDragSelectsWholeRowsAndKeepsItsCursorAcrossText() throws {
        let (backend, surface, session) = try makeSurface("first\nsecond\nthird\nlast\n")
        defer { session.detach() }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 330, height: 220), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = surface
        defer { window.contentViewController = nil }
        surface.performInput { try session.setNumber(true) }
        let view = surface.editorView
        let rows = try XCTUnwrap(surface.layoutSnapshot).rows
        func event(_ type: NSEvent.EventType, _ row: Int, overText: Bool = false) throws -> NSEvent {
            let y = view.viewPoint(fromLayoutPoint: CGPoint(x: 0, y: CGFloat(rows[row].y + rows[row].ascent * 0.5))).y
            let local = NSPoint(x: overText ? view.textViewportRect.midX : view.lineNumberGutterWidth / 2, y: y)
            return try XCTUnwrap(NSEvent.mouseEvent(with: type, location: view.convert(local, to: nil), modifierFlags: [],
                timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        }
        let textPoint = NSPoint(x: view.textViewportRect.midX, y: view.textViewportRect.midY)
        XCTAssertTrue(view.pointerCursor(at: textPoint) === NSCursor.arrow)
        view.mouseDown(with: try event(.leftMouseDown, 1))
        XCTAssertEqual(surface.selectedUTF8Range(), 6..<13)
        view.mouseDragged(with: try event(.leftMouseDragged, 2, overText: true))
        XCTAssertEqual(surface.selectedUTF8Range(), 6..<19)
        XCTAssertTrue(view.pointerCursor(at: textPoint) === EVLineNumberGutter.selectionCursor)
        view.mouseDragged(with: try event(.leftMouseDragged, 0))
        XCTAssertEqual(surface.selectedUTF8Range(), 0..<13)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 0)
        view.mouseDragged(with: try event(.leftMouseDragged, 1))
        XCTAssertEqual(surface.selectedUTF8Range(), 6..<13)
        view.mouseUp(with: try event(.leftMouseUp, 1, overText: true))
        XCTAssertFalse(view.gutterSelectionDragging)
        XCTAssertTrue(view.pointerCursor(at: textPoint) === NSCursor.iBeam)
        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        XCTAssertTrue(view.pointerCursor(at: textPoint) === NSCursor.arrow)
        surface.performInput { _ = try session.sendText("v") }
        XCTAssertTrue(view.pointerCursor(at: textPoint) === NSCursor.arrow)
        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 103) }
        XCTAssertTrue(view.pointerCursor(at: textPoint) === NSCursor.iBeam)
        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        surface.performInput { _ = try session.sendText("i") }
        XCTAssertTrue(view.pointerCursor(at: textPoint) === NSCursor.iBeam)
        XCTAssertEqual(try backend.formattedText(), "first\nsecond\nthird\nlast\n")
        XCTAssertEqual(try session.lineNumberInfo().source_line_count, 5)
        XCTAssertEqual(view.lineNumberGutter.bands.map(\.line), [1, 2, 3, 4, 5])
    }

    func testGutterDragAutoscrollIsVerticalAndRetainsInitialRow() throws {
        let (_, surface, session) = try makeSurface(Array(repeating: "row of text", count: 300).joined(separator: "\n"))
        defer { session.detach() }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 330, height: 220), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = surface
        defer { window.contentViewController = nil }
        surface.performInput { try session.setNumber(true) }
        let view = surface.editorView
        func event(_ type: NSEvent.EventType, _ point: NSPoint) throws -> NSEvent {
            try XCTUnwrap(NSEvent.mouseEvent(with: type, location: view.convert(point, to: nil), modifierFlags: [],
                timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        }
        let first = try XCTUnwrap(surface.layoutSnapshot?.rows.first)
        let start = view.viewPoint(fromLayoutPoint: CGPoint(x: 0, y: CGFloat(first.y + first.ascent / 2)))
        view.mouseDown(with: try event(.leftMouseDown, NSPoint(x: 1, y: start.y)))
        let outside = NSPoint(x: -20, y: view.textViewportRect.maxY + 60)
        view.mouseDragged(with: try event(.leftMouseDragged, outside))
        XCTAssertTrue(view.performDragAutoscrollStep())
        XCTAssertGreaterThan(surface.viewportState.top, 0)
        XCTAssertEqual(surface.viewportState.left, 0)
        XCTAssertEqual(surface.selectedUTF8Range()?.lowerBound, 0)
        XCTAssertGreaterThan(surface.selectedUTF8Range()?.upperBound ?? 0, 12)
        XCTAssertTrue(view.pointerCursor(at: outside) === EVLineNumberGutter.selectionCursor)
        view.mouseUp(with: try event(.leftMouseUp, outside))
    }

    func testExNumberReservesRealChromeAndPreservesSourceAndHistory() throws {
        let source = (1...120).map { "Line \($0)" }.joined(separator: "\n")
        let (backend, surface, session) = try makeSurface(source)
        defer { session.detach() }
        let before = try backend.recoverySnapshot()
        let width = surface.editorView.layoutViewportSize.width
        surface.performInput {
            _ = try session.sendText(":set number")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        }
        let gutter = surface.editorView.lineNumberGutter
        XCTAssertTrue(try session.number())
        XCTAssertFalse(gutter.isHidden)
        XCTAssertGreaterThan(surface.lineNumberGutterWidth, 0)
        XCTAssertEqual(surface.editorView.layoutViewportSize.width, width - surface.lineNumberGutterWidth, accuracy: 0.01)
        XCTAssertEqual(surface.editorView.textViewportRect.minX, surface.lineNumberGutterWidth)
        let point = surface.editorView.viewPoint(fromLayoutPoint: CGPoint(x: 19, y: 31))
        XCTAssertEqual(surface.editorView.layoutPoint(fromViewPoint: point), CGPoint(x: 19, y: 31))
        XCTAssertFalse(gutter.bands.isEmpty)
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertFalse(surface.canUndo)
        XCTAssertNil(surface.commandOutput)
        XCTAssertEqual(surface.presentation(for: .showLineNumbers).state, .on)
        surface.performInput {
            _ = try session.sendText(":set nu?")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        }
        XCTAssertEqual(surface.commandOutput, "number")
        surface.performInput {
            _ = try session.sendText(":set nonumber")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        }
        XCTAssertFalse(try session.number())
        XCTAssertTrue(gutter.isHidden)
        XCTAssertEqual(surface.presentation(for: .showLineNumbers).state, .off)
        XCTAssertEqual(surface.editorView.layoutViewportSize.width, width)
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        surface.perform(menuCommand: .showLineNumbers, sender: nil)
        XCTAssertTrue(try session.number())
        XCTAssertEqual(surface.presentation(for: .showLineNumbers).state, .on)
        surface.perform(menuCommand: .showLineNumbers, sender: nil)
        XCTAssertFalse(try session.number())
        XCTAssertEqual(surface.presentation(for: .showLineNumbers).state, .off)
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertFalse(surface.canUndo)
    }

    func testProportionalDigitsCurrentStyleAndDecimalThresholdCache() throws {
        let (backend, _, session) = try makeSurface("x", declarations: [
            (EVStyleTestFixtures.block("Builtin current line number"), .characterSize, .float(19)),
        ])
        defer { session.detach() }
        let gutter = EVLineNumberGutter()
        let sheet = try backend.styleSheetSnapshot()
        let one = gutter.width(lineCount: 9, sheet: sheet, scale: 1, metricsGeneration: 1)
        XCTAssertEqual(gutter.digitMeasurementCount, 20)
        let two = gutter.width(lineCount: 10, sheet: sheet, scale: 1, metricsGeneration: 1)
        XCTAssertGreaterThan(two, one)
        XCTAssertEqual(gutter.width(lineCount: 99, sheet: sheet, scale: 1, metricsGeneration: 1), two)
        XCTAssertEqual(gutter.digitMeasurementCount, 20, "Decimal width changes reuse digit metrics")
        gutter.present(bands: [], viewportTop: 0)
        for number in 1...99 {
            let layout = try XCTUnwrap(gutter.numberLayout(.init(line: UInt64(number), top: 0, bottom: 100,
                isCurrent: number == 55)))
            let width = CGFloat(CTLineGetTypographicBounds(layout.line, nil, nil, nil))
            let style = number == 55 ? gutter.current : gutter.normal
            XCTAssertGreaterThanOrEqual(layout.baseline.x + 0.01, style.margin.left + style.padding.left + style.leftBorder.width)
            XCTAssertLessThanOrEqual(layout.baseline.x + width, two - style.margin.right - style.padding.right - style.rightBorder.width + 0.01)
        }
        XCTAssertEqual(gutter.width(lineCount: 9, sheet: sheet, scale: 1, metricsGeneration: 1), one)
        XCTAssertEqual(gutter.digitMeasurementCount, 20, "Deleting across a decimal boundary reuses digit metrics")
        _ = gutter.width(lineCount: 9, sheet: sheet, scale: 2, metricsGeneration: 1)
        XCTAssertGreaterThan(gutter.columnWidth, one)
        XCTAssertEqual(gutter.digitMeasurementCount, 40)
        _ = gutter.width(lineCount: 9, sheet: sheet, scale: 2, metricsGeneration: 2)
        XCTAssertEqual(gutter.digitMeasurementCount, 60)
    }

    func testThemeForegroundChangesRefreshPaintWithoutMeasuringDigits() throws {
        let (backend, _, session) = try makeSurface("x", declarations: [
            (.baseParagraph, .characterForeground, nil),
            (EVStyleTestFixtures.block("Builtin line number"), .characterForeground, nil),
            (EVStyleTestFixtures.block("Builtin current line number"), .characterForeground, nil),
        ])
        defer { session.detach() }
        let gutter = EVLineNumberGutter()
        let sheet = try backend.styleSheetSnapshot()
        let first = EVThemeColor(0.1, 0.2, 0.3)
        let second = EVThemeColor(0.8, 0.7, 0.6)
        let width = gutter.width(lineCount: 99, sheet: sheet, scale: 1, metricsGeneration: 1,
            themeForeground: first)
        XCTAssertEqual(gutter.normal.foreground, first.color)
        XCTAssertEqual(gutter.current.foreground, first.color)
        let measurements = gutter.digitMeasurementCount
        XCTAssertEqual(measurements, 10, "Inherited typography shares the ordinary numeral measurements")
        XCTAssertEqual(gutter.width(lineCount: 99, sheet: sheet, scale: 1, metricsGeneration: 1,
            themeForeground: second), width)
        XCTAssertEqual(gutter.normal.foreground, second.color)
        XCTAssertEqual(gutter.current.foreground, second.color)
        XCTAssertEqual(gutter.digitMeasurementCount, measurements)
    }

    func testBuiltinGutterFlagsKeepSameNamedContentStylesIndependent() throws {
        let (backend, _, session) = try makeSurface("x")
        defer { session.detach() }
        let sheet = try backend.styleSheetSnapshot()
        let normal = try XCTUnwrap(sheet.lineNumberDefinition(current: false))
        let current = try XCTUnwrap(sheet.lineNumberDefinition(current: true))
        XCTAssertEqual(normal.name, "Line number")
        XCTAssertEqual(current.name, "Current line number")
        let paragraph = try XCTUnwrap(sheet.definition(for: .baseParagraph))
        func content(_ namespace: EVStyleNamespace, id: String, origin: EVStyleOrigin) -> EVStyleDefinition {
            EVStyleDefinition(key: .init(namespace: namespace, id: .init(rawValue: id)),
                name: "Line number", kind: namespace == .block ? .paragraph : .character,
                origin: origin, flags: [], capabilities: [.declarations], parentID: nil,
                nextStyleID: nil, properties: paragraph.properties)
        }
        let collision = EVStyleSheetSnapshot(identity: sheet.identity, definitions: [
            content(.block, id: "Line number", origin: .generatedConfiguration),
            content(.character, id: normal.key.id.rawValue, origin: .syntheticReadOnly),
        ] + sheet.definitions)
        XCTAssertEqual(collision.lineNumberDefinition(current: false)?.key, normal.key)
        XCTAssertEqual(collision.lineNumberDefinition(current: true)?.key, current.key)
        let baseline = EVLineNumberGutter(), gutter = EVLineNumberGutter()
        let width = baseline.width(lineCount: 999, sheet: sheet, scale: 1, metricsGeneration: 1)
        XCTAssertEqual(gutter.width(lineCount: 999, sheet: collision, scale: 1, metricsGeneration: 1), width)
        XCTAssertEqual(gutter.normal.font, baseline.normal.font)
        XCTAssertEqual(gutter.current.font, baseline.current.font)
    }

    func testFoldedRowShowsFirstNumberAndUsesCurrentStyleForItsSecondSourceLine() throws {
        let (backend, surface, session) = try makeSurface("first\nsecond", format: .markdown, declarations: [
            (EVStyleTestFixtures.block("Builtin current line number"), .characterSize, .float(19)),
        ])
        defer { session.detach() }
        surface.setFormattedView(true)
        let before = try backend.recoverySnapshot()
        surface.performInput {
            try session.setNumber(true)
            _ = try session.sendText("w")
        }
        let info = try session.lineNumberInfo()
        XCTAssertEqual(info.current_source_line, 2)
        let bands = surface.editorView.lineNumberGutter.bands
        XCTAssertEqual(bands.map(\.line), [1])
        let band = try XCTUnwrap(bands.first)
        XCTAssertTrue(band.isCurrent)
        let gutter = surface.editorView.lineNumberGutter
        let selected = try XCTUnwrap(gutter.numberLayout(band))
        var ordinary = band
        ordinary.isCurrent = false
        let unselected = try XCTUnwrap(gutter.numberLayout(ordinary))
        XCTAssertGreaterThan(CTLineGetTypographicBounds(selected.line, nil, nil, nil),
            CTLineGetTypographicBounds(unselected.line, nil, nil, nil))
        XCTAssertEqual(try backend.recoverySnapshot(), before)
    }

    func testPhysicalWrappedBandKeepsItsCenterWhenTheViewportScrollsAndShrinksToFit() throws {
        let (backend, _, session) = try makeSurface("x", declarations: [
            (EVStyleTestFixtures.block("Builtin line number"), .characterSize, .float(70)),
        ])
        defer { session.detach() }
        let gutter = EVLineNumberGutter()
        _ = gutter.width(lineCount: 100, sheet: try backend.styleSheetSnapshot(), scale: 1, metricsGeneration: 1)
        let wrapped = EVLineNumberBand(line: 1, top: 20, bottom: 320)
        gutter.present(bands: [wrapped], viewportTop: 0)
        let first = try XCTUnwrap(gutter.numberLayout(wrapped))
        gutter.present(bands: [wrapped], viewportTop: 80)
        let scrolled = try XCTUnwrap(gutter.numberLayout(wrapped))
        XCTAssertEqual(first.baseline.y - scrolled.baseline.y, 80, accuracy: 0.01)
        let short = EVLineNumberBand(line: 2, top: 0, bottom: 8)
        let smaller = try XCTUnwrap(gutter.numberLayout(short))
        var ascent: CGFloat = 0, descent: CGFloat = 0, leading: CGFloat = 0
        _ = CTLineGetTypographicBounds(smaller.line, &ascent, &descent, &leading)
        XCTAssertLessThanOrEqual(ascent + descent + leading, 8.01)
        XCTAssertEqual(smaller.baseline.y + 80 - (ascent - descent) / 2, 4, accuracy: 0.01)
    }

    func testStartAndEndAlignmentHonorExplicitRTLParagraphDirection() throws {
        for (alignment, expected) in [(UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_START), NSTextAlignment.right),
                                      (UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_END), NSTextAlignment.left)] {
            let (backend, _, session) = try makeSurface("x", declarations: [
                (EVStyleTestFixtures.block("Builtin line number"), .paragraphBaseDirection,
                    .writingDirection(UInt32(VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT))),
                (EVStyleTestFixtures.block("Builtin line number"), .paragraphAlignment, .paragraphAlignment(alignment)),
            ])
            defer { session.detach() }
            let gutter = EVLineNumberGutter()
            _ = gutter.width(lineCount: 100, sheet: try backend.styleSheetSnapshot(), scale: 1, metricsGeneration: 1)
            let layout = try XCTUnwrap(gutter.numberLayout(.init(line: 1, top: 0, bottom: 100)))
            let style = gutter.current
            let textWidth = CGFloat(CTLineGetTypographicBounds(layout.line, nil, nil, nil))
            XCTAssertEqual(style.alignment, expected)
            if expected == .left {
                XCTAssertEqual(layout.baseline.x, style.margin.left + style.leftBorder.width + style.padding.left, accuracy: 0.01)
            } else {
                XCTAssertEqual(layout.baseline.x + textWidth,
                    gutter.columnWidth - style.margin.right - style.rightBorder.width - style.padding.right, accuracy: 0.01)
            }
        }
    }

    func testPhysicalNumbersStayIndependentOfVisualCommandLineModeAndSplitCopiesOption() throws {
        let source = "first\nsecond\n\n## Heading\n\n" + String(repeating: "body words ", count: 150)
        let (backend, surface, session) = try makeSurface(source, format: .markdown)
        defer { session.detach() }
        surface.setFormattedView(true)
        surface.performInput {
            _ = try session.setViewportOrigin(left: 0, top: 0)
            try session.setNumber(true)
        }
        let info = try session.lineNumberInfo()
        let exported = try session.lineNumberBands(info)
        XCTAssertEqual(exported.map(\.line), surface.editorView.lineNumberGutter.bands.map(\.line))
        let ordinals = surface.editorView.lineNumberGutter.bands.map(\.line)
        let geometry = "lines \(ordinals), viewport \(info.viewport), cursor \(surface.viewPresentation.cursor_utf8_offset), rows \(surface.layoutSnapshot?.rows.map { $0.text_start } ?? [])"
        XCTAssertTrue(ordinals.contains(1), geometry)
        XCTAssertFalse(ordinals.contains(2), geometry)
        XCTAssertTrue(ordinals.contains(4), geometry)
        surface.performInput { try session.setLineMode(.physicalSource) }
        XCTAssertEqual(surface.editorView.lineNumberGutter.bands.map(\.line), ordinals)
        let next = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        defer { next.session?.detach() }
        next.inheritLineNumberOption(from: surface)
        XCTAssertTrue(try XCTUnwrap(next.session).number())
        next.performInput { try next.session?.setNumber(false) }
        XCTAssertTrue(try session.number(), "The copied option remains view-local")
    }

    func testTranslucentGutterBackgroundPaintsEachLineBoxOnce() throws {
        let (backend, _, session) = try makeSurface("x", declarations: [
            (EVStyleTestFixtures.block("Builtin line number"), .blockBackground,
                .color(EVStyleColor(red: 0, green: 0, blue: 0, alpha: 0.5))),
        ])
        defer { session.detach() }
        let gutter = EVLineNumberGutter()
        _ = gutter.width(lineCount: 1, sheet: try backend.styleSheetSnapshot(), scale: 1, metricsGeneration: 1)
        gutter.frame = NSRect(x: 0, y: 0, width: gutter.columnWidth, height: 80)
        gutter.present(bands: [.init(line: 1, top: 10, bottom: 70)], viewportTop: 0)
        let bitmap = try render(gutter)
        for y in [5, 20, 75] {
            let color = try XCTUnwrap(bitmap.colorAt(x: 1, y: y)?.usingColorSpace(.deviceRGB))
            XCTAssertEqual(color.redComponent, 0.5, accuracy: 0.01)
            XCTAssertEqual(color.greenComponent, 0.5, accuracy: 0.01)
            XCTAssertEqual(color.blueComponent, 0.5, accuracy: 0.01)
        }
    }

    func testOverlappingNegativeMarginsDoNotRepaintTheBaseBackground() throws {
        let (backend, _, session) = try makeSurface("x", declarations: [
            (EVStyleTestFixtures.block("Builtin line number"), .blockBackground,
                .color(EVStyleColor(red: 0, green: 0, blue: 0, alpha: 0.5))),
            (EVStyleTestFixtures.block("Builtin line number"), .blockMarginTop, .float(-5)),
            (EVStyleTestFixtures.block("Builtin line number"), .blockMarginBottom, .float(-5)),
        ])
        defer { session.detach() }
        let gutter = EVLineNumberGutter()
        _ = gutter.width(lineCount: 2, sheet: try backend.styleSheetSnapshot(), scale: 1, metricsGeneration: 1)
        gutter.frame = NSRect(x: 0, y: 0, width: gutter.columnWidth, height: 80)
        gutter.present(bands: [.init(line: 1, top: 10, bottom: 40), .init(line: 2, top: 40, bottom: 70)],
            viewportTop: 0)
        let bitmap = try render(gutter)
        let overlap = try XCTUnwrap(bitmap.colorAt(x: 1, y: 40)?.usingColorSpace(.deviceRGB))
        XCTAssertEqual(overlap.redComponent, 0.25, accuracy: 0.01,
            "Only the two deliberately overlapping line boxes contribute paint")
    }

    func testFractionalScrollAndDeviceScalesLeaveNoBackgroundOrRightBorderSeams() throws {
        let (backend, _, session) = try makeSurface("x", declarations: [
            (EVStyleTestFixtures.block("Builtin line number"), .blockBackground,
                .color(EVStyleColor(red: 0, green: 0, blue: 0, alpha: 0.5))),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderRightColor,
                .color(EVStyleColor(red: 1, green: 0, blue: 0, alpha: 0.5))),
            (EVStyleTestFixtures.block("Builtin current line number"), .blockBackground,
                .color(EVStyleColor(red: 0, green: 1, blue: 0, alpha: 0.5))),
        ])
        defer { session.detach() }
        let sheet = try backend.styleSheetSnapshot()
        let origin = CGPoint(x: 0.37, y: 0.29)
        for zoom in [CGFloat(0.75), 1.15, 1.65] {
            let gutter = EVLineNumberGutter()
            _ = gutter.width(lineCount: 99, sheet: sheet, scale: zoom, metricsGeneration: 1)
            gutter.frame = NSRect(x: 0, y: 0, width: gutter.columnWidth, height: 80)
            gutter.present(bands: [.init(line: 1, top: 8.2, bottom: 21.37),
                .init(line: 2, top: 21.37, bottom: 36.41, isCurrent: true),
                .init(line: 3, top: 36.41, bottom: 54.79),
                .init(line: 4, top: 54.79, bottom: 69.99)], viewportTop: 7.43)
            for scale in [CGFloat(1), 1.25, 1.5, 2] {
                for flipped in [false, true] {
                    let bitmap = try render(gutter, scale: scale, origin: origin, flipped: flipped)
                    let right = Int(ceil(gutter.columnWidth * scale + origin.x)) - 1
                    for y in 0..<Int(ceil(80 * scale + origin.y)) {
                        let background = try XCTUnwrap(bitmap.colorAt(x: 1, y: y)?.usingColorSpace(.deviceRGB))
                        let border = try XCTUnwrap(bitmap.colorAt(x: right, y: y)?.usingColorSpace(.deviceRGB))
                        XCTAssertEqual(background.redComponent, 0.5, accuracy: 0.01,
                            "Background seam at zoom \(zoom), scale \(scale), y \(y), flipped \(flipped)")
                        XCTAssertEqual(border.redComponent, 0.75, accuracy: 0.01,
                            "Border seam at zoom \(zoom), scale \(scale), y \(y), flipped \(flipped)")
                        XCTAssertEqual(border.blueComponent, 0.25, accuracy: 0.01)
                    }
                }
            }
        }
    }

    func testTranslucentBorderCornersAndShortBoxesPaintEachPixelOnce() throws {
        let edge: EVStyleValue = .color(EVStyleColor(red: 0, green: 0, blue: 0, alpha: 0.5))
        let (backend, _, session) = try makeSurface("x", declarations: [
            (EVStyleTestFixtures.block("Builtin line number"), .blockBackground, nil),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderLeftWidth, .float(1)),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderTopWidth, .float(1)),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderBottomWidth, .float(1)),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderLeftColor, edge),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderRightColor, edge),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderTopColor, edge),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderBottomColor, edge),
        ])
        defer { session.detach() }
        let gutter = EVLineNumberGutter()
        _ = gutter.width(lineCount: 2, sheet: try backend.styleSheetSnapshot(), scale: 1, metricsGeneration: 1)
        gutter.frame = NSRect(x: 0, y: 0, width: gutter.columnWidth, height: 80)
        gutter.present(bands: [.init(line: 1, top: 10.3, bottom: 40.3),
            .init(line: 2, top: 40.3, bottom: 41.3)], viewportTop: 0)
        let bitmap = try render(gutter)
        for x in [0, Int(gutter.columnWidth) - 1] {
            for y in [10, 39, 40] {
                let color = try devicePixelColor(bitmap, x: x, y: y)
                XCTAssertEqual(color.redComponent, 0.5, accuracy: 0.01)
            }
        }
        let short = try devicePixelColor(bitmap, x: 3, y: 40)
        XCTAssertEqual(short.redComponent, 0.5, accuracy: 0.01)
    }

    func testHorizontalBordersReachCanvasEdgesWithZeroVerticalBorders() throws {
        let edge: EVStyleValue = .color(EVStyleColor(red: 0, green: 0, blue: 0, alpha: 0.5))
        let (backend, _, session) = try makeSurface("x", declarations: [
            (EVStyleTestFixtures.block("Builtin line number"), .blockBackground, nil),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderLeftWidth, .float(0)),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderRightWidth, .float(0)),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderTopWidth, .float(1)),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderBottomWidth, .float(1)),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderTopColor, edge),
            (EVStyleTestFixtures.block("Builtin line number"), .blockBorderBottomColor, edge),
        ])
        defer { session.detach() }
        let gutter = EVLineNumberGutter()
        _ = gutter.width(lineCount: 2, sheet: try backend.styleSheetSnapshot(), scale: 1, metricsGeneration: 1)
        gutter.frame = NSRect(x: 0, y: 0, width: gutter.columnWidth, height: 80)
        gutter.present(bands: [.init(line: 1, top: 10.3, bottom: 40.3)], viewportTop: 0)
        let bitmap = try render(gutter, origin: CGPoint(x: 0.37, y: 0.29))
        for y in [11, 40] {
            for x in 0..<bitmap.pixelsWide {
                let color = try devicePixelColor(bitmap, x: x, y: y)
                XCTAssertEqual(color.redComponent, 0.5, accuracy: 0.01,
                    "Horizontal border gap at x \(x), y \(y)")
            }
        }
    }

    func testEmptyLineBoxesDoNotGainPixelCoverageAtFractionalCanvasEdges() throws {
        let (backend, _, session) = try makeSurface("x", declarations: [
            (EVStyleTestFixtures.block("Builtin line number"), .blockBackground,
                .color(EVStyleColor(red: 0, green: 0, blue: 0, alpha: 0.5))),
            (EVStyleTestFixtures.block("Builtin current line number"), .blockBackground,
                .color(EVStyleColor(red: 0, green: 1, blue: 0, alpha: 0.5))),
            (EVStyleTestFixtures.block("Builtin current line number"), .blockMarginTop, .float(1)),
        ])
        defer { session.detach() }
        let gutter = EVLineNumberGutter()
        _ = gutter.width(lineCount: 2, sheet: try backend.styleSheetSnapshot(), scale: 1, metricsGeneration: 1)
        gutter.frame = NSRect(x: 0, y: 0, width: gutter.columnWidth, height: 80)
        gutter.present(bands: [.init(line: 1, top: 79, bottom: 80, isCurrent: true)], viewportTop: 0)
        let bitmap = try render(gutter, origin: CGPoint(x: 0.37, y: 0.29))
        let color = try devicePixelColor(bitmap, x: 1, y: 80)
        XCTAssertEqual(color.redComponent, 0.5, accuracy: 0.01)
        XCTAssertEqual(color.greenComponent, 0.5, accuracy: 0.01)
        XCTAssertEqual(color.blueComponent, 0.5, accuracy: 0.01)
    }

    // CGContext device coordinates run bottom-up; bitmap scanlines run top-down.
    private func devicePixelColor(_ bitmap: NSBitmapImageRep, x: Int, y: Int) throws -> NSColor {
        try XCTUnwrap(bitmap.colorAt(x: x, y: bitmap.pixelsHigh - 1 - y)?.usingColorSpace(.deviceRGB))
    }

    private func render(_ gutter: EVLineNumberGutter, scale: CGFloat = 1, origin: CGPoint = .zero,
                        flipped: Bool = false) throws -> NSBitmapImageRep {
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil,
            pixelsWide: Int(ceil(gutter.columnWidth * scale + origin.x)),
            pixelsHigh: Int(ceil(80 * scale + origin.y)), bitsPerSample: 8,
            samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
            bytesPerRow: 0, bitsPerPixel: 0))
        let graphics = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: bitmap))
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = graphics
        NSColor.white.setFill()
        graphics.cgContext.fill(CGRect(x: 0, y: 0, width: CGFloat(bitmap.pixelsWide), height: CGFloat(bitmap.pixelsHigh)))
        graphics.cgContext.translateBy(x: origin.x, y: origin.y + (flipped ? 80 * scale : 0))
        graphics.cgContext.scaleBy(x: scale, y: flipped ? -scale : scale)
        gutter.draw(gutter.bounds)
        NSGraphicsContext.restoreGraphicsState()
        return bitmap
    }
}
