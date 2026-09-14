import AppKit
import XCTest

@testable import ViemAppShell

@MainActor
final class EVDocumentWindowPlacementTests: XCTestCase {
  private final class PlacementTestWindow: NSWindow {
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
  }
  private let desktop = NSRect(x: 0, y: 30, width: 1600, height: 1000)

  private func window() -> NSWindow {
    let window = PlacementTestWindow(
      contentRect: NSRect(x: 0, y: 0, width: 920, height: 680),
      styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentMinSize = NSSize(width: 480, height: 280)
    return window
  }

  private func configurationDirectory() -> URL {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      "viem-window-placement-\(UUID().uuidString)", isDirectory: true)
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    return directory
  }

  func testSavedFrameRestoresExactlyAcrossPlacementInstances() throws {
    let directory = configurationDirectory()
    let saved = NSRect(x: 175, y: 245, width: 1000, height: 720)
    let first = EVDocumentWindowPlacement(configuration: EVConfigurationStore(directory: directory))
    first.record(frame: saved)
    let reopened = EVConfigurationStore(directory: directory)
    let next = EVDocumentWindowPlacement(loadFrame: { reopened.documentWindowFrame }, visibleScreens: { [self.desktop] })
    let window = window()
    defer { window.close() }
    XCTAssertEqual(next.initialFrame(for: window, defaultContentSize: NSSize(width: 920, height: 680)), saved)
    XCTAssertEqual(reopened.documentWindowFrame, saved)
  }

  func testOffscreenFrameMovesBeforeAnyDimensionShrinks() {
    let saved = NSRect(x: 1350, y: -100, width: 1000, height: 700)
    let fitted = EVDocumentWindowPlacement.fitting(saved, to: [desktop])
    XCTAssertEqual(fitted, NSRect(x: 600, y: 30, width: 1000, height: 700))
    XCTAssertEqual(fitted.size, saved.size)
    XCTAssertTrue(desktop.contains(fitted))
  }

  func testOnlyOversizedDimensionsShrinkAndTheWholeFrameFits() {
    let saved = NSRect(x: 1500, y: -2000, width: 1900, height: 700)
    let fitted = EVDocumentWindowPlacement.fitting(saved, to: [desktop])
    XCTAssertEqual(fitted, NSRect(x: 0, y: 30, width: 1600, height: 700))
    XCTAssertEqual(EVDocumentWindowPlacement.fitting(
      NSRect(x: -300, y: -300, width: 2000, height: 2000), to: [desktop]), desktop)
  }

  func testRestorationSelectsExistingDisplayOrNearestAfterDisconnection() {
    let secondary = NSRect(x: -1500, y: 50, width: 1400, height: 900)
    let saved = NSRect(x: -1400, y: 250, width: 900, height: 600)
    XCTAssertEqual(EVDocumentWindowPlacement.fitting(saved, to: [desktop, secondary]), saved)
    XCTAssertEqual(EVDocumentWindowPlacement.fitting(saved, to: [desktop]),
      NSRect(x: 0, y: 250, width: 900, height: 600))
    let right = NSRect(x: 2000, y: 0, width: 1400, height: 1000)
    XCTAssertEqual(EVDocumentWindowPlacement.fitting(
      NSRect(x: 4000, y: 200, width: 900, height: 600), to: [desktop, right]),
      NSRect(x: 2500, y: 200, width: 900, height: 600))
  }

  func testEverySubsequentWindowUsesNativeCascadePointAndLatestRecordedFrame() {
    let original = NSRect(x: 200, y: 200, width: 700, height: 500)
    var cascadeCalls = 0
    let placement = EVDocumentWindowPlacement(
      loadFrame: { original }, visibleScreens: { [self.desktop] },
      nextCascadePoint: { window in
        cascadeCalls += 1
        return NSPoint(x: window.frame.minX + 27, y: window.frame.maxY - 27)
      })
    let windows = [window(), window(), window()]
    defer { windows.forEach { $0.close() } }
    XCTAssertEqual(placement.initialFrame(for: windows[0], defaultContentSize: .zero), original)
    XCTAssertEqual(cascadeCalls, 0)
    XCTAssertEqual(placement.initialFrame(for: windows[1], defaultContentSize: .zero),
      original.offsetBy(dx: 27, dy: -27))
    let moved = NSRect(x: 350, y: 250, width: 800, height: 600)
    placement.record(frame: moved)
    XCTAssertEqual(placement.initialFrame(for: windows[2], defaultContentSize: .zero),
      moved.offsetBy(dx: 27, dy: -27))
    XCTAssertEqual(cascadeCalls, 2)
  }

  func testDefaultSizeAndEmptyDesktopRemainUsableBeforeScreenAssignment() {
    let placement = EVDocumentWindowPlacement(loadFrame: { nil })
    let window = window()
    defer { window.close() }
    let size = NSSize(width: 920, height: 680)
    let frame = placement.initialFrame(for: window, defaultContentSize: size)
    XCTAssertEqual(window.contentRect(forFrameRect: frame).size, size)
    XCTAssertEqual(EVDocumentWindowPlacement.fitting(frame, to: []), frame)
  }

  func testConfigurationPreservesOtherPreferencesAndRejectsInvalidGeometry() throws {
    let directory = configurationDirectory()
    let configuration = EVConfigurationStore(directory: directory)
    try configuration.setSmartQuotes(true)
    let saved = NSRect(x: -900, y: 200, width: 800, height: 600)
    try configuration.setDocumentWindowFrame(saved)
    XCTAssertThrowsError(try configuration.setDocumentWindowFrame(
      NSRect(x: 0, y: 0, width: 0, height: 200)))
    XCTAssertThrowsError(try configuration.setDocumentWindowFrame(
      NSRect(x: CGFloat.infinity, y: 0, width: 800, height: 200)))
    let reopened = EVConfigurationStore(directory: directory)
    XCTAssertEqual(reopened.documentWindowFrame, saved)
    XCTAssertTrue(reopened.smartQuotes)
    let file = directory.appendingPathComponent("config.json")
    let invalid = Data(#"{"version":1,"windows":{"documentFrame":{"x":true,"y":0,"width":800,"height":600}}}"#.utf8)
    try invalid.write(to: file, options: .atomic)
    let broken = EVConfigurationStore(directory: directory)
    XCTAssertNotNil(broken.lastError)
    XCTAssertNil(broken.documentWindowFrame)
    XCTAssertThrowsError(try broken.setDocumentWindowFrame(saved))
    XCTAssertEqual(try Data(contentsOf: file), invalid)
  }
}
