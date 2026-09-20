import AppKit
import CoreText
import CViemCore
import ViemAppShell
import ViemCoreTextProvider

extension EVEditorView {
    /// Whitespace is decoration on an exact snapshot. Only ink is scaled and
    /// clipped here; the original text, hit testing and row widths stay intact.
    func drawWhitespaceMarkers(_ snapshot: EVLayoutExport, dirtyRect: NSRect? = nil, in context: CGContext) {
        guard let session = surface?.session else { return }
        let damage = dirtyRect ?? visibleRect
        let markers = snapshot.whitespace.markers.filter { marker in
            guard snapshot.rows.indices.contains(marker.rowIndex), marker.width > 0, marker.height > 0 else { return false }
            let origin = viewPoint(fromLayoutPoint: CGPoint(x: marker.x, y: marker.y))
            return CGRect(origin: origin, size: CGSize(width: marker.width, height: marker.height)).intersects(damage)
        }
        guard !markers.isEmpty else { return }
        let style = snapshot.whitespace.style
        let scale = CGFloat(surface?.zoomScale ?? 1)
        let markerRows = Set(markers.map(\.rowIndex))
        let clustersByRow = Dictionary(grouping: snapshot.clusters.filter { markerRows.contains(Int($0.row_index)) }, by: { Int($0.row_index) })
            .mapValues { $0.sorted { $0.x < $1.x } }
        let paint = surface?.layoutPaint.flatMap { $0.info.identity.isSameLayout(as: snapshot.info.identity) ? $0 : nil }
        var fonts: [WhitespaceFontKey: (CTFont, CoreTextRenderAttributes)] = [:]
        var lines: [WhitespaceLineKey: CTLine] = [:]
        for marker in markers {
            let origin = viewPoint(fromLayoutPoint: CGPoint(x: marker.x, y: marker.y))
            let rect = CGRect(origin: origin, size: CGSize(width: marker.width, height: marker.height))
            let row = snapshot.rows[marker.rowIndex]
            let cluster = Self.whitespaceCluster(at: marker.x, in: clustersByRow[marker.rowIndex] ?? [])
            let fontKey = WhitespaceFontKey(identifier: cluster?.render_run.identifier ?? 0,
                generation: cluster?.render_run.metrics_generation ?? 0,
                fallbackSize: cluster == nil ? CGFloat(row.ascent + row.descent) : 0)
            let font: CTFont
            let textAttributes: CoreTextRenderAttributes
            if let cached = fonts[fontKey] { (font, textAttributes) = cached }
            else {
                let inherited = cluster.flatMap {
                    session.provider.renderRegistry.resolvedFont(identifier: $0.render_run.identifier, metricsGeneration: $0.render_run.metrics_generation)
                } ?? CTFontCreateWithName(CoreTextMeasurementProvider.defaultFontFamily as CFString, CGFloat(row.ascent + row.descent), nil)
                let inheritedAttributes = cluster.flatMap {
                    session.provider.renderRegistry.textAttributes(identifier: $0.render_run.identifier,
                        metricsGeneration: $0.render_run.metrics_generation)
                } ?? CoreTextRenderAttributes(scriptBaseSize: CTFontGetSize(inherited))
                textAttributes = Self.whitespaceTextAttributes(style, inherited: inheritedAttributes, scale: scale)
                font = Self.whitespaceFont(style, inherited: inherited, scale: scale, inheritedAttributes: inheritedAttributes)
                fonts[fontKey] = (font, textAttributes)
            }
            let foreground = style.foreground?.color
                ?? cluster.flatMap { cluster in paint.map { resolvedTextPaint(for: cluster, paint: $0).foreground } }
                ?? EVThemeStore.shared.theme.foreground.color
            let rgba = EVThemeColor(foreground)
            let lineKey = WhitespaceLineKey(font: fontKey, text: marker.text,
                red: rgba.red, green: rgba.green, blue: rgba.blue, alpha: rgba.alpha)
            var attributes: [NSAttributedString.Key: Any] = [
                NSAttributedString.Key(kCTFontAttributeName as String): font,
                NSAttributedString.Key(kCTForegroundColorAttributeName as String): foreground.cgColor,
            ]
            if let value = textAttributes.language { attributes[NSAttributedString.Key(kCTLanguageAttributeName as String)] = value }
            attributes.merge(letterSpacingAttributes(textAttributes.letterSpacing)) { _, value in value }
            if textAttributes.writingDirection != .natural {
                attributes[NSAttributedString.Key(kCTWritingDirectionAttributeName as String)] = [textAttributes.writingDirection.rawValue | NSWritingDirectionFormatType.override.rawValue]
            }
            let line = lines[lineKey] ?? CTLineCreateWithAttributedString(NSAttributedString(string: marker.text, attributes: attributes))
            lines[lineKey] = line
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            let width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, nil))
            let fit = Self.whitespaceInkScale(width: width, ascent: ascent, descent: descent, slot: rect.size)
            let shift = textAttributes.scriptOffset * fit
            context.saveGState()
            context.clip(to: rect)
            if let background = style.background { context.setFillColor(background.color.cgColor); context.fill(rect) }
            let baseline = viewPoint(fromLayoutPoint: CGPoint(x: marker.x, y: CGFloat(row.baseline)))
            // Baseline-aligned for ordinary markers; clamp oversized ink into
            // its original slot without changing any text or caret geometry.
            let fittedBaseline = min(rect.maxY - descent * fit, max(rect.minY + ascent * fit, baseline.y)) - shift
            context.translateBy(x: baseline.x, y: fittedBaseline)
            context.scaleBy(x: fit, y: -fit)
            context.textMatrix = .identity
            context.textPosition = .zero
            CTLineDraw(line, context)
            if style.underline == true || style.strikethrough == true {
                context.setFillColor(foreground.cgColor)
                let thickness = max(1 / max(fit, 0.001), CTFontGetUnderlineThickness(font))
                if style.underline == true {
                    context.fill(CGRect(x: 0, y: CTFontGetUnderlinePosition(font), width: width, height: thickness))
                }
                if style.strikethrough == true {
                    context.fill(CGRect(x: 0, y: CTFontGetXHeight(font) * 0.5, width: width, height: thickness))
                }
            }
            context.restoreGState()
        }
    }

    static func whitespaceInkScale(width: CGFloat, ascent: CGFloat, descent: CGFloat, slot: CGSize) -> CGFloat {
        min(1, slot.width / max(0.001, width), slot.height / max(0.001, ascent + descent))
    }

    static func whitespaceTextAttributes(_ style: EVVisibleWhitespaceStyle,
        inherited: CoreTextRenderAttributes, scale: CGFloat) -> CoreTextRenderAttributes {
        CoreTextRenderAttributes(
            scriptPosition: style.scriptPosition.map { $0 == .normal ? 0 : $0 == .superscript ? 1 : 2 } ?? inherited.scriptPosition,
            scriptBaseSize: style.size.map { CGFloat($0) * scale } ?? inherited.scriptBaseSize,
            letterSpacing: style.letterSpacing.map { CGFloat($0) * scale } ?? inherited.letterSpacing,
            language: style.language ?? inherited.language,
            writingDirection: style.direction.map {
                $0 == .rightToLeft ? .rightToLeft : $0 == .leftToRight ? .leftToRight : .natural
            } ?? inherited.writingDirection)
    }

    /// The marker inherits the contributor whose slot contains its origin.
    /// Rows are sorted by x once, so a long whitespace row stays O(n log n).
    static func whitespaceCluster(at x: CGFloat, in clusters: [ViemPositionedClusterV1]) -> ViemPositionedClusterV1? {
        guard !clusters.isEmpty else { return nil }
        var lower = 0
        var upper = clusters.count
        while lower < upper {
            let middle = lower + (upper - lower) / 2
            if CGFloat(clusters[middle].x) <= x { lower = middle + 1 }
            else { upper = middle }
        }
        if lower == 0 { return clusters.first }
        return clusters[lower - 1]
    }

    static func whitespaceFont(_ style: EVVisibleWhitespaceStyle, inherited: CTFont, scale: CGFloat,
        inheritedAttributes: CoreTextRenderAttributes? = nil) -> CTFont {
        let inheritedSize = inheritedAttributes?.scriptBaseSize ?? CTFontGetSize(inherited)
        let position = style.scriptPosition.map { $0 == .normal ? 0 : $0 == .superscript ? 1 : 2 }
            ?? Int(inheritedAttributes?.scriptPosition ?? 0)
        let size = (style.size.map { CGFloat($0) * scale } ?? inheritedSize) * (position == 0 ? 1 : 0.7)
        let descriptor = CTFontCopyFontDescriptor(inherited)
        if style.fontFamilies == nil, style.weight == nil, style.bold == nil, style.slant == nil {
            let overrides = style.openTypeFeatures.map { features in
                CTFontDescriptorCreateWithAttributes([
                    kCTFontFeatureSettingsAttribute: features.filter { $0.key != "kern" }.sorted { $0.key < $1.key }.map {
                        [kCTFontOpenTypeFeatureTag as String: $0.key, kCTFontOpenTypeFeatureValue as String: $0.value] as [String: Any]
                    }
                ] as CFDictionary)
            }
            return CTFontCreateCopyWithAttributes(inherited, size, nil, overrides)
        }
        let inheritedSettings = CTFontDescriptorCopyAttribute(descriptor, kCTFontFeatureSettingsAttribute) as? [[String: Any]] ?? []
        let inheritedFeatures: [(String, UInt32)] = inheritedSettings.compactMap {
            guard let tag = $0[kCTFontOpenTypeFeatureTag as String] as? String,
                  let value = $0[kCTFontOpenTypeFeatureValue as String] as? NSNumber else { return nil }
            return (tag, value.uint32Value)
        }
        let features = style.openTypeFeatures.map { $0.sorted { $0.key < $1.key }.map { ($0.key, $0.value) } } ?? inheritedFeatures
        let inheritedTraits = CTFontGetSymbolicTraits(inherited)
        let weight = style.weight.map(CGFloat.init)
            ?? (style.bold == false && inheritedTraits.contains(.traitBold) ? 400 : CGFloat(EVFontCatalog.weight(of: inherited)))
        let slant: UInt32 = style.slant.map { $0 == .upright ? 0 : $0 == .italic ? 1 : 2 }
            ?? (inheritedTraits.contains(.traitItalic) ? 1 : 0)
        return resolveFont(families: style.fontFamilies ?? [CTFontCopyFamilyName(inherited) as String],
            size: size, cssWeight: weight, slant: slant, features: features, relativeBold: style.bold == true)
    }

}

private struct WhitespaceFontKey: Hashable {
    let identifier: UInt64
    let generation: UInt64
    let fallbackSize: CGFloat
}

private struct WhitespaceLineKey: Hashable {
    let font: WhitespaceFontKey
    let text: String
    let red: Double
    let green: Double
    let blue: Double
    let alpha: Double
}
