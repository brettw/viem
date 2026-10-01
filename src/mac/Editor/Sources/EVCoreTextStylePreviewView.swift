import AppKit
import CViemCore
import CoreText
import ViemCoreTextProvider

struct EVCoreTextStylePreviewLine: Equatable {
    let origin: CGPoint
    let typographicWidth: CGFloat
    let ascent: CGFloat
    let descent: CGFloat
    let stringRange: NSRange
    let isCurrentStyle: Bool
}

struct EVCoreTextStylePreviewBox: Equatable {
    let rect: CGRect
    let color: EVStyleColor
    let isBackground: Bool
}

struct EVCoreTextStylePreviewInspection: Equatable {
    let kind: EVStyleKind?
    let effectiveValues: [EVStyleProperty: EVStyleValue]
    let requestedFontFamilies: [String]
    let requestedFontSize: CGFloat
    let resolvedFontFamily: String
    let resolvedFontPostScriptName: String
    let resolvedFontSize: CGFloat
    let canvasBackground: EVStyleColor
    let accessibilityText: String
    let lines: [EVCoreTextStylePreviewLine]
    let boxes: [EVCoreTextStylePreviewBox]
    let blockPreviewError: String?

    var currentStyleLines: [EVCoreTextStylePreviewLine] {
        lines.filter(\.isCurrentStyle)
    }
}

/// A small, read-only style specimen that deliberately shares the editor's
/// Core Text shaping path instead of relying on TextKit's private defaults.
/// The value map comes from the committed core cascade, with absent foreground
/// declarations resolved to the current theme for presentation only.
@MainActor
final class EVCoreTextStylePreviewView: NSView {
    private static let contentInset: CGFloat = 12
    private static let paragraphCurrentText =
        "A calm writing surface shaped with the selected style,\u{2028}with line spacing and alignment visible."
    private static let paragraphAccessibilityText =
        "Previous paragraph gives the style context. "
        + paragraphCurrentText.replacingOccurrences(of: "\u{2028}", with: " ")
        + " Following paragraph shows spacing and inheritance."
    private static let characterAccessibilityText =
        "Surrounding text. The quick brown fox writes beautifully. Surrounding text."
    private static let unavailableText =
        "Select a document style to preview its effective formatting."

    private var blockPreview: EVCoreBlockStylePreview?
    private var blockPreviewError: String?
    private var kind: EVStyleKind?
    private var effectiveValues: [EVStyleProperty: EVStyleValue] = [:]
    private var canvasBackground = EVStyleColor(red: 1, green: 1, blue: 1, alpha: 1)
    private var attributedContent = NSAttributedString(string: unavailableText)
    private var currentStyleRange = NSRange(location: 0, length: 0)

