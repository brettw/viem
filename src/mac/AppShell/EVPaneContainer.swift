import AppKit
import CViemCore

/// Native views follow the shared split tree; buffers retain independent views.
@MainActor
final class EVPaneContainer: NSViewController {
  private(set) var panes: [EVDocumentContentViewController]
  private var preferredIndex = 0
  private let stack = EVPaneStackView()
  private let geometry = EVPaneLayout()
  private var ids: [ObjectIdentifier: UInt64] = [:]
  private var splitters: [UInt64: EVVerticalSplitter] = [:]
  private var applyingLayout = false
  private weak var lastAccessedPane: EVDocumentContentViewController?

  init(first: EVDocumentContentViewController) {
    panes = [first]; ids[ObjectIdentifier(first)] = 1
    super.init(nibName: nil, bundle: nil)
  }
  @available(*, unavailable) required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
  private func id(_ pane: EVDocumentContentViewController) -> UInt64 { ids[ObjectIdentifier(pane)]! }
  var activePane: EVDocumentContentViewController {
    if let responder = viewIfLoaded?.window?.firstResponder as? NSView,
       let index = panes.firstIndex(where: { responder.isDescendant(of: $0.view) }) { noteFocus(movingTo: index) }
    return panes[min(preferredIndex, panes.count - 1)]
  }
  var activeIndex: Int { min(preferredIndex, max(0, panes.count - 1)) }
  private func noteFocus(movingTo index: Int) {
    guard index != preferredIndex else { return }
    if panes.indices.contains(preferredIndex) { lastAccessedPane = panes[preferredIndex] }
    preferredIndex = index
  }
  func focusPane(at index: Int) {
    guard panes.indices.contains(index) else { return }
    noteFocus(movingTo: index)
    view.window?.makeFirstResponder(panes[index].editorSurface.viewController.view)
  }
  private func focus(_ pane: EVDocumentContentViewController) {
    if let index = panes.firstIndex(where: { $0 === pane }) { focusPane(at: index) }
  }
  func perform(_ request: EVWindowRequest) {
    loadViewIfNeeded(); layoutStack()
    let current = activePane, currentID = id(current), index = activeIndex, count = panes.count
    let line = Double(current.editorSurface.defaultLineHeight ?? 16)
    func columnWidth() -> Double { Double(current.editorSurface.defaultColumnWidth ?? 8) }
    let localPoint = current.editorSurface.windowFocusPoint ?? NSPoint(x: current.view.bounds.midX, y: current.view.bounds.midY)
    let point = NSPoint(x: current.view.frame.minX + localPoint.x, y: current.view.frame.minY + localPoint.y)
    @discardableResult
    func action(_ operation: UInt32, _ n: Int = 0, _ value: Double = 0, _ flags: UInt32 = 0) throws -> UInt64 {
      try geometry.action(currentID, operation: operation, count: UInt64(max(0,n)), value: value, flags: flags, point: point)
    }
    var nextID = currentID
    do {
      switch request {
      case let .focusDown(n): nextID = try action(1,0,Double(n))
      case let .focusUp(n): nextID = try action(1,1,Double(n))
      case let .focusLeft(n): nextID = try action(1,2,Double(n))
      case let .focusRight(n): nextID = try action(1,3,Double(n))
      case let .focusNext(n): focusPane(at: n.map { min(max($0-1,0),count-1) } ?? (index+1)%count); return
      case let .focusPrevious(n): focusPane(at: n.map { min(max($0-1,0),count-1) } ?? (index+count-1)%count); return
      case .focusTop: focusPane(at: 0); return
      case .focusBottom: focusPane(at: count-1); return
      case .focusLastAccessed: if let last = lastAccessedPane { focus(last) }; return
      case let .rotateDown(n): try action(2,n)
      case let .rotateUp(n): try action(2,n,0,1)
      case let .exchange(n): try action(3,n ?? 0)
      case .moveToTop: try action(4,1)
      case .moveToBottom: try action(4,0)
      case .moveToLeft: try action(4,2)
      case .moveToRight: try action(4,3)
      case .closeOthers: for pane in panes where pane !== current { remove(pane) }
      case let .grow(n): try action(5,0,Double(n)*line,1)
      case let .shrink(n): try action(5,0,-Double(n)*line,1)
      case let .setHeight(n): try action(5,0,Double(n ?? 0)*line,n == nil ? 2 : 0)
      case let .growWidth(n): try action(5,1,Double(n)*columnWidth(),1)
      case let .shrinkWidth(n): try action(5,1,-Double(n)*columnWidth(),1)
      case let .setWidth(n): try action(5,1,Double(n ?? 0)*columnWidth(),n == nil ? 2 : 0)
      case let .resize(targetIndex, width, size, change):
        if let targetIndex, !panes.indices.contains(targetIndex-1) { current.editorSurface.showDocumentMessage("Invalid window number."); return }
        let target = targetIndex.map { panes[$0-1] } ?? current
        let unit = Double(width ? target.editorSurface.defaultColumnWidth ?? 8 : target.editorSurface.defaultLineHeight ?? 16)
        try geometry.action(id(target),operation:5,count:width ? 1 : 0,value:Double(size ?? 0)*unit*Double(change < 0 ? -1 : 1),flags:size == nil ? 2 : change == 0 ? 0 : 1)
      case .equalizeHeights: try action(6)
      case .equalizeHeightOnly: try action(6,1)
      case .equalizeWidthOnly: try action(6,2)
      }
    } catch { current.editorSurface.showDocumentMessage(error.localizedDescription); return }
    layoutStack()
    if let pane = panes.first(where: { id($0) == nextID }) { focus(pane) }
  }
  func requireSplitRoom(in pane: EVDocumentContentViewController? = nil, vertical: Bool = false) throws {
    loadViewIfNeeded(); layoutStack()
    let source = pane ?? activePane
    guard panes.contains(where: { $0 === source }) else { throw EVDocumentHostError.noRoomToSplit }
    try geometry.requireSplit(id(source), vertical: vertical, height: EVStatusBarView.preferredHeight)
  }
  func dragStatusBar(of pane: EVDocumentContentViewController, by delta: CGFloat) {
    guard panes.contains(where: { $0 === pane }), delta.isFinite else { return }
    layoutStack(); geometry.drag(id(pane), delta: delta); layoutStack()
  }
  override func loadView() {
    stack.autoresizingMask = [.width,.height]; stack.didResize = { [weak self] in self?.layoutStack() }; view = stack
    for pane in panes { addChild(pane); stack.addSubview(pane.view); pane.view.autoresizingMask = []; bind(pane) }
    layoutStack()
  }
  private func bind(_ pane: EVDocumentContentViewController) {
    pane.statusBar.clicked = { [weak self, weak pane] in
      guard let self, let pane, let index = self.panes.firstIndex(where: { $0 === pane }) else { return }
      if let responder = self.view.window?.firstResponder as? NSView, responder.isDescendant(of: pane.statusBar) { self.noteFocus(movingTo: index) }
      else { self.focus(pane) }
    }
    pane.statusBarHeightDidChange = { [weak self] in self?.layoutStack() }
  }
  private func unbind(_ pane: EVDocumentContentViewController) {
    pane.statusBar.dragDidMove = nil; pane.statusBar.clicked = nil; pane.statusBarHeightDidChange = nil
  }
  func insert(_ pane: EVDocumentContentViewController, splitting source: EVDocumentContentViewController? = nil, vertical: Bool = false) throws {
    let source = source ?? activePane
    try requireSplitRoom(in: source, vertical: vertical)
    let newID = try geometry.split(id(source), vertical: vertical, height: pane.statusBarHeight)
    ids[ObjectIdentifier(pane)] = newID; panes.append(pane)
    addChild(pane); stack.addSubview(pane.view); pane.view.autoresizingMask = []; bind(pane); layoutStack(); focus(pane)
  }
  func remove(_ pane: EVDocumentContentViewController) {
    guard panes.count > 1, let index = panes.firstIndex(where: { $0 === pane }) else { return }
    geometry.remove(id(pane)); unbind(pane); pane.view.removeFromSuperview(); pane.removeFromParent()
    ids.removeValue(forKey: ObjectIdentifier(pane)); panes.remove(at: index)
    if lastAccessedPane === pane { lastAccessedPane = nil }
    preferredIndex = min(index,panes.count-1)
    try? geometry.action(id(panes[preferredIndex]),operation:6)
    layoutStack(); focusPane(at: preferredIndex)
  }
  func replace(_ old: EVDocumentContentViewController, with next: EVDocumentContentViewController) {
    guard let index = panes.firstIndex(where: { $0 === old }) else { return }
    ids[ObjectIdentifier(next)] = ids.removeValue(forKey: ObjectIdentifier(old))
    unbind(old); old.view.removeFromSuperview(); old.removeFromParent(); panes[index] = next
    if lastAccessedPane === old { lastAccessedPane = nil }
    addChild(next); stack.addSubview(next.view); next.view.autoresizingMask = []; bind(next); preferredIndex = index; layoutStack(); focus(next)
  }
  func layoutPanes() { layoutStack() }
  private func layoutStack() {
    guard isViewLoaded, !applyingLayout else { return }
    applyingLayout = true; defer { applyingLayout = false }
    let focused = panes[min(preferredIndex,panes.count-1)]
    geometry.update(size:stack.bounds.size,chrome:panes.map { ViemPaneChrome(id:id($0),status_height:$0.statusBarHeight) })
    let snapshot = geometry.snapshot()
    if let window = view.window { window.contentMinSize = NSSize(width:max(480,snapshot.minimum.width),height:max(280,snapshot.minimum.height)) }
    var next: [EVDocumentContentViewController] = []
    var visibleSplitters = Set<UInt64>()
    for frame in snapshot.frames {
      let rect = NSRect(x:frame.x,y:frame.y,width:frame.width,height:frame.height)
      if frame.kind == 1 {
        visibleSplitters.insert(frame.id)
        let splitter = splitters[frame.id] ?? EVVerticalSplitter(frame:rect)
        if splitters[frame.id] == nil {
          splitters[frame.id] = splitter; stack.addSubview(splitter)
          let token = frame.id
          splitter.dragDidMove = { [weak self] delta in guard let self else { return }; self.geometry.drag(token,splitter:true,delta:delta); self.layoutStack() }
        }
        splitter.frame = rect
      } else if let pane = panes.first(where: { id($0) == frame.id }) {
        pane.view.frame = rect; pane.layoutContent(); next.append(pane)
        if frame.flags & 1 == 0 { pane.statusBar.dragDidMove = nil }
        else if pane.statusBar.dragDidMove == nil { pane.statusBar.dragDidMove = { [weak self,weak pane] delta in if let pane { self?.dragStatusBar(of:pane,by:delta) } } }
      }
    }
    for token in Array(splitters.keys) where !visibleSplitters.contains(token) { splitters.removeValue(forKey:token)?.removeFromSuperview() }
    panes = next; preferredIndex = panes.firstIndex(where: { $0 === focused }) ?? 0
  }
  override func viewDidLayout() { super.viewDidLayout(); layoutStack() }
}
@MainActor
private final class EVPaneStackView: NSView {
  var didResize: (() -> Void)?
  override var isFlipped: Bool { true }
  override func resizeSubviews(withOldSize oldSize: NSSize) { didResize?() }
}
