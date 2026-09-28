import AppKit
import XCTest

@testable import ViemAppShell

/// Status line layout: the caret position widget is right-aligned, contents
/// clear the window's rounded corner, and an active command line replaces the
/// left group in inverse colors.
@MainActor
final class EVStatusLineTests: XCTestCase {
  private func makeBar(width: CGFloat = 600) -> EVStatusBarView {
    let bar = EVStatusBarView()
    bar.frame = NSRect(x: 0, y: 0, width: width, height: EVStatusBarView.preferredHeight)
    bar.apply(EVStatusBarState(mode: "NORMAL", message: "", location: "Ln 12, Col 34"))
    bar.layoutSubtreeIfNeeded()
    return bar
  }

  private func descendants(_ view: NSView) -> [NSView] {
    [view] + view.subviews.flatMap { descendants($0) }
  }

  private func locationWidget(_ bar: EVStatusBarView) throws -> NSView {
    try XCTUnwrap(
      descendants(bar).first {
        ($0.accessibilityLabel() ?? "").contains("lines: Ln 12, Col 34")
      })
  }

  private func modeLabel(_ bar: EVStatusBarView) throws -> NSView {
    try XCTUnwrap(descendants(bar).first { $0.accessibilityLabel() == "Mode: NORMAL" })
  }

  func testCornerInsetIsTheWidthTheCornerClipsByMoreThanOnePoint() {
    // At x = r - sqrt(2r - 1) the corner has removed exactly one point of
    // height, which is the definition this inset uses.
    for radius in [4.0, 10.0, 16.0, 24.0] as [CGFloat] {
      let inset = EVStatusBarView.cornerInset(forRadius: radius)
      let clipped = radius - (radius * radius - (radius - inset) * (radius - inset)).squareRoot()
      XCTAssertEqual(clipped, 1, accuracy: 0.001, "radius \(radius)")
      XCTAssertGreaterThan(inset, 0, "radius \(radius)")
      XCTAssertLessThan(inset, radius, "radius \(radius)")
    }
    // A square or nearly square corner clips nothing worth insetting.
    XCTAssertEqual(EVStatusBarView.cornerInset(forRadius: 0), 0)
    XCTAssertEqual(EVStatusBarView.cornerInset(forRadius: 1), 0)
    XCTAssertGreaterThan(EVStatusBarView.contentInset, 0)
  }

  func testCaretPositionIsRightAlignedAndEverythingElseIsLeftAligned() throws {
    let bar = makeBar()
    bar.apply(EVStatusBarState(mode: "NORMAL", message: "Saved", location: "Ln 12, Col 34"))
    bar.layoutSubtreeIfNeeded()
    let inset = EVStatusBarView.contentInset
    let location = try locationWidget(bar)
    let mode = try modeLabel(bar)
    // Auto Layout aligns by alignment rects, which sit a couple of points
    // inside a text field's frame.
    let locationFrame = location.alignmentRect(
      forFrame: location.convert(location.bounds, to: bar))
    let modeFrame = mode.alignmentRect(forFrame: mode.convert(mode.bounds, to: bar))
    XCTAssertEqual(locationFrame.maxX, bar.bounds.maxX - inset, accuracy: 0.5)
    XCTAssertEqual(modeFrame.minX, inset, accuracy: 0.5)
    XCTAssertLessThan(modeFrame.maxX, locationFrame.minX)
    let message = try XCTUnwrap(
      descendants(bar).compactMap { $0 as? NSTextField }.first { $0.stringValue == "Saved" })
    let messageFrame = message.convert(message.bounds, to: bar)
    XCTAssertLessThan(messageFrame.maxX, locationFrame.minX)
    XCTAssertGreaterThan(messageFrame.minX, modeFrame.maxX)
  }

  func testACommandLineReplacesTheLeftGroupAndKeepsTheCaretWidget() throws {
    let bar = makeBar()
    let mode = try modeLabel(bar)
    XCTAssertFalse(mode.isHiddenOrHasHiddenAncestor)
    bar.apply(
      EVStatusBarState(
        mode: "NORMAL", location: "Ln 12, Col 34",
        commandLine: EVStatusCommandLine(prompt: ":", text: "%s/foo/bar", cursorUTF8Offset: 10),
        isActive: true))
    bar.layoutSubtreeIfNeeded()
    XCTAssertTrue(mode.isHiddenOrHasHiddenAncestor, "the left group gives way to the command line")
    let location = try locationWidget(bar)
    XCTAssertFalse(location.isHiddenOrHasHiddenAncestor, "the caret widget always remains")
    let locationFrame = location.alignmentRect(
      forFrame: location.convert(location.bounds, to: bar))
    XCTAssertEqual(
      locationFrame.maxX, bar.bounds.maxX - EVStatusBarView.contentInset, accuracy: 0.5)

    // The command background reaches the leading window edge and stops five
    // points short of the caret position widget.
    let area = bar.commandAreaRect
    XCTAssertEqual(area.minX, 0)
    XCTAssertEqual(area.maxX, location.frame.minX - 5, accuracy: 0.5)

    // Its text and caret are inset by the corner amount.
    let caret = try XCTUnwrap(bar.commandCaretRect())
    XCTAssertGreaterThanOrEqual(caret.minX, EVStatusBarView.contentInset)
    XCTAssertLessThan(caret.maxX, area.maxX)
    bar.apply(EVStatusBarState(mode: "NORMAL", location: "Ln 12, Col 34"))
    bar.layoutSubtreeIfNeeded()
    XCTAssertFalse(mode.isHiddenOrHasHiddenAncestor, "leaving the command line restores it")
    XCTAssertNil(bar.commandCaretRect())
  }

