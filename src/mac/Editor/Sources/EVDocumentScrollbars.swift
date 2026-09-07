import AppKit

/// Native controls for the core-owned viewport. This view never moves the
/// document or creates an NSClipView; its actions request a new core position.
@MainActor
final class EVDocumentScrollbars: NSView {
    enum Axis: Hashable { case vertical, horizontal }

    struct AxisState {
        var position: CGFloat
        var maximum: CGFloat
        var viewportLength: CGFloat
        var lineStep: CGFloat

        static let empty = Self(position: 0, maximum: 0, viewportLength: 0, lineStep: 1)

        var normalizedPosition: Double {
            guard maximum.isFinite, maximum > 0, position.isFinite else { return 0 }
            return Double(min(1, max(0, position / maximum)))
        }

        var knobProportion: CGFloat {
            guard maximum.isFinite, maximum > 0 else { return 1 }
            let viewport = viewportLength.isFinite ? max(0, viewportLength) : 0
            return min(1, max(0, viewport / (viewport + maximum)))
        }
    }

    var onScroll: ((Axis, Double) -> Void)?
    var onGeometryChange: (() -> Void)?

    let verticalScroller = EVDocumentScroller(frame: NSRect(x: 0, y: 0, width: 15, height: 100))
    let horizontalScroller = EVDocumentScroller(frame: NSRect(x: 0, y: 0, width: 100, height: 15))
    private let verticalKnob = EVScrollerKnobOverlay()
    private let horizontalKnob = EVScrollerKnobOverlay()
    private(set) var scrollerStyle: NSScroller.Style
    private(set) var horizontalAvailable = false
    private(set) var verticalState: AxisState = .empty
    private(set) var horizontalState: AxisState = .empty

    private let preferredStyleProvider: @MainActor () -> NSScroller.Style
    private let clock: () -> TimeInterval
    private let automaticallySchedulesVisibility: Bool
    private let notificationCenter: NotificationCenter
    private var styleObserver: NSObjectProtocol?
    private var visibilityTimer: Timer?
    private var lastActivity = -TimeInterval.infinity
    private var horizontalAvailabilityLostAt: TimeInterval?
    private var lastTick: TimeInterval
    private var trackingAxes: Set<Axis> = []
    private var hoveredAxes: Set<Axis> = []
    private var verticalAlpha: CGFloat = 0
    private var horizontalAlpha: CGFloat = 0

    private static let idleDelay: TimeInterval = 1
    private static let fadeInDuration: TimeInterval = 0.12
    private static let fadeOutDuration: TimeInterval = 0.2
    private static let availabilityLossDelay: TimeInterval = 0.18

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { false }

    /// Legacy gutters stay fixed while horizontal availability fades. Reflowing
    /// the viewport as its bottom row changes would itself change availability.
    var contentInsets: NSEdgeInsets {
        guard scrollerStyle == .legacy else { return NSEdgeInsets() }
        let width = NSScroller.scrollerWidth(for: .regular, scrollerStyle: .legacy)
        return NSEdgeInsets(top: 0, left: 0, bottom: width, right: width)
    }

    var isVisibilityTimerScheduled: Bool { visibilityTimer != nil }

