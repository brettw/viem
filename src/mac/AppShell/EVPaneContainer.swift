import AppKit

/// Only stacked panes are supported. Every child owns its status bar and a
/// distinct core view; documents/backends can be shared across any children.
@MainActor
final class EVPaneContainer: NSViewController {
  private(set) var panes: [EVDocumentContentViewController]
  private var preferredIndex = 0
  private let stack = EVPaneStackView()
  private var heights: [CGFloat] = []
  private var applyingLayout = false

  init(first: EVDocumentContentViewController) {
    panes = [first]
    super.init(nibName: nil, bundle: nil)
  }
  @available(*, unavailable) required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }

  /// The pane focused before the current one, remembered by identity so it
  /// survives reordering. `CTRL-W p` alternates between two panes.
  private weak var lastAccessedPane: EVDocumentContentViewController?

  var activePane: EVDocumentContentViewController {
    if let responder = viewIfLoaded?.window?.firstResponder as? NSView,
      let index = panes.firstIndex(where: { responder.isDescendant(of: $0.view) })
    {
      noteFocus(movingTo: index)
    }
    return panes[min(preferredIndex, panes.count - 1)]
  }

  var activeIndex: Int { min(preferredIndex, max(0, panes.count - 1)) }

  private func noteFocus(movingTo index: Int) {
    guard index != preferredIndex else { return }
    if panes.indices.contains(preferredIndex) { lastAccessedPane = panes[preferredIndex] }
    preferredIndex = index
  }

  /// Focus one pane by its top-to-bottom position.
  func focusPane(at index: Int) {
    guard panes.indices.contains(index) else { return }
    noteFocus(movingTo: index)
    view.window?.makeFirstResponder(panes[index].editorSurface.viewController.view)
  }

  /// Perform one `CTRL-W` window effect. Pane geometry and focus belong here;
  /// the core only named the effect.
  func perform(_ request: EVWindowRequest) {
    loadViewIfNeeded()
    let current = activeIndex
    let count = panes.count
    switch request {
    case let .focusDown(step):
      focusPane(at: current + min(step, count - 1 - current))
    case let .focusUp(step):
      focusPane(at: max(current - step, 0))
    case let .focusNext(index):
      focusPane(at: index.map { clampedIndex($0) } ?? (current + 1) % count)
    case let .focusPrevious(index):
      focusPane(at: index.map { clampedIndex($0) } ?? (current + count - 1) % count)
    case .focusTop:
      focusPane(at: 0)
    case .focusBottom:
      focusPane(at: count - 1)
    case .focusLastAccessed:
      if let last = lastAccessedPane, let index = panes.firstIndex(where: { $0 === last }) {
        focusPane(at: index)
      }
    case let .rotateDown(steps):
      rotate(by: steps)
    case let .rotateUp(steps):
      rotate(by: -steps)
    case let .exchange(index):
      let other = index.map { clampedIndex($0) }
        ?? (current + 1 < count ? current + 1 : current - 1)
      exchange(current, with: other)
    case .moveToTop:
      move(current, to: 0)
    case .moveToBottom:
      move(current, to: count - 1)
    case .closeOthers:
      closeOthers()
    case let .grow(rows):
      resizeActive(byRows: rows)
    case let .shrink(rows):
      resizeActive(byRows: -rows)
    case let .setHeight(rows):
      setActiveHeight(rows: rows)
    case .equalizeHeights:
      distributeEvenly()
      layoutPanes()
    }
  }

  private func clampedIndex(_ oneBased: Int) -> Int {
    min(max(oneBased - 1, 0), panes.count - 1)
  }

  /// Rotate pane order. Positive steps move every pane down one slot and the
  /// bottom pane to the top. Slot heights stay put so only order changes.
  private func rotate(by steps: Int) {
    let count = panes.count
    guard count > 1 else { return }
    let shift = ((steps % count) + count) % count
    guard shift != 0 else { return }
    let focused = panes[activeIndex]
    var next = Array(panes[(count - shift)...])
    next.append(contentsOf: panes[..<(count - shift)])
    reorder(to: next, keepingFocusOn: focused)
  }

  private func exchange(_ first: Int, with second: Int) {
    guard panes.indices.contains(first), panes.indices.contains(second), first != second else {
      return
    }
    let focused = panes[first]
    var next = panes
    next.swapAt(first, second)
    reorder(to: next, keepingFocusOn: focused)
  }

  private func move(_ index: Int, to destination: Int) {
    guard panes.indices.contains(index), panes.indices.contains(destination),
      index != destination
    else { return }
    let focused = panes[index]
    var next = panes
    next.remove(at: index)
    next.insert(focused, at: destination)
    reorder(to: next, keepingFocusOn: focused)
  }

  /// Change the stack's order, leaving each slot the height it had, and
  /// keep the focused pane focused wherever it landed.
  private func reorder(
    to next: [EVDocumentContentViewController],
    keepingFocusOn focused: EVDocumentContentViewController
  ) {
    let heights = panes.map(\.view.frame.height)
    panes = next
    updateStatusDragging()
    applyHeights(heights)
    if let index = panes.firstIndex(where: { $0 === focused }) {
      preferredIndex = index
      view.window?.makeFirstResponder(panes[index].editorSurface.viewController.view)
    }
    layoutPanes()
  }

  private func closeOthers() {
    let focused = activePane
    for pane in panes where pane !== focused { remove(pane) }
  }

  private func activeRowHeight() -> CGFloat {
    let height = activePane.editorSurface.defaultLineHeight ?? 16
    return height.isFinite && height > 0 ? height : 16
  }

  private func resizeActive(byRows rows: Int) {
    guard panes.count > 1, rows != 0 else { return }
    let current = activeIndex
    var heights = panes.map(\.view.frame.height)
    let delta = CGFloat(rows) * activeRowHeight()
    let minimums = minimumPaneHeights()
    let target = max(minimums[current], heights[current] + delta)
    var remaining = target - heights[current]
    guard remaining != 0 else { return }
    // Take from, or give back to, the neighbours nearest the focused pane.
    let order = neighbourOrder(around: current)
    for index in order where remaining != 0 {
      let available = remaining > 0 ? heights[index] - minimums[index] : .greatestFiniteMagnitude
      let change = remaining > 0 ? min(remaining, max(0, available)) : remaining
      heights[index] -= change
      heights[current] += change
      remaining -= change
    }
    applyHeights(heights)
  }

  private func setActiveHeight(rows: Int?) {
    guard panes.count > 1 else { return }
    let current = activeIndex
    let minimums = minimumPaneHeights()
    let total = max(0, stack.bounds.height)
    let othersMinimum = minimums.enumerated().reduce(CGFloat(0)) { result, entry in
      result + (entry.offset == current ? 0 : entry.element)
    }
    let largest = max(minimums[current], total - othersMinimum)
    // A row count describes the editor area, excluding its status bar.
    let pane = panes[current]
    let target = rows.map { max(minimums[current], CGFloat($0) * activeRowHeight() + pane.statusBarHeight) } ?? largest
    var heights = panes.map(\.view.frame.height)
    let delta = min(target, largest) - heights[current]
    guard delta != 0 else { return }
    var remaining = delta
    for index in neighbourOrder(around: current) where remaining != 0 {
      let available = remaining > 0 ? heights[index] - minimums[index] : .greatestFiniteMagnitude
      let change = remaining > 0 ? min(remaining, max(0, available)) : remaining
      heights[index] -= change
      heights[current] += change
      remaining -= change
    }
    applyHeights(heights)
  }

  /// Neighbours nearest the focused pane first, below before above, so a grow
  /// command takes space from the pane a user is looking at next.
  private func neighbourOrder(around index: Int) -> [Int] {
    var order: [Int] = []
    var below = index + 1
    var above = index - 1
    while below < panes.count || above >= 0 {
      if below < panes.count {
        order.append(below)
        below += 1
      }
      if above >= 0 {
        order.append(above)
        above -= 1
      }
    }
    return order
  }

  /// A collapsed editor still leaves its entire status bar visible.
  private func minimumPaneHeights() -> [CGFloat] { panes.map(\.statusBarHeight) }

  func requireSplitRoom(in pane: EVDocumentContentViewController? = nil) throws {
    loadViewIfNeeded()
    layoutStack()
    let source = pane ?? activePane
    guard panes.contains(where: { $0 === source }),
      source.view.frame.height - source.statusBarHeight >= EVStatusBarView.preferredHeight
    else { throw EVDocumentHostError.noRoomToSplit }
  }

  /// Each pointer delta acts on the grabbed bar alone, pushing neighbours only
  /// when their editor gaps close. No pushed group survives a reversal.
  func dragStatusBar(of pane: EVDocumentContentViewController, by delta: CGFloat) {
    guard let index = panes.firstIndex(where: { $0 === pane }), index < panes.count - 1,
      delta.isFinite, delta != 0 else { return }
    layoutStack()
    var next = heights
    let minimums = minimumPaneHeights()
    var remaining = abs(delta)
    let donors = delta > 0 ? Array((index + 1)..<panes.count) : Array((0...index).reversed())
    let receiver = delta > 0 ? index : index + 1
    for donor in donors {
      let change = min(remaining, max(0, next[donor] - minimums[donor]))
      next[donor] -= change
      next[receiver] += change
      remaining -= change
      if remaining <= 0 { break }
    }
    applyHeights(next)
  }

  private func updateStatusDragging() {
    for (index, pane) in panes.enumerated() {
      pane.statusBar.dragDidMove = index == panes.count - 1 ? nil : { [weak self, weak pane] delta in
        guard let pane else { return }
        self?.dragStatusBar(of: pane, by: delta)
      }
      pane.statusBarHeightDidChange = { [weak self] in self?.layoutStack() }
    }
  }

  private func applyHeights(_ next: [CGFloat]) {
    heights = next
    layoutStack()
  }

  override func loadView() {
    stack.autoresizingMask = [.width, .height]
    stack.didResize = { [weak self] in self?.layoutStack() }
    view = stack
    for pane in panes {
      addChild(pane)
      stack.addSubview(pane.view)
    }
    updateStatusDragging()
    distributeEvenly()
  }

  func insert(_ pane: EVDocumentContentViewController, splitting source: EVDocumentContentViewController? = nil) throws {
    let source = source ?? activePane
    try requireSplitRoom(in: source)
    let current = panes.firstIndex(where: { $0 === source }) ?? 0
    let insertion = current + 1
    let textHeight = max(0, heights[current] - panes[current].statusBarHeight - pane.statusBarHeight) / 2
    heights[current] = panes[current].statusBarHeight + textHeight
    heights.insert(pane.statusBarHeight + textHeight, at: insertion)
    panes.insert(pane, at: insertion)
    addChild(pane)
    stack.addSubview(pane.view)
    preferredIndex = insertion
    updateStatusDragging()
    layoutStack()
    view.window?.makeFirstResponder(pane.editorSurface.viewController.view)
  }

  func remove(_ pane: EVDocumentContentViewController) {
    guard panes.count > 1, let index = panes.firstIndex(where: { $0 === pane }) else { return }
    pane.statusBar.dragDidMove = nil
    pane.statusBarHeightDidChange = nil
    pane.view.removeFromSuperview()
    pane.removeFromParent()
    panes.remove(at: index)
    if lastAccessedPane === pane { lastAccessedPane = nil }
    preferredIndex = min(index, panes.count - 1)
    updateStatusDragging()
    distributeEvenly()
    view.window?.makeFirstResponder(activePane.editorSurface.viewController.view)
  }

  func replace(_ old: EVDocumentContentViewController, with next: EVDocumentContentViewController) {
    guard let index = panes.firstIndex(where: { $0 === old }) else { return }
    old.statusBar.dragDidMove = nil
    old.statusBarHeightDidChange = nil
    old.view.removeFromSuperview()
    old.removeFromParent()
    panes[index] = next
    addChild(next)
    stack.addSubview(next.view)
    preferredIndex = index
    updateStatusDragging()
    layoutStack()
    view.window?.makeFirstResponder(next.editorSurface.viewController.view)
  }

  func layoutPanes() { panes.forEach { $0.layoutContent() } }

  private func distributeEvenly() {
    let minimums = minimumPaneHeights()
    let gap = max(0, stack.bounds.height - minimums.reduce(0, +)) / CGFloat(panes.count)
    heights = minimums.map { $0 + gap }
    layoutStack()
  }

  private func layoutStack() {
    guard isViewLoaded, !applyingLayout else { return }
    applyingLayout = true
    defer { applyingLayout = false }
    let minimums = minimumPaneHeights()
    let required = minimums.reduce(0, +)
    if let window = view.window {
      var minimumSize = window.contentMinSize
      minimumSize.height = max(EVDocumentWindowController.minimumContentSize.height, required)
      window.contentMinSize = minimumSize
    }
    // A programmatically undersized frame clips the stack rather than making
    // the bars overlap. Native windows enforce the full chrome minimum above.
    let total = max(required, stack.bounds.height)
    if heights.count != panes.count { heights = minimums }
    let gaps = zip(heights, minimums).map { max(0, $0 - $1) }
    let gapTotal = gaps.reduce(0, +)
    let available = total - required
    if abs(heights.reduce(0, +) - total) > 0.001 || zip(heights, minimums).contains(where: { $0 < $1 }) {
      heights = minimums.enumerated().map { index, minimum in
        minimum + (gapTotal > 0 ? available * gaps[index] / gapTotal : available / CGFloat(panes.count))
      }
    }
    var offset: CGFloat = 0
    for (index, pane) in panes.enumerated() {
      pane.view.frame = NSRect(x: 0, y: offset, width: stack.bounds.width, height: heights[index])
      offset += heights[index]
    }
    layoutPanes()
  }

  override func viewDidLayout() {
    super.viewDidLayout()
    layoutStack()
  }
}

@MainActor
private final class EVPaneStackView: NSView {
  var didResize: (() -> Void)?
  override var isFlipped: Bool { true }
  override func resizeSubviews(withOldSize oldSize: NSSize) { didResize?() }
}
