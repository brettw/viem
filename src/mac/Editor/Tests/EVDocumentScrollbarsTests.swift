import AppKit
import XCTest

@testable import ViemEditor

@MainActor
final class EVDocumentScrollbarsTests: XCTestCase {
    typealias State = EVDocumentScrollbars.AxisState

    func testLegacyUsesNativeControlsAndStableGuttersWhileHorizontalAvailabilityFades() {
        let clock = ScrollbarTestClock()
        let bars = makeBars(style: .legacy, clock: clock)
        bars.update(vertical: vertical(), horizontal: horizontal(), horizontalAvailable: true)
        advance(bars, clock: clock, to: 0.12)
        bars.layout()

        XCTAssertEqual(bars.verticalScroller.scrollerStyle, .legacy)
        XCTAssertEqual(bars.horizontalScroller.scrollerStyle, .legacy)
        XCTAssertEqual(bars.verticalScroller.doubleValue, 0.25, accuracy: 0.001)
        XCTAssertEqual(bars.verticalScroller.knobProportion, 0.2, accuracy: 0.001)
        XCTAssertEqual(bars.horizontalScroller.doubleValue, 0.5, accuracy: 0.001)
        XCTAssertEqual(bars.horizontalScroller.knobProportion, 0.5, accuracy: 0.001)
        XCTAssertFalse(bars.verticalScroller.isHidden)
        XCTAssertFalse(bars.horizontalScroller.isHidden)
        XCTAssertGreaterThan(bars.contentInsets.right, 0)
        XCTAssertEqual(bars.verticalScroller.frame.width, bars.contentInsets.right)
        XCTAssertEqual(bars.contentInsets.bottom, 0, "Horizontal controls must not reserve an unpainted strip")
        XCTAssertEqual(bars.horizontalScroller.frame.height,
            NSScroller.scrollerWidth(for: .regular, scrollerStyle: .legacy))
        XCTAssertEqual(bars.verticalScroller.frame.maxY, bars.horizontalScroller.frame.minY)
        let insets = bars.contentInsets
        let verticalFrame = bars.verticalScroller.frame
        bars.update(vertical: .empty, horizontal: .empty, horizontalAvailable: false)
        advance(bars, clock: clock, to: 0.22)
        XCTAssertEqual(bars.horizontalScroller.alphaValue, 1, "brief row-edge changes retain the knob before fading")
        XCTAssertTrue(bars.horizontalScroller.isEnabled, "disabling a native scroller would erase its knob before the fade")
        bars.layout()
        XCTAssertNil(bars.hitTest(NSPoint(x: 30, y: bars.horizontalScroller.frame.midY)))
        advance(bars, clock: clock, to: 0.4)
        XCTAssertGreaterThan(bars.horizontalScroller.alphaValue, 0)
        XCTAssertLessThan(bars.horizontalScroller.alphaValue, 1)
        advance(bars, clock: clock, to: 0.52)
        bars.layout()
        XCTAssertTrue(bars.horizontalScroller.isHidden)
        XCTAssertFalse(bars.horizontalScroller.isEnabled)
        XCTAssertFalse(bars.verticalScroller.isHidden, "Always-show style retains its vertical track even when the document fits")
        XCTAssertEqual(bars.contentInsets.right, insets.right)
        XCTAssertEqual(bars.contentInsets.bottom, insets.bottom)
        XCTAssertEqual(bars.verticalScroller.frame, verticalFrame)
        XCTAssertNil(bars.hitTest(NSPoint(x: 30, y: 30)), "the overlay container must not intercept document input")
    }

    func testOverlayActivityFadesAndKeyboardViewportChangesRevealItAgain() {
        let clock = ScrollbarTestClock()
        let bars = makeBars(style: .overlay, clock: clock)
        bars.update(vertical: vertical(), horizontal: .empty, horizontalAvailable: false)
        XCTAssertEqual(bars.contentInsets.right, 0)
        XCTAssertEqual(bars.contentInsets.bottom, 0)
        advance(bars, clock: clock, to: 0.06)
        XCTAssertEqual(bars.verticalScroller.alphaValue, 0.5, accuracy: 0.01)
        advance(bars, clock: clock, to: 0.12)
        XCTAssertEqual(bars.verticalScroller.alphaValue, 1)
        advance(bars, clock: clock, to: 0.9)
        XCTAssertEqual(bars.verticalScroller.alphaValue, 1)
        advance(bars, clock: clock, to: 1.1)
        XCTAssertEqual(bars.verticalScroller.alphaValue, 0.5, accuracy: 0.01)
        advance(bars, clock: clock, to: 1.21)
        XCTAssertTrue(bars.verticalScroller.isHidden)
        var moved = vertical()
        moved.position += 40
        bars.update(vertical: moved, horizontal: .empty, horizontalAvailable: false)
        advance(bars, clock: clock, to: 1.33)
        XCTAssertFalse(bars.verticalScroller.isHidden)
        XCTAssertEqual(bars.verticalScroller.alphaValue, 1, accuracy: 0.001)
        XCTAssertTrue(bars.horizontalScroller.isHidden)
    }

