import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVDiffCommitPickerTests: XCTestCase {
  func testChoosingACommitThenFreeingThePickerFreesTheSheetSafely() async throws {
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 700, height: 500),
                          styleMask: [.titled, .closable], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    defer { window.close() }
    window.makeKeyAndOrderFront(nil)
    let commits = (0..<30).map { EVDiffCommit(hash: "\($0)" + String(repeating: "a", count: 39),
                                              short: "abc\($0)", title: "Commit \($0)") }
    weak var freedSheet: NSWindow?
    try await {
      var picker: EVDiffCommitPicker? = EVDiffCommitPicker(commits)
      let sheet = try XCTUnwrap(picker?.alert.window)
      freedSheet = sheet
      let chosen = Task { @MainActor in await picker?.pick(in: window) }
      while window.attachedSheet == nil { await Task.yield() }
      let table = try XCTUnwrap(Self.descendants(sheet.contentView).compactMap { $0 as? NSTableView }.first)
      table.selectRowIndexes(IndexSet(integer: 3), byExtendingSelection: false)
      picker?.alert.buttons[0].performClick(nil)
      let hash = await chosen.value
      XCTAssertEqual(hash, commits[3].hash)
      picker = nil
    }()
    // Freeing the sheet walks the key-view links of every view it holds. Rows
    // the table built before the sheet appeared must not leave a freed label
    // in those links.
    for _ in 0..<5 where freedSheet != nil {
      autoreleasepool { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
      await Task.yield()
    }
    XCTAssertNil(freedSheet)
  }

  private static func descendants(_ view: NSView?) -> [NSView] {
    guard let view else { return [] }
    return [view] + view.subviews.flatMap { descendants($0) }
  }
}
