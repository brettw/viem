import AppKit

/// Original 24-point SVG drawings share one optical grid and stroke weight.
@MainActor
enum EVStyleIcons {
    enum StyleType: Hashable, Sendable { case paragraph, character, block, builtin }
    private static var typeImages: [StyleType: NSImage] = [:]
    private static var scriptImages: [Bool: NSImage] = [:]

    static func scriptImage(raised: Bool) -> NSImage {
        if let image = scriptImages[raised] { return image }
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: false) { _ in
            let fallback = NSFont.systemFont(ofSize: 11.7)
            let serif = fallback.fontDescriptor.withDesign(.serif).flatMap { NSFont(descriptor: $0, size: 11.7) } ?? fallback
            let font = NSFont(name: "TimesNewRomanPS-ItalicMT", size: 11.7)
                ?? NSFont(name: "Georgia-Italic", size: 11.7)
                ?? NSFontManager.shared.convert(serif, toHaveTrait: .italicFontMask)
            ("x" as NSString).draw(at: NSPoint(x: 3, y: 2), withAttributes: [.font: font, .foregroundColor: NSColor.black])
            ("2" as NSString).draw(at: NSPoint(x: 10.5, y: raised ? 8.5 : 0.5),
                withAttributes: [.font: NSFont.systemFont(ofSize: 7), .foregroundColor: NSColor.black])
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = raised ? "Superscript" : "Subscript"
        scriptImages[raised] = image
        return image
    }

    static func type(for definition: EVStyleDefinition) -> StyleType {
        if definition.flags.contains(.internalList) || definition.kind == .list { return .paragraph }
        if definition.flags.contains(.internalSyntax)
            || definition.origin == .syntheticReadOnly { return .builtin }
        return definition.kind == .paragraph ? .paragraph : definition.kind == .character ? .character : .block
    }

    static func typeImage(for definition: EVStyleDefinition) -> NSImage { typeImage(type(for: definition)) }

    static func typeImage(_ type: StyleType) -> NSImage {
        if let image = typeImages[type] { return image }
        let image: NSImage
        if type == .builtin {
            image = NSImage(systemSymbolName: "gearshape", accessibilityDescription: "Built-in style")
                ?? NSImage(size: NSSize(width: 16, height: 16))
        } else {
            image = NSImage(size: NSSize(width: 16, height: 16), flipped: false) { _ in
                if type == .block {
                    NSColor.black.setStroke()
                    let path = NSBezierPath()
                    for (x, y, dx, dy) in [(2.0, 2.0, 1.0, 1.0), (14.0, 2.0, -1.0, 1.0),
                                           (2.0, 14.0, 1.0, -1.0), (14.0, 14.0, -1.0, -1.0)] {
                        path.move(to: NSPoint(x: x + 4 * dx, y: y))
                        path.line(to: NSPoint(x: x, y: y))
                        path.line(to: NSPoint(x: x, y: y + 4 * dy))
                    }
                    path.lineWidth = 1.3
                    path.stroke()
                } else {
                    let fallback = NSFont.systemFont(ofSize: 16, weight: type == .character ? .bold : .regular)
                    let font = NSFont(name: type == .character ? "TimesNewRomanPS-BoldMT" : "TimesNewRomanPSMT", size: 16)
                        ?? NSFont(name: type == .character ? "Georgia-Bold" : "Georgia", size: 16)
                        ?? fallback.fontDescriptor.withDesign(.serif).flatMap { NSFont(descriptor: $0, size: 16) }
                        ?? fallback
                    let text = type == .paragraph ? "¶" : "a"
                    let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor.black]
                    let size = (text as NSString).size(withAttributes: attributes)
                    (text as NSString).draw(at: NSPoint(x: (16 - size.width) / 2, y: (16 - size.height) / 2), withAttributes: attributes)
                }
                return true
            }
        }
        image.size = NSSize(width: 16, height: 16)
        image.isTemplate = true
        image.accessibilityDescription = type == .paragraph ? "Paragraph style" : type == .character ? "Character style" : type == .block ? "Block style" : "Built-in style"
        typeImages[type] = image
        return image
    }

    enum Symbol: String { case alignStart, alignCenter, alignEnd, before, after, firstIndent, leadingIndent, trailingIndent, tracking, features, lineSpacing }
    static func image(_ symbol: Symbol) -> NSImage {
        let strokes: String
        switch symbol {
        case .alignStart: strokes = "M4 5h16M4 10h10M4 15h16M4 20h10"
        case .alignCenter: strokes = "M4 5h16M7 10h10M4 15h16M7 20h10"
        case .alignEnd: strokes = "M4 5h16M10 10h10M4 15h16M10 20h10"
        case .before: strokes = "M4 12h16M4 16h12M4 20h16M12 3v6M9 6l3 3 3-3"
        case .after: strokes = "M4 4h16M4 8h12M4 12h16M12 15v6M9 18l3 3 3-3"
        case .firstIndent: strokes = "M11 5h9M4 10h16M4 15h16M4 20h16M3 2l4 3-4 3"
        case .leadingIndent: strokes = "M10 5h10M10 10h7M10 15h10M10 20h7M2 12h5M4 9l3 3-3 3"
        case .trailingIndent: strokes = "M4 5h10M7 10h7M4 15h10M7 20h7M22 12h-5M20 9l-3 3 3 3"
        case .tracking: strokes = "M3 15l4-11 4 11M5 11h4M13 15l4-11 4 11M15 11h4M3 20h18M6 18l-3 2 3 2M18 18l3 2-3 2"
        case .features: strokes = "M7 20V7c0-5 6-5 6-2M4 10h9M16 10v10M16 5v.2"
        case .lineSpacing: strokes = "M10 5h11M10 12h8M10 19h11M4 4v16M2 6l2-2 2 2M2 18l2 2 2-2"
        }
        let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24' viewBox='0 0 24 24'><path d='\(strokes)' fill='none' stroke='black' stroke-width='1.65' stroke-linecap='round' stroke-linejoin='round'/></svg>"
        let image = NSImage(data: Data(svg.utf8)) ?? NSImage(size: NSSize(width: 20, height: 20))
        image.isTemplate = true
        image.size = NSSize(width: 19, height: 19)
        return image
    }
}
