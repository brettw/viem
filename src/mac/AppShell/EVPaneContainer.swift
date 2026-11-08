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
  /// The narrowest width that fits every pane and splitter, as last laid out.
  private(set) var minimumContentWidth: CGFloat = 0
  private var applyingLayout = false
  private var adjustingGutterWindow = false
  private var placementGutters: [ObjectIdentifier: CGFloat] = [:]
  var placementGutterWidth: CGFloat {
    guard let first = panes.first else { return 0 }
    return placementGutters[ObjectIdentifier(first)] ?? first.editorSurface.lineNumberGutterWidth
  }
  private weak var lastAccessedPane: EVDocumentContentViewController?
  var splitForStatusDrag: ((EVDocumentContentViewController, CGFloat) throws -> EVDocumentContentViewController)?
  /// A status bar's Close pane command.
  var closePaneRequest: ((EVDocumentContentViewController) -> Void)?
  /// Panes were added, removed or preview-closed outside a window command.
  var panesDidChange: (() -> Void)?
  private weak var statusDragTarget: EVDocumentContentViewController?
  private var statusDragNeedsSplit = false
  /// Pointer offset below the dragged bar's top edge, retained for the gesture.
  private var statusDragOffset: CGFloat = 0
  private weak var statusDragFocus: EVDocumentContentViewController?
  /// Views a status drag has closed, retained until the drag ends so that
  /// dragging back restores them unchanged.
  private(set) var previewClosedPanes: [EVDocumentContentViewController] = []

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
    NotificationCenter.default.post(name: .viemActiveEditorSurfaceDidChange,
                                    object: panes[index].editorSurface)
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
    finishStatusDrag()
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
  /// With `closing`, overlapping views of the same buffer below the bar close
  /// until the drag ends.
  func dragStatusBar(of pane: EVDocumentContentViewController, by delta: CGFloat, closing: Bool = false) {
    guard panes.contains(where: { $0 === pane }), delta.isFinite else { return }
    layoutStack()
    // Only further views of the same buffer may close, so a drag never
    // closes a document.
    let closable = !closing ? [] : (panes + previewClosedPanes).filter { $0 !== pane && $0.document != nil && $0.document === pane.document }
      .compactMap { ids[ObjectIdentifier($0)] }
    geometry.drag(id(pane), delta: delta, closable: closable)
    synchronizePreviewClosedPanes()
    layoutStack()
  }
  private func statusBarTop(of pane: EVDocumentContentViewController) -> CGFloat {
    pane.view.frame.maxY - pane.statusBarHeight
  }
  /// Hide views whose panes the drag closed and show those it restored.
  private func synchronizePreviewClosedPanes() {
    let visible = Set(geometry.snapshot().frames.filter { $0.kind == 0 }.map(\.id))
    let hidden = panes.filter { !visible.contains(id($0)) }
    let restored = previewClosedPanes.filter { visible.contains(id($0)) }
    guard !hidden.isEmpty || !restored.isEmpty else { return }
    let focus = statusDragFocus ?? panes[activeIndex]
    for pane in hidden { pane.view.isHidden = true }
    for pane in restored { pane.view.isHidden = false }
    previewClosedPanes = previewClosedPanes.filter { !visible.contains(id($0)) } + hidden
    panes = panes.filter { visible.contains(id($0)) } + restored
    preferredIndex = panes.firstIndex(where: { $0 === focus })
      ?? panes.firstIndex(where: { $0 === statusDragTarget }) ?? 0
    panesDidChange?()
  }
  /// Release ends a status drag, committing the views it closed.
  func finishStatusDrag() {
    let target = statusDragTarget, focus = statusDragFocus
    statusDragTarget = nil; statusDragFocus = nil
    statusDragNeedsSplit = false
    geometry.endDrag()
    guard !previewClosedPanes.isEmpty else { return }
    let closed = previewClosedPanes
    previewClosedPanes = []
    for pane in closed {
      unbind(pane); pane.view.removeFromSuperview(); pane.removeFromParent()
      ids.removeValue(forKey: ObjectIdentifier(pane))
      if lastAccessedPane === pane { lastAccessedPane = nil }
    }
    layoutStack()
    // A closed focused view hands focus to the view whose bar was dragged.
    if let focus, closed.contains(where: { $0 === focus }),
       let index = panes.firstIndex(where: { $0 === target }) ?? panes.indices.last { focusPane(at: index) }
    panesDidChange?()
  }
  override func loadView() {
    stack.autoresizingMask = [.width,.height]; stack.didResize = { [weak self] in self?.layoutStack() }; view = stack
    for pane in panes { addChild(pane); stack.addSubview(pane.view); pane.view.autoresizingMask = []; bind(pane) }
    layoutStack()
  }
  private func bind(_ pane: EVDocumentContentViewController) {
    pane.statusBar.dragDidBegin = { [weak self, weak pane] press in
      guard let self, let pane else { return }
      self.finishStatusDrag()
      self.layoutStack()
      self.statusDragTarget = pane
      self.statusDragFocus = self.panes[self.activeIndex]
      self.statusDragOffset = self.stack.convert(press, from: nil).y - self.statusBarTop(of: pane)
      self.statusDragNeedsSplit = self.geometry.snapshot().frames.contains {
        $0.kind == 0 && $0.id == self.id(pane) && $0.flags & UInt32(VIEM_PANE_FRAME_STATUS_DRAGGABLE) == 0
      }
    }
    pane.statusBar.dragDidEnd = { [weak self] in self?.finishStatusDrag() }
    pane.statusBar.dragDidMove = { [weak self, weak pane] point in
      guard let self, let pane, let target = self.statusDragTarget,
            self.panes.contains(where: { $0 === target }) else { return }
      let y = self.stack.convert(point, from: nil).y
      if self.statusDragNeedsSplit {
        guard y < self.statusBarTop(of: pane) else { return }
        // Attempt once per gesture. The new view opens below the dragged bar,
        // so the rest of the gesture is an ordinary drag that can close it.
        self.statusDragNeedsSplit = false
        do {
          _ = try self.splitForStatusDrag?(pane, y - self.statusDragOffset + pane.statusBarHeight)
        } catch { pane.editorSurface.showDocumentMessage(error.localizedDescription) }
      } else {
        // Track the pointer absolutely: blocked movement past an edge is not
        // retained, and the bar resumes only once the pointer returns to it.
        self.layoutStack()
        self.dragStatusBar(of: target, by: y - self.statusDragOffset - self.statusBarTop(of: target), closing: true)
      }
    }
    pane.statusBar.closePane = { [weak self, weak pane] in
      guard let self, let pane else { return }
      self.closePaneRequest?(pane)
    }
    pane.statusBar.clicked = { [weak self, weak pane] in
      guard let self, let pane, let index = self.panes.firstIndex(where: { $0 === pane }) else { return }
      if let responder = self.view.window?.firstResponder as? NSView, responder.isDescendant(of: pane.statusBar) { self.noteFocus(movingTo: index) }
      else { self.focus(pane) }
    }
    pane.statusBarHeightDidChange = { [weak self] in self?.layoutStack() }
    pane.editorSurface.lineNumberGutterWidthWillChange = { [weak self, weak pane] old, new in
      guard let self, let pane else { return }
      // Native resize notifications can arrive before the editor publishes
      // its new gutter. Save using the incoming width.
      self.placementGutters[ObjectIdentifier(pane)] = new
      self.reserveGutterSpace(in: pane, delta: new - old)
    }
  }
  private func unbind(_ pane: EVDocumentContentViewController) {
    placementGutters.removeValue(forKey: ObjectIdentifier(pane))
    pane.statusBar.dragDidMove = nil; pane.statusBar.clicked = nil; pane.statusBarHeightDidChange = nil
    pane.statusBar.dragDidBegin = nil; pane.statusBar.dragDidEnd = nil; pane.statusBar.closePane = nil
    pane.editorSurface.lineNumberGutterWidthWillChange = nil
  }
  private func reserveGutterSpace(in pane: EVDocumentContentViewController, delta: CGFloat) {
    guard !adjustingGutterWindow, panes.contains(where: { $0 === pane }), delta != 0,
          let window = view.window, !window.isZoomed, !window.styleMask.contains(.fullScreen),
          (window.tabGroup?.windows.count ?? 1) == 1,
          Set(panes.compactMap(\.document).map(ObjectIdentifier.init)).count <= 1,
          !geometry.snapshot().frames.contains(where: { $0.kind == 1 }),
          let screen = window.screen?.visibleFrame else { return }
    adjustingGutterWindow = true; defer { adjustingGutterWindow = false }
    let next = EVDocumentWindowPlacement.frameForGutterChange(window.frame, delta: delta,
      screen: screen, minimumWidth: max(window.minSize.width,
        window.frameRect(forContentRect: NSRect(origin: .zero, size: window.contentMinSize)).width))
    if next != window.frame { window.setFrame(next, display: true); layoutStack() }
  }
  func insert(_ pane: EVDocumentContentViewController, splitting source: EVDocumentContentViewController? = nil, vertical: Bool = false, before: Bool = false, focusNewPane: Bool = true, initialStatusBarBottom: CGFloat? = nil) throws {
    // A drag-created split continues its own gesture; other insertions end one.
    if initialStatusBarBottom == nil { finishStatusDrag() }
    let source = source ?? activePane
    try requireSplitRoom(in: source, vertical: vertical)
    pane.loadViewIfNeeded()
    let newID = try geometry.split(id(source), vertical: vertical, height: pane.statusBarHeight, before: before)
    // A drag-created split starts at the pointer, not the default half split.
    // Resolve this in portable geometry before either editor sees a new size.
    let upper = before ? newID : id(source)
    if let bottom = initialStatusBarBottom,
       let frame = geometry.snapshot().frames.first(where: { $0.kind == 0 && $0.id == upper }) {
      geometry.drag(upper, delta: bottom - (frame.y + frame.height))
    }
    ids[ObjectIdentifier(pane)] = newID; panes.append(pane)
    addChild(pane); pane.view.autoresizingMask = []; bind(pane); layoutStack()
    // Attaching invokes AppKit presentation callbacks. Prepare the detached
    // editor at its final size first so it cannot flash empty or half-sized.
    stack.addSubview(pane.view)
    if focusNewPane { focus(pane) }
  }
  func remove(_ pane: EVDocumentContentViewController) {
    finishStatusDrag()
    guard panes.count > 1, let index = panes.firstIndex(where: { $0 === pane }) else { return }
    geometry.remove(id(pane)); unbind(pane); pane.view.removeFromSuperview(); pane.removeFromParent()
    ids.removeValue(forKey: ObjectIdentifier(pane)); panes.remove(at: index)
    if lastAccessedPane === pane { lastAccessedPane = nil }
    preferredIndex = min(index,panes.count-1)
    _ = try? geometry.action(id(panes[preferredIndex]),operation:6)
    layoutStack(); focusPane(at: preferredIndex)
  }
  func replace(_ old: EVDocumentContentViewController, with next: EVDocumentContentViewController) {
    finishStatusDrag()
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
    for frame in geometry.snapshot().frames where frame.kind == 0 {
      panes.first(where: { id($0) == frame.id })?.statusBar.hidesBottomBorder = frame.flags & UInt32(VIEM_PANE_FRAME_STATUS_DRAGGABLE) == 0
    }
    geometry.update(size:stack.bounds.size,chrome:panes.map { ViemPaneChrome(id:id($0),status_height:$0.statusBarHeight) })
    let snapshot = geometry.snapshot()
    minimumContentWidth = snapshot.minimum.width
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
        splitter.updateBorderVisibility(frame: frame, containerHeight: stack.bounds.height)
      } else if let pane = panes.first(where: { id($0) == frame.id }) {
        pane.view.frame = rect; pane.layoutContent(); next.append(pane)
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
