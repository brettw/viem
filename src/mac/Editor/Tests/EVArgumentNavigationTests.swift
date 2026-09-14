import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor final class EVArgumentNavigationTests: XCTestCase {
  private func assertError<E: Error & Equatable>(_ result: Result<String?, Error>, _ expected: E,
    file: StaticString = #filePath, line: UInt = #line) {
    XCTAssertEqual(result.failure as? E, expected, file: file, line: line)
  }

  private func assertFailed(_ result: Result<String?, Error>, file: StaticString = #filePath, line: UInt = #line) {
    XCTAssertNotNil(result.failure, file: file, line: line)
  }

  @MainActor private final class Fixture {
    let directory: URL
    let urls: [URL]
    let window: EVDocumentWindowController
    var documents: [EVDocument] = []

    init(count: Int = 4) throws {
      EVFrontendRegistry.install { EVCoreDocumentBackend() }
      EVCoreArgumentListPolicy.install()
      let root = FileManager.default.temporaryDirectory.appendingPathComponent("viem-arguments-\(UUID())")
      directory = root
      try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
      urls = (0..<count).map { root.appendingPathComponent("file \($0).txt") }
      for (index, url) in urls.enumerated() { try Data("file \(index)\nsecond\nthird".utf8).write(to: url) }
      let blank = EVDocument(editorBackend: EVCoreDocumentBackend())
      window = EVDocumentWindowController(document: blank, editorSurface: blank.editorBackend.makeEditorSurface(),
        placement: EVDocumentWindowPlacement(loadFrame: { nil }))
      blank.addWindowController(window)
      documents.append(blank)
      window.argumentDocumentOpener = { [weak self] url, _, completion in
        guard let self else { return }
        do { completion(try self.open(url), nil) } catch { completion(nil, error) }
      }
    }

    func open(_ url: URL) throws -> EVDocument {
      if let existing = EVDocumentIdentity.existingDocument(at: url) { return existing }
      let document = EVDocument(editorBackend: EVCoreDocumentBackend())
      document.recordRecentDocument = { _ in }
      try document.read(from: url, ofType: EVDocument.plainTextType)
      document.fileURL = url
      document.fileType = EVDocument.plainTextType
      NSDocumentController.shared.addDocument(document)
      documents.append(document)
      return document
    }

    func launch(split: Int? = nil, line: UInt64? = nil) async throws {
      let result: Result<String?, Error> = await withCheckedContinuation { continuation in
        window.openArgumentList(urls, splitCount: split, initialLine: line) { continuation.resume(returning: $0) }
      }
      _ = try result.get()
    }

    var document: EVDocument { window.activeDocument! }
    var backend: EVCoreDocumentBackend { document.editorBackend as! EVCoreDocumentBackend }
    var surface: EVEditorSurfaceController { window.editorSurface as! EVEditorSurfaceController }

    func modify(_ text: String = "changed ") throws {
      let session = try XCTUnwrap(surface.session)
      _ = try session.sendText("i" + text)
      _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
    }

    func perform(_ kind: EVDocumentHostRequest.Kind, path: String? = nil, force: Bool = false,
      navigation: EVArgumentNavigation? = nil) async -> Result<String?, Error> {
      let state = backend.persistenceState
      return await withCheckedContinuation { continuation in
        window.perform(documentHostRequests: [.init(kind: kind, documentID: state.documentID,
          documentRevision: state.documentRevision, force: force, path: path, argumentNavigation: navigation)]) {
          continuation.resume(returning: $0)
        }
      }
    }

    func navigate(_ target: EVArgumentNavigation.Target = .next, count: UInt64 = 1,
      force: Bool = false, write: Bool = false, path: String? = nil) async -> Result<String?, Error> {
      await perform(.navigateArgument, path: path, force: force,
        navigation: .init(target: target, count: count, writeFirst: write))
    }

    func close() {
      window.close()
      for document in documents { document.close() }
      try? FileManager.default.removeItem(at: directory)
    }
  }

  func testDefaultLaunchIsLazyAndForcedNextDiscardsOnlyTheLastView() async throws {
    let f = try Fixture()
    defer { f.close() }
    try await f.launch(line: 2)
    XCTAssertEqual(f.window.paneCount, 1)
    XCTAssertEqual(f.document.fileURL, f.urls[0])
    XCTAssertEqual(f.surface.viewPresentation.cursor_utf8_offset, 7)
    XCTAssertNil(EVDocumentIdentity.existingDocument(at: f.urls[1]))
    try f.modify()
    let refused = await f.navigate()
    XCTAssertEqual(refused.failure as? EVDocumentHostError, .documentModified)
    XCTAssertEqual(f.document.fileURL, f.urls[0])
    _ = try await f.navigate(force: true).get()
    XCTAssertEqual(f.document.fileURL, f.urls[1])
    _ = try await f.navigate(.previous).get()
    XCTAssertFalse(f.backend.persistenceState.isDirty)
    XCTAssertEqual(try f.backend.serializedSource(typeName: EVDocument.plainTextType), try Data(contentsOf: f.urls[0]))
    assertError(await f.navigate(.previous), EVArgumentListError.beforeFirst)
  }

  func testSplitNavigationRetainsModifiedSharedBufferAndUsesEachCurrentFile() async throws {
    let f = try Fixture()
    defer { f.close() }
    try await f.launch()
    let original = f.document
    let firstSurface = f.surface
    try f.modify()
    _ = try await f.perform(.split).get()
    _ = try await f.navigate().get()
    XCTAssertEqual(f.document.fileURL, f.urls[1])
    XCTAssertTrue(original.editorBackend.persistenceState.isDirty)
    _ = try await f.navigate(.previous, force: true).get()
    XCTAssertTrue(f.document === original, "Returning reuses the edited buffer, even with bang")
    _ = try await f.navigate(.next, count: 2).get()
    XCTAssertEqual(f.document.fileURL, f.urls[2])
    f.window.perform(windowRequests: [.focusTop], from: f.surface)
    XCTAssertTrue(f.surface === firstSurface)
    assertError(await f.navigate(), EVDocumentHostError.documentModified)
    _ = try await f.navigate(force: true).get()
    XCTAssertEqual(f.document.fileURL, f.urls[1], "The other pane's position must not affect this pane")
  }

  func testEditOutsideListRetainsPositionAndEditInsideListUsesCurrentIdentity() async throws {
    let f = try Fixture()
    defer { f.close() }
    try await f.launch()
    _ = try await f.navigate().get()
    let outside = f.directory.appendingPathComponent("outside.txt")
    try Data("outside".utf8).write(to: outside)
    _ = try f.open(outside)
    _ = try await f.perform(.edit, path: outside.path).get()
    _ = try await f.navigate().get()
    XCTAssertEqual(f.document.fileURL, f.urls[2])
    _ = try f.open(f.urls[0])
    _ = try await f.perform(.edit, path: f.urls[0].path).get()
    _ = try await f.navigate().get()
    XCTAssertEqual(f.document.fileURL, f.urls[1])
  }

  func testWriteNextSavesBeforeNavigationIncludingEndOfListAndReadOnlyFailure() async throws {
    let f = try Fixture(count: 2)
    defer { f.close() }
    try await f.launch()
    try f.modify()
    let bytes = try f.backend.serializedSource(typeName: EVDocument.plainTextType)
    let firstWrite = await f.navigate(write: true)
    XCTAssertNil(firstWrite.failure, "First write-next failed: \(String(describing: firstWrite.failure))")
    guard firstWrite.failure == nil else { return }
    XCTAssertEqual(try Data(contentsOf: f.urls[0]), bytes)
    try f.modify("last ")
    let lastBytes = try f.backend.serializedSource(typeName: EVDocument.plainTextType)
    assertError(await f.navigate(write: true), EVArgumentListError.afterLast)
    XCTAssertEqual(try Data(contentsOf: f.urls[1]), lastBytes)
    XCTAssertFalse(f.backend.persistenceState.isDirty)
    try f.backend.setReadOnly(true)
    assertFailed(await f.navigate(.previous, write: true))
    XCTAssertEqual(f.document.fileURL, f.urls[1])
    let forcedPrevious = await f.navigate(.previous, force: true, write: true)
    XCTAssertNil(forcedPrevious.failure, "Forced write-previous failed: \(String(describing: forcedPrevious.failure))")
    XCTAssertEqual(f.document.fileURL, f.urls[0])
  }

  func testAlternateWriteNextPreservesBindingAndDirtyGuard() async throws {
    let f = try Fixture(count: 2)
    defer { f.close() }
    try await f.launch()
    try f.modify()
    let copy = f.directory.appendingPathComponent("copy.txt")
    assertError(await f.navigate(write: true, path: copy.path), EVDocumentHostError.documentModified)
    XCTAssertEqual(f.document.fileURL, f.urls[0])
    XCTAssertTrue(f.backend.persistenceState.isDirty)
    XCTAssertEqual(try Data(contentsOf: copy), try f.backend.serializedSource(typeName: EVDocument.plainTextType))
    _ = try await f.navigate(force: true, write: true, path: copy.path).get()
    XCTAssertEqual(f.document.fileURL, f.urls[1])
  }

  func testFailedAndStaleOpensPreservePaneAndListPosition() async throws {
    let f = try Fixture()
    defer { f.close() }
    try await f.launch()
    let original = f.document
    f.window.argumentDocumentOpener = { _, _, completion in completion(nil, CocoaError(.fileReadNoPermission)) }
    assertFailed(await f.navigate())
    XCTAssertTrue(f.document === original)
    f.window.argumentDocumentOpener = { url, _, completion in
      do {
        let opened = try f.open(url)
        try f.modify("intervening ")
        completion(opened, nil)
      } catch { completion(nil, error) }
    }
    assertError(await f.navigate(force: true), EVDocumentHostError.staleRequest)
    XCTAssertTrue(f.document === original)
    f.window.argumentDocumentOpener = { url, _, completion in
      do { completion(try f.open(url), nil) } catch { completion(nil, error) }
    }
    _ = try await f.navigate(force: true).get()
    XCTAssertEqual(f.document.fileURL, f.urls[1])
  }

  func testSplitLaunchKeepsArgumentOrderFocusAndInitialLineOnlyInFirstPane() async throws {
    let f = try Fixture(count: 3)
    defer { f.close() }
    try await f.launch(split: 0, line: 3)
    XCTAssertEqual(f.window.paneCount, 3)
    XCTAssertEqual(f.document.fileURL, f.urls[0])
    XCTAssertEqual(f.surface.viewPresentation.cursor_utf8_offset, 14)
    f.window.perform(windowRequests: [.focusNext(index: 2)], from: f.surface)
    XCTAssertEqual(f.document.fileURL, f.urls[1])
    XCTAssertEqual(f.surface.viewPresentation.cursor_utf8_offset, 0)
    _ = try await f.navigate().get()
    XCTAssertEqual(f.document.fileURL, f.urls[2])
    f.window.perform(windowRequests: [.focusBottom], from: f.surface)
    XCTAssertEqual(f.document.fileURL, f.urls[2])
  }

  func testEditModifiedBufferInSplitNeedsNoDiscardButReloadStillDoes() async throws {
    let f = try Fixture(count: 2)
    defer { f.close() }
    try await f.launch()
    try f.modify()
    let original = f.document
    _ = try await f.perform(.split).get()
    assertError(await f.perform(.edit), EVDocumentHostError.documentModified)
    _ = try f.open(f.urls[1])
    _ = try await f.perform(.edit, path: f.urls[1].path).get()
    XCTAssertEqual(f.document.fileURL, f.urls[1])
    XCTAssertTrue(original.editorBackend.persistenceState.isDirty)
  }

  func testExplicitSplitCountCreatesExtraBlankPanesAndSupportsNoFiles() async throws {
    for fileCount in [0, 1] {
      let f = try Fixture(count: fileCount)
      defer { f.close() }
      try await f.launch(split: 3)
      XCTAssertEqual(f.window.paneCount, 3)
      XCTAssertTrue(f.window.window?.isVisible == true)
      f.window.perform(windowRequests: [.focusBottom], from: f.surface)
      XCTAssertNil(f.document.fileURL)
      XCTAssertFalse(f.backend.persistenceState.isDirty)
      if fileCount == 0 {
        assertError(await f.navigate(), EVArgumentListError.empty)
      } else {
        _ = try await f.navigate(.first).get()
        XCTAssertEqual(f.document.fileURL, f.urls[0])
      }
    }
  }

  func testNewWindowForBufferInheritsListAndKeepsOriginalModifiedView() async throws {
    let f = try Fixture(count: 2)
    defer { f.close() }
    try await f.launch()
    try f.modify()
    let original = f.document
    let second = EVDocumentWindowController(document: original,
      editorSurface: original.editorBackend.makeEditorSurface(), placement: EVDocumentWindowPlacement(loadFrame: { nil }))
    original.addWindowController(second)
    defer { second.close() }
    let state = original.editorBackend.persistenceState
    let result: Result<String?, Error> = await withCheckedContinuation { continuation in
      second.perform(documentHostRequests: [.init(kind: .navigateArgument,
        documentID: state.documentID, documentRevision: state.documentRevision,
        argumentNavigation: .init(target: .next))]) { continuation.resume(returning: $0) }
    }
    _ = try result.get()
    XCTAssertEqual(second.activeDocument?.fileURL, f.urls[1])
    XCTAssertTrue(f.document === original)
    XCTAssertTrue(f.backend.persistenceState.isDirty)
  }
}

private extension Result {
  var failure: Failure? { if case .failure(let error) = self { error } else { nil } }
}
