import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVExternalFileChangeTests: XCTestCase {
  private final class Surface: EVEditorSurface {
    let viewController = NSViewController()
    var statusBarState = EVStatusBarState()
    var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
    var received: ((String) -> Void)?
    init() { viewController.view = NSView(frame: NSRect(x: 0, y: 0, width: 520, height: 320)) }
    func perform(menuCommand: EVMenuCommand, sender: Any?) {}
    func presentation(for: EVMenuCommand) -> EVMenuItemPresentation { .disabled }
    func showDocumentMessage(_ message: String) { received?(message) }
  }
  private final class Backend: EVDocumentBackend {
    var sourceDidChange: (() -> Void)?
    var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)?
    var persistenceState = EVDocumentPersistenceState(documentID: 1, documentRevision: 0)
    let sourceFormat = EVSourceFormat.plainText
    var data = Data()
    var beforeSaveSnapshot: (() -> Void)?
    func makeEditorSurface() -> any EVEditorSurface { Surface() }
    func read(source: Data, typeName: String) throws { data = source }
    func serializedSource(typeName: String) throws -> Data { data }
    func nativeSaveSnapshot(typeName: String) throws -> EVDocumentSaveSnapshot {
      beforeSaveSnapshot?()
      return EVDocumentSaveSnapshot(data: data, documentID: persistenceState.documentID, documentRevision: persistenceState.documentRevision)
    }
    func nativeSaveSnapshot(typeName: String, hardLineRange: ClosedRange<UInt64>) throws -> EVDocumentSaveSnapshot {
      EVDocumentSaveSnapshot(data: Data("selected source".utf8), documentID: persistenceState.documentID,
        documentRevision: persistenceState.documentRevision, isCompleteSource: false)
    }
    func acknowledgeNativeSave(_ snapshot: EVDocumentSaveSnapshot) throws {
      persistenceState.isDirty = false; persistenceStateDidChange?(persistenceState)
    }
  }
  private func fixture() throws -> (EVDocument, Backend, URL) {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-external-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    let url = directory.appendingPathComponent("source.txt")
    try Data("original".utf8).write(to: url)
    let backend = Backend(); let document = EVDocument(editorBackend: backend)
    try document.read(from: url, ofType: EVDocument.plainTextType)
    document.fileURL = url; document.fileType = EVDocument.plainTextType
    addTeardownBlock { MainActor.assumeIsolated { document.close() } }
    return (document, backend, url)
  }
  private func check(_ document: EVDocument) -> EVExternalFileChange? {
    let completed = expectation(description: "file checked")
    var change: EVExternalFileChange?
    document.checkForExternalChanges { change = $0; completed.fulfill() }
    wait(for: [completed], timeout: 5)
    return change
  }
  func testReplacementDeletionAndSameLengthChangesNeverReloadDirtyBuffer() throws {
    let (document, backend, url) = try fixture()
    XCTAssertNil(check(document))
    backend.data = Data("local edits".utf8); backend.persistenceState.isDirty = true
    try Data("external".utf8).write(to: url, options: .atomic)
    XCTAssertEqual(check(document), .replaced)
    XCTAssertEqual(backend.data, Data("local edits".utf8)); XCTAssertTrue(backend.persistenceState.isDirty)
    document.externalSaveDecisionHandler = { _ in false }
    XCTAssertThrowsError(try document.authorizeExternalWrite(to: url))
    document.externalSaveDecisionHandler = { _ in true }
    XCTAssertNoThrow(try document.authorizeExternalWrite(to: url))
    try FileManager.default.removeItem(at: url)
    XCTAssertEqual(check(document), .deleted)
    document.externalSaveDecisionHandler = { _ in false }
    XCTAssertThrowsError(try document.authorizeExternalWrite(to: url))
    XCTAssertEqual(backend.data, Data("local edits".utf8))
  }
  func testMetadataPreservingChangeIsDetectedAndStaleCheckCannotReplaceNewBaseline() throws {
    let (document, _, url) = try fixture()
    let before = try FileManager.default.attributesOfItem(atPath: url.path)
    try Data("modified".utf8).write(to: url)
    if let date = before[.modificationDate] { try FileManager.default.setAttributes([.modificationDate: date], ofItemAtPath: url.path) }
    XCTAssertNotNil(check(document))
    let completed = expectation(description: "obsolete check ignored")
    document.checkForExternalChanges { change in XCTAssertNil(change); completed.fulfill() }
    document.recordFileBaseline(Data("modified".utf8), at: url)
    wait(for: [completed], timeout: 5)
    XCTAssertNil(document.externalFileChange)
    XCTAssertNil(check(document))
  }
  func testActivationAndChecktimePublishPersistentWarnings() throws {
    let (document, _, url) = try fixture()
    let surface = Surface(); let window = EVDocumentWindowController(document: document, editorSurface: surface)
    defer { window.close() }
    try Data("external".utf8).write(to: url, options: .atomic)
    let activation = expectation(description: "activation warning")
    surface.received = { message in XCTAssertTrue(message.contains("outside Viem")); activation.fulfill() }
    window.windowDidBecomeKey(Notification(name: NSWindow.didBecomeKeyNotification))
    wait(for: [activation], timeout: 5)
    surface.received = nil
    let command = expectation(description: "checktime output")
    window.perform(documentHostRequests: [EVDocumentHostRequest(kind: .checkTime, documentID: 1, documentRevision: 0)]) { result in
      XCTAssertTrue(((try? result.get()) ?? nil)?.contains("outside Viem") ?? false)
      command.fulfill()
    }
    wait(for: [command], timeout: 5)
  }
  func testNativeSaveSaveAsAndSaveToRequireOneChoiceAndKeepDirtyOnCancel() throws {
    for operation in [NSDocument.SaveOperationType.saveOperation, .saveAsOperation, .saveToOperation] {
      let (document, backend, url) = try fixture()
      try Data("external".utf8).write(to: url, options: .atomic)
      backend.data = Data("local".utf8); backend.persistenceState.isDirty = true
      var prompts = 0
      document.externalSaveDecisionHandler = { change in
        XCTAssertEqual(change, .replaced); prompts += 1; return false
      }
      let cancelled = expectation(description: "cancel native conflict")
      document.save(to: url, ofType: EVDocument.plainTextType, for: operation) { error in
        XCTAssertEqual((error as? CocoaError)?.code, .userCancelled); cancelled.fulfill()
      }
      wait(for: [cancelled], timeout: 5)
      XCTAssertEqual(prompts, 1)
      XCTAssertEqual(try Data(contentsOf: url), Data("external".utf8))
      XCTAssertTrue(backend.persistenceState.isDirty)
      document.externalSaveDecisionHandler = { _ in prompts += 1; return true }
      let saved = expectation(description: "save anyway native")
      document.save(to: url, ofType: EVDocument.plainTextType, for: operation) { error in
        XCTAssertNil(error); saved.fulfill()
      }
      wait(for: [saved], timeout: 5)
      XCTAssertEqual(prompts, 2, "One decision per save attempt, including the actual write")
      XCTAssertEqual(try Data(contentsOf: url), Data("local".utf8))
      XCTAssertEqual(backend.persistenceState.isDirty, operation == .saveToOperation)
    }
  }

  func testAllExSaveRoutesIncludingForceCancelWithoutWritingOrClosing() throws {
    for kind in [EVDocumentHostRequest.Kind.write, .saveAs, .writeQuit, .xit, .writeAll] {
      for forced in [false, true] {
        for explicitPath in [false, true] {
          if kind == .saveAs && !explicitPath || kind == .writeAll && explicitPath { continue }
          let (document, backend, url) = try fixture()
          let window = EVDocumentWindowController(document: document, editorSurface: Surface())
          document.addWindowController(window)
          window.showWindow(nil)
          defer { window.close() }
          backend.data = Data("local".utf8); backend.persistenceState.isDirty = true
          try Data("external".utf8).write(to: url, options: .atomic)
          var prompts = 0
          document.externalSaveDecisionHandler = { _ in prompts += 1; return false }
          let request = EVDocumentHostRequest(kind: kind, documentID: 1, documentRevision: 0,
            force: forced, path: explicitPath ? url.path : nil)
          let cancelled = expectation(description: "cancel \(kind), force \(forced), explicit \(explicitPath)")
          window.perform(documentHostRequests: [request]) { result in
            if case .success = result { XCTFail("Cancelled save must fail") }
            cancelled.fulfill()
          }
          wait(for: [cancelled], timeout: 5)
          XCTAssertEqual(prompts, 1)
          XCTAssertTrue(window.window?.isVisible ?? false)
          XCTAssertTrue(backend.persistenceState.isDirty)
          XCTAssertEqual(try Data(contentsOf: url), Data("external".utf8))

          document.externalSaveDecisionHandler = { _ in prompts += 1; return true }
          let saved = expectation(description: "save anyway \(kind)")
          window.perform(documentHostRequests: [request]) { result in
            if case .failure(let error) = result { XCTFail(error.localizedDescription) }
            saved.fulfill()
          }
          wait(for: [saved], timeout: 5)
          XCTAssertEqual(prompts, 2, "Save Anyway accepts this disk state once")
          XCTAssertEqual(try Data(contentsOf: url), Data("local".utf8))
          XCTAssertFalse(backend.persistenceState.isDirty)
          XCTAssertNil(check(document))
          if kind == .writeQuit || kind == .xit { XCTAssertFalse(window.window?.isVisible ?? true) }
        }
      }
    }
  }

  func testForcedRangedWriteQuitStillAsksAndDoesNotCloseOnCancel() throws {
    let (document, backend, url) = try fixture()
    let window = EVDocumentWindowController(document: document, editorSurface: Surface())
    document.addWindowController(window); window.showWindow(nil)
    defer { window.close() }
    backend.data = Data("local".utf8); backend.persistenceState.isDirty = true
    try Data("external".utf8).write(to: url, options: .atomic)
    var prompts = 0
    document.externalSaveDecisionHandler = { _ in prompts += 1; return false }
    let request = EVDocumentHostRequest(kind: .writeQuit, documentID: 1, documentRevision: 0,
      force: true, hardLineRange: 0...0)
    let completed = expectation(description: "cancel forced ranged write")
    window.perform(documentHostRequests: [request]) { result in
      if case .success = result { XCTFail("The cancelled ranged write must fail") }
      completed.fulfill()
    }
    wait(for: [completed], timeout: 5)
    XCTAssertEqual(prompts, 1)
    XCTAssertTrue(window.window?.isVisible ?? false)
    XCTAssertTrue(backend.persistenceState.isDirty)
    XCTAssertEqual(try Data(contentsOf: url), Data("external".utf8))
  }

  func testRetargetedSymbolicLinkStillChecksTheLoadedFingerprint() throws {
    let (document, _, original) = try fixture()
    let directory = original.deletingLastPathComponent()
    let alias = directory.appendingPathComponent("alias.txt")
    let replacement = directory.appendingPathComponent("replacement.txt")
    try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: original)
    document.fileURL = alias
    try Data("replacement".utf8).write(to: replacement)
    try FileManager.default.removeItem(at: alias)
    try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: replacement)
    var prompts = 0
    document.externalSaveDecisionHandler = { change in
      XCTAssertEqual(change, .replaced); prompts += 1; return false
    }
    XCTAssertThrowsError(try document.authorizeExternalWrite(to: alias))
    XCTAssertThrowsError(try document.authorizeExternalWrite(to: EVDocumentIdentity.canonicalURL(alias)))
    XCTAssertEqual(prompts, 2)
    XCTAssertEqual(try Data(contentsOf: replacement), Data("replacement".utf8))
  }

  func testNativeWriterRechecksAfterSnapshotPreparation() throws {
    let (document, backend, url) = try fixture()
    backend.data = Data("local".utf8); backend.persistenceState.isDirty = true
    backend.beforeSaveSnapshot = { try! Data("late external write".utf8).write(to: url, options: .atomic) }
    var prompts = 0
    document.externalSaveDecisionHandler = { _ in prompts += 1; return false }
    let completed = expectation(description: "late native conflict")
    document.save(to: url, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
      XCTAssertEqual((error as? CocoaError)?.code, .userCancelled); completed.fulfill()
    }
    wait(for: [completed], timeout: 5)
    XCTAssertEqual(prompts, 1)
    XCTAssertEqual(try Data(contentsOf: url), Data("late external write".utf8))
    XCTAssertTrue(backend.persistenceState.isDirty)
  }

  func testDeletedFileAndFurtherQueuedChangesRequireDistinctChoices() throws {
    let (document, backend, url) = try fixture()
    backend.data = Data("local".utf8)
    try FileManager.default.removeItem(at: url)
    var choices: [EVExternalFileChange] = []
    document.externalSaveDecisionHandler = { choices.append($0); return true }
    let accepted = try document.authorizeExternalWrite(to: url)
    XCTAssertEqual(choices, [.deleted])
    _ = try document.authorizeExternalWrite(to: url, since: accepted.fingerprint)
    XCTAssertEqual(choices.count, 1)
    try Data("new writer".utf8).write(to: url)
    document.externalSaveDecisionHandler = { choices.append($0); return false }
    XCTAssertThrowsError(try document.authorizeExternalWrite(to: url, since: accepted.fingerprint))
    XCTAssertEqual(choices.count, 2)
    XCTAssertEqual(try Data(contentsOf: url), Data("new writer".utf8))
    try FileManager.default.removeItem(at: url)
    document.externalSaveDecisionHandler = { choices.append($0); return true }
    let saved = expectation(description: "recreate deleted current file")
    document.saveHostRevision(documentID: 1, documentRevision: 0, force: true,
      to: url, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
      XCTAssertNil(error); saved.fulfill()
    }
    wait(for: [saved], timeout: 5)
    XCTAssertEqual(try Data(contentsOf: url), Data("local".utf8))
    XCTAssertNil(check(document))
  }
}