    init(
        frame: NSRect = .zero,
        preferredStyleProvider: @escaping @MainActor () -> NSScroller.Style = { NSScroller.preferredScrollerStyle },
        clock: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
        automaticallySchedulesVisibility: Bool = true,
        notificationCenter: NotificationCenter = .default
    ) {
        self.preferredStyleProvider = preferredStyleProvider
        self.clock = clock
        self.automaticallySchedulesVisibility = automaticallySchedulesVisibility
        self.notificationCenter = notificationCenter
        scrollerStyle = preferredStyleProvider()
        lastTick = clock()
        super.init(frame: frame)
        setAccessibilityElement(false)
        for (axis, scroller) in [(Axis.vertical, verticalScroller), (.horizontal, horizontalScroller)] {
            scroller.scrollerStyle = scrollerStyle
            scroller.controlSize = .regular
            scroller.target = self
            scroller.action = #selector(scrollerAction(_:))
            scroller.isContinuous = true
            scroller.setAccessibilityLabel(axis == .vertical ? "Vertical document scroll bar" : "Horizontal document scroll bar")
            scroller.onTrackingChange = { [weak self] tracking in
                self?.setTracking(tracking, axis: axis)
            }
            scroller.onHoverChange = { [weak self] hovering in
                self?.setHovering(hovering, axis: axis)
            }
            scroller.onAccessibilityStep = { [weak self, weak scroller] increment in
                guard let self, let scroller, self.onScroll != nil,
                      scroller.isEnabled,
                      axis == .vertical || self.horizontalAvailable else { return false }
                self.performScrollAction(axis: axis, part: increment ? .incrementLine : .decrementLine, value: 0)
                NSAccessibility.post(element: scroller, notification: .valueChanged)
                return true
            }
            addSubview(scroller)
        }
        addSubview(verticalKnob)
        addSubview(horizontalKnob)
        styleObserver = notificationCenter.addObserver(
            forName: NSScroller.preferredScrollerStyleDidChangeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.refreshPreferredStyle() }
        }
        advanceVisibility(to: lastTick)
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) { fatalError("init(coder:) is unavailable") }

    deinit {
        visibilityTimer?.invalidate()
        if let styleObserver { notificationCenter.removeObserver(styleObserver) }
    }

    override func layout() {
        super.layout()
        let width = NSScroller.scrollerWidth(for: .regular, scrollerStyle: scrollerStyle)
        let horizontalInset = scrollerStyle == .legacy || horizontalAvailable || horizontalAlpha > 0 ? width : 0
        verticalScroller.frame = NSRect(
            x: max(0, bounds.width - width), y: 0,
            width: width, height: max(0, bounds.height - horizontalInset)
        )
        horizontalScroller.frame = NSRect(
            x: 0, y: max(0, bounds.height - width),
            width: max(0, bounds.width - width), height: width
        )
        updateKnobOverlays()
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        resolveHitTest(point, nativeHit: super.hitTest(point))
    }

    /// Native standalone overlay scrollers can reject hits while their private
    /// opacity says "hidden", even though our public presentation is visible
    /// and testPart correctly identifies the knob. Recover only a live native
    /// part; NSScroller still receives mouseDown and performs all tracking.
    func resolveHitTest(_ point: NSPoint, nativeHit: NSView?) -> NSView? {
        guard !isHiddenOrHasHiddenAncestor, alphaValue > 0.01 else { return nil }
        if nativeHit === horizontalScroller && !horizontalAvailable { return nil }
        if let nativeHit, nativeHit !== self { return nativeHit }
        guard scrollerStyle == .overlay else { return nil }
        let local = convert(point, from: superview)
        guard isMousePoint(local, in: bounds) else { return nil }
        for scroller in [verticalScroller, horizontalScroller] {
            guard !scroller.isHidden, scroller.isEnabled, scroller.alphaValue > 0.01,
                  scroller !== horizontalScroller || horizontalAvailable else { continue }
            let inside = scroller.convert(local, from: self)
            guard scroller.isMousePoint(inside, in: scroller.bounds) else { continue }
            if scroller.testPart(convert(local, to: nil)) != .noPart { return scroller }
        }
        return nil
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil {
            stopVisibilityTimer()
            trackingAxes.removeAll()
            hoveredAxes.removeAll()
        } else {
            refreshPreferredStyle()
            updateVisibility()
        }
    }

    func update(vertical: AxisState, horizontal: AxisState, horizontalAvailable: Bool) {
        let now = clock()
        advanceVisibility(to: now)
        let available = horizontalAvailable && horizontal.maximum.isFinite && horizontal.maximum > 0
        let moved = vertical.position != verticalState.position || horizontal.position != horizontalState.position
        let appeared = (vertical.maximum > 0 && verticalState.maximum <= 0) || (available && !self.horizontalAvailable)
        if self.horizontalAvailable && !available { horizontalAvailabilityLostAt = now }
        if available { horizontalAvailabilityLostAt = nil }
        verticalState = vertical
        horizontalState = horizontal
        self.horizontalAvailable = available
        if moved || appeared { lastActivity = now }
        updateControl(verticalScroller, state: vertical, axis: .vertical, available: vertical.maximum > 0)
        updateControl(horizontalScroller, state: horizontal, axis: .horizontal, available: self.horizontalAvailable)
        needsLayout = true
        updateVisibility()
    }

    func noteScrollActivity() {
        let now = clock()
        advanceVisibility(to: now)
        lastActivity = now
        updateVisibility()
    }

    func refreshPreferredStyle() {
        let preferred = preferredStyleProvider()
        guard scrollerStyle != preferred else { return }
        advanceVisibility(to: clock())
        scrollerStyle = preferred
        verticalScroller.scrollerStyle = preferred
        horizontalScroller.scrollerStyle = preferred
        needsLayout = true
        updateVisibility()
        onGeometryChange?()
    }

    private func updateControl(_ scroller: EVDocumentScroller, state: AxisState, axis: Axis, available: Bool) {
        // Native knob tracking owns its fractional value until mouse-up. A
        // synchronous re-export may refine estimated extents while it tracks.
        guard !trackingAxes.contains(axis) else { return }
        // A disabled native scroller stops drawing its knob. Preserve its last
        // enabled appearance until the availability fade finishes; hit testing
        // and the callback already reject unavailable horizontal scrolling.
        if available || axis == .vertical || horizontalAlpha == 0 {
            scroller.isEnabled = available
        }
        if available || scroller.isHidden || axis == .vertical {
            scroller.doubleValue = state.normalizedPosition
            scroller.knobProportion = state.knobProportion
        }
    }

    @objc private func scrollerAction(_ sender: NSScroller) {
        let axis: Axis = sender === verticalScroller ? .vertical : .horizontal
        performScrollAction(axis: axis, part: sender.hitPart, value: sender.doubleValue)
        updateKnobOverlays()
    }

    /// Shared by the actual native target/action and tests of page, line and
    /// accessibility/value actions. The callback always contains a fraction.
    func performScrollAction(axis: Axis, part: NSScroller.Part, value: Double) {
        let state = axis == .vertical ? verticalState : horizontalState
        guard state.maximum.isFinite, state.maximum > 0,
              axis == .vertical || horizontalAvailable else { return }
        let current = state.normalizedPosition
        let line = state.lineStep.isFinite ? max(1, state.lineStep) : 1
        let viewport = state.viewportLength.isFinite ? max(0, state.viewportLength) : 0
        let page = max(line, viewport - line)
        let requested: Double
        switch part {
        case .decrementLine: requested = current - Double(line / state.maximum)
        case .incrementLine: requested = current + Double(line / state.maximum)
        case .decrementPage: requested = current - Double(page / state.maximum)
        case .incrementPage: requested = current + Double(page / state.maximum)
        case .knob, .knobSlot, .noPart: requested = value
        @unknown default: return
        }
        guard requested.isFinite else { return }
        noteScrollActivity()
        onScroll?(axis, min(1, max(0, requested)))
    }

    func setTracking(_ tracking: Bool, axis: Axis) {
        advanceVisibility(to: clock())
        if tracking { trackingAxes.insert(axis) } else { trackingAxes.remove(axis) }
        noteScrollActivity()
        if !tracking {
            let state = axis == .vertical ? verticalState : horizontalState
            let scroller = axis == .vertical ? verticalScroller : horizontalScroller
            updateControl(scroller, state: state, axis: axis, available: axis == .vertical ? state.maximum > 0 : horizontalAvailable)
        }
        updateKnobOverlays()
    }

    private func setHovering(_ hovering: Bool, axis: Axis) {
        advanceVisibility(to: clock())
        if hovering { hoveredAxes.insert(axis) } else { hoveredAxes.remove(axis) }
        noteScrollActivity()
    }

    private func updateVisibility() {
        advanceVisibility(to: clock())
        scheduleVisibilityTimerIfNeeded()
    }

    /// A deterministic clock boundary also used by the run-loop animation.
    /// Frames never alter viewport geometry; only a system style change does.
    func advanceVisibility(to now: TimeInterval) {
        let elapsed = max(0, now - lastTick)
        let idleFadeElapsed = max(0, now - max(lastTick, lastActivity + Self.idleDelay))
        let horizontalLossFadeElapsed = max(0, now - max(lastTick, (horizontalAvailabilityLostAt ?? now) + Self.availabilityLossDelay))
        lastTick = now
        let active = now < lastActivity + Self.idleDelay || !trackingAxes.isEmpty || !hoveredAxes.isEmpty
        let showVertical = scrollerStyle == .legacy || ((verticalState.maximum > 0 || trackingAxes.contains(.vertical)) && active)
        let showHorizontal = (horizontalAvailable || trackingAxes.contains(.horizontal) || retainingHorizontalAvailability(at: now)) && (scrollerStyle == .legacy || active)
        verticalAlpha = scrollerStyle == .legacy ? 1 : nextAlpha(
            verticalAlpha, showing: showVertical,
            elapsed: !showVertical && verticalState.maximum > 0 ? idleFadeElapsed : elapsed
        )
        horizontalAlpha = nextAlpha(
            horizontalAlpha, showing: showHorizontal,
            elapsed: !showHorizontal && !horizontalAvailable ? horizontalLossFadeElapsed
                : !showHorizontal && scrollerStyle == .overlay ? idleFadeElapsed : elapsed
        )
        applyAlpha(verticalAlpha, to: verticalScroller)
        applyAlpha(horizontalAlpha, to: horizontalScroller)
        if horizontalAlpha == 0 && !horizontalAvailable { horizontalScroller.isEnabled = false }
        updateKnobOverlays()
        needsLayout = true
        if !visibilityNeedsTimer(now: now) { stopVisibilityTimer() }
    }

    private func nextAlpha(_ alpha: CGFloat, showing: Bool, elapsed: TimeInterval) -> CGFloat {
        let duration = showing ? Self.fadeInDuration : Self.fadeOutDuration
        let step = CGFloat(elapsed / duration)
        return showing ? min(1, alpha + step) : max(0, alpha - step)
    }

    private func updateKnobOverlays() {
        for (scroller, knob) in [(verticalScroller, verticalKnob), (horizontalScroller, horizontalKnob)] {
            knob.frame = scroller.frame
            knob.nativeIsFlipped = scroller.isFlipped
            knob.knobRect = scroller.rect(for: .knob)
            knob.knobStyle = scroller.knobStyle
            knob.alphaValue = scroller.alphaValue
            knob.isHidden = scrollerStyle != .overlay || scroller.isHidden || !scroller.isEnabled
            knob.needsDisplay = true
        }
    }

    private func applyAlpha(_ alpha: CGFloat, to scroller: NSScroller) {
        scroller.alphaValue = alpha
        scroller.isHidden = alpha == 0
    }

    private func visibilityNeedsTimer(now: TimeInterval) -> Bool {
        if (0 < verticalAlpha && verticalAlpha < 1) || (0 < horizontalAlpha && horizontalAlpha < 1) { return true }
        let active = now < lastActivity + Self.idleDelay || !trackingAxes.isEmpty || !hoveredAxes.isEmpty
        let verticalTarget: CGFloat = scrollerStyle == .legacy || ((verticalState.maximum > 0 || trackingAxes.contains(.vertical)) && active) ? 1 : 0
        let horizontalTarget: CGFloat = (horizontalAvailable || trackingAxes.contains(.horizontal) || retainingHorizontalAvailability(at: now)) && (scrollerStyle == .legacy || active) ? 1 : 0
        if verticalAlpha != verticalTarget || horizontalAlpha != horizontalTarget { return true }
        if retainingHorizontalAvailability(at: now) { return true }
        return scrollerStyle == .overlay && now < lastActivity + Self.idleDelay && trackingAxes.isEmpty && hoveredAxes.isEmpty
    }

    private func retainingHorizontalAvailability(at now: TimeInterval) -> Bool {
        guard horizontalAlpha > 0, let lostAt = horizontalAvailabilityLostAt else { return false }
        return now < lostAt + Self.availabilityLossDelay
    }

    private func scheduleVisibilityTimerIfNeeded() {
        guard automaticallySchedulesVisibility, window != nil, visibilityTimer == nil,
              visibilityNeedsTimer(now: clock()) else { return }
        let timer = Timer(timeInterval: 1 / 60, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.advanceVisibility(to: self.clock())
            }
        }
        visibilityTimer = timer
        RunLoop.main.add(timer, forMode: .common)
    }

    private func stopVisibilityTimer() {
        visibilityTimer?.invalidate()
        visibilityTimer = nil
    }
}

