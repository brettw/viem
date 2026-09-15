import Foundation
import XCTest
@testable import ViemAppShell

@MainActor
final class EVFileChangeMonitorTests: XCTestCase {
  private func destination(createFile: Bool = true) throws -> URL {
    let directory = FileManager.default.temporaryDirectory
      .appendingPathComponent("viem-file-watch-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    let url = directory.appendingPathComponent("source.txt")
    if createFile { try Data("original".utf8).write(to: url) }
    return url
  }

  func testInPlaceWritesNotifyOnMainThreadAndCoalesceBurst() throws {
    let url = try destination()
    let changed = expectation(description: "in-place writes observed")
    let duplicate = expectation(description: "one event burst produces one callback")
    duplicate.isInverted = true
    var callbacks = 0
    let monitor = EVFileChangeMonitor(url: url) {
      XCTAssertTrue(Thread.isMainThread)
      callbacks += 1
      if callbacks == 1 { changed.fulfill() } else { duplicate.fulfill() }
    }
    defer { monitor.stop() }
    let file = try FileHandle(forWritingTo: url)
    defer { try? file.close() }
    for _ in 0..<12 { try file.write(contentsOf: Data("updated".utf8)) }
    try file.synchronize()
    wait(for: [changed], timeout: 5)
    wait(for: [duplicate], timeout: 0.2)
    XCTAssertEqual(callbacks, 1)
  }

  func testAtomicReplacementReattachesToNewFile() throws {
    let url = try destination()
    var pending: XCTestExpectation? = expectation(description: "atomic replacement observed")
    let monitor = EVFileChangeMonitor(url: url) { pending?.fulfill(); pending = nil }
    defer { monitor.stop() }
    let replaced = try XCTUnwrap(pending)
    try Data("replacement".utf8).write(to: url, options: .atomic)
    wait(for: [replaced], timeout: 5)

    let editedReplacement = expectation(description: "new vnode writes observed")
    pending = editedReplacement
    let file = try FileHandle(forWritingTo: url)
    defer { try? file.close() }
    try file.write(contentsOf: Data("edited".utf8))
    wait(for: [editedReplacement], timeout: 5)
  }

  func testDeletionAndRecreationContinueWatching() throws {
    let url = try destination()
    var pending: XCTestExpectation? = expectation(description: "deletion observed")
    let monitor = EVFileChangeMonitor(url: url) { pending?.fulfill(); pending = nil }
    defer { monitor.stop() }
    let deleted = try XCTUnwrap(pending)
    try FileManager.default.removeItem(at: url)
    wait(for: [deleted], timeout: 5)

    let recreated = expectation(description: "recreation observed")
    pending = recreated
    try Data("recreated".utf8).write(to: url)
    wait(for: [recreated], timeout: 5)

    let edited = expectation(description: "recreated vnode writes observed")
    pending = edited
    let file = try FileHandle(forWritingTo: url)
    defer { try? file.close() }
    try file.write(contentsOf: Data("edited".utf8))
    wait(for: [edited], timeout: 5)
  }

  func testInitiallyMissingFileIsWatchedThroughItsDirectory() throws {
    let url = try destination(createFile: false)
    let created = expectation(description: "initially absent file appears")
    let monitor = EVFileChangeMonitor(url: url) { created.fulfill() }
    defer { monitor.stop() }
    try Data("created".utf8).write(to: url)
    wait(for: [created], timeout: 5)
  }

  func testRemovedContainingDirectoryCanBeRecreatedAndEditedAgain() throws {
    try assertContainingDirectoryCanBeRecreated(moveDirectory: false)
  }

  func testMovedContainingDirectoryCanBeRecreatedAndEditedAgain() throws {
    try assertContainingDirectoryCanBeRecreated(moveDirectory: true)
  }

  private func assertContainingDirectoryCanBeRecreated(moveDirectory: Bool) throws {
    let root = try destination(createFile: false).deletingLastPathComponent()
    let parent = root.appendingPathComponent("nested", isDirectory: true)
    try FileManager.default.createDirectory(at: parent, withIntermediateDirectories: true)
    let url = parent.appendingPathComponent("source.txt")
    try Data("original".utf8).write(to: url)
    var pending: XCTestExpectation? = expectation(description: "containing directory disappearance observed")
    let monitor = EVFileChangeMonitor(url: url) { pending?.fulfill(); pending = nil }
    defer { monitor.stop() }
    let disappeared = try XCTUnwrap(pending)
    if moveDirectory {
      try FileManager.default.moveItem(at: parent, to: root.appendingPathComponent("moved", isDirectory: true))
    } else {
      try FileManager.default.removeItem(at: parent)
    }
    wait(for: [disappeared], timeout: 5)

    let unrelated = expectation(description: "ancestor activity without a file change is ignored")
    unrelated.isInverted = true
    pending = unrelated
    let sibling = root.appendingPathComponent("sibling.txt")
    try Data("unrelated".utf8).write(to: sibling)
    try FileManager.default.removeItem(at: sibling)
    if moveDirectory {
      try Data("old location".utf8).write(to: root.appendingPathComponent("moved/source.txt"))
    }
    try FileManager.default.createDirectory(at: parent, withIntermediateDirectories: true)
    wait(for: [unrelated], timeout: 0.25)

    let recreated = expectation(description: "file in recreated containing directory observed")
    pending = recreated
    try Data("recreated".utf8).write(to: url)
    wait(for: [recreated], timeout: 5)

    let edited = expectation(description: "file in recreated containing directory remains watched")
    pending = edited
    let file = try FileHandle(forWritingTo: url)
    defer { try? file.close() }
    try file.write(contentsOf: Data("edited".utf8))
    wait(for: [edited], timeout: 5)
  }

  func testSymbolicLinkTargetInAnotherDirectoryCanBeDeletedAndRecreated() throws {
    let target = try destination()
    let aliasDirectory = target.deletingLastPathComponent().appendingPathComponent("aliases", isDirectory: true)
    try FileManager.default.createDirectory(at: aliasDirectory, withIntermediateDirectories: true)
    let alias = aliasDirectory.appendingPathComponent("alias.txt")
    try FileManager.default.createSymbolicLink(atPath: alias.path, withDestinationPath: "../source.txt")
    var pending: XCTestExpectation? = expectation(description: "symlink target deletion observed")
    let monitor = EVFileChangeMonitor(url: alias) { pending?.fulfill(); pending = nil }
    defer { monitor.stop() }
    let deleted = try XCTUnwrap(pending)
    try FileManager.default.removeItem(at: target)
    wait(for: [deleted], timeout: 5)

    let recreated = expectation(description: "symlink target recreation in another directory observed")
    pending = recreated
    try Data("recreated target".utf8).write(to: target)
    wait(for: [recreated], timeout: 5)

    let edited = expectation(description: "recreated symlink target vnode writes observed")
    pending = edited
    let file = try FileHandle(forWritingTo: target)
    defer { try? file.close() }
    try file.write(contentsOf: Data("edited".utf8))
    wait(for: [edited], timeout: 5)
  }

  func testInitiallyBrokenSymbolicLinkObservesTargetCreationElsewhere() throws {
    let target = try destination(createFile: false)
    let aliasDirectory = target.deletingLastPathComponent().appendingPathComponent("aliases", isDirectory: true)
    try FileManager.default.createDirectory(at: aliasDirectory, withIntermediateDirectories: true)
    let alias = aliasDirectory.appendingPathComponent("alias.txt")
    try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: target)
    let created = expectation(description: "broken symlink target appears")
    let monitor = EVFileChangeMonitor(url: alias) { created.fulfill() }
    defer { monitor.stop() }
    try Data("created target".utf8).write(to: target)
    wait(for: [created], timeout: 5)
  }

