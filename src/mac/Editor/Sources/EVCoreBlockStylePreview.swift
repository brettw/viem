import AppKit
import CViemCore

/// The block specimen is an isolated Markdown document. The ordinary Rust cascade,
/// box layout and Core Text provider produce both its geometry and its paint;
/// the style dialog does not implement a second margin or padding algorithm.
@MainActor
final class EVCoreBlockStylePreview {
    let kind: EVStyleKind
    private let backend: EVCoreDocumentBackend
    private let session: EVCoreViewSession
    private let selectedRange: NSRange
    private let text: String
    private let supportedProperties: Set<EVStyleProperty>
    private let selectedStyleKey: EVStyleKey
    private var appliedValues: [EVStyleProperty: EVStyleValue]?
    private var contextForeground: EVStyleColor?
    private var size: CGSize = .zero
    private var metricsGeneration: UInt64 = 0
    private var snapshot: EVLayoutExport?
    private var paint: EVLayoutPaintExport?
    var accessibilityText: String { text.replacingOccurrences(of: "\n", with: " ") }

    init(kind: EVStyleKind, values: [EVStyleProperty: EVStyleValue], contextForeground: EVStyleColor) throws {
        self.kind = kind
        let source: String
        let target: String
        let selected: String
        switch kind {
        case .quote:
            target = "Block quote"
            selected = "A quotation contains a paragraph.\nAnother paragraph shares its border.\nA nested quotation has its own box."
            source = " > A quotation contains a paragraph.\n >\n > Another paragraph shares its border.\n >\n >> A nested quotation has its own box."
        case .codeBlock:
            target = "Code Block"
            selected = "A literal code block\n\nkeeps its lines and spaces."
            source = "```\nA literal code block\n\nkeeps its lines and spaces.\n```"
        case .table:
            target = "Table"
            selected = "Header\nDetail\nOne\nTwo"
            source = "| Header | Detail |\n| --- | --- |\n| One | Two |"
        case .list, .listItem:
            target = kind == .list ? "Bulleted List" : "List item"
            selected = "A list item contains a paragraph.\nAnd a second paragraph.\nAnother item."
            source = "- A list item contains a paragraph.\n\n  And a second paragraph.\n\n- Another item."
        default:
            target = "Heading1"
            selected = "A calm writing surface shaped with the selected style, with line spacing and alignment visible."
            source = "Previous paragraph gives the style context.\n\n# A calm writing surface shaped with the selected style, with line spacing and alignment visible.\n\nFollowing paragraph shows spacing and inheritance."
        }
        backend = try EVCoreDocumentBackend(stylePreviewMarkdown: source)
        session = try EVCoreViewSession(document: backend, width: 560, height: 500)
        let sheet = try backend.styleSheetSnapshot()
        let selectedDefinition = sheet.definitions.first { $0.key.id.rawValue == target && $0.kind == kind }
            ?? sheet.definitions.first { $0.kind == kind }
        guard let selectedDefinition else { throw EVStyleBridgeError.malformedSnapshot("Missing block specimen style") }
        selectedStyleKey = selectedDefinition.key
        supportedProperties = Set(selectedDefinition.properties.keys)
        text = try backend.formattedText()
        selectedRange = (text as NSString).range(of: selected)
        try update(values: values, contextForeground: contextForeground)
    }

    func update(values: [EVStyleProperty: EVStyleValue], contextForeground: EVStyleColor) throws {
        if appliedValues == values && self.contextForeground == contextForeground { return }
        if self.contextForeground != contextForeground {
            let identity = try EVCoreStyleBridge.styleIdentity(core: backend.core)
            _ = try session.editStyle(key: .baseParagraph, expected: identity,
                mutation: .setDeclaration(.characterForeground, .color(contextForeground)))
            self.contextForeground = contextForeground
        }
        for property in EVStyleProperty.characterProperties + EVStyleProperty.paragraphProperties + EVStyleProperty.blockProperties {
            guard supportedProperties.contains(property),
                  appliedValues == nil || appliedValues?[property] != values[property] else { continue }
            let identity = try EVCoreStyleBridge.styleIdentity(core: backend.core)
            let mutation: EVStyleMutation = values[property].map { .setDeclaration(property, $0) } ?? .clearDeclaration(property)
            _ = try session.editStyle(key: selectedStyleKey, expected: identity, mutation: mutation)
        }
        appliedValues = values
        snapshot = nil
        paint = nil
    }

    private func prepare(_ requested: CGSize) throws -> EVLayoutExport {
        if snapshot == nil || size != requested || metricsGeneration != session.provider.metricsGeneration {
            _ = try session.resize(width: max(1, requested.width), height: max(1, requested.height))
            let fresh = try session.layoutExport()
            let freshPaint = try session.layoutPaintExport()
            snapshot = fresh
            paint = freshPaint
            size = requested
            metricsGeneration = session.provider.metricsGeneration
        }
        return snapshot!
    }

