import AppKit
import CoreText
import CViemCore
import ViemAppShell
import ViemCoreTextProvider

struct EVLineNumberBand {
    let line: UInt64
    let top: CGFloat
    let bottom: CGFloat
    var isCurrent = false
}

@MainActor
extension EVCoreViewSession {
    func number() throws -> Bool {
        var value: UInt32 = 0
        try lineNumberCheck(viem_core_view_number(document.core, viewID, &value))
        return value != 0
    }
    func setNumber(_ enabled: Bool) throws {
        try lineNumberCheck(viem_core_view_set_number(document.core, viewID, enabled ? 1 : 0))
    }
    func lineNumberInfo() throws -> ViemLineNumberSnapshotInfoV1 {
        var info = ViemLineNumberSnapshotInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemLineNumberSnapshotInfoV1>.size)
        try lineNumberCheck(viem_core_view_line_number_snapshot_info(document.core, viewID, &info))
        return info
    }
    func lineNumberBands(_ info: ViemLineNumberSnapshotInfoV1) throws -> [EVLineNumberBand] {
        guard let count = Int(exactly: info.region_count), count <= 1_000_000 else {
            throw EVCoreFrontendError.core(operation: "Read line numbers", status: UInt32(VIEM_STATUS_INVALID_ARGUMENT))
        }
        var rows = Array(repeating: ViemLineNumberRegionV1(), count: count)
        var identity = info.identity, viewport = info.viewport, copied: UInt64 = 0
        let status = rows.withUnsafeMutableBufferPointer { buffer in
            viem_core_view_copy_line_number_regions(document.core, viewID, &identity, &viewport,
                buffer.baseAddress, UInt64(buffer.count), &copied)
        }
        try lineNumberCheck(status)
        guard copied == info.region_count else {
            throw EVCoreFrontendError.core(operation: "Read line numbers", status: UInt32(VIEM_STATUS_STALE_REVISION))
        }
        return rows.map { EVLineNumberBand(line: $0.source_line, top: CGFloat($0.top), bottom: CGFloat($0.bottom),
            isCurrent: $0.flags & UInt32(VIEM_LINE_NUMBER_REGION_CURRENT) != 0) }
    }
    private func lineNumberCheck(_ status: UInt32) throws {
        guard status == UInt32(VIEM_STATUS_OK) else {
            throw EVCoreFrontendError.core(operation: "Read line numbers", status: status)
        }
    }
}

/// Width changes reuse digit advances until the font or metrics change.
/// Cursor changes never measure digits or resize the gutter.
@MainActor
final class EVLineNumberGutter: NSView {
    private struct WidthKey: Equatable {
        let digits: Int
        let styleRevision: UInt64
        let documentID: UInt64
        let metricsGeneration: UInt64
        let scale: CGFloat
        let themeForeground: EVThemeColor
    }
    private struct NumeralKey: Equatable {
        let font: NSFont
        let letterSpacing: CGFloat
        let documentID: UInt64
        let metricsGeneration: UInt64
    }
    private struct NumeralMeasurement {
        let key: NumeralKey
        let widest: CGFloat
    }
    private var widthKey: WidthKey?
    private var normalMeasurement: NumeralMeasurement?
    private var currentMeasurement: NumeralMeasurement?
    private(set) var columnWidth: CGFloat = 0
    private(set) var digitMeasurementCount = 0
    private(set) var normal = EVChromeStyle()
    private(set) var current = EVChromeStyle()
    private(set) var bands: [EVLineNumberBand] = []
    private var viewportTop: CGFloat = 0

    override var isFlipped: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    static func digits(for lineCount: UInt64) -> Int {
        var value = max(1, lineCount), digits = 1
        while value >= 10 { value /= 10; digits += 1 }
        return digits
    }