    func testNativePageLineAndKnobActionsRequestCheckedNormalizedCorePositions() {
        let bars = makeBars(style: .legacy, clock: ScrollbarTestClock())
        bars.update(vertical: vertical(), horizontal: horizontal(), horizontalAvailable: true)
        var requests: [(EVDocumentScrollbars.Axis, Double)] = []
        bars.onScroll = { requests.append(($0, $1)) }
        for (part, value, expected) in [
            (NSScroller.Part.decrementPage, 0, 0.025),
            (.incrementPage, 0, 0.475),
            (.knob, 0.76, 0.76),
            (.knobSlot, 2, 1),
            (.noPart, -1, 0),
        ] {
            bars.performScrollAction(axis: .vertical, part: part, value: value)
            XCTAssertEqual(requests.last?.0, .vertical)
            XCTAssertEqual(requests.last?.1 ?? -1, expected, accuracy: 0.0001)
        }
        bars.performScrollAction(axis: .horizontal, part: .knob, value: 0.8)
        XCTAssertEqual(requests.last?.0, .horizontal)
        XCTAssertEqual(requests.last?.1, 0.8)
        let beforeAccessibility = requests.count
        XCTAssertTrue(bars.verticalScroller.accessibilityPerformIncrement())
        XCTAssertEqual(requests.count, beforeAccessibility + 1)
        XCTAssertEqual(requests.last?.1 ?? -1, 0.275, accuracy: 0.0001)
        XCTAssertTrue(bars.verticalScroller.accessibilityPerformDecrement())
        XCTAssertEqual(requests.last?.1 ?? -1, 0.225, accuracy: 0.0001)
        bars.verticalScroller.setAccessibilityValue(NSNumber(value: 0.65))
        XCTAssertEqual(requests.last?.1 ?? -1, 0.65, accuracy: 0.001)
        let count = requests.count
        bars.performScrollAction(axis: .vertical, part: .knob, value: .nan)
        bars.update(vertical: .empty, horizontal: .empty, horizontalAvailable: false)
        bars.performScrollAction(axis: .horizontal, part: .knob, value: 0.5)
        bars.performScrollAction(axis: .vertical, part: .incrementPage, value: 0)
        XCTAssertEqual(requests.count, count)
        XCTAssertFalse(bars.verticalScroller.accessibilityPerformIncrement())
        XCTAssertFalse(bars.horizontalScroller.accessibilityPerformDecrement())
        XCTAssertEqual(bars.verticalScroller.accessibilityLabel(), "Vertical document scroll bar")
        XCTAssertEqual(bars.horizontalScroller.accessibilityLabel(), "Horizontal document scroll bar")
        XCTAssertEqual(bars.verticalScroller.accessibilityRole(), .scrollBar)
    }

    func testKnobTrackingPreventsHideAndReexportsDoNotJumpTheNativeThumb() {
        let clock = ScrollbarTestClock()
        let bars = makeBars(style: .overlay, clock: clock)
        bars.update(vertical: vertical(), horizontal: horizontal(), horizontalAvailable: true)
        advance(bars, clock: clock, to: 0.12)
        bars.setTracking(true, axis: .vertical)
        bars.verticalScroller.doubleValue = 0.72
        let proportion = bars.verticalScroller.knobProportion
        bars.update(
            vertical: State(position: 80, maximum: 1600, viewportLength: 200, lineStep: 20),
            horizontal: horizontal(), horizontalAvailable: true
        )
        advance(bars, clock: clock, to: 10)
        XCTAssertEqual(bars.verticalScroller.doubleValue, 0.72, accuracy: 0.001)
        XCTAssertEqual(bars.verticalScroller.knobProportion, proportion)
        XCTAssertEqual(bars.verticalScroller.alphaValue, 1)
        bars.setTracking(false, axis: .vertical)
        XCTAssertEqual(bars.verticalScroller.doubleValue, 0.05, accuracy: 0.001)
        advance(bars, clock: clock, to: 10.9)
        XCTAssertEqual(bars.verticalScroller.alphaValue, 1)
        advance(bars, clock: clock, to: 11.21)
        XCTAssertTrue(bars.verticalScroller.isHidden)
        XCTAssertTrue(EVDocumentScroller.isCompatibleWithOverlayScrollers)
    }