    func lines(in bounds: CGRect) -> [EVCoreTextStylePreviewLine] {
        guard let snapshot = try? prepare(bounds.size) else { return [] }
        let bytes = Array(text.utf8)
        return snapshot.rows.map { row in
            let start = min(Int(row.text_start), bytes.count), end = min(Int(row.text_end), bytes.count)
            let location = String(decoding: bytes[..<start], as: UTF8.self).utf16.count
            let length = String(decoding: bytes[start..<end], as: UTF8.self).utf16.count
            let range = NSRange(location: location, length: length)
            return EVCoreTextStylePreviewLine(origin: CGPoint(x: CGFloat(snapshot.clusters.first { $0.row_index == row.row_index }?.x ?? row.paragraph_content_x), y: bounds.height - CGFloat(row.baseline)),
                typographicWidth: CGFloat(row.width), ascent: CGFloat(row.ascent), descent: CGFloat(row.descent),
                stringRange: range, isCurrentStyle: selectedRange.location != NSNotFound && NSIntersectionRange(range, selectedRange).length > 0)
        }
    }

    func boxes(in bounds: CGRect) -> [EVCoreTextStylePreviewBox] {
        guard let snapshot = try? prepare(bounds.size) else { return [] }
        return snapshot.decorations.compactMap { decoration in
            let background = decoration.flags & UInt32(VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND) != 0
            let border = decoration.flags & UInt32(VIEM_LAYOUT_DECORATION_BLOCK_BORDER | VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER) != 0
            guard background || border else { return nil }
            return EVCoreTextStylePreviewBox(rect: rect(decoration.typographic_bounds),
                color: EVStyleColor(red: decoration.paint.foreground.red, green: decoration.paint.foreground.green,
                    blue: decoration.paint.foreground.blue, alpha: decoration.paint.foreground.alpha), isBackground: background)
        }
    }

    func draw(in bounds: CGRect, context: CGContext) {
        guard let snapshot = try? prepare(bounds.size), let paint,
              let viewport = try? session.viewportState(),
              viewport.configuration_generation == snapshot.info.identity.configuration_generation,
              viewport.layout_revision == snapshot.info.identity.layout_revision else { return }
        // Core exports complete boxes in owner order. Keep backgrounds and
        // borders together so an overlapping child paints above its parent.
        context.saveGState()
        context.setShouldAntialias(false)
        context.setBlendMode(.normal)
        for item in snapshot.decorations where item.flags & UInt32(VIEM_LAYOUT_DECORATION_BLOCK_BACKGROUND | VIEM_LAYOUT_DECORATION_BLOCK_BORDER | VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER) != 0 {
            EVBlockBoxDrawing.fill(rect(item.typographic_bounds), color: color(item.paint.foreground).cgColor, in: context)
        }
        context.restoreGState()
        for cluster in snapshot.clusters {
            let style = paint.runs.first { $0.text_start <= cluster.text_start && cluster.text_start < $0.text_end }?.paint ?? paint.info.default_paint
            if style.flags & UInt32(VIEM_TEXT_PAINT_HAS_BACKGROUND) != 0 {
                color(style.background).setFill(); rect(cluster.typographic_bounds).fill()
            }
        }
        for cluster in snapshot.clusters {
            guard let row = snapshot.rows.first(where: { $0.row_index == cluster.row_index }) else { continue }
            let style = paint.runs.first { $0.text_start <= cluster.text_start && cluster.text_start < $0.text_end }?.paint ?? paint.info.default_paint
            _ = session.provider.renderRegistry.draw(identifier: cluster.render_run.identifier,
                metricsGeneration: cluster.render_run.metrics_generation,
                atBaseline: CGPoint(x: CGFloat(cluster.x), y: CGFloat(row.baseline)),
                color: color(style.foreground).cgColor, in: context)
            color(style.foreground).setFill()
            let baseline = CGFloat(row.baseline) - CGFloat(style.baseline_offset) * CGFloat(viewport.scale)
            let ascent = max(0, baseline - CGFloat(cluster.typographic_bounds.y))
            let descent = max(0, CGFloat(cluster.typographic_bounds.y + cluster.typographic_bounds.height) - baseline)
            if style.flags & UInt32(VIEM_TEXT_PAINT_UNDERLINE) != 0 {
                CGRect(x: CGFloat(cluster.typographic_bounds.x),
                    y: floor(baseline + max(1, descent * 0.35)),
                    width: CGFloat(cluster.typographic_bounds.width), height: 1).fill()
            }
            if style.flags & UInt32(VIEM_TEXT_PAINT_STRIKETHROUGH) != 0 {
                CGRect(x: CGFloat(cluster.typographic_bounds.x), y: floor(baseline - ascent * 0.32),
                    width: CGFloat(cluster.typographic_bounds.width), height: 1).fill()
            }
        }
        for item in snapshot.decorations where item.flags & UInt32(VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN) != 0 {
            guard let row = snapshot.rows.first(where: { $0.row_index == item.row_index }) else { continue }
            _ = session.provider.renderRegistry.draw(identifier: item.render_run.identifier,
                metricsGeneration: item.render_run.metrics_generation,
                atBaseline: CGPoint(x: CGFloat(item.x), y: CGFloat(row.baseline)),
                color: color(item.paint.foreground).cgColor, in: context)
        }
    }

    private func rect(_ value: ViemLayoutRectV1) -> CGRect {
        CGRect(x: CGFloat(value.x), y: CGFloat(value.y), width: CGFloat(value.width), height: CGFloat(value.height))
    }
    private func color(_ value: ViemRgbaV1) -> NSColor {
        NSColor(srgbRed: CGFloat(value.red), green: CGFloat(value.green), blue: CGFloat(value.blue), alpha: CGFloat(value.alpha))
    }
}