/// Keep AppKit's knob tracking, paging, keyboard/accessibility actions and
/// appearance. NSScroller's public overlay contract permits trackKnob and
/// parts-based overrides, but explicitly warns against replacing mouseDown.
@MainActor
final class EVDocumentScroller: NSScroller {
    var onTrackingChange: ((Bool) -> Void)?
    var onHoverChange: ((Bool) -> Void)?
    var onAccessibilityStep: ((Bool) -> Bool)?
    private var hoverArea: NSTrackingArea?

    override class var isCompatibleWithOverlayScrollers: Bool { true }

    // Standalone NSScroller implements AX value-setting (which sends its
    // target/action) but does not supply increment/decrement actions itself.
    override func accessibilityPerformIncrement() -> Bool {
        onAccessibilityStep?(true) ?? false
    }

    override func accessibilityPerformDecrement() -> Bool {
        onAccessibilityStep?(false) ?? false
    }

    override func trackKnob(with event: NSEvent) {
        onTrackingChange?(true)
        defer { onTrackingChange?(false) }
        super.trackKnob(with: event)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverArea { removeTrackingArea(hoverArea) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(area)
        hoverArea = area
    }

    override func mouseEntered(with event: NSEvent) {
        super.mouseEntered(with: event)
        onHoverChange?(true)
    }

    override func mouseExited(with event: NSEvent) {
        super.mouseExited(with: event)
        onHoverChange?(false)
    }
}

/// NSScrollView privately coordinates overlay knob painting. A standalone
/// NSScroller can therefore have a transparent knob even at alpha 1. This
/// separate paint-only sibling uses its public geometry and leaves native
/// pointer tracking, hit testing and accessibility entirely on NSScroller.
@MainActor
private final class EVScrollerKnobOverlay: NSView {
    var nativeIsFlipped = false
    var knobRect = NSRect.zero
    var knobStyle = NSScroller.KnobStyle.default
    override var isFlipped: Bool { nativeIsFlipped }
    override var acceptsFirstResponder: Bool { false }

    init() {
        super.init(frame: .zero)
        setAccessibilityElement(false)
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) { fatalError("init(coder:) is unavailable") }

    override func hitTest(_: NSPoint) -> NSView? { nil }

    override func draw(_: NSRect) {
        guard !knobRect.isEmpty else { return }
        let light: Bool
        switch knobStyle {
        case .light: light = true
        case .dark: light = false
        case .default: light = effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        @unknown default: light = false
        }
        (light ? NSColor.white.withAlphaComponent(0.65) : NSColor.black.withAlphaComponent(0.5)).setFill()
        let radius = min(knobRect.width, knobRect.height) / 2
        NSBezierPath(roundedRect: knobRect, xRadius: radius, yRadius: radius).fill()
    }
}
