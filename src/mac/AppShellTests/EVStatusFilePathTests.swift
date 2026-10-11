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

  func testFittingDropsDirectoriesBeforeClippingFilenameAtGraphemeBoundaries() {
    func fit(_ path: String, _ width: CGFloat) -> String {
      EVStatusFilePath.fitting(path, width: width) { CGFloat($0.count) }
    }
    XCTAssertEqual(fit("parent/drafts/notes.md", 22), "parent/drafts/notes.md")
    XCTAssertEqual(fit("parent/drafts/notes.md", 15), "drafts/notes.md")
    XCTAssertEqual(fit("parent/drafts/notes.md", 8), "notes.md")
    XCTAssertEqual(fit("parent/drafts/notes.md", 7), "note...")
    XCTAssertEqual(fit("parent/👩‍💻é-notes.md", 6), "👩‍💻é-...")
    XCTAssertEqual(fit("parent/\u{301}notes.md", 9), "\u{301}notes.md")
    XCTAssertEqual(fit("notes.md", 3), "...")
    XCTAssertEqual(fit("notes.md", 2), "")
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
    var changedDirectory: URL?
    bar.changeDirectory = { changedDirectory = $0 }
    func descendants(_ view: NSView) -> [NSView] {
      view.subviews.flatMap { [$0] + descendants($0) }
    }
    let label = try XCTUnwrap(descendants(bar).compactMap { $0 as? NSTextField }
      .first { $0.accessibilityLabel() == "File: Untitled" })
    let menu = try XCTUnwrap(label.menu)
    XCTAssertEqual(menu.items.map(\.title), ["Copy full path", "Copy relative path", "", "Change directory to here"])
    XCTAssertTrue(menu.items[2].isSeparatorItem)
    menu.update()
    XCTAssertTrue(menu.items.filter { !$0.isSeparatorItem }.allSatisfy { !$0.isEnabled })
    XCTAssertTrue(copied.isEmpty)

    let original = cwd.appendingPathComponent("original.md")
    bar.fileURL = original
    menu.update()
    XCTAssertTrue(menu.items.filter { !$0.isSeparatorItem }.allSatisfy(\.isEnabled))
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
    menu.performActionForItem(at: 3)
    XCTAssertEqual(changedDirectory?.standardizedFileURL, renamed.deletingLastPathComponent().standardizedFileURL)

    bar.fileURL = nil
    menu.update()
    XCTAssertTrue(menu.items.filter { !$0.isSeparatorItem }.allSatisfy { !$0.isEnabled })
    XCTAssertEqual(copied.count, 3)
  }

  @MainActor
  func testPathStartsAtTheSamePositionInEveryModeWhenThereIsRoom() throws {
    let bar = EVStatusBarView()
    bar.frame = NSRect(x: 0, y: 0, width: 1200, height: EVStatusBarView.preferredHeight)
    bar.fileURL = URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent("notes.md")
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
    XCTAssertEqual(path.lineBreakMode, .byClipping)
    XCTAssertGreaterThan(path.frame.width, 0)
    XCTAssertEqual(path.toolTip, bar.filePath)
  }

  @MainActor
  func testNarrowingHidesModeThenDirectoriesThenClipsFilenameWhilePositionRemains() throws {
    _ = NSApplication.shared
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1200, height: 100),
      styleMask: [.borderless], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    defer { window.close() }
    let bar = EVStatusBarView()
    bar.translatesAutoresizingMaskIntoConstraints = true
    bar.frame = NSRect(x: 0, y: 0, width: 1200, height: EVStatusBarView.preferredHeight)
    try XCTUnwrap(window.contentView).addSubview(bar)
    bar.fileURL = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
      .appendingPathComponent("parent/drafts/long-filename-with-café.md")
    func descendants(_ view: NSView) -> [NSView] { view.subviews.flatMap { [$0] + descendants($0) } }
    let path = try XCTUnwrap(descendants(bar).compactMap { $0 as? NSTextField }.first { $0.accessibilityLabel()?.hasPrefix("File: ") == true })
    let mode = try XCTUnwrap(descendants(bar).compactMap { $0 as? NSTextField }.first { $0.accessibilityLabel() == "Mode: NORMAL" })
    let position = try XCTUnwrap(descendants(bar).compactMap { $0 as? NSButton }.first { $0.title == "Ln 1, Col 1" })
    bar.layoutSubtreeIfNeeded()
    let style = EVThemeStore.shared.chromeStyles.statusBar
    let reserved = style.padding.left + style.padding.right + position.intrinsicContentSize.width + 14
    func width(_ text: String) -> CGFloat { ceil(NSAttributedString(string: text, attributes: path.attributedStringValue.attributes(at: 0, effectiveRange: nil)).size().width) }
    let fullWidth = width(bar.filePath), filenameWidth = width("long-filename-with-café.md")
    func resize(_ width: CGFloat) {
      window.setContentSize(NSSize(width: width, height: 100))
      bar.setFrameSize(NSSize(width: width, height: bar.frame.height))
      // Pane layout explicitly keeps frame and bounds in DIPs. An unattached
      // Auto Layout root can instead retain its old bounds and scale its text.
      bar.setBoundsSize(bar.frame.size)
      bar.needsLayout = true
      bar.layoutSubtreeIfNeeded()
      XCTAssertEqual(bar.frame.width, width, accuracy: 0.01)
      XCTAssertEqual(bar.bounds.width, width, accuracy: 0.01)
      XCTAssertFalse(position.isHiddenOrHasHiddenAncestor)
      XCTAssertEqual(position.convert(position.bounds, to: bar).maxX, bar.bounds.maxX - style.padding.right, accuracy: 2)
    }
    XCTAssertFalse(mode.isHiddenOrHasHiddenAncestor)
    resize(reserved + fullWidth + 1)
    XCTAssertTrue(mode.isHiddenOrHasHiddenAncestor)
    XCTAssertEqual(path.stringValue, bar.filePath)
    XCTAssertEqual(path.convert(path.alignmentRect(forFrame: path.bounds), to: bar).minX, style.padding.left, accuracy: 2)
    resize(reserved + filenameWidth + 1)
    XCTAssertEqual(path.stringValue, "long-filename-with-café.md")
    resize(reserved + filenameWidth / 2)
    XCTAssertTrue(path.stringValue.hasSuffix("..."))
    XCTAssertTrue(path.stringValue.hasPrefix("long"))
    XCTAssertLessThanOrEqual(path.convert(path.bounds, to: bar).maxX, position.frame.minX)
    XCTAssertEqual(path.toolTip, bar.filePath)
    XCTAssertEqual(path.accessibilityLabel(), "File: \(bar.filePath)")
    resize(1200)
    XCTAssertFalse(mode.isHiddenOrHasHiddenAncestor)
    XCTAssertEqual(path.stringValue, bar.filePath)
  }
}
