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

  var activePane: EVDocumentContentViewController {
    if let responder = viewIfLoaded?.window?.firstResponder as? NSView,
      let index = panes.firstIndex(where: { responder.isDescendant(of: $0.view) })
    {
      preferredIndex = index
    }
    return panes[min(preferredIndex, panes.count - 1)]
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
