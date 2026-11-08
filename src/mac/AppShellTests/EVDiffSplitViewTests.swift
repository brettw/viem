import AppKit
import XCTest

@testable import ViemAppShell

/// The diff window's two sides share the pane splitter, which moves the
/// boundary between them within both sides' minimum widths.
@MainActor
final class EVDiffSplitViewTests: XCTestCase {
  private func split(width: CGFloat = 805) -> EVDiffSplitView {
    let view = EVDiffSplitView(base: NSView(), document: NSView())
    view.frame = NSRect(x: 0, y: 0, width: width, height: 300)
    view.layoutSubtreeIfNeeded()
    return view
  }

  func testThePaneSplitterDividesTheSidesEvenly() {
    let view = split()
    XCTAssertEqual(view.base.frame, NSRect(x: 0, y: 0, width: 400, height: 300))
    XCTAssertEqual(view.splitter.frame, NSRect(x: 400, y: 0, width: 5, height: 300))
    XCTAssertEqual(view.document.frame, NSRect(x: 405, y: 0, width: 400, height: 300))
    XCTAssertEqual(view.splitter.accessibilityRole(), .splitter)
  }

  func testDraggingTheSplitterMovesTheBoundaryWithinBothMinimums() {
    let view = split()
    var reported: [CGFloat] = []
    view.baseFractionDidChange = { reported.append($0) }
    view.splitter.dragDidMove?(150)
    XCTAssertEqual(view.base.frame.width, 550)
    XCTAssertEqual(view.document.frame, NSRect(x: 555, y: 0, width: 250, height: 300))
    view.splitter.dragDidMove?(1000)
    XCTAssertEqual(view.document.frame.width, EVDiffSplitView.minimumWidth)
    // Blocked overshoot is discarded, so reversing moves at once.
    view.splitter.dragDidMove?(-10)
    XCTAssertEqual(view.document.frame.width, EVDiffSplitView.minimumWidth + 10)
    view.splitter.dragDidMove?(-2000)
    XCTAssertEqual(view.base.frame.width, EVDiffSplitView.minimumWidth)
    XCTAssertEqual(reported.last, view.baseFraction)
    XCTAssertEqual(reported.count, 4)
  }

  func testTheBaseKeepsItsShareWhileTheWindowResizes() {
    let view = split()
    view.splitter.dragDidMove?(-200)
    XCTAssertEqual(view.base.frame.width, 200)
    view.frame.size.width = 1605
    view.layoutSubtreeIfNeeded()
    XCTAssertEqual(view.base.frame.width, 400)
    XCTAssertEqual(view.document.frame.width, 1200)
  }

  func testAReplacementSplitStartsFromTheSavedShare() {
    let view = EVDiffSplitView(base: NSView(), document: NSView(), baseFraction: 0.25)
    view.frame = NSRect(x: 0, y: 0, width: 805, height: 300)
    view.layoutSubtreeIfNeeded()
    XCTAssertEqual(view.base.frame.width, 200)
  }
}