    func width(lineCount: UInt64, sheet: EVStyleSheetSnapshot, scale: CGFloat,
               metricsGeneration: UInt64, themeForeground: EVThemeColor? = nil) -> CGFloat {
        let foreground = themeForeground ?? EVThemeStore.shared.theme.foreground
        let key = WidthKey(digits: Self.digits(for: lineCount), styleRevision: sheet.identity.styleSheetRevision,
            documentID: sheet.identity.documentID, metricsGeneration: metricsGeneration, scale: scale,
            themeForeground: foreground)
        if key == widthKey { return columnWidth }
        let normalDefinition = sheet.lineNumberDefinition(current: false)
        normal = Self.style(normalDefinition, scale: scale, themeForeground: foreground.color)
        current = Self.style(sheet.lineNumberDefinition(current: true) ?? normalDefinition,
            scale: scale, themeForeground: foreground.color)
        func measuredWidth(_ style: EVChromeStyle, cache: inout NumeralMeasurement?,
                           shared: NumeralMeasurement? = nil) -> CGFloat {
            let numeralKey = NumeralKey(font: style.font, letterSpacing: style.letterSpacing,
                documentID: key.documentID, metricsGeneration: key.metricsGeneration)
            if cache?.key != numeralKey {
                if shared?.key == numeralKey {
                    cache = shared
                } else {
                    let widest = (0...9).map { number -> CGFloat in
                        let line = Self.textLine(String(number), style: style)
                        return CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
                    }.max() ?? 0
                    cache = NumeralMeasurement(key: numeralKey, widest: widest)
                    digitMeasurementCount += 10
                }
            }
            let widest = cache?.widest ?? 0
            return max(0, widest * CGFloat(key.digits)
                + style.margin.left + style.margin.right + style.padding.left + style.padding.right
                + style.leftBorder.width + style.rightBorder.width)
        }
        let normalWidth = measuredWidth(normal, cache: &normalMeasurement)
        let currentWidth = measuredWidth(current, cache: &currentMeasurement, shared: normalMeasurement)
        columnWidth = ceil(max(normalWidth, currentWidth))
        widthKey = key
        return columnWidth
    }

    func present(bands: [EVLineNumberBand], viewportTop: CGFloat) {
        self.bands = bands; self.viewportTop = viewportTop
        needsDisplay = true
    }