  func testTheCommandCaretTracksItsOffsetAndOnlyBlinksWhenActive() throws {
    let bar = makeBar()
    func caretX(cursor: Int, active: Bool = true) -> CGFloat? {
      bar.apply(
        EVStatusBarState(
          location: "Ln 12, Col 34",
          commandLine: EVStatusCommandLine(
            prompt: ":", text: "%s/foo/bar", cursorUTF8Offset: cursor),
          isActive: active))
      bar.layoutSubtreeIfNeeded()
      return bar.commandCaretRect()?.minX
    }
    let start = try XCTUnwrap(caretX(cursor: 0))
    let middle = try XCTUnwrap(caretX(cursor: 5))
    let end = try XCTUnwrap(caretX(cursor: 10))
    XCTAssertLessThan(start, middle)
    XCTAssertLessThan(middle, end)
    // The caret sits after the prompt, which is drawn at the inset.
    XCTAssertGreaterThan(start, EVStatusBarView.contentInset)
    XCTAssertNil(caretX(cursor: 5, active: false), "an inactive pane outlines instead")
  }

  func testClickingTheCommandAreaReportsTheNearestUTF8Offset() throws {
    let bar = makeBar()
    var reported: [(Int, Bool)] = []
    bar.commandLineDidSelect = { reported.append(($0, $1)) }
    bar.apply(
      EVStatusBarState(
        location: "Ln 12, Col 34",
        commandLine: EVStatusCommandLine(prompt: ":", text: "abcdef", cursorUTF8Offset: 6),
        isActive: true))
    bar.layoutSubtreeIfNeeded()
    let start = NSPoint(x: EVStatusBarView.contentInset, y: bar.bounds.midY)
    XCTAssertEqual(bar.commandUTF8Offset(at: start), 0)
    let caret = try XCTUnwrap(bar.commandCaretRect())
    XCTAssertEqual(bar.commandUTF8Offset(at: NSPoint(x: caret.midX, y: bar.bounds.midY)), 6)

    let window = NSWindow(
      contentRect: bar.bounds, styleMask: [.titled], backing: .buffered, defer: false)
    window.contentView?.addSubview(bar)
    let point = bar.convert(start, to: nil)
    let event = try XCTUnwrap(
      NSEvent.mouseEvent(
        with: .leftMouseDown, location: point, modifierFlags: [], timestamp: 0,
        windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
    bar.mouseDown(with: event)
    XCTAssertEqual(reported.count, 1)
    XCTAssertEqual(reported.first?.0, 0)
    XCTAssertEqual(reported.first?.1, false)
  }

  func testMultibyteCommandTextMeasuresByUTF8Offsets() throws {
    let bar = makeBar()
    bar.apply(
      EVStatusBarState(
        location: "Ln 12, Col 34",
        commandLine: EVStatusCommandLine(
          prompt: ":", text: "café", cursorUTF8Offset: "café".utf8.count),
        isActive: true))
    bar.layoutSubtreeIfNeeded()
    let caret = try XCTUnwrap(bar.commandCaretRect())
    XCTAssertGreaterThan(caret.minX, EVStatusBarView.contentInset)
    // A caret offset inside the accented cluster must not measure past its end.
    bar.apply(
      EVStatusBarState(
        location: "Ln 12, Col 34",
        commandLine: EVStatusCommandLine(prompt: ":", text: "café", cursorUTF8Offset: 3),
        isActive: true))
    bar.layoutSubtreeIfNeeded()
    let inside = try XCTUnwrap(bar.commandCaretRect())
    XCTAssertLessThan(inside.minX, caret.minX)
  }

  func testOutputUsesNormalColorsWithLeftCloseAndNeverCoversPosition() throws {
    for width in [260.0, 600.0] {
      let bar = makeBar(width: width)
      bar.apply(EVStatusBarState(
        location: "Ln 12, Col 34", commandOutput: String(repeating: "long output 🙂 ", count: 40)))
      bar.layoutSubtreeIfNeeded()
      let location = try locationWidget(bar)
      let text = bar.outputTextView
      let scroll = try XCTUnwrap(text.enclosingScrollView)
      let close = try XCTUnwrap(bar.subviews.compactMap { $0 as? NSButton }
        .first { $0.accessibilityLabel() == "Close command output" })
      XCTAssertTrue(try modeLabel(bar).isHiddenOrHasHiddenAncestor)
      XCTAssertFalse(location.isHiddenOrHasHiddenAncestor)
      XCTAssertFalse(text.isHiddenOrHasHiddenAncestor)
      XCTAssertFalse(text.isEditable)
      XCTAssertTrue(text.isSelectable)
      XCTAssertLessThan(close.frame.maxX, scroll.frame.minX)
      XCTAssertEqual(close.frame.minX, EVStatusBarView.contentInset, accuracy: 0.5)
      XCTAssertLessThanOrEqual(scroll.frame.maxX, bar.commandAreaRect.maxX)
      XCTAssertLessThan(scroll.frame.maxX, location.frame.minX)
      XCTAssertEqual(bar.frame.height, EVStatusBarView.preferredHeight)
      XCTAssertEqual(text.textColor, EVThemeStore.shared.theme.statusForeground.color)
      XCTAssertFalse(bar.isCommandCaretShowing)
      var dismissed = false
      bar.commandOutputDidDismiss = { dismissed = true }
      close.performClick(nil)
      XCTAssertTrue(dismissed)
      bar.apply(EVStatusBarState(location: "Ln 12, Col 34"))
      XCTAssertTrue(text.isHiddenOrHasHiddenAncestor)
    }
  }

  func testCommandPromptTakesPriorityOverOutput() throws {
    let bar = makeBar()
    bar.apply(EVStatusBarState(
      commandLine: EVStatusCommandLine(prompt: ":", text: "w", cursorUTF8Offset: 1),
      commandOutput: "previous output", isActive: true))
    bar.layoutSubtreeIfNeeded()
    XCTAssertTrue(bar.outputTextView.isHiddenOrHasHiddenAncestor)
    XCTAssertNotNil(bar.commandCaretRect())
  }
}

/// A hidden status line still has to appear while a command line is active,
/// because the command line no longer covers the document.
@MainActor
final class EVStatusLineVisibilityTests: XCTestCase {
  private final class EditorTestView: NSView {
    override var acceptsFirstResponder: Bool { true }
  }
  private final class Surface: EVEditorSurface {
    let viewController: NSViewController = {
      let controller = NSViewController()
      controller.view = EditorTestView(frame: NSRect(x: 0, y: 0, width: 600, height: 400))
      return controller
    }()
    var statusBarState = EVStatusBarState()
    var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
    func perform(menuCommand _: EVMenuCommand, sender _: Any?) {}
    func presentation(for _: EVMenuCommand) -> EVMenuItemPresentation { .enabled }
    func publish(_ state: EVStatusBarState) {
      statusBarState = state
      statusBarStateDidChange?(state)
    }
  }

  func testAHiddenStatusLineAppearsForACommandLineAndHidesAgain() throws {
    let surface = Surface()
    let pane = EVDocumentContentViewController(editorSurface: surface)
    pane.loadViewIfNeeded()
    pane.view.frame = NSRect(x: 0, y: 0, width: 600, height: 400)
    pane.layoutContent()
    let statusBar = try XCTUnwrap(
      pane.view.subviews.compactMap { $0 as? EVStatusBarView }.first)
    if !statusBar.isHidden { pane.toggleStatusBar(nil) }
    XCTAssertTrue(statusBar.isHidden)
    let editorHeight = surface.viewController.view.frame.height

    surface.publish(
      EVStatusBarState(
        commandLine: EVStatusCommandLine(prompt: ":", text: "w", cursorUTF8Offset: 1),
        isActive: true))
    XCTAssertFalse(statusBar.isHidden, "the command line needs somewhere to appear")
    XCTAssertLessThan(surface.viewController.view.frame.height, editorHeight)

    surface.publish(EVStatusBarState())
    XCTAssertTrue(statusBar.isHidden, "it hides again when the command line ends")
    XCTAssertEqual(surface.viewController.view.frame.height, editorHeight, accuracy: 0.5)
    surface.publish(EVStatusBarState(commandOutput: "file written"))
    XCTAssertFalse(statusBar.isHidden)
    XCTAssertEqual(surface.viewController.view.frame.height,
      editorHeight - EVStatusBarView.preferredHeight, accuracy: 0.5)
    surface.publish(EVStatusBarState())
    XCTAssertTrue(statusBar.isHidden)
    XCTAssertEqual(surface.viewController.view.frame.height, editorHeight, accuracy: 0.5)
    pane.toggleStatusBar(nil)
    XCTAssertFalse(statusBar.isHidden)
  }
}