    func testPreferredStyleNotificationChangesNativeStyleAndGeometryOnce() {
        let center = NotificationCenter()
        let clock = ScrollbarTestClock()
        var style = NSScroller.Style.overlay
        let bars = EVDocumentScrollbars(
            frame: NSRect(x: 0, y: 0, width: 300, height: 200),
            preferredStyleProvider: { style }, clock: { clock.now },
            automaticallySchedulesVisibility: false, notificationCenter: center
        )
        var changes = 0
        bars.onGeometryChange = { changes += 1 }
        style = .legacy
        center.post(name: NSScroller.preferredScrollerStyleDidChangeNotification, object: nil)
        XCTAssertEqual(changes, 1)
        XCTAssertEqual(bars.scrollerStyle, .legacy)
        XCTAssertEqual(bars.verticalScroller.scrollerStyle, .legacy)
        XCTAssertGreaterThan(bars.contentInsets.right, 0)
        XCTAssertFalse(bars.verticalScroller.isHidden)
        center.post(name: NSScroller.preferredScrollerStyleDidChangeNotification, object: nil)
        XCTAssertEqual(changes, 1)
        style = .overlay
        center.post(name: NSScroller.preferredScrollerStyleDidChangeNotification, object: nil)
        XCTAssertEqual(changes, 2)
        XCTAssertEqual(bars.contentInsets.right, 0)
    }

