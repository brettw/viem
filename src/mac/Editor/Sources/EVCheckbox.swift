import AppKit
import CViemCore
import ViemAppShell

extension EVCoreViewSession {
    @discardableResult
    func toggleCheckbox(paragraphID: UInt64, expected identity: ViemLayoutSnapshotIdentityV1) throws -> ViemCoreOutcomeV1 {
        var identity = identity
        return try performCoreOperation("Toggle checkbox") { outcome in
            viem_core_view_toggle_checkbox(document.core, viewID, &identity, paragraphID, outcome)
        }
    }
}

@MainActor
extension EVEditorSurfaceController: EVInsertMenuProviding {
    public func makeInsertTableMenuView(in menu: NSMenu) -> NSView? {
        formattingToolbar.makeTableMenuView(in: menu)
    }
}

/// The painted outline fills the core's exact baseline-to-cap-height square.
/// All artwork scales with that square; AppKit supplies appearance-aware colors.
@MainActor
enum EVCheckboxDrawing {
    static func draw(in rect: NSRect, checked: Bool, enabled: Bool, view: NSView, context: CGContext) {
        guard rect.width > 0, rect.height > 0 else { return }
        context.saveGState()
        defer { context.restoreGState() }
        context.clip(to: rect)
        context.translateBy(x: rect.minX, y: rect.minY)
        context.scaleBy(x: rect.width, y: rect.height)
        view.effectiveAppearance.performAsCurrentDrawingAppearance {
            let contrast = NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast
            let borderWidth: CGFloat = contrast ? 0.09 : 0.065
            let radius: CGFloat = 0.21
            let bounds = CGRect(x: 0, y: 0, width: 1, height: 1)
            let outline = CGPath(roundedRect: bounds, cornerWidth: radius, cornerHeight: radius, transform: nil)
            let inset = bounds.insetBy(dx: borderWidth / 2, dy: borderWidth / 2)
            let border = CGPath(roundedRect: inset, cornerWidth: radius - borderWidth / 2,
                                cornerHeight: radius - borderWidth / 2, transform: nil)
            let fill = checked ? NSColor.controlAccentColor : NSColor.controlBackgroundColor
            let edge = checked
                ? fill.blended(withFraction: contrast ? 0.45 : 0.18, of: .controlTextColor) ?? fill
                : NSColor.controlTextColor.withAlphaComponent(contrast ? 0.7 : 0.28)
            if !enabled { context.setAlpha(0.45) }
            context.addPath(outline)
            context.setFillColor(fill.cgColor)
            context.fillPath()
            context.addPath(border)
            context.setStrokeColor(edge.cgColor)
            context.setLineWidth(borderWidth)
            context.strokePath()
            if checked {
                context.move(to: CGPoint(x: 0.23, y: 0.51))
                context.addLine(to: CGPoint(x: 0.43, y: 0.71))
                context.addLine(to: CGPoint(x: 0.78, y: 0.29))
                context.setStrokeColor(NSColor.alternateSelectedControlTextColor.cgColor)
                context.setLineWidth(0.14)
                context.setLineCap(.round)
                context.setLineJoin(.round)
                context.strokePath()
            }
        }
    }
}
