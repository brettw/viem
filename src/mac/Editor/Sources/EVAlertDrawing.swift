import AppKit
import CViemCore

/// GitHub alert furniture: an outlined icon in the alert's text color and its
/// title in the system UI font. Icons are drawn on a 16-unit, y-down grid
/// scaled to the core's square bounds; the title is centered in its band.
@MainActor
enum EVAlertDrawing {
    /// The `VIEM_ALERT_*` kind carried by an alert decoration's flags.
    static func kind(_ flags: UInt32) -> UInt32 {
        (flags & UInt32(VIEM_LAYOUT_DECORATION_ALERT_KIND_MASK)) >> UInt32(VIEM_LAYOUT_DECORATION_ALERT_KIND_SHIFT)
    }

    static func isAlert(_ flags: UInt32) -> Bool {
        flags & UInt32(VIEM_LAYOUT_DECORATION_ALERT_ICON | VIEM_LAYOUT_DECORATION_ALERT_TITLE) != 0
    }

    static func draw(_ item: ViemLayoutDecorationV1, label: String, in rect: NSRect,
                     color: NSColor, context: CGContext) {
        if item.flags & UInt32(VIEM_LAYOUT_DECORATION_ALERT_ICON) != 0 {
            drawIcon(kind: kind(item.flags), in: rect, color: color.cgColor, context: context)
        } else {
            drawTitle(label, size: CGFloat(item.font_size), in: rect, color: color)
        }
    }

    static func drawTitle(_ title: String, size: CGFloat, in rect: NSRect, color: NSColor) {
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: size, weight: .semibold), .foregroundColor: color]
        let measured = (title as NSString).size(withAttributes: attributes)
        (title as NSString).draw(at: NSPoint(x: rect.minX, y: rect.midY - measured.height / 2),
                                 withAttributes: attributes)
    }

    static func drawIcon(kind: UInt32, in rect: NSRect, color: CGColor, context: CGContext) {
        guard rect.width > 0, rect.height > 0 else { return }
        context.saveGState()
        defer { context.restoreGState() }
        context.translateBy(x: rect.minX, y: rect.minY)
        context.scaleBy(x: rect.width / 16, y: rect.height / 16)
        context.setStrokeColor(color)
        context.setFillColor(color)
        context.setLineWidth(1.5)
        context.setLineCap(.round)
        context.setLineJoin(.round)
        let outline = CGMutablePath()
        // The exclamation (or info) stroke and its dot, by kind.
        var stroke = (CGPoint(x: 8, y: 7.5), CGPoint(x: 8, y: 11.5))
        var dot = CGPoint(x: 8, y: 4.75)
        switch kind {
        case UInt32(VIEM_ALERT_TIP):
            outline.move(to: CGPoint(x: 5.75, y: 11))
            outline.addCurve(to: CGPoint(x: 3.5, y: 6.25), control1: CGPoint(x: 5.75, y: 9.6), control2: CGPoint(x: 3.5, y: 8.8))
            outline.addCurve(to: CGPoint(x: 8, y: 1.75), control1: CGPoint(x: 3.5, y: 3.75), control2: CGPoint(x: 5.5, y: 1.75))
            outline.addCurve(to: CGPoint(x: 12.5, y: 6.25), control1: CGPoint(x: 10.5, y: 1.75), control2: CGPoint(x: 12.5, y: 3.75))
            outline.addCurve(to: CGPoint(x: 10.25, y: 11), control1: CGPoint(x: 12.5, y: 8.8), control2: CGPoint(x: 10.25, y: 9.6))
            outline.closeSubpath()
            outline.move(to: CGPoint(x: 6, y: 13.25)); outline.addLine(to: CGPoint(x: 10, y: 13.25))
            outline.move(to: CGPoint(x: 6.75, y: 15)); outline.addLine(to: CGPoint(x: 9.25, y: 15))
            context.addPath(outline)
            context.strokePath()
            return
        case UInt32(VIEM_ALERT_IMPORTANT):
            outline.move(to: CGPoint(x: 3.25, y: 1.75))
            outline.addLine(to: CGPoint(x: 12.75, y: 1.75))
            outline.addQuadCurve(to: CGPoint(x: 14.25, y: 3.25), control: CGPoint(x: 14.25, y: 1.75))
            outline.addLine(to: CGPoint(x: 14.25, y: 9.75))
            outline.addQuadCurve(to: CGPoint(x: 12.75, y: 11.25), control: CGPoint(x: 14.25, y: 11.25))
            outline.addLine(to: CGPoint(x: 7.75, y: 11.25))
            outline.addLine(to: CGPoint(x: 4.75, y: 14.25))
            outline.addLine(to: CGPoint(x: 4.75, y: 11.25))
            outline.addLine(to: CGPoint(x: 3.25, y: 11.25))
            outline.addQuadCurve(to: CGPoint(x: 1.75, y: 9.75), control: CGPoint(x: 1.75, y: 11.25))
            outline.addLine(to: CGPoint(x: 1.75, y: 3.25))
            outline.addQuadCurve(to: CGPoint(x: 3.25, y: 1.75), control: CGPoint(x: 1.75, y: 1.75))
            outline.closeSubpath()
            stroke = (CGPoint(x: 8, y: 4.25), CGPoint(x: 8, y: 6.75)); dot = CGPoint(x: 8, y: 8.75)
        case UInt32(VIEM_ALERT_WARNING):
            outline.move(to: CGPoint(x: 8, y: 1.75))
            outline.addLine(to: CGPoint(x: 14.75, y: 14))
            outline.addLine(to: CGPoint(x: 1.25, y: 14))
            outline.closeSubpath()
            stroke = (CGPoint(x: 8, y: 6.25), CGPoint(x: 8, y: 9.25)); dot = CGPoint(x: 8, y: 11.5)
        case UInt32(VIEM_ALERT_CAUTION):
            let points = [(5.25, 1.75), (10.75, 1.75), (14.25, 5.25), (14.25, 10.75),
                          (10.75, 14.25), (5.25, 14.25), (1.75, 10.75), (1.75, 5.25)]
            outline.addLines(between: points.map { CGPoint(x: $0.0, y: $0.1) })
            outline.closeSubpath()
            stroke = (CGPoint(x: 8, y: 4.75), CGPoint(x: 8, y: 8.25)); dot = CGPoint(x: 8, y: 10.75)
        default:
            outline.addEllipse(in: CGRect(x: 1.5, y: 1.5, width: 13, height: 13))
        }
        outline.move(to: stroke.0)
        outline.addLine(to: stroke.1)
        context.addPath(outline)
        context.strokePath()
        context.fillEllipse(in: CGRect(x: dot.x - 1, y: dot.y - 1, width: 2, height: 2))
    }
}