    func testVisibilityTimerStopsWhenDetachedAndDoesNotRetainTheComponent() {
        weak var released: EVDocumentScrollbars?
        autoreleasepool {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 300, height: 200), styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            let bars = EVDocumentScrollbars(preferredStyleProvider: { .overlay })
            released = bars
            window.contentView?.addSubview(bars)
            bars.update(vertical: vertical(), horizontal: horizontal(), horizontalAvailable: true)
            XCTAssertTrue(bars.isVisibilityTimerScheduled)
            bars.removeFromSuperview()
            XCTAssertFalse(bars.isVisibilityTimerScheduled)
            window.close()
        }
        XCTAssertNil(released)
    }

    func testKnobsPaintVisibleInkInBothOrientationsOnLightAndDarkCanvases() throws {
        for appearanceName in [NSAppearance.Name.aqua, .darkAqua] {
            for style in [NSScroller.Style.legacy, .overlay] {
                let clock = ScrollbarTestClock()
                let bars = makeBars(style: style, clock: clock)
                bars.appearance = NSAppearance(named: appearanceName)
                bars.update(vertical: vertical(), horizontal: horizontal(), horizontalAvailable: true)
                advance(bars, clock: clock, to: 0.12)
                bars.layout()
                for scroller in [bars.verticalScroller, bars.horizontalScroller] {
                    let darkCanvas = appearanceName == .darkAqua
                    scroller.knobStyle = darkCanvas ? .light : .dark
                    XCTAssertFalse(scroller.rect(for: .knob).isEmpty)
                    bars.layout()
                    let bitmap = try XCTUnwrap(bars.bitmapImageRepForCachingDisplay(in: bars.bounds))
                    bars.cacheDisplay(in: bars.bounds, to: bitmap)
                    XCTAssertGreaterThan(
                        maximumContrast(bitmap, in: scroller.frame, bounds: bars.bounds, darkCanvas: darkCanvas), 0.3,
                        "\(style) \(appearanceName.rawValue) \(scroller.frame.size): an enabled scroller must paint its thumb, not just a faint track"
                    )
                }
            }
        }
    }

    func testPaintedKnobMidpointsRouteToNativeControlsInBothOrientations() throws {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 300), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        for style in [NSScroller.Style.legacy, .overlay] {
            let clock = ScrollbarTestClock()
            let bars = makeBars(style: style, clock: clock)
            bars.appearance = NSAppearance(named: .aqua)
            bars.setFrameOrigin(NSPoint(x: 20, y: 30))
            window.contentView?.addSubview(bars)
            defer { bars.removeFromSuperview() }
            for fraction in [CGFloat(0), 0.5, 1] {
                var vertical = vertical()
                var horizontal = horizontal()
                vertical.position = vertical.maximum * fraction
                horizontal.position = horizontal.maximum * fraction
                bars.update(vertical: vertical, horizontal: horizontal, horizontalAvailable: true)
                clock.now += 0.15
                bars.advanceVisibility(to: clock.now)
                bars.layout()
                bars.verticalScroller.knobStyle = .dark
                bars.horizontalScroller.knobStyle = .dark
                bars.layout()
                let bitmap = try XCTUnwrap(bars.bitmapImageRepForCachingDisplay(in: bars.bounds))
                bars.cacheDisplay(in: bars.bounds, to: bitmap)
                for scroller in [bars.verticalScroller, bars.horizontalScroller] {
                    let knob = scroller.rect(for: .knob)
                    let midpoint = NSPoint(x: knob.midX, y: knob.midY)
                    let pointInBars = scroller.convert(midpoint, to: bars)
                    let pointInParent = bars.convert(pointInBars, to: bars.superview)
                    let label = "\(style) fraction=\(fraction) frame=\(scroller.frame) flipped=\(scroller.isFlipped)"
                    XCTAssertTrue(scroller.hitTest(pointInBars) === scroller, "native \(label)")
                    XCTAssertTrue(bars.hitTest(pointInParent) === scroller, "composed \(label)")
                    XCTAssertEqual(scroller.testPart(scroller.convert(midpoint, to: nil)), .knob, label)
                    let painted = try XCTUnwrap(visibleInkRect(bitmap, in: scroller.frame, bounds: bars.bounds))
                    let paintedMidpoint = NSPoint(x: painted.midX, y: painted.midY)
                    XCTAssertTrue(
                        bars.hitTest(bars.convert(paintedMidpoint, to: bars.superview)) === scroller,
                        "actual painted thumb at \(paintedMidpoint): \(label)"
                    )
                    XCTAssertEqual(
                        scroller.testPart(bars.convert(paintedMidpoint, to: nil)), .knob,
                        "painted midpoint \(paintedMidpoint), native midpoint \(pointInBars): \(label)"
                    )
                    if style == .overlay {
                        XCTAssertTrue(
                            bars.resolveHitTest(pointInParent, nativeHit: bars) === scroller,
                            "a visible knob still routes to its native control when AppKit's private overlay opacity rejects the hit"
                        )
                    }
                }
            }
            if style == .overlay {
                let horizontal = bars.horizontalScroller
                let knob = horizontal.rect(for: .knob)
                let midpoint = horizontal.convert(NSPoint(x: knob.midX, y: knob.midY), to: bars.superview)
                bars.update(vertical: vertical(), horizontal: .empty, horizontalAvailable: false)
                XCTAssertNil(bars.resolveHitTest(midpoint, nativeHit: bars), "an unavailable bar must not capture input while its old thumb fades")
                clock.now += 2
                bars.advanceVisibility(to: clock.now)
                XCTAssertNil(bars.resolveHitTest(midpoint, nativeHit: bars))
            }
        }
    }

    private func visibleInkRect(_ bitmap: NSBitmapImageRep, in rect: NSRect, bounds: NSRect) -> NSRect? {
        let scaleX = CGFloat(bitmap.pixelsWide) / bounds.width
        let scaleY = CGFloat(bitmap.pixelsHigh) / bounds.height
        var ink: NSRect?
        for y in max(0, Int(rect.minY * scaleY))..<min(bitmap.pixelsHigh, Int(rect.maxY * scaleY)) {
            for x in max(0, Int(rect.minX * scaleX))..<min(bitmap.pixelsWide, Int(rect.maxX * scaleX)) {
                guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                let brightness = (color.redComponent + color.greenComponent + color.blueComponent) / 3
                guard (1 - brightness) * color.alphaComponent > 0.3 else { continue }
                let pixel = NSRect(x: CGFloat(x) / scaleX, y: CGFloat(y) / scaleY, width: 1 / scaleX, height: 1 / scaleY)
                ink = ink.map { $0.union(pixel) } ?? pixel
            }
        }
        return ink
    }

    private func maximumContrast(_ bitmap: NSBitmapImageRep, in rect: NSRect, bounds: NSRect, darkCanvas: Bool) -> CGFloat {
        var result: CGFloat = 0
        let scaleX = CGFloat(bitmap.pixelsWide) / bounds.width
        let scaleY = CGFloat(bitmap.pixelsHigh) / bounds.height
        for y in max(0, Int(rect.minY * scaleY))..<min(bitmap.pixelsHigh, Int(rect.maxY * scaleY)) {
            for x in max(0, Int(rect.minX * scaleX))..<min(bitmap.pixelsWide, Int(rect.maxX * scaleX)) {
                guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                let brightness = (color.redComponent + color.greenComponent + color.blueComponent) / 3
                result = max(result, (darkCanvas ? brightness : 1 - brightness) * color.alphaComponent)
            }
        }
        return result
    }

    private func vertical() -> State { State(position: 200, maximum: 800, viewportLength: 200, lineStep: 20) }
    private func horizontal() -> State { State(position: 150, maximum: 300, viewportLength: 300, lineStep: 20) }

    private func makeBars(style: NSScroller.Style, clock: ScrollbarTestClock) -> EVDocumentScrollbars {
        EVDocumentScrollbars(
            frame: NSRect(x: 0, y: 0, width: 300, height: 200),
            preferredStyleProvider: { style }, clock: { clock.now },
            automaticallySchedulesVisibility: false
        )
    }

    private func advance(_ bars: EVDocumentScrollbars, clock: ScrollbarTestClock, to time: TimeInterval) {
        clock.now = time
        bars.advanceVisibility(to: time)
    }
}

private final class ScrollbarTestClock {
    var now: TimeInterval = 0
}
