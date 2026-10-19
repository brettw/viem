import AppKit
import CoreText
import XCTest

@testable import ViemAppShell

/// Status line layout: the caret position widget is right-aligned, contents
/// honor the style's padding, and an active command line replaces the
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
        ($0.accessibilityLabel() ?? "").contains("position: Ln 12, Col 34")
      })
  }

  private func modeLabel(_ bar: EVStatusBarView) throws -> NSView {
    try XCTUnwrap(descendants(bar).first { $0.accessibilityLabel() == "Mode: NORMAL" })
  }

  func testStatusInsetsAndHeightUseTheStyleBox() {
    let style = EVThemeStore.shared.chromeStyles.statusBar
    XCTAssertEqual(EVStatusBarView.contentInset, style.padding.left)
    XCTAssertEqual(EVStatusBarView.preferredHeight, ceil(style.fontHeight + style.padding.top + style.padding.bottom
      + style.topBorder.width + style.bottomBorder.width))
  }

  func testThickBordersCenterTextInsidePaddingAndClipOuterEdges() throws {
    let previous = try XCTUnwrap(EVConfigurationStore.shared.styleDefaults(named: "text"))
    defer { try? EVConfigurationStore.shared.saveStyleDefaults(previous, named: "text") }
    try editStatusStyle(character: ["size": 13], block: [
      "padding_top": 3, "padding_bottom": 11, "padding_left": 9, "padding_right": 12,
      "border_top_width": 7, "border_bottom_width": 30,
      "border_left_width": 24, "border_right_width": 28])
    let bar = makeBar()
    let total = EVStatusBarView.preferredHeight
    for bottom in [false, true] {
      bar.hidesBottomBorder = bottom
      bar.frame.size.height = bar.visiblePreferredHeight
      bar.layoutSubtreeIfNeeded()
      XCTAssertEqual(bar.frame.height, total - (bottom ? 30 : 0))
      let expected = (bar.bounds.height + (bottom ? 0 : 30) + 11 - 7 - 3) / 2
      let mode = try XCTUnwrap(try modeLabel(bar) as? NSTextField)
      let textRect = try XCTUnwrap(mode.cell).titleRect(forBounds: mode.bounds)
      XCTAssertEqual(mode.convert(textRect, to: bar).midY, expected, accuracy: 1.5)
      // NSTextField's alignment rect accounts for its horizontal optical inset.
      let alignmentRect = mode.alignmentRect(forFrame: mode.convert(mode.bounds, to: bar))
      XCTAssertEqual(alignmentRect.minX, 9, accuracy: 1)
      let widget = try locationWidget(bar)
      XCTAssertEqual(widget.frame.maxX, bar.bounds.maxX - 12, accuracy: 1)
      bar.apply(EVStatusBarState(location: "Ln 12, Col 34", commandLine:
        EVStatusCommandLine(prompt: ":", text: "write", cursorUTF8Offset: 5), isActive: true))
      bar.layoutSubtreeIfNeeded()
      XCTAssertEqual(try XCTUnwrap(bar.commandCaretRect()).midY, expected, accuracy: 1)
      XCTAssertEqual(bar.commandAreaRect.minX, 0)
      XCTAssertEqual(bar.commandAreaRect.minY, bottom ? 0 : 30)
      bar.apply(EVStatusBarState(mode: "NORMAL", location: "Ln 12, Col 34"))
    }
  }

  func testOversizedCommandFontClipsInsideBarWithoutChangingContainerHeight() throws {
    let previous = try XCTUnwrap(EVConfigurationStore.shared.styleDefaults(named: "text"))
    defer { try? EVConfigurationStore.shared.saveStyleDefaults(previous, named: "text") }
    try editStatusStyle(character: ["size": 11], block: ["padding_top": 2, "padding_bottom": 3,
      "border_top_width": 4, "border_bottom_width": 2, "margin_top": 80, "alignment": "End"])
    let height = EVStatusBarView.preferredHeight
    let bar = makeBar()
    try editStatusStyle(id: "Status bar command", character: ["size": 70, "letter_spacing": 4])
    bar.apply(EVStatusBarState(commandLine: EVStatusCommandLine(prompt: ":", text: "write", cursorUTF8Offset: 5), isActive: true))
    bar.layoutSubtreeIfNeeded()
    XCTAssertEqual(EVStatusBarView.preferredHeight, height)
    XCTAssertEqual(bar.frame.height, height)
    let caret = try XCTUnwrap(bar.commandCaretRect())
    XCTAssertGreaterThanOrEqual(caret.minY, 2)
    XCTAssertLessThanOrEqual(caret.maxY, bar.bounds.maxY - 4)
    XCTAssertTrue(bar.commandAreaRect.contains(caret))
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

  func testResizeHoverCoversOnlyEmptyMovableBackgroundAndFollowsPresentation() throws {
    let bar = makeBar(width: 900)
    bar.dragDidMove = { _, _ in }
    func resizeCursor(at point: NSPoint) -> Bool { bar.resizeCursorRects.contains { $0.contains(point) } }
    let empty = NSPoint(x: bar.commandAreaRect.maxX - 2, y: bar.bounds.midY)
    XCTAssertTrue(resizeCursor(at: empty))
    for control in [try locationWidget(bar), try modeLabel(bar),
                    try XCTUnwrap(descendants(bar).compactMap { $0 as? NSTextField }.first { $0.stringValue == "Untitled" })] {
      let text = (control as? NSTextField)?.cell?.titleRect(forBounds: control.bounds) ?? control.bounds
      let point = control.convert(NSPoint(x: text.minX + 3, y: text.midY), to: bar)
      XCTAssertFalse(resizeCursor(at: point))
    }
    let mode = try modeLabel(bar)
    XCTAssertTrue(resizeCursor(at: mode.convert(NSPoint(x: mode.bounds.maxX - 2, y: mode.bounds.midY), to: bar)), "empty reserved mode space is background")
    bar.apply(EVStatusBarState(location: "Ln 12, Col 34", commandLine: EVStatusCommandLine(prompt: ":", text: "sp", cursorUTF8Offset: 2)))
    bar.layoutSubtreeIfNeeded()
    XCTAssertFalse(resizeCursor(at: empty), "the command area retains pointer selection")
    bar.apply(EVStatusBarState(location: "Ln 12, Col 34", commandOutput: "Output"))
    bar.layoutSubtreeIfNeeded()
    let text = try XCTUnwrap(bar.outputTextView)
    let output = text.convert(text.bounds, to: bar)
    XCTAssertFalse(resizeCursor(at: NSPoint(x: output.minX + 2, y: output.midY)))
    bar.apply(EVStatusBarState(location: "Ln 12, Col 34"))
    bar.layoutSubtreeIfNeeded()
    XCTAssertTrue(resizeCursor(at: empty))
    bar.dragDidMove = nil
    XCTAssertTrue(bar.resizeCursorRects.isEmpty, "a detached status bar retains normal hover cursors")
  }

  func testStatusPixelsAfterNarrowingAndWideningMatchFreshLayout() throws {
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 150),
                          styleMask: [.titled], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.makeKeyAndOrderFront(nil)
    defer { window.close() }
    let changed = makeBar(width: 900)
    changed.translatesAutoresizingMaskIntoConstraints = true
    changed.autoresizingMask = []
    changed.frame.origin.y = 40
    window.contentView?.addSubview(changed)
    for width in [100.0, 500, 200, 900] {
      changed.frame.size.width = width
      changed.layoutSubtreeIfNeeded()
      RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.01))
    }
    let fresh = makeBar(width: 900)
    fresh.translatesAutoresizingMaskIntoConstraints = true
    fresh.autoresizingMask = []
    fresh.frame.origin.y = 80
    window.contentView?.addSubview(fresh)
    window.contentView?.layoutSubtreeIfNeeded()
    XCTAssertEqual(try locationWidget(changed).frame, try locationWidget(fresh).frame)
    func pixels(_ bar: EVStatusBarView) throws -> Data {
      let bitmap = try XCTUnwrap(bar.bitmapImageRepForCachingDisplay(in: bar.bounds))
      bar.cacheDisplay(in: bar.bounds, to: bitmap)
      return Data(bytes: try XCTUnwrap(bitmap.bitmapData), count: bitmap.bytesPerRow * bitmap.pixelsHigh)
    }
    XCTAssertEqual(try pixels(changed), try pixels(fresh), "resized status text and borders match a fresh render")
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

    // Its text and caret use the status container's leading padding.
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

  func testCommandCaretUsesFullProportionalShapingAndUnicodeScalarPositions() throws {
    let previous = try XCTUnwrap(EVConfigurationStore.shared.styleDefaults(named: "text"))
    defer { try? EVConfigurationStore.shared.saveStyleDefaults(previous, named: "text") }
    let text = "AV office café e\u{301} 🙂"
    let bar = makeBar(width: 900)
    // The same command must invalidate its cached shape when font or tracking
    // changes. Ligature/kerning context comes from the complete command line.
    for (size, spacing) in [(24.0, 1.25), (20, 0)] {
      try editStatusStyle(id: "Status bar command", character: [
        "font_families": ["Georgia"], "size": size, "letter_spacing": spacing,
        "open_type_features": ["liga": 1],
      ])
      let style = EVThemeStore.shared.chromeStyles.statusCommand
      var attributes: [NSAttributedString.Key: Any] = [.font: style.font]
      if spacing != 0 { attributes[.tracking] = spacing; attributes[.ligature] = 1 }
      let line = CTLineCreateWithAttributedString(NSAttributedString(string: ":" + text, attributes: attributes))
      var utf8 = 0
      var utf16 = 1 // The prompt is part of the shaped line.
      for scalar in text.unicodeScalars {
        bar.apply(EVStatusBarState(commandLine: EVStatusCommandLine(
          prompt: ":", text: text, cursorUTF8Offset: utf8), isActive: true))
        bar.layoutSubtreeIfNeeded()
        let caret = try XCTUnwrap(bar.commandCaretRect())
        let expected = CTLineGetOffsetForStringIndex(line, utf16, nil)
        XCTAssertEqual(caret.minX, floor(EVStatusBarView.contentInset + expected), accuracy: 0.01,
          "UTF-8 cursor \(utf8) must map to UTF-16 position \(utf16) in the whole shaped line")
        utf8 += scalar.utf8.count
        utf16 += scalar.utf16.count
      }
      bar.apply(EVStatusBarState(commandLine: EVStatusCommandLine(
        prompt: ":", text: text, cursorUTF8Offset: utf8), isActive: true))
      bar.layoutSubtreeIfNeeded()
      var characterUTF8 = 0
      var characterUTF16 = 1
      for character in text {
        let x = EVStatusBarView.contentInset + CTLineGetOffsetForStringIndex(line, characterUTF16, nil)
        XCTAssertEqual(bar.commandUTF8Offset(at: NSPoint(x: x, y: bar.bounds.midY)), characterUTF8,
          "Pointer selection must use grapheme boundaries")
        let characterText = String(character)
        characterUTF8 += characterText.utf8.count
        characterUTF16 += characterText.utf16.count
      }
    }
  }

  func testOrdinaryStatusAndCommandPromptsDoNotConstructOutputControls() throws {
    let bar = makeBar()
    bar.dragDidMove = { _, _ in }
    for state in [
      EVStatusBarState(mode: "INSERT", message: "Saved", location: "Ln 12, Col 34"),
      EVStatusBarState(commandLine: EVStatusCommandLine(prompt: ":", text: "set", cursorUTF8Offset: 3), isActive: true),
      EVStatusBarState(),
    ] {
      bar.apply(state)
      bar.layoutSubtreeIfNeeded()
      _ = bar.resizeCursorRects
      XCTAssertFalse(bar.isCommandOutputFocused)
      XCTAssertNil(bar.commandOutputPresentation(for: .copy))
      XCTAssertFalse(bar.performCommandOutputAction(.copy))
      XCTAssertNil(bar.outputTextView)
      XCTAssertFalse(descendants(bar).contains { $0 is NSTextView || $0 is NSScrollView })
    }
  }

  func testDeferredOutputUsesCurrentThemeAndPreservesSelectionAcrossStatusUpdates() throws {
    let previousStyles = try XCTUnwrap(EVConfigurationStore.shared.styleDefaults(named: "text"))
    defer { try? EVConfigurationStore.shared.saveStyleDefaults(previousStyles, named: "text") }
    let bar = makeBar()
    let tint = EVThemeColor(0.2, 0.6, 0.8)
    try editStatusStyle(character: ["font_families": ["system-ui"], "size": 15,
      "foreground": ["red": tint.red, "green": tint.green, "blue": tint.blue, "alpha": 1]])
    XCTAssertNil(bar.outputTextView, "a theme change must not initialize hidden output")
    bar.apply(EVStatusBarState(commandOutput: "Selected output", isActive: true))
    bar.layoutSubtreeIfNeeded()
    let text = try XCTUnwrap(bar.outputTextView)
    XCTAssertEqual(text.font?.pointSize, 15)
    XCTAssertEqual(text.textColor, EVThemeStore.shared.chromeStyles.statusBar.foreground)
    text.setSelectedRange(NSRange(location: 0, length: 8))

    bar.apply(EVStatusBarState(location: "Ln 12, Col 34", commandOutput: "Selected output"))
    XCTAssertEqual(text.selectedRange(), NSRange(location: 0, length: 8))
    let location = try XCTUnwrap(try locationWidget(bar) as? NSButton)
    XCTAssertEqual(location.attributedTitle.string, "Ln 12, Col 34")
    XCTAssertEqual((location.attributedTitle.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)?.pointSize, 15)

    try editStatusStyle(character: ["size": 17,
      "foreground": ["red": 0.8, "green": 0.4, "blue": 0.1, "alpha": 1]])
    XCTAssertEqual(text.font?.pointSize, 17)
    XCTAssertEqual(text.textColor, EVThemeStore.shared.chromeStyles.statusBar.foreground)
    XCTAssertEqual((location.attributedTitle.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)?.pointSize, 17)
    XCTAssertEqual(text.selectedRange(), NSRange(location: 0, length: 8))

    bar.apply(EVStatusBarState())
    bar.layoutSubtreeIfNeeded()
    XCTAssertTrue(text.isHiddenOrHasHiddenAncestor)
    bar.apply(EVStatusBarState(commandOutput: "Selected output"))
    bar.layoutSubtreeIfNeeded()
    XCTAssertTrue(bar.outputTextView === text)
    XCTAssertFalse(text.isHiddenOrHasHiddenAncestor)
    XCTAssertEqual(text.selectedRange(), NSRange(location: 0, length: 8))
  }

  func testOutputUsesNormalColorsWithLeftCloseAndNeverCoversPosition() throws {
    for width in [260.0, 600.0] {
      let bar = makeBar(width: width)
      bar.apply(EVStatusBarState(
        location: "Ln 12, Col 34", commandOutput: String(repeating: "long output 🙂 ", count: 40)))
      bar.layoutSubtreeIfNeeded()
      let location = try locationWidget(bar)
      let text = try XCTUnwrap(bar.outputTextView)
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
      XCTAssertEqual(text.textColor, EVThemeStore.shared.chromeStyles.statusBar.foreground)
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
    XCTAssertTrue(try XCTUnwrap(bar.outputTextView).isHiddenOrHasHiddenAncestor)
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