    /// Full physical-line bounds are retained even when only their edge is on
    /// screen. Wrapping and clipping cannot recenter a number while scrolling.
    func numberLayout(_ band: EVLineNumberBand) -> (line: CTLine, rect: NSRect, baseline: CGPoint)? {
        let style = band.isCurrent ? current : normal
        let box = numberBox(band, style: style)
        let content = NSRect(x: box.minX + style.leftBorder.width + style.padding.left,
            y: box.minY + style.topBorder.width + style.padding.top,
            width: max(0, box.width - style.leftBorder.width - style.rightBorder.width - style.padding.left - style.padding.right),
            height: max(0, box.height - style.topBorder.width - style.bottomBorder.width - style.padding.top - style.padding.bottom))
        guard content.height > 0, content.width > 0 else { return nil }
        var line = Self.textLine(String(band.line), style: style)
        var ascent: CGFloat = 0, descent: CGFloat = 0, leading: CGFloat = 0
        var width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, &leading))
        let height = ascent + descent + leading
        if height > content.height {
            var smaller = style
            smaller.font = NSFont(descriptor: style.font.fontDescriptor,
                size: style.font.pointSize * content.height / height) ?? style.font
            smaller.letterSpacing *= content.height / height
            line = Self.textLine(String(band.line), style: smaller)
            width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, &leading))
        }
        let x: CGFloat
        switch style.alignment {
        case .center: x = content.midX - width / 2
        case .left: x = content.minX
        default: x = content.maxX - width
        }
        return (line, box, CGPoint(x: x, y: content.midY + (ascent - descent) / 2))
    }

    private func numberBox(_ band: EVLineNumberBand, style: EVChromeStyle) -> NSRect {
        NSRect(x: style.margin.left, y: band.top - viewportTop + style.margin.top,
            width: max(0, columnWidth - style.margin.left - style.margin.right),
            height: max(0, band.bottom - band.top - style.margin.top - style.margin.bottom))
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        context.saveGState(); defer { context.restoreGState() }
        context.clip(to: paintRect(dirtyRect.intersection(bounds), context: context, coverCanvasEdges: true))
        // Paint the ordinary box through the gaps between line boxes. Excluding
        // those boxes avoids applying translucent backgrounds and borders twice.
        let rawBase = NSRect(x: normal.margin.left, y: bounds.minY,
            width: max(0, bounds.width - normal.margin.left - normal.margin.right), height: bounds.height)
        let base = paintRect(rawBase, context: context, coverCanvasEdges: true)
        context.saveGState()
        context.setShouldAntialias(false)
        context.clip(to: base)
        for band in bands {
            let style = band.isCurrent ? current : normal
            let hole = paintRect(numberBox(band, style: style), context: context, coverCanvasEdges: true).intersection(base)
            if !hole.isNull, !hole.isEmpty {
                // Subtract separately: overlapping boxes from negative margins
                // must not cancel each other's holes in an even-odd path.
                context.addRect(base)
                context.addRect(hole)
                context.clip(using: .evenOdd)
            }
        }
        if let background = normal.background { background.setFill(); base.fill() }
        drawBorders(normal, rawBox: rawBase, box: base, context: context, verticalOnly: true)
        context.restoreGState()
        for band in bands {
            let style = band.isCurrent ? current : normal
            let rawBox = numberBox(band, style: style)
            let box = paintRect(rawBox, context: context, coverCanvasEdges: true)
            guard box.intersects(dirtyRect) else { continue }
            context.saveGState()
            context.clip(to: box)
            context.saveGState()
            context.setShouldAntialias(false)
            if let background = style.background { background.setFill(); box.fill() }
            drawBorders(style, rawBox: rawBox, box: box, context: context)
            context.restoreGState()
            if let layout = numberLayout(band) {
                context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
                context.textPosition = layout.baseline
                CTLineDraw(layout.line, context)
            }
            context.restoreGState()
        }
    }

    /// Quantize shared paint edges in device space, including fractional view
    /// origins and backing scales. Typography retains its unrounded geometry.
    private func paintRect(_ rect: NSRect, context: CGContext, coverCanvasEdges: Bool = false) -> NSRect {
        guard !rect.isNull else { return .null }
        guard !rect.isEmpty else { return NSRect(origin: rect.origin, size: .zero) }
        let pixels = context.convertToDeviceSpace(rect)
        let canvas = context.convertToDeviceSpace(bounds)
        let left = coverCanvasEdges && pixels.minX <= canvas.minX ? floor(pixels.minX) : pixels.minX.rounded()
        let right = coverCanvasEdges && pixels.maxX >= canvas.maxX ? ceil(pixels.maxX) : pixels.maxX.rounded()
        let top = coverCanvasEdges && pixels.minY <= canvas.minY ? floor(pixels.minY) : pixels.minY.rounded()
        let bottom = coverCanvasEdges && pixels.maxY >= canvas.maxY ? ceil(pixels.maxY) : pixels.maxY.rounded()
        return context.convertToUserSpace(CGRect(x: left, y: top,
            width: max(0, right - left), height: max(0, bottom - top)))
    }

    private func drawBorders(_ style: EVChromeStyle, rawBox: NSRect, box: NSRect, context: CGContext, verticalOnly: Bool = false) {
        let leftEnd = style.leftBorder.width == 0 ? box.minX : min(box.maxX, max(box.minX, paintRect(NSRect(x: rawBox.minX, y: rawBox.minY,
            width: style.leftBorder.width, height: rawBox.height), context: context).maxX))
        let rightStart = style.rightBorder.width == 0 ? box.maxX : min(box.maxX, max(leftEnd, paintRect(NSRect(x: rawBox.maxX - style.rightBorder.width, y: rawBox.minY,
            width: style.rightBorder.width, height: rawBox.height), context: context).minX))
        let topEnd = verticalOnly || style.topBorder.width == 0 ? box.minY : min(box.maxY, max(box.minY, paintRect(NSRect(x: rawBox.minX, y: rawBox.minY,
            width: rawBox.width, height: style.topBorder.width), context: context).maxY))
        let bottomStart = verticalOnly || style.bottomBorder.width == 0 ? box.maxY : min(box.maxY, max(topEnd, paintRect(NSRect(x: rawBox.minX, y: rawBox.maxY - style.bottomBorder.width,
            width: rawBox.width, height: style.bottomBorder.width), context: context).minY))
        // Vertical edges own their corners; a translucent border is painted
        // once even where two edges meet or a very short box clips them.
        let edges = [(style.leftBorder, NSRect(x: box.minX, y: box.minY, width: leftEnd - box.minX, height: box.height)),
            (style.rightBorder, NSRect(x: rightStart, y: box.minY, width: box.maxX - rightStart, height: box.height))]
            + (verticalOnly ? [] : [(style.topBorder, NSRect(x: leftEnd, y: box.minY, width: rightStart - leftEnd, height: topEnd - box.minY)),
                (style.bottomBorder, NSRect(x: leftEnd, y: bottomStart, width: rightStart - leftEnd, height: box.maxY - bottomStart))])
        for (border, rect) in edges where border.width > 0 {
            context.setFillColor((border.color ?? style.foreground).cgColor)
            context.fill(paintRect(rect, context: context))
        }
    }

    private static func textLine(_ text: String, style: EVChromeStyle) -> CTLine {
        var attributes: [NSAttributedString.Key: Any] = [.font: style.font, .foregroundColor: style.foreground, .ligature: 1]
        attributes.merge(letterSpacingAttributes(style.letterSpacing)) { _, value in value }
        return CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: attributes))
    }

    private static func style(_ definition: EVStyleDefinition?, scale: CGFloat, themeForeground: NSColor) -> EVChromeStyle {
        let values = definition?.properties.compactMapValues(\.effective) ?? [:]
        func number(_ property: EVStyleProperty, _ fallback: CGFloat = 0) -> CGFloat {
            if case let .float(value)? = values[property] { return CGFloat(value) * scale }
            return fallback * scale
        }
        func color(_ property: EVStyleProperty) -> NSColor? {
            if definition?.properties[property]?.usesThemeDefault == true
                || definition?.properties[property]?.usesTextColor == true { return nil }
            guard case let .color(value)? = values[property] else { return nil }
            return NSColor(srgbRed: CGFloat(value.red), green: CGFloat(value.green), blue: CGFloat(value.blue), alpha: CGFloat(value.alpha))
        }
        let families: [String]
        if case let .stringList(value)? = values[.characterFontFamilies] { families = value } else { families = ["system-ui"] }
        let weight: UInt32
        if case let .unsigned(value)? = values[.characterWeight] { weight = value } else { weight = 400 }
        let bold: Bool
        if case let .boolean(value)? = values[.characterBold] { bold = value } else { bold = false }
        let slant: UInt32
        if case let .fontSlant(value)? = values[.characterSlant] { slant = value } else { slant = 0 }
        let features: [(String, UInt32)]
        if case let .openTypeFeatures(value)? = values[.characterOpenTypeFeatures] { features = value.map { ($0.tag, $0.setting) } } else { features = [] }
        let axes: [String: Double]
        if case let .string(value)? = values[.characterFontAxes] { axes = EVFontVariations.decode(value) } else { axes = [:] }
        let face: String
        if case let .string(value)? = values[.characterFontFace] { face = value } else { face = "" }
        let font = resolveFont(families: families, size: number(.characterSize, 11),
            cssWeight: CGFloat(bold ? min(1000, weight + 300) : weight), slant: slant,
            features: features, relativeBold: bold, axes: axes, faceName: face) as NSFont
        let alignment: NSTextAlignment
        let rtl: Bool
        if case let .writingDirection(value)? = values[.paragraphBaseDirection] {
            rtl = value == UInt32(VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT)
        } else { rtl = false }
        if case let .paragraphAlignment(value)? = values[.paragraphAlignment] {
            alignment = value == UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER) ? .center
                : (value == UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_START)) != rtl ? .left : .right
        } else { alignment = .right }
        func border(_ width: EVStyleProperty, _ paint: EVStyleProperty) -> EVChromeBorder {
            EVChromeBorder(width: number(width), color: color(paint))
        }
        return EVChromeStyle(font: font, foreground: color(.characterForeground) ?? themeForeground,
            background: color(.blockBackground),
            margin: NSEdgeInsets(top: number(.blockMarginTop), left: number(.blockMarginLeft),
                bottom: number(.blockMarginBottom), right: number(.blockMarginRight)),
            padding: NSEdgeInsets(top: number(.blockPaddingTop), left: number(.blockPaddingLeft),
                bottom: number(.blockPaddingBottom), right: number(.blockPaddingRight)),
            topBorder: border(.blockBorderTopWidth, .blockBorderTopColor),
            rightBorder: border(.blockBorderRightWidth, .blockBorderRightColor),
            bottomBorder: border(.blockBorderBottomWidth, .blockBorderBottomColor),
            leftBorder: border(.blockBorderLeftWidth, .blockBorderLeftColor), alignment: alignment,
            letterSpacing: number(.characterLetterSpacing))
    }
}