    override var isFlipped: Bool { true }
    override var intrinsicContentSize: NSSize { NSSize(width: NSView.noIntrinsicMetric, height: 132) }
    override var acceptsFirstResponder: Bool { false }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        configureAccessibility()
        showUnavailable()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        configureAccessibility()
        showUnavailable()
    }

    func apply(
        kind: EVStyleKind,
        effectiveValues: [EVStyleProperty: EVStyleValue],
        canvasBackground: EVStyleColor = EVStyleColor(red: 1, green: 1, blue: 1, alpha: 1)
    ) {
        self.kind = kind
        self.effectiveValues = effectiveValues
        self.canvasBackground = canvasBackground
        let content = makeAttributedContent(kind: kind, effectiveValues: effectiveValues)
        attributedContent = content.value
        currentStyleRange = content.currentRange
        blockPreviewError = nil
        if kind != .character {
            do {
                let context = contextualForegroundColor().usingColorSpace(.sRGB) ?? .gray
                let foreground = EVStyleColor(red: Float(context.redComponent), green: Float(context.greenComponent),
                    blue: Float(context.blueComponent), alpha: Float(context.alphaComponent))
                if let blockPreview, blockPreview.kind == kind {
                    try blockPreview.update(values: effectiveValues, contextForeground: foreground)
                } else {
                    blockPreview = try EVCoreBlockStylePreview(kind: kind, values: effectiveValues, contextForeground: foreground)
                }
            } catch {
                blockPreview = nil
                blockPreviewError = error.localizedDescription
            }
        } else { blockPreview = nil }
        setAccessibilityValue(blockPreview?.accessibilityText ?? content.accessibilityText)
        needsDisplay = true
    }

    func showUnavailable() {
        kind = nil
        blockPreview = nil
        blockPreviewError = nil
        effectiveValues = [:]
        let attributes = contextualAttributes(size: CoreTextMeasurementProvider.defaultFontSize)
        attributedContent = NSAttributedString(string: Self.unavailableText, attributes: attributes)
        currentStyleRange = NSRange(location: 0, length: 0)
        setAccessibilityValue(Self.unavailableText)
        needsDisplay = true
    }

    func inspection(layoutSize: CGSize? = nil) -> EVCoreTextStylePreviewInspection {
        let size = layoutSize ?? CGSize(
            width: max(bounds.width, 560),
            height: max(bounds.height, intrinsicContentSize.height)
        )
        let requested = requestedFontDescription(from: effectiveValues)
        // Inspect the font installed in the specimen, so tests observe stale
        // preview content instead of resolving a fresh font from its values.
        let font = currentStyleRange.length > 0
            ? attributedContent.attribute(
                NSAttributedString.Key(kCTFontAttributeName as String),
                at: currentStyleRange.location, effectiveRange: nil
            ) as! CTFont
            : makeFont(from: effectiveValues)
        return EVCoreTextStylePreviewInspection(
            kind: kind,
            effectiveValues: effectiveValues,
            requestedFontFamilies: requested.families,
            requestedFontSize: requested.size,
            resolvedFontFamily: resolvedFamilyName(font, requested: requested.families),
            resolvedFontPostScriptName: CTFontCopyPostScriptName(font) as String,
            resolvedFontSize: CTFontGetSize(font),
            canvasBackground: canvasBackground,
            accessibilityText: (accessibilityValue() as? String) ?? "",
            lines: lineGeometry(in: CGRect(origin: .zero, size: size)),
            boxes: blockPreview?.boxes(in: CGRect(origin: .zero, size: size)) ?? [],
            blockPreviewError: blockPreviewError
        )
    }

    override func draw(_ dirtyRect: NSRect) {
        canvasBackground.appKitColor.setFill()
        dirtyRect.fill()
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        if let blockPreview {
            blockPreview.draw(in: bounds, context: context)
            return
        }
        context.saveGState()
        context.textMatrix = .identity
        context.translateBy(x: 0, y: bounds.height)
        context.scaleBy(x: 1, y: -1)
        if kind == .character {
            drawCharacterLine(in: bounds, context: context)
        } else {
            CTFrameDraw(makeParagraphFrame(in: bounds), context)
        }
        context.restoreGState()
    }

    private func configureAccessibility() {
        setAccessibilityElement(true)
        setAccessibilityRole(.staticText)
        setAccessibilityLabel("Live style preview")
        setAccessibilityHelp("Read-only preview of the committed, resolved style.")
    }

    private func makeAttributedContent(
        kind: EVStyleKind,
        effectiveValues: [EVStyleProperty: EVStyleValue]
    ) -> (value: NSAttributedString, currentRange: NSRange, accessibilityText: String) {
        let selectedAttributes = styleAttributes(from: effectiveValues)
        let contextual = contextualAttributes(size: 12)
        let value = NSMutableAttributedString()
        if kind != .character {
            value.append(NSAttributedString(
                string: "Previous paragraph gives the style context.\n",
                attributes: contextual
            ))
            let location = value.length
            let selectedText = Self.paragraphCurrentText + "\n"
            value.append(NSAttributedString(string: selectedText, attributes: selectedAttributes))
            let selectedRange = NSRange(location: location, length: (selectedText as NSString).length)
            value.append(NSAttributedString(
                string: "Following paragraph shows spacing and inheritance.",
                attributes: contextual
            ))
            return (value, selectedRange, Self.paragraphAccessibilityText)
        }

        value.append(NSAttributedString(string: "Surrounding text · ", attributes: contextual))
        let location = value.length
        let selectedText = "The quick brown fox writes beautifully"
        value.append(NSAttributedString(string: selectedText, attributes: selectedAttributes))
        let selectedRange = NSRange(location: location, length: (selectedText as NSString).length)
        value.append(NSAttributedString(string: " · surrounding text", attributes: contextual))
        return (value, selectedRange, Self.characterAccessibilityText)
    }

    private func contextualAttributes(size: CGFloat) -> [NSAttributedString.Key: Any] {
        let font = CTFontCreateUIFontForLanguage(.system, size, nil)
            ?? CTFontCreateWithName("Helvetica" as CFString, size, nil)
        let paragraph = NSMutableParagraphStyle()
        paragraph.paragraphSpacing = 5
        return [
            NSAttributedString.Key(kCTFontAttributeName as String): font,
            NSAttributedString.Key(kCTForegroundColorAttributeName as String):
                contextualForegroundColor().cgColor,
            .paragraphStyle: paragraph,
        ]
    }

    private func contextualForegroundColor() -> NSColor {
        let luminance = 0.2126 * canvasBackground.red
            + 0.7152 * canvasBackground.green
            + 0.0722 * canvasBackground.blue
        return luminance > 0.5
            ? NSColor(calibratedWhite: 0.38, alpha: 1)
            : NSColor(calibratedWhite: 0.72, alpha: 1)
    }

    private func styleAttributes(
        from values: [EVStyleProperty: EVStyleValue]
    ) -> [NSAttributedString.Key: Any] {
        let font = makeFont(from: values)
        var attributes: [NSAttributedString.Key: Any] = [
            NSAttributedString.Key(kCTFontAttributeName as String): font,
            .paragraphStyle: makeParagraphStyle(from: values),
        ]
        let base: UInt32
        if case let .unsigned(value)? = values[.characterWeight] { base = value } else { base = 400 }
        let bold: Bool
        if case let .boolean(value)? = values[.characterBold] { bold = value } else { bold = false }
        let target = bold ? min(base + 300, 1000) : base
        if target >= 500 && UInt32(EVFontCatalog.weight(of: font)) < target {
            attributes[NSAttributedString.Key(kCTStrokeWidthAttributeName as String)] = -3.0
        }
        if case let .color(value)? = values[.characterForeground] {
            attributes[NSAttributedString.Key(kCTForegroundColorAttributeName as String)] =
                value.appKitColor.cgColor
        }
        if case let .color(value)? = values[.characterBackground] {
            attributes[NSAttributedString.Key(kCTBackgroundColorAttributeName as String)] =
                value.appKitColor.cgColor
        }
        if case let .boolean(value)? = values[.characterUnderline], value {
            attributes[NSAttributedString.Key(kCTUnderlineStyleAttributeName as String)] =
                CTUnderlineStyle.single.rawValue
        }
        if case let .boolean(value)? = values[.characterStrikethrough], value {
            attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue
        }
        if case let .string(value)? = values[.characterLanguage], !value.isEmpty {
            attributes[NSAttributedString.Key(kCTLanguageAttributeName as String)] = value
        }
        if case let .writingDirection(value)? = values[.characterDirection],
           value != UInt32(VIEM_TEXT_DIRECTION_AUTO)
        {
            let direction = value == UInt32(VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT)
                ? NSWritingDirection.rightToLeft : .leftToRight
            attributes[.writingDirection] = [
                NSNumber(value: direction.rawValue | NSWritingDirectionFormatType.override.rawValue),
            ]
        }
        if case let .float(value)? = values[.characterLetterSpacing] {
            attributes.merge(letterSpacingAttributes(CGFloat(value))) { _, value in value }
        }
        return attributes
    }

    private func requestedFontDescription(
        from values: [EVStyleProperty: EVStyleValue]
    ) -> (families: [String], size: CGFloat) {
        let families: [String]
        if case let .stringList(value)? = values[.characterFontFamilies], !value.isEmpty {
            families = value
        } else {
            families = [CoreTextMeasurementProvider.defaultFontFamily]
        }
        let size: CGFloat
        if case let .float(value)? = values[.characterSize], value > 0 {
            size = CGFloat(value)
        } else {
            size = CoreTextMeasurementProvider.defaultFontSize
        }
        return (families, size)
    }

    private func makeFont(from values: [EVStyleProperty: EVStyleValue]) -> CTFont {
        let requested = requestedFontDescription(from: values)
        let weight: UInt32
        if case let .unsigned(value)? = values[.characterWeight] { weight = value }
        else { weight = 400 }
        let bold: Bool
        if case let .boolean(value)? = values[.characterBold] { bold = value } else { bold = false }
        let slant: UInt32
        if case let .fontSlant(value)? = values[.characterSlant] { slant = value }
        else { slant = UInt32(VIEM_FONT_SLANT_UPRIGHT) }
        let features: [(String, UInt32)]
        if case let .openTypeFeatures(value)? = values[.characterOpenTypeFeatures] {
            features = value.map { ($0.tag, $0.setting) }
        } else { features = [] }
        return resolveFont(families: requested.families, size: requested.size,
            cssWeight: CGFloat(bold ? min(weight + 300, 1000) : weight),
            slant: slant, features: features, relativeBold: bold,
            axes: { if case let .string(value)? = values[.characterFontAxes] { return EVFontVariations.decode(value) }; return [:] }())

    }

    private func isSystemFontRequest(_ family: String) -> Bool {
        switch family.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "sf pro", "system-ui", "-apple-system": true
        default: false
        }
    }

    private func resolvedFamilyName(_ font: CTFont, requested: [String]) -> String {
        if let first = requested.first,
           isSystemFontRequest(first),
           (CTFontCopyPostScriptName(font) as String).hasPrefix(".SF")
        {
            return CoreTextMeasurementProvider.defaultFontFamily
        }
        return CTFontCopyFamilyName(font) as String
    }

    private func makeParagraphStyle(
        from values: [EVStyleProperty: EVStyleValue]
    ) -> NSParagraphStyle {
        let paragraph = NSMutableParagraphStyle()
        if case let .float(value)? = values[.blockMarginTop] {
            paragraph.paragraphSpacingBefore = CGFloat(value)
        }
        if case let .float(value)? = values[.blockMarginBottom] {
            paragraph.paragraphSpacing = CGFloat(value)
        }
        if case let .float(value)? = values[.paragraphFirstLineIndent] {
            paragraph.firstLineHeadIndent = CGFloat(value)
        }
        if case let .float(value)? = values[.paragraphLeadingIndent] {
            paragraph.headIndent = CGFloat(value)
        }
        if case let .float(value)? = values[.paragraphTrailingIndent] {
            paragraph.tailIndent = -CGFloat(value)
        }

        let direction: NSWritingDirection
        if case let .writingDirection(value)? = values[.paragraphBaseDirection] {
            switch value {
            case UInt32(VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT): direction = .leftToRight
            case UInt32(VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT): direction = .rightToLeft
            default: direction = .natural
            }
        } else {
            direction = .natural
        }
        paragraph.baseWritingDirection = direction

        if case let .paragraphAlignment(value)? = values[.paragraphAlignment] {
            switch value {
            case UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER):
                paragraph.alignment = .center
            case UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_END):
                paragraph.alignment = direction == .rightToLeft ? .left : .right
            default:
                paragraph.alignment = direction == .rightToLeft ? .right : .left
            }
        }
        if case let .lineSpacing(value)? = values[.paragraphLineSpacing] {
            switch value.kind {
            case UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER):
                paragraph.lineHeightMultiple = CGFloat(value.value)
            case UInt32(VIEM_STYLE_LINE_SPACING_AT_LEAST):
                paragraph.minimumLineHeight = CGFloat(value.value)
            case UInt32(VIEM_STYLE_LINE_SPACING_EXACT):
                paragraph.minimumLineHeight = CGFloat(value.value)
                paragraph.maximumLineHeight = CGFloat(value.value)
            default:
                break
            }
        }
        return paragraph
    }

    private func drawCharacterLine(in bounds: CGRect, context: CGContext) {
        let line = CTLineCreateWithAttributedString(attributedContent)
        var ascent: CGFloat = 0
        var descent: CGFloat = 0
        let width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, nil))
        let available = max(0, bounds.width - Self.contentInset * 2)
        let x = Self.contentInset + max(0, (available - width) / 2)
        let y = max(Self.contentInset + descent, (bounds.height + ascent - descent) / 2)
        context.textPosition = CGPoint(x: x, y: y)
        CTLineDraw(line, context)
    }

    private func makeParagraphFrame(in bounds: CGRect) -> CTFrame {
        let insetBounds = bounds.insetBy(dx: Self.contentInset, dy: Self.contentInset)
        let path = CGPath(rect: insetBounds, transform: nil)
        let framesetter = CTFramesetterCreateWithAttributedString(attributedContent)
        return CTFramesetterCreateFrame(
            framesetter,
            CFRange(location: 0, length: attributedContent.length),
            path,
            nil
        )
    }

    private func lineGeometry(in bounds: CGRect) -> [EVCoreTextStylePreviewLine] {
        if let blockPreview { return blockPreview.lines(in: bounds) }
        if kind == .character {
            let line = CTLineCreateWithAttributedString(attributedContent)
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            let width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, nil))
            let available = max(0, bounds.width - Self.contentInset * 2)
            let x = Self.contentInset + max(0, (available - width) / 2)
            let y = max(Self.contentInset + descent, (bounds.height + ascent - descent) / 2)
            return [EVCoreTextStylePreviewLine(
                origin: CGPoint(x: x, y: y),
                typographicWidth: width,
                ascent: ascent,
                descent: descent,
                stringRange: NSRange(location: 0, length: attributedContent.length),
                isCurrentStyle: true
            )]
        }

        let frame = makeParagraphFrame(in: bounds)
        let lines = CTFrameGetLines(frame) as! [CTLine]
        var origins = Array(repeating: CGPoint.zero, count: lines.count)
        if !origins.isEmpty {
            CTFrameGetLineOrigins(frame, CFRange(location: 0, length: 0), &origins)
        }
        return lines.enumerated().map { index, line in
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            let width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, nil))
            let range = CTLineGetStringRange(line)
            let nsRange = NSRange(location: range.location, length: range.length)
            return EVCoreTextStylePreviewLine(
                origin: origins[index],
                typographicWidth: width,
                ascent: ascent,
                descent: descent,
                stringRange: nsRange,
                isCurrentStyle: NSIntersectionRange(nsRange, currentStyleRange).length > 0
            )
        }
    }
}
