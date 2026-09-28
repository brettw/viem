import AppKit
import XCTest
@testable import ViemAppShell

final class EVStatusFilePathTests: XCTestCase {
  func testPathsBelowACommonDirectoryStayRelative() {
    func path(_ file: String, _ cwd: String) -> String {
      EVStatusFilePath.display(URL(fileURLWithPath: file), relativeTo: URL(fileURLWithPath: cwd))
    }
    XCTAssertEqual(path("/Users/writer/project/notes.md", "/Users/writer/project"), "notes.md")
    XCTAssertEqual(path("/Users/writer/drafts/notes.md", "/Users/writer/project"), "../drafts/notes.md")
    XCTAssertEqual(path("/Users/writer/project/a/../café.md", "/Users/writer/project/"), "café.md")
    XCTAssertEqual(path("/Volumes/notes.md", "/Users/writer/project"), "/Volumes/notes.md")
    XCTAssertEqual(path("/Users/notes.md", "/"), "/Users/notes.md")
    XCTAssertEqual(EVStatusFilePath.display(nil), "Untitled")
  }

  func testCopiedRelativePathTraversesRootAndKeepsTheWholeFilename() {
    let file = URL(fileURLWithPath: "/Volumes/notes with café.md")
    let cwd = URL(fileURLWithPath: "/Users/writer/project")
    XCTAssertEqual(EVStatusFilePath.display(file, relativeTo: cwd), file.path)
    XCTAssertEqual(EVStatusFilePath.relative(file, relativeTo: cwd), "../../../Volumes/notes with café.md")
    XCTAssertEqual(EVStatusFilePath.relative(file, relativeTo: URL(fileURLWithPath: "/")), "Volumes/notes with café.md")
  }

  @MainActor
  func testFilenameMenuCopiesCurrentBindingAndWorkingDirectoryWithoutUsingDisplayText() throws {
    _ = NSApplication.shared
    let previousDirectory = FileManager.default.currentDirectoryPath
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory.appendingPathComponent("child"), withIntermediateDirectories: true)
    defer {
      _ = FileManager.default.changeCurrentDirectoryPath(previousDirectory)
      try? FileManager.default.removeItem(at: directory)
    }
    XCTAssertTrue(FileManager.default.changeCurrentDirectoryPath(directory.path))
    let cwd = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
    let bar = EVStatusBarView()
    var copied: [String] = []
    bar.writeCopiedPath = { copied.append($0) }
    func descendants(_ view: NSView) -> [NSView] {
      view.subviews.flatMap { [$0] + descendants($0) }
    }
    let label = try XCTUnwrap(descendants(bar).compactMap { $0 as? NSTextField }
      .first { $0.accessibilityLabel() == "File: Untitled" })
    let menu = try XCTUnwrap(label.menu)
    XCTAssertEqual(menu.items.map(\.title), ["Copy full path", "Copy relative path"])
    menu.update()
    XCTAssertTrue(menu.items.allSatisfy { !$0.isEnabled })
    XCTAssertTrue(copied.isEmpty)

    let original = cwd.appendingPathComponent("original.md")
    bar.fileURL = original
    menu.update()
    XCTAssertTrue(menu.items.allSatisfy(\.isEnabled))
    menu.performActionForItem(at: 0)
    XCTAssertEqual(copied.last, original.path)

    // Reuse the menu after rebinding; copy must resolve the current URL at action time.
    let renamed = cwd.appendingPathComponent("renamed café.md")
    bar.fileURL = renamed
    menu.performActionForItem(at: 0)
    XCTAssertEqual(copied.last, renamed.path)
    XCTAssertTrue(FileManager.default.changeCurrentDirectoryPath(cwd.appendingPathComponent("child").path))
    menu.performActionForItem(at: 1)
    XCTAssertEqual(copied.last, "../renamed café.md")

    bar.fileURL = nil
    menu.update()
    XCTAssertTrue(menu.items.allSatisfy { !$0.isEnabled })
    XCTAssertEqual(copied.count, 3)
  }

  @MainActor
  func testPathStartsAtTheSamePositionInEveryModeAndTruncatesItsHead() throws {
    let bar = EVStatusBarView()
    bar.frame = NSRect(x: 0, y: 0, width: 480, height: EVStatusBarView.preferredHeight)
    bar.fileURL = URL(fileURLWithPath: "/" + String(repeating: "parent/", count: 30) + "notes.md")
    func descendants(_ view: NSView) -> [NSView] {
      view.subviews.flatMap { [$0] + descendants($0) }
    }
    let path = try XCTUnwrap(descendants(bar).compactMap { $0 as? NSTextField }
      .first { $0.stringValue == bar.filePath })
    var origin: CGFloat?
    for mode in ["NORMAL", "INSERT", "REPLACE", "VISUAL", "VISUAL LINE", "VISUAL BLOCK",
      "SELECTION", "SELECT", "SELECT LINE", "SELECT BLOCK", "COMMAND"] {
      bar.apply(EVStatusBarState(mode: mode))
      bar.layoutSubtreeIfNeeded()
      let x = path.convert(path.bounds, to: bar).minX
      if let origin { XCTAssertEqual(x, origin, accuracy: 0.5) } else { origin = x }
      let label = try XCTUnwrap(descendants(bar).compactMap { $0 as? NSTextField }
        .first { $0.accessibilityLabel() == "Mode: \(mode)" })
      let needed = (mode as NSString).size(withAttributes: [.font: try XCTUnwrap(label.font)]).width
      XCTAssertGreaterThanOrEqual(label.frame.width, needed)
    }
    XCTAssertEqual(path.lineBreakMode, .byTruncatingHead)
    XCTAssertLessThan(path.frame.width, path.intrinsicContentSize.width)
    XCTAssertGreaterThan(path.frame.width, 0)
    XCTAssertEqual(path.toolTip, bar.filePath)
  }
}
