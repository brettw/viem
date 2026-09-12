import AppKit

/// Only stacked panes are supported. Every child owns its status bar and a
/// distinct core view; documents/backends can be shared across any children.
@MainActor
final class EVPaneContainer: NSViewController, NSSplitViewDelegate {
  private(set) var panes: [EVDocumentContentViewController]
  private var preferredIndex = 0
  private let split = NSSplitView()

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
      focusPane(at: min(current + step, count - 1))
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

  /// Rebuild the split view's order, leaving each slot the height it had, and
  /// keep the focused pane focused wherever it landed.
  private func reorder(
    to next: [EVDocumentContentViewController],
    keepingFocusOn focused: EVDocumentContentViewController
  ) {
    let heights = panes.map(\.view.frame.height)
    // A split view keeps its own arranged list, so dropping the subview alone
    // leaves the old order in place and re-adding it does nothing.
    for pane in panes {
      split.removeArrangedSubview(pane.view)
      pane.view.removeFromSuperview()
    }
    panes = next
    for pane in panes { split.addArrangedSubview(pane.view) }
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

  /// One visual row of the focused pane, used by the height commands. Falls
  /// back to the status bar's height when no row has been laid out yet.
  private func activeRowHeight() -> CGFloat {
    max(1, activePane.editorSurface.visualRowHeight ?? EVStatusBarView.preferredHeight)
  }

  private func resizeActive(byRows rows: Int) {
    guard panes.count > 1, rows != 0 else { return }
    let current = activeIndex
    var heights = panes.map(\.view.frame.height)
    let delta = CGFloat(rows) * activeRowHeight()
    let minimum = minimumPaneHeight()
    let target = max(minimum, heights[current] + delta)
    var remaining = target - heights[current]
    guard remaining != 0 else { return }
    // Take from, or give back to, the neighbours nearest the focused pane.
    let order = neighbourOrder(around: current)
    for index in order where remaining != 0 {
      let available = remaining > 0 ? heights[index] - minimum : .greatestFiniteMagnitude
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
    let minimum = minimumPaneHeight()
    let total = split.bounds.height - split.dividerThickness * CGFloat(panes.count - 1)
    let largest = max(minimum, total - minimum * CGFloat(panes.count - 1))
    let target = rows.map { max(minimum, CGFloat($0) * activeRowHeight()) } ?? largest
    resizeActive(byRows: 0)
    var heights = panes.map(\.view.frame.height)
    let delta = min(target, largest) - heights[current]
    guard delta != 0 else { return }
    var remaining = delta
    for index in neighbourOrder(around: current) where remaining != 0 {
      let available = remaining > 0 ? heights[index] - minimum : .greatestFiniteMagnitude
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

  private func minimumPaneHeight() -> CGFloat { EVStatusBarView.preferredHeight + 1 }

  /// Lay out exact pane heights top to bottom. Divider positions are measured
  /// from the split view's top edge.
  private func applyHeights(_ heights: [CGFloat]) {
    guard heights.count == panes.count, panes.count > 1 else {
      split.adjustSubviews()
      layoutPanes()
      return
    }
    // Re-added arranged subviews keep stale frames until the split view lays
    // out, and divider positions only stick afterwards.
    split.adjustSubviews()
    split.layoutSubtreeIfNeeded()
    var offset: CGFloat = 0
    for index in 0 ..< (panes.count - 1) {
      offset += heights[index]
      split.setPosition(offset, ofDividerAt: index)
      offset += split.dividerThickness
    }
    split.layoutSubtreeIfNeeded()
    layoutPanes()
  }

  override func loadView() {
    split.isVertical = false
    split.dividerStyle = .thin
    split.delegate = self
    split.autoresizingMask = [.width, .height]
    view = split
    for pane in panes {
      addChild(pane)
      split.addArrangedSubview(pane.view)
    }
    split.adjustSubviews()
  }

  func insert(_ pane: EVDocumentContentViewController) {
    loadViewIfNeeded()
    let insertion = (panes.firstIndex(where: { $0 === activePane }) ?? 0) + 1
    panes.insert(pane, at: insertion)
    addChild(pane)
    split.insertArrangedSubview(pane.view, at: insertion)
    preferredIndex = insertion
    distributeEvenly()
    layoutPanes()
    view.window?.makeFirstResponder(pane.editorSurface.viewController.view)
  }

  func remove(_ pane: EVDocumentContentViewController) {
    guard panes.count > 1, let index = panes.firstIndex(where: { $0 === pane }) else { return }
    pane.view.removeFromSuperview()
    pane.removeFromParent()
    panes.remove(at: index)
    if lastAccessedPane === pane { lastAccessedPane = nil }
    preferredIndex = min(index, panes.count - 1)
    distributeEvenly()
    layoutPanes()
    view.window?.makeFirstResponder(activePane.editorSurface.viewController.view)
  }

  func replace(_ old: EVDocumentContentViewController, with next: EVDocumentContentViewController) {
    guard let index = panes.firstIndex(where: { $0 === old }) else { return }
    let frame = old.view.frame
    old.view.removeFromSuperview()
    old.removeFromParent()
    panes[index] = next
    addChild(next)
    split.insertArrangedSubview(next.view, at: index)
    next.view.frame = frame
    preferredIndex = index
    split.adjustSubviews()
    layoutPanes()
    view.window?.makeFirstResponder(next.editorSurface.viewController.view)
  }

  func layoutPanes() { panes.forEach { $0.layoutContent() } }
  private func distributeEvenly() {
    let height = max(
      0,
      (split.bounds.height - split.dividerThickness * CGFloat(panes.count - 1))
        / CGFloat(panes.count))
    for (index, pane) in panes.enumerated() {
      pane.view.frame = NSRect(
        x: 0, y: CGFloat(index) * (height + split.dividerThickness), width: split.bounds.width,
        height: height)
    }
    split.adjustSubviews()
  }
  override func viewDidLayout() {
    super.viewDidLayout()
    layoutPanes()
  }
  func splitViewDidResizeSubviews(_ notification: Notification) { layoutPanes() }
  func splitView(
    _ splitView: NSSplitView, constrainMinCoordinate proposedMinimumPosition: CGFloat,
    ofSubviewAt dividerIndex: Int
  ) -> CGFloat {
    proposedMinimumPosition + 100
  }
  func splitView(
    _ splitView: NSSplitView, constrainMaxCoordinate proposedMaximumPosition: CGFloat,
    ofSubviewAt dividerIndex: Int
  ) -> CGFloat {
    proposedMaximumPosition - 100
  }
}
