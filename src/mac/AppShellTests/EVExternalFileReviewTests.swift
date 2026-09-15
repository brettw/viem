import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVExternalFileReviewTests: XCTestCase {
  private final class Backend: EVDocumentBackend {
    var sourceDidChange: (() -> Void)?
    var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)?
    var persistenceState = EVDocumentPersistenceState(documentID: 71, documentRevision: 0)
    var sourceFormat = EVSourceFormat.plainText
    var data = Data()
    var failReads = false
    func makeEditorSurface() -> any EVEditorSurface { Surface() }
    func read(source: Data, typeName: String) throws {
      if failReads { throw CocoaError(.fileReadCorruptFile) }
      sourceFormat = EVDocument.sourceFormat(forTypeName: typeName) ?? .plainText
      data = source
      persistenceState.documentRevision += 1
      persistenceState.isDirty = false
      persistenceStateDidChange?(persistenceState)
    }
    func edit(_ text: String) {
      data = Data(text.utf8)
      persistenceState.documentRevision += 1
      persistenceState.isDirty = true
      sourceDidChange?()
      persistenceStateDidChange?(persistenceState)
    }
    func serializedSource(typeName: String) throws -> Data { data }
    func nativeSaveSnapshot(typeName: String) throws -> EVDocumentSaveSnapshot {
      EVDocumentSaveSnapshot(data: data, documentID: persistenceState.documentID,
        documentRevision: persistenceState.documentRevision)
    }
    func acknowledgeNativeSave(_ snapshot: EVDocumentSaveSnapshot) throws {
      persistenceState.isDirty = false
      persistenceStateDidChange?(persistenceState)
    }
  }
  private final class Surface: EVEditorSurface {
    let viewController = NSViewController()
    var statusBarState = EVStatusBarState()
    var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
    func perform(menuCommand: EVMenuCommand, sender: Any?) {}
    func presentation(for: EVMenuCommand) -> EVMenuItemPresentation { .disabled }
  }

  private func fixture() throws -> (EVDocument, Backend, URL) {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-file-review-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let url = directory.appendingPathComponent("source.txt")
    try Data("original".utf8).write(to: url)
    let backend = Backend()
    let document = EVDocument(editorBackend: backend)
    document.recordRecentDocument = { _ in }
    try document.read(from: url, ofType: EVDocument.plainTextType)
    document.fileURL = url
    document.fileType = EVDocument.plainTextType
    addTeardownBlock {
      MainActor.assumeIsolated { document.close() }
      try? FileManager.default.removeItem(at: directory)
    }
    return (document, backend, url)
  }

  @discardableResult
  private func review(_ document: EVDocument) -> Result<String?, Error> {
    let done = expectation(description: "check and review completed")
    var result: Result<String?, Error> = .success(nil)
    document.checkForExternalChangesAndReview { result = $0; done.fulfill() }
    wait(for: [done], timeout: 5)
    return result
  }

  func testKeepBufferDeduplicatesOneDiskStateButPreservesOverwriteProtection() throws {
    let (document, backend, url) = try fixture()
    backend.edit("unsaved text")
    let baseline = document.fileBaseline
    var prompts = 0
    document.externalFileReviewDecisionHandler = { _, options, decide in
      prompts += 1
      XCTAssertTrue(options.canReload)
      XCTAssertTrue(options.discardsUnsavedChanges)
      decide(.keepBuffer)
    }
    try Data("outside one".utf8).write(to: url)
    try review(document).get()
    try review(document).get()
    XCTAssertEqual(prompts, 1)
    XCTAssertEqual(backend.data, Data("unsaved text".utf8))
    XCTAssertTrue(backend.persistenceState.isDirty)
    XCTAssertEqual(document.fileBaseline, baseline)
    var overwrites = 0
    document.externalSaveDecisionHandler = { _ in overwrites += 1; return false }
    XCTAssertThrowsError(try document.authorizeExternalWrite(to: url))
    XCTAssertEqual(overwrites, 1)
    try Data("outside two".utf8).write(to: url)
    try review(document).get()
    XCTAssertEqual(prompts, 2)
  }

  func testLoadFileRequiresChoiceAndRefreshesSharedBufferAndBaseline() throws {
    for dirty in [false, true] {
      let (document, backend, url) = try fixture()
      if dirty { backend.edit("unsaved") }
      let previousRevision = backend.persistenceState.documentRevision
      var prompts = 0
      document.externalFileReviewDecisionHandler = { change, options, decide in
        prompts += 1
        XCTAssertEqual(change, .replaced)
        XCTAssertTrue(options.canReload)
        XCTAssertEqual(options.discardsUnsavedChanges, dirty)
        XCTAssertEqual(backend.data, Data((dirty ? "unsaved" : "original").utf8))
        decide(.loadFile)
      }
      try Data("new disk text".utf8).write(to: url, options: .atomic)
      let message = try review(document).get()
      XCTAssertNil(message, "The dialog handles a successful reload without extra status output")
      XCTAssertEqual(prompts, 1)
      XCTAssertEqual(backend.data, Data("new disk text".utf8))
      XCTAssertGreaterThan(backend.persistenceState.documentRevision, previousRevision)
      XCTAssertFalse(backend.persistenceState.isDirty)
      XCTAssertFalse(document.isDocumentEdited)
      XCTAssertEqual(document.fileBaseline, try EVFileFingerprint.read(url))
      try review(document).get()
      XCTAssertEqual(prompts, 1)
    }
  }

  func testReturningToBaselineAllowsSameExternalContentsToPromptAgain() throws {
    let (document, _, url) = try fixture()
    var prompts = 0
    document.externalFileReviewDecisionHandler = { _, _, decide in
      prompts += 1; decide(.keepBuffer)
    }
    try Data("external".utf8).write(to: url)
    try review(document).get()
    try Data("original".utf8).write(to: url)
    try review(document).get()
    XCTAssertEqual(prompts, 1)
    try Data("external".utf8).write(to: url)
    try review(document).get()
    XCTAssertEqual(prompts, 2)
  }

  func testReloadPreservesCodeAndSourceViewFormats() throws {
    for format in [EVSourceFormat.code, .markdownSource, .htmlSource] {
      let (document, backend, url) = try fixture()
      backend.sourceFormat = format
      document.externalFileReviewDecisionHandler = { _, _, decide in decide(.loadFile) }
      try Data("# new source".utf8).write(to: url)
      try review(document).get()
      XCTAssertEqual(backend.sourceFormat, format)
      XCTAssertEqual(backend.data, Data("# new source".utf8))
    }
  }

  func testReloadKeepsWatchingSelectedSymlinkAfterCanonicalBaselineReceipt() throws {
    let (document, backend, url) = try fixture()
    let alias = url.deletingLastPathComponent().appendingPathComponent("alias.txt")
    try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: url)
    document.fileURL = alias
    document.externalFileReviewDecisionHandler = { _, _, decide in decide(.loadFile) }
    try Data("reloaded target".utf8).write(to: url, options: .atomic)
    try review(document).get()
    XCTAssertEqual(document.externalFileMonitorURL, alias)
    XCTAssertEqual(backend.data, Data("reloaded target".utf8))

    let other = url.deletingLastPathComponent().appendingPathComponent("other.txt")
    try Data("new alias target".utf8).write(to: other)
    let prompted = expectation(description: "alias retarget triggers live review")
    document.externalFileReviewDecisionHandler = { _, _, decide in
      decide(.keepBuffer); prompted.fulfill()
    }
    try FileManager.default.removeItem(at: alias)
    try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: other)
    wait(for: [prompted], timeout: 5)
    XCTAssertEqual(backend.data, Data("reloaded target".utf8))
  }

  func testReturnToBaselineWhilePromptIsOpenDoesNotAcknowledgeObsoleteChange() throws {
    let (document, _, url) = try fixture()
    let shown = expectation(description: "initial review")
    var pending: ((EVExternalFileDecision) -> Void)?
    var prompts = 0
    document.externalFileReviewDecisionHandler = { _, _, decide in
      prompts += 1
      if prompts == 1 { pending = decide; shown.fulfill() }
      else { decide(.keepBuffer) }
    }
    try Data("external".utf8).write(to: url)
    document.checkForExternalChangesAndReview { _ in }
    wait(for: [shown], timeout: 5)
    try Data("original".utf8).write(to: url)
    try review(document).get()
    pending?(.keepBuffer)
    try Data("external".utf8).write(to: url)
    try review(document).get()
    XCTAssertEqual(prompts, 2)
  }

  func testReloadInstallsOnlyVerifiedBytesWhenPathIsReplacedAfterRead() throws {
    let (document, backend, url) = try fixture()
    let reviewedNewVersion = expectation(description: "later replacement gets its own review")
    var prompts = 0
    document.externalFileSnapshotReader = { target in
      let snapshot = try EVExternalFileSnapshot.read(target)
      try Data("unreviewed later version".utf8).write(to: target, options: .atomic)
      return snapshot
    }
    document.externalFileReviewDecisionHandler = { _, _, decide in
      prompts += 1
      if prompts == 1 { decide(.loadFile) }
      else { decide(.keepBuffer); reviewedNewVersion.fulfill() }
    }
    try Data("reviewed version".utf8).write(to: url)
    try review(document).get()
    wait(for: [reviewedNewVersion], timeout: 5)
    XCTAssertEqual(backend.data, Data("reviewed version".utf8))
    XCTAssertEqual(try Data(contentsOf: url), Data("unreviewed later version".utf8))
    XCTAssertNotEqual(document.fileBaseline, try EVFileFingerprint.read(url))
  }

  func testDeletionKeepsBufferAndRecreationOffersAnotherChoice() throws {
    let (document, backend, url) = try fixture()
    var choices: [EVExternalFileChange] = []
    document.externalFileReviewDecisionHandler = { change, options, decide in
      choices.append(change)
      XCTAssertEqual(options.canReload, change != .deleted)
      decide(.keepBuffer)
    }
    try FileManager.default.removeItem(at: url)
    try review(document).get()
    try review(document).get()
    XCTAssertEqual(choices, [.deleted])
    XCTAssertEqual(backend.data, Data("original".utf8))
    try Data("recreated".utf8).write(to: url)
    try review(document).get()
    XCTAssertEqual(choices.count, 2)
    XCTAssertEqual(backend.data, Data("original".utf8))
  }

  func testMonitorPromptsForAtomicReplacementWithoutActivationOrChecktime() throws {
    let (document, backend, url) = try fixture()
    let prompted = expectation(description: "live file change prompt")
    document.externalFileReviewDecisionHandler = { change, _, decide in
      XCTAssertEqual(change, .replaced)
      decide(.keepBuffer)
      prompted.fulfill()
    }
    try Data("external".utf8).write(to: url, options: .atomic)
    wait(for: [prompted], timeout: 5)
    XCTAssertEqual(backend.data, Data("original".utf8))
  }

  func testSaveDoesNotPromptForItsOwnAtomicReplacement() throws {
    let (document, backend, url) = try fixture()
    backend.edit("saved locally")
    let unwanted = expectation(description: "no prompt for own save")
    unwanted.isInverted = true
    document.externalFileReviewDecisionHandler = { _, _, decide in
      unwanted.fulfill(); decide(.keepBuffer)
    }
    let saved = expectation(description: "native save completed")
    document.save(to: url, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
      XCTAssertNil(error); saved.fulfill()
    }
    wait(for: [saved], timeout: 5)
    try review(document).get()
    wait(for: [unwanted], timeout: 0.35)
    XCTAssertEqual(try Data(contentsOf: url), Data("saved locally".utf8))
  }

  func testSaveToBoundPathUpdatesWatchBaselineWithoutClearingDirtyState() throws {
    let (document, backend, url) = try fixture()
    backend.edit("written copy")
    let unwanted = expectation(description: "no prompt for own Save To")
    unwanted.isInverted = true
    document.externalFileReviewDecisionHandler = { _, _, decide in
      unwanted.fulfill(); decide(.keepBuffer)
    }
    let saved = expectation(description: "Save To completed")
    document.save(to: url, ofType: EVDocument.plainTextType, for: .saveToOperation) { error in
      XCTAssertNil(error); saved.fulfill()
    }
    wait(for: [saved], timeout: 5)
    try review(document).get()
    wait(for: [unwanted], timeout: 0.35)
    XCTAssertEqual(try Data(contentsOf: url), Data("written copy".utf8))
    XCTAssertEqual(document.fileBaseline, try EVFileFingerprint.read(url))
    XCTAssertTrue(backend.persistenceState.isDirty)
    XCTAssertTrue(document.isDocumentEdited)
  }

  func testOnePendingReviewAcrossConcurrentChecksAndNewerEditsNeedAnotherChoice() throws {
    let (document, backend, url) = try fixture()
    let first = expectation(description: "first review")
    let second = expectation(description: "review newer local edit")
    var pending: ((EVExternalFileDecision) -> Void)?
    var prompts = 0
    document.externalFileReviewDecisionHandler = { _, options, decide in
      prompts += 1
      if prompts == 1 { pending = decide; first.fulfill() }
      else {
        XCTAssertTrue(options.discardsUnsavedChanges)
        decide(.keepBuffer); second.fulfill()
      }
    }
    try Data("external".utf8).write(to: url)
    document.checkForExternalChangesAndReview { _ in }
    wait(for: [first], timeout: 5)
    try review(document).get()
    XCTAssertEqual(prompts, 1)
    backend.edit("new edit while prompt open")
    pending?(.loadFile)
    wait(for: [second], timeout: 5)
    XCTAssertEqual(prompts, 2)
    XCTAssertEqual(backend.data, Data("new edit while prompt open".utf8))
  }

  func testDiskChangingDuringPromptIsReviewedAgainBeforeLoading() throws {
    let (document, backend, url) = try fixture()
    let reviewedNewVersion = expectation(description: "new disk version reviewed")
    var prompts = 0
    document.externalFileReviewDecisionHandler = { _, _, decide in
      prompts += 1
      if prompts == 1 {
        try! Data("second external version".utf8).write(to: url, options: .atomic)
        decide(.loadFile)
      } else { decide(.keepBuffer); reviewedNewVersion.fulfill() }
    }
    try Data("first external version".utf8).write(to: url)
    try review(document).get()
    wait(for: [reviewedNewVersion], timeout: 5)
    XCTAssertEqual(backend.data, Data("original".utf8))
  }

  func testRetargetAndCloseInvalidatePendingConsentAndStopOldWatch() throws {
    let (document, backend, url) = try fixture()
    let shown = expectation(description: "review before retarget")
    var pending: ((EVExternalFileDecision) -> Void)?
    document.externalFileReviewDecisionHandler = { _, _, decide in pending = decide; shown.fulfill() }
    try Data("external".utf8).write(to: url)
    document.checkForExternalChangesAndReview { _ in }
    wait(for: [shown], timeout: 5)
    let other = url.deletingLastPathComponent().appendingPathComponent("other.txt")
    try Data("other file".utf8).write(to: other)
    document.fileURL = other
    pending?(.loadFile)
    XCTAssertEqual(backend.data, Data("original".utf8))
    XCTAssertEqual(document.externalFileMonitorURL, other)
    document.close()
    XCTAssertNil(document.externalFileMonitor)
    let ignored = expectation(description: "closed check ignored")
    document.checkForExternalChanges { change in XCTAssertNil(change); ignored.fulfill() }
    wait(for: [ignored], timeout: 5)
  }

  func testFailedReloadPreservesBufferAndCanBeRetried() throws {
    let (document, backend, url) = try fixture()
    document.externalFileReviewDecisionHandler = { _, _, decide in decide(.loadFile) }
    try Data("external".utf8).write(to: url)
    backend.failReads = true
    XCTAssertThrowsError(try review(document).get())
    XCTAssertEqual(backend.data, Data("original".utf8))
    backend.failReads = false
    try review(document).get()
    XCTAssertEqual(backend.data, Data("external".utf8))
  }
}
