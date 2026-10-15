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
        XCTAssertEqual(try session.lineNumberInfo().source_line_count, 4)
        XCTAssertEqual(view.lineNumberGutter.bands.map(\.line), [1, 2, 3, 4])
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

    private func pointerEvent(_ type: NSEvent.EventType, at point: NSPoint,
        view: EVEditorView, window: NSWindow) throws -> NSEvent {
        try XCTUnwrap(NSEvent.mouseEvent(with: type, location: view.convert(point, to: nil), modifierFlags: [],
            timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
    }

    func testParagraphMarginSelectionIncludesPaddingAndWrappedRowsWithoutNumbers() throws {
        let source = String(repeating: "word ", count: 50) + "\nlast"
        let (backend, surface, session) = try makeSurface(source, declarations: [
            (.baseParagraph, .blockMarginLeft, .float(28)),
            (.baseParagraph, .blockPaddingLeft, .float(14)),
        ])
        defer { session.detach() }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 330, height: 220), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = surface
        defer { window.contentViewController = nil }
        let view = surface.editorView
        XCTAssertEqual(view.lineNumberGutterWidth, 0)
        let rows = try XCTUnwrap(surface.layoutSnapshot).rows
        XCTAssertGreaterThan(rows.count, 2)
        func point(_ row: Int, inset: CGFloat) -> CGPoint {
            view.viewPoint(fromLayoutPoint: CGPoint(x: CGFloat(rows[row].paragraph_content_x) - inset,
                y: CGFloat(rows[row].y + rows[row].ascent / 2)))
        }
        for row in 0..<2 {
            XCTAssertTrue(view.isRowSelectionPoint(point(row, inset: 2.1)))
            XCTAssertFalse(view.isRowSelectionPoint(point(row, inset: 2)))
            XCTAssertFalse(view.isRowSelectionPoint(point(row, inset: 1)))
            XCTAssertTrue(view.pointerCursor(at: point(row, inset: 3)) === EVLineNumberGutter.selectionCursor)
        }
        view.mouseDown(with: try pointerEvent(.leftMouseDown, at: point(1, inset: 8), view: view, window: window))
        XCTAssertTrue(view.gutterSelectionDragging)
        XCTAssertEqual(surface.selectedUTF8Range(), Int(rows[1].text_start)..<Int(rows[1].text_end))
        let outside = CGPoint(x: view.bounds.maxX + 50, y: point(0, inset: 1).y)
        view.mouseDragged(with: try pointerEvent(.leftMouseDragged, at: outside, view: view, window: window))
        XCTAssertEqual(surface.selectedUTF8Range(), Int(rows[0].text_start)..<Int(rows[1].text_end))
        XCTAssertTrue(view.pointerCursor(at: outside) === EVLineNumberGutter.selectionCursor)
        XCTAssertFalse(window.areCursorRectsEnabled)
        view.mouseUp(with: try pointerEvent(.leftMouseUp, at: outside, view: view, window: window))
        XCTAssertTrue(window.areCursorRectsEnabled)
        XCTAssertEqual(try backend.formattedText(), source)
    }

    func testOrdinarySelectionCursorStaysIBeamAcrossMarginsAndOutsideViewInEveryMode() throws {
        let (_, surface, session) = try makeSurface("first line\nsecond line", declarations: [
            (.baseParagraph, .blockMarginLeft, .float(28)),
        ])
        defer { session.detach() }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 330, height: 220), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = surface
        defer { window.contentViewController = nil }
        surface.performInput { try session.setNumber(true) }
        let view = surface.editorView
        for mode in ["normal", "insert", "visual", "select"] {
            surface.performInput {
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
                if mode == "insert" { _ = try session.sendText("i") }
                if mode == "visual" || mode == "select" { _ = try session.sendText("v") }
                if mode == "select" { _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 103) }
            }
            let row = try XCTUnwrap(surface.layoutSnapshot?.rows.first)
            let text = view.viewPoint(fromLayoutPoint: CGPoint(x: CGFloat(row.paragraph_content_x) + 20,
                y: CGFloat(row.y + row.ascent / 2)))
            view.mouseDown(with: try pointerEvent(.leftMouseDown, at: text, view: view, window: window))
            XCTAssertFalse(view.gutterSelectionDragging, mode)
            XCTAssertTrue(view.pointerCursor(at: text) === NSCursor.iBeam, mode)
            let gutter = CGPoint(x: 1, y: text.y)
            view.mouseDragged(with: try pointerEvent(.leftMouseDragged, at: gutter, view: view, window: window))
            XCTAssertTrue(view.pointerCursor(at: gutter) === NSCursor.iBeam, mode)
            let outside = CGPoint(x: -50, y: view.bounds.maxY + 50)
            XCTAssertTrue(view.pointerCursor(at: outside) === NSCursor.iBeam, mode)
            XCTAssertFalse(window.areCursorRectsEnabled, mode)
            view.mouseUp(with: try pointerEvent(.leftMouseUp, at: gutter, view: view, window: window))
            XCTAssertTrue(window.areCursorRectsEnabled, mode)
            XCTAssertTrue(view.pointerCursor(at: gutter) === EVLineNumberGutter.selectionCursor, mode)
        }
    }

    func testScrolledMarginHitTestingUsesDocumentCoordinatesAndFixedNumberColumn() throws {
        let (_, surface, session) = try makeSurface(Array(repeating: String(repeating: "word ", count: 300), count: 100).joined(separator: "\n"), declarations: [
            (.baseParagraph, .blockMarginLeft, .float(28)),
        ])
        defer { session.detach() }
        surface.performInput {
            try session.setNumber(true)
            _ = try session.setWrap(false)
            _ = try session.setViewportOrigin(left: 100, top: 400)
        }
        let view = surface.editorView
        XCTAssertGreaterThan(surface.viewportState.left, 50)
        XCTAssertGreaterThan(surface.viewportState.top, 100)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let row = try XCTUnwrap(snapshot.rows.first { row in
            let y = view.viewPoint(fromLayoutPoint: CGPoint(x: 0, y: CGFloat(row.y + row.ascent / 2))).y
            return y >= view.textViewportRect.minY && y < view.textViewportRect.maxY
        })
        let y = view.viewPoint(fromLayoutPoint: CGPoint(x: 0, y: CGFloat(row.y + row.ascent / 2))).y
        XCTAssertTrue(view.isRowSelectionPoint(CGPoint(x: 1, y: y)))
        XCTAssertFalse(view.isRowSelectionPoint(CGPoint(x: view.textViewportRect.minX + 1, y: y)))
        XCTAssertFalse(try session.rowSelectionHitTest(view.layoutPoint(fromViewPoint: CGPoint(x: view.textViewportRect.minX + 1, y: y)), in: snapshot.info))
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