  func testRetargetedSymbolicLinkChainWatchesItsNewTargetDirectory() throws {
    let target = try destination()
    let root = target.deletingLastPathComponent()
    let aliasDirectory = root.appendingPathComponent("aliases", isDirectory: true)
    let linkDirectory = root.appendingPathComponent("links", isDirectory: true)
    let replacementDirectory = root.appendingPathComponent("replacement", isDirectory: true)
    for directory in [aliasDirectory, linkDirectory, replacementDirectory] {
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }
    let alias = aliasDirectory.appendingPathComponent("alias.txt")
    let link = linkDirectory.appendingPathComponent("link.txt")
    let replacement = replacementDirectory.appendingPathComponent("new.txt")
    try Data("replacement target".utf8).write(to: replacement)
    try FileManager.default.createSymbolicLink(at: link, withDestinationURL: target)
    try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: link)
    var pending: XCTestExpectation? = expectation(description: "intermediate link retarget observed")
    let monitor = EVFileChangeMonitor(url: alias) { pending?.fulfill(); pending = nil }
    defer { monitor.stop() }
    let retargeted = try XCTUnwrap(pending)
    try FileManager.default.removeItem(at: link)
    try FileManager.default.createSymbolicLink(at: link, withDestinationURL: replacement)
    wait(for: [retargeted], timeout: 5)

    let deleted = expectation(description: "new target deletion observed")
    pending = deleted
    try FileManager.default.removeItem(at: replacement)
    wait(for: [deleted], timeout: 5)

    let recreated = expectation(description: "new target parent remains watched")
    pending = recreated
    try Data("recreated replacement".utf8).write(to: replacement)
    wait(for: [recreated], timeout: 5)
  }

  func testSiblingFileChangesDoNotNotify() throws {
    let url = try destination()
    let changed = expectation(description: "sibling changes are ignored")
    changed.isInverted = true
    let monitor = EVFileChangeMonitor(url: url) { changed.fulfill() }
    defer { monitor.stop() }
    let sibling = url.deletingLastPathComponent().appendingPathComponent("sibling.txt")
    try Data("unrelated".utf8).write(to: sibling, options: .atomic)
    try FileManager.default.removeItem(at: sibling)
    wait(for: [changed], timeout: 0.25)
  }

  func testStopSuppressesQueuedEventsAndIsIdempotent() throws {
    let url = try destination()
    let changed = expectation(description: "stopped monitor never calls back")
    changed.isInverted = true
    let monitor = EVFileChangeMonitor(url: url) { changed.fulfill() }
    try Data("queued before stop".utf8).write(to: url, options: .atomic)
    monitor.stop()
    monitor.stop()
    try Data("written after stop".utf8).write(to: url, options: .atomic)
    wait(for: [changed], timeout: 0.25)
  }

  func testReleasingMonitorStopsWatchingWithoutExplicitStop() throws {
    let url = try destination()
    let changed = expectation(description: "released monitor never calls back")
    changed.isInverted = true
    var monitor: EVFileChangeMonitor? = EVFileChangeMonitor(url: url) { changed.fulfill() }
    weak var released = monitor
    try Data("queued before release".utf8).write(to: url, options: .atomic)
    monitor = nil
    XCTAssertNil(released)
    try Data("written after release".utf8).write(to: url, options: .atomic)
    wait(for: [changed], timeout: 0.25)
  }
}
