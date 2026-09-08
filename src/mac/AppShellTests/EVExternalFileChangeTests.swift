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
    func makeEditorSurface() -> any EVEditorSurface { Surface() }
    func read(source: Data, typeName: String) throws { data = source }
    func serializedSource(typeName: String) throws -> Data { data }
    func nativeSaveSnapshot(typeName: String) throws -> EVDocumentSaveSnapshot {
      EVDocumentSaveSnapshot(data: data, documentID: persistenceState.documentID, documentRevision: persistenceState.documentRevision)
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
    XCTAssertThrowsError(try document.validateExternalWrite(to: url, force: false))
    XCTAssertNoThrow(try document.validateExternalWrite(to: url, force: true))
    try FileManager.default.removeItem(at: url)
    XCTAssertEqual(check(document), .deleted)
    XCTAssertThrowsError(try document.validateExternalWrite(to: url, force: false))
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
  func testMenuSaveCanCancelConflictAndForcedExSaveReplacesOnlyAfterAuthorization() throws {
    let (document, backend, url) = try fixture()
    try Data("external".utf8).write(to: url, options: .atomic)
    backend.data = Data("local".utf8); backend.persistenceState.isDirty = true
    document.externalSaveDecisionHandler = { _ in false }
    let cancelled = expectation(description: "cancel conflict")
    document.save(to: url, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
      XCTAssertNotNil(error); cancelled.fulfill()
    }
    wait(for: [cancelled], timeout: 5)
    XCTAssertEqual(try Data(contentsOf: url), Data("external".utf8))
    let rejected = expectation(description: "reject unforced Ex")
    document.saveHostRevision(documentID: 1, documentRevision: 0, to: url, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
      XCTAssertTrue(error is EVExternalFileError); rejected.fulfill()
    }
    wait(for: [rejected], timeout: 5)
    let forced = expectation(description: "forced Ex saves")
    document.saveHostRevision(documentID: 1, documentRevision: 0, force: true, to: url, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
      XCTAssertNil(error); forced.fulfill()
    }
    wait(for: [forced], timeout: 5)
    XCTAssertEqual(try Data(contentsOf: url), Data("local".utf8)); XCTAssertNil(check(document))
    try FileManager.default.removeItem(at: url)
    XCTAssertEqual(check(document), .deleted)
    let recreated = expectation(description: "forced Ex recreates deleted source")
    document.saveHostRevision(documentID: 1, documentRevision: 0, force: true, to: url, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
      XCTAssertNil(error); recreated.fulfill()
    }
    wait(for: [recreated], timeout: 5)
    XCTAssertEqual(try Data(contentsOf: url), Data("local".utf8))
    XCTAssertNil(document.externalFileChange)
    XCTAssertNil(check(document))
  }
}
