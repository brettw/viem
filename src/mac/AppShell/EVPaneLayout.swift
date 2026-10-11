import AppKit
import CViemCore

/// A window owns this serial geometry handle, independently of its buffers.
@MainActor
final class EVPaneLayout {
  private var handle: UInt64 = 0
  init() { precondition(viem_pane_layout_create(&handle) == VIEM_STATUS_OK) }
  deinit { viem_pane_layout_destroy(handle) }
  private func check(_ status: UInt32) throws {
    guard status != VIEM_STATUS_OK else { return }
    if status == VIEM_STATUS_POLICY_REQUIRED { throw EVDocumentHostError.noRoomToSplit }
    throw EVPaneLayoutError.operation(status)
  }
  func update(size: NSSize, chrome: [ViemPaneChrome]) {
    precondition(viem_pane_layout_update(handle, max(0, size.width), max(0, size.height), chrome, UInt64(chrome.count)) == VIEM_STATUS_OK)
  }
  func snapshot() -> (frames: [ViemPaneFrame], minimum: NSSize) {
    var frames = Array(repeating: ViemPaneFrame(), count: 511)
    var info = ViemPaneSnapshot()
    precondition(viem_pane_layout_copy(handle, &frames, UInt64(frames.count), &info) == VIEM_STATUS_OK)
    return (Array(frames.prefix(Int(info.count))), NSSize(width: info.minimum_width, height: info.minimum_height))
  }
  func requireSplit(_ pane: UInt64, vertical: Bool, height: CGFloat) throws {
    try check(viem_pane_layout_can_split(handle, pane, vertical ? 1 : 0, height))
  }
  func split(_ pane: UInt64, vertical: Bool, height: CGFloat) throws -> UInt64 {
    var id: UInt64 = 0
    try check(viem_pane_layout_split(handle, pane, vertical ? 1 : 0, height, &id))
    return id
  }
  func remove(_ pane: UInt64) { precondition(viem_pane_layout_remove(handle, pane) == VIEM_STATUS_OK) }
  func drag(_ id: UInt64, splitter: Bool = false, delta: CGFloat) {
    precondition(viem_pane_layout_drag(handle, id, splitter ? 1 : 0, delta) == VIEM_STATUS_OK)
  }
  @discardableResult
  func action(_ pane: UInt64, operation: UInt32, count: UInt64 = 0, value: Double = 0,
              flags: UInt32 = 0, point: NSPoint = .zero) throws -> UInt64 {
    var focus: UInt64 = 0
    try check(viem_pane_layout_action(handle, pane, operation, count, value, flags, point.x, point.y, &focus))
    return focus
  }
}
private enum EVPaneLayoutError: LocalizedError {
  case operation(UInt32)
  var errorDescription: String? { "Cannot rotate or exchange a row or column containing nested splits." }
}

@MainActor
final class EVVerticalSplitter: NSView {
  var dragDidMove: ((CGFloat) -> Void)?
  private(set) var hidesTopBorder = true
  private(set) var hidesBottomBorder = true
  private var themeObserver: NSObjectProtocol?
  override init(frame: NSRect) {
    super.init(frame: frame)
    wantsLayer = true
    layer?.masksToBounds = true
    setAccessibilityRole(.splitter)
    setAccessibilityLabel("Resize views horizontally")
    applyTheme()
    themeObserver = NotificationCenter.default.addObserver(forName: .viemThemeDidChange, object: nil, queue: .main) { [weak self] _ in MainActor.assumeIsolated { self?.applyTheme() } }
  }
  required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
  deinit { if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) } }
  func updateOuterEdges(frame: ViemPaneFrame, containerHeight: CGFloat) {
    let top = frame.y <= 0.000001
    let bottom = frame.y + frame.height >= containerHeight - 0.000001
    guard top != hidesTopBorder || bottom != hidesBottomBorder else { return }
    hidesTopBorder = top; hidesBottomBorder = bottom; needsDisplay = true
  }
  override func draw(_ dirtyRect: NSRect) {
    drawBorders(EVThemeStore.shared.chromeStyles.statusBar)
  }
  func drawBorders(_ style: EVChromeStyle) {
    // Vertical edges own the corners. Partition narrow splitters as well, so
    // translucent or oversized authored borders never composite twice.
    let left = min(style.leftBorder.width, bounds.width)
    let right = min(style.rightBorder.width, max(0, bounds.width - left))
    for (border, x, width) in [(style.leftBorder, CGFloat(0), left), (style.rightBorder, bounds.width - right, right)] where width > 0 {
      (border.color ?? style.foreground).setFill()
      NSRect(x:x,y:0,width:width,height:bounds.height).fill()
    }
    let top = hidesTopBorder ? 0 : min(style.topBorder.width, bounds.height)
    let bottom = hidesBottomBorder ? 0 : min(style.bottomBorder.width, max(0, bounds.height - top))
    for (border, y, height) in [
      (style.topBorder, bounds.height - top, top),
      (style.bottomBorder, CGFloat(0), bottom),
    ] where height > 0 && bounds.width > left + right {
      (border.color ?? style.foreground).setFill()
      NSRect(x:left,y:y,width:bounds.width-left-right,height:height).fill()
    }
  }
  override func resetCursorRects() { addCursorRect(bounds, cursor: .resizeLeftRight) }
  private func applyTheme() { layer?.backgroundColor = EVThemeStore.shared.chromeStyles.statusBar.background?.cgColor; needsDisplay = true }
  override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
  override func mouseDown(with event: NSEvent) {
    guard let window else { return }
    let press = event.locationInWindow
    var last = press
    var dragging = false
    defer { if dragging { NSCursor.pop() } }
    while let next = window.nextEvent(matching: [.leftMouseDragged, .leftMouseUp]) {
      if next.type == .leftMouseUp { break }
      let point = next.locationInWindow
      if !dragging {
        if hypot(point.x - press.x, point.y - press.y) < 4 { continue }
        dragging = true; NSCursor.resizeLeftRight.push()
      }
      let delta = point.x - last.x
      last = point // Blocked overshoot is discarded, allowing immediate reversal.
      dragDidMove?(delta)
    }
  }
}
