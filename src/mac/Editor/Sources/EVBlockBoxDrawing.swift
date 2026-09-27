import CoreGraphics

enum EVBlockBoxDrawing {
    /// Round shared edges to the same device-pixel boundary. Aliased fills can
    /// still cover both pixels touched by a fractional edge, darkening a seam
    /// when adjacent slices of one translucent owner are painted separately.
    static func fill(_ rect: CGRect, color: CGColor, in context: CGContext) {
        let device = context.convertToDeviceSpace(rect)
        let left = device.minX.rounded(), top = device.minY.rounded()
        let right = device.maxX.rounded(), bottom = device.maxY.rounded()
        guard right > left, bottom > top else { return }
        let aligned = CGRect(x: left, y: top, width: right - left, height: bottom - top)
        context.setFillColor(color)
        context.fill(context.convertToUserSpace(aligned))
    }
}
