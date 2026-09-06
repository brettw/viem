import AppKit
import XCTest
@testable import EvimAppShell

@MainActor
final class EVRecoveryDocumentTests: XCTestCase {
    private final class Surface: EVEditorSurface {
        let viewController = NSViewController()
        var statusBarState = EVStatusBarState()
        var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
        func perform(menuCommand: EVMenuCommand, sender: Any?) {}
        func presentation(for command: EVMenuCommand) -> EVMenuItemPresentation { .disabled }
    }
    private final class Backend: EVDocumentBackend {
        var sourceDidChange: (() -> Void)?
        var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)?
        var persistenceState = EVDocumentPersistenceState(documentID: 8, documentRevision: 1)
        var sourceFormat = EVSourceFormat.plainText
        var data = Data()
        var recoveryReads: [EVRecoverySnapshot] = []
        var snapshotCount = 0
        var readOnly = false
        var acknowledgementAction: (() throws -> Void)?
        func makeEditorSurface() -> any EVEditorSurface { Surface() }
        func read(source: Data, typeName: String) throws { data = source; sourceFormat = EVDocument.sourceFormat(forTypeName: typeName) ?? .plainText }
        func serializedSource(typeName: String) throws -> Data { data }
        func nativeSaveSnapshot(typeName: String) throws -> EVDocumentSaveSnapshot { EVDocumentSaveSnapshot(data: data, documentID: 8, documentRevision: persistenceState.documentRevision) }
        func acknowledgeNativeSave(_ snapshot: EVDocumentSaveSnapshot) throws { try acknowledgementAction?(); persistenceState.isDirty = false; persistenceStateDidChange?(persistenceState) }
        func recoverySnapshot() throws -> EVRecoverySnapshot {
            snapshotCount += 1
            return EVRecoverySnapshot(source: data, format: sourceFormat, encoding: 1, fileFormat: 1, documentID: 8, documentRevision: persistenceState.documentRevision)
        }
        func restoreRecovery(_ snapshot: EVRecoverySnapshot) throws {
            recoveryReads.append(snapshot); data = snapshot.source; sourceFormat = snapshot.format
            persistenceState.isDirty = true; persistenceStateDidChange?(persistenceState)
        }
        func setReadOnly(_ value: Bool) throws { readOnly = value }
        func edit(_ text: String) { data = Data(text.utf8); persistenceState.isDirty = true; persistenceState.documentRevision += 1; sourceDidChange?(); persistenceStateDidChange?(persistenceState) }
    }
    private func fixture() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("evim-recovery-document-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let target = directory.appendingPathComponent("writing.txt")
        try Data("original".utf8).write(to: target)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return target
    }

    func testRecoverLoadsUnsavedBytesAndLeavesOriginalAndOtherOwnerUntouched() throws {
        let target = try fixture()
        let old = try EVRecoveryStore.claim(for: target)
        let snapshot = EVRecoverySnapshot(source: Data("unsaved".utf8), format: .markdownSource, encoding: 2, fileFormat: 3, documentID: 7, documentRevision: 3)
        old.write(snapshot); old.drainForTesting()
        let backend = Backend()
        let recovered = EVDocument(editorBackend: backend)
        recovered.recoveryDecisionHandler = { candidates in .recover(candidates.firstIndex { $0.snapshot != nil }!) }
        try recovered.read(from: target, ofType: EVDocument.plainTextType)
        recovered.fileURL = target
        recovered.drainRecoveryForTesting()
        XCTAssertEqual(backend.recoveryReads, [snapshot])
        XCTAssertTrue(recovered.wasRecovered)
        XCTAssertTrue(recovered.isDocumentEdited)
        XCTAssertNotEqual(recovered.recoveryURLForTesting, old.url)
        XCTAssertEqual(try Data(contentsOf: target), Data("original".utf8))
        recovered.close()
        XCTAssertTrue(FileManager.default.fileExists(atPath: old.url.path))
        old.closeAndRemove(); old.drainForTesting()
    }

    func testReadOnlyBlocksAllUnforcedWriteEntryPointsAndNativeSaveCanCancel() throws {
        let target = try fixture(), backend = Backend()
        let reader = EVDocument(editorBackend: backend)
        try reader.read(from: target, ofType: EVDocument.plainTextType)
        defer { reader.close() }
        try reader.setReadOnly(true)
        backend.edit("edited")
        XCTAssertTrue(backend.readOnly)
        XCTAssertThrowsError(try reader.write(to: target, ofType: EVDocument.plainTextType))
        XCTAssertThrowsError(try reader.writeSafely(to: target, ofType: EVDocument.plainTextType, for: .saveOperation))
        var hostError: Error?
        reader.saveHostRevision(documentID: 8, documentRevision: backend.persistenceState.documentRevision, to: target, ofType: EVDocument.plainTextType, for: .saveOperation) { hostError = $0 }
        XCTAssertNotNil(hostError)
        reader.readOnlySaveDecisionHandler = { false }
        var nativeError: Error?
        reader.save(to: target, ofType: EVDocument.plainTextType, for: .saveOperation) { nativeError = $0 }
        XCTAssertNotNil(nativeError)
        XCTAssertEqual(try Data(contentsOf: target), Data("original".utf8))
    }

    func testReadOnlyExplicitForceWritesAndAcknowledgesExactSnapshot() async throws {
        let target = try fixture(), backend = Backend()
        let document = EVDocument(editorBackend: backend)
        try document.read(from: target, ofType: EVDocument.plainTextType)
        document.fileURL = target
        defer { document.close() }
        try document.setReadOnly(true)
        backend.edit("force saved")
        let saved = expectation(description: "forced save")
        document.saveHostRevision(documentID: 8, documentRevision: backend.persistenceState.documentRevision, force: true, to: target, ofType: EVDocument.plainTextType, for: .saveOperation) { error in XCTAssertNil(error); saved.fulfill() }
        await fulfillment(of: [saved], timeout: 3)
        XCTAssertEqual(try Data(contentsOf: target), Data("force saved".utf8))
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertTrue(document.isReadOnly, "A forced save does not silently clear the readonly policy")
    }

    func testFailedRecoveryRebindingKeepsThePreviousValidSnapshot() async throws {
        let original = try fixture(), backend = Backend()
        let document = EVDocument(editorBackend: backend)
        try document.read(from: original, ofType: EVDocument.plainTextType)
        defer { document.close() }
        backend.edit("last good")
        document.flushRecoverySnapshot(); document.drainRecoveryForTesting()
        let previousURL = try XCTUnwrap(document.recoveryURLForTesting)
        let previous = try Data(contentsOf: previousURL)
        let destination = original.deletingLastPathComponent().appendingPathComponent("renamed.txt")
        document.recoveryStoreFactory = { _ in throw EVRecoveryError.noAvailableSlot }
        document.configureRecovery(for: destination)
        document.flushRecoverySnapshot()
        XCTAssertEqual(document.recoveryURLForTesting, previousURL)
        XCTAssertEqual(try Data(contentsOf: previousURL), previous)
        XCTAssertNotNil(document.recoveryFailure)

        document.recoveryStoreFactory = { target in
            let candidate = try EVRecoveryStore.claim(for: target)
            let collision = candidate.url.deletingLastPathComponent().appendingPathComponent(".evim-recovery-\(candidate.owner.uuidString)-1.tmp")
            try Data().write(to: collision)
            return candidate
        }
        document.configureRecovery(for: destination)
        document.drainRecoveryForTesting()
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertEqual(try Data(contentsOf: previousURL), previous, "Failure to publish the replacement must retain the last good recovery")
        XCTAssertNotNil(document.recoveryFailure)
    }

    func testSaveAsRebindsRecoveryAndSubsequentIdleEditUsesNewTarget() async throws {
        let original = try fixture(), backend = Backend()
        let document = EVDocument(editorBackend: backend)
        document.recoveryIdleDelay = 0.03
        try document.read(from: original, ofType: EVDocument.plainTextType)
        document.fileURL = original
        defer { document.close() }
        backend.edit("save as contents")
        document.flushRecoverySnapshot(); document.drainRecoveryForTesting()
        let oldSlot = try XCTUnwrap(document.recoveryURLForTesting)
        let destination = original.deletingLastPathComponent().appendingPathComponent("renamed.txt")
        let saved = expectation(description: "Save As")
        document.save(to: destination, ofType: EVDocument.plainTextType, for: .saveAsOperation) { error in XCTAssertNil(error); saved.fulfill() }
        await fulfillment(of: [saved], timeout: 3)
        backend.edit("new target idle contents")
        try await Task.sleep(for: .milliseconds(120))
        document.drainRecoveryForTesting()
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertFalse(FileManager.default.fileExists(atPath: oldSlot.path))
        XCTAssertEqual(EVRecoveryStore.candidates(for: destination).first?.snapshot?.source, Data("new target idle contents".utf8))
        XCTAssertEqual(try Data(contentsOf: original), Data("original".utf8))
        XCTAssertEqual(try Data(contentsOf: destination), Data("save as contents".utf8))
        let activeSlot = try XCTUnwrap(document.recoveryURLForTesting)
        document.close()
        XCTAssertFalse(FileManager.default.fileExists(atPath: activeSlot.path), "Actual document close, including :q!, finishes owned cleanup")
    }

    func testSuccessfulSaveAsKeepsNewRecoveryBindingWhenAcknowledgementIsStale() async throws {
        let original = try fixture(), backend = Backend()
        let document = EVDocument(editorBackend: backend)
        document.recoveryIdleDelay = 0.03
        try document.read(from: original, ofType: EVDocument.plainTextType)
        document.fileURL = original
        defer { document.close() }
        backend.edit("saved revision")
        backend.acknowledgementAction = {
            backend.edit("newer unsaved revision")
            throw EVDocumentHostError.staleRequest
        }
        let destination = original.deletingLastPathComponent().appendingPathComponent("new-target.txt")
        let saved = expectation(description: "stale acknowledgement")
        document.save(to: destination, ofType: EVDocument.plainTextType, for: .saveAsOperation) { error in XCTAssertNotNil(error); saved.fulfill() }
        await fulfillment(of: [saved], timeout: 3)
        try await Task.sleep(for: .milliseconds(120))
        document.drainRecoveryForTesting()
        XCTAssertEqual(document.fileURL, destination)
        XCTAssertTrue(backend.persistenceState.isDirty)
        XCTAssertEqual(EVRecoveryStore.candidates(for: destination).first?.snapshot?.source, Data("newer unsaved revision".utf8))
        XCTAssertEqual(try Data(contentsOf: destination), Data("saved revision".utf8))
        XCTAssertEqual(try Data(contentsOf: original), Data("original".utf8))
    }

    func testFailedURLReadPreservesPendingRecoveryForCurrentDocument() async throws {
        let original = try fixture(), backend = Backend()
        let document = EVDocument(editorBackend: backend)
        document.recoveryIdleDelay = 0.03
        try document.read(from: original, ofType: EVDocument.plainTextType)
        defer { document.close() }
        backend.edit("pending current contents")
        let slot = try XCTUnwrap(document.recoveryURLForTesting)
        let absent = original.deletingLastPathComponent().appendingPathComponent("missing.txt")
        XCTAssertThrowsError(try document.read(from: absent, ofType: EVDocument.plainTextType))
        try await Task.sleep(for: .milliseconds(120))
        document.drainRecoveryForTesting()
        XCTAssertEqual(document.recoveryURLForTesting, slot)
        XCTAssertEqual(EVRecoveryStore.candidates(for: original).first?.snapshot?.source, Data("pending current contents".utf8))
    }

    func testNativeReadOnlySaveRequiresAndHonorsConfirmation() async throws {
        let target = try fixture(), backend = Backend()
        let document = EVDocument(editorBackend: backend)
        try document.read(from: target, ofType: EVDocument.plainTextType)
        document.fileURL = target
        defer { document.close() }
        try document.setReadOnly(true)
        backend.edit("confirmed save")
        var confirmations = 0
        document.readOnlySaveDecisionHandler = { confirmations += 1; return true }
        let saved = expectation(description: "confirmed native save")
        document.save(to: target, ofType: EVDocument.plainTextType, for: .saveOperation) { error in XCTAssertNil(error); saved.fulfill() }
        await fulfillment(of: [saved], timeout: 3)
        XCTAssertEqual(confirmations, 1)
        XCTAssertEqual(try Data(contentsOf: target), Data("confirmed save".utf8))
        XCTAssertTrue(document.isReadOnly)
    }

    func testForeignSwapOffersReadOnlyAndCancelWithoutAdoptingItsBytes() throws {
        let target = try fixture()
        let foreign = target.deletingLastPathComponent().appendingPathComponent(".writing.txt.swp")
        let bytes = Data([0x62, 0x30, 0x56, 0x49, 0x4d, 0xff])
        try bytes.write(to: foreign)
        let backend = Backend(), document = EVDocument(editorBackend: Backend())
        document.recoveryDecisionHandler = { candidates in
            XCTAssertTrue(candidates.contains { $0.url == foreign && $0.snapshot == nil && !$0.isEVimRecovery })
            return .readOnly
        }
        try document.read(from: target, ofType: EVDocument.plainTextType)
        XCTAssertTrue(document.isReadOnly)
        document.close()
        let cancelled = EVDocument(editorBackend: backend)
        cancelled.recoveryDecisionHandler = { _ in .cancel }
        XCTAssertThrowsError(try cancelled.read(from: target, ofType: EVDocument.plainTextType))
        XCTAssertEqual(backend.data, Data())
        XCTAssertNil(cancelled.recoveryURLForTesting)
        XCTAssertEqual(try Data(contentsOf: foreign), bytes)
        cancelled.close()
    }

    func testIdleRecoveryCoalescesEditsAndNeverAutosavesOriginalInPlace() async throws {
        let target = try fixture(), backend = Backend()
        let document = EVDocument(editorBackend: backend)
        document.recoveryIdleDelay = 0.05
        try document.read(from: target, ofType: EVDocument.plainTextType)
        document.fileURL = target
        defer { document.close() }
        let initial = backend.snapshotCount
        backend.edit("one"); backend.edit("two"); backend.edit("latest")
        XCTAssertEqual(backend.snapshotCount, initial, "Source is copied once after idle, never on each keystroke")
        try await Task.sleep(for: .milliseconds(130))
        document.drainRecoveryForTesting()
        XCTAssertEqual(backend.snapshotCount, initial + 1)
        XCTAssertEqual(EVRecoveryStore.candidates(for: target).first?.snapshot?.source, Data("latest".utf8))
        XCTAssertEqual(try Data(contentsOf: target), Data("original".utf8))
        XCTAssertFalse(EVDocument.autosavesInPlace)
        var error: Error?
        document.save(to: target, ofType: EVDocument.plainTextType, for: .autosaveInPlaceOperation) { error = $0 }
        XCTAssertNil(error)
        XCTAssertEqual(try Data(contentsOf: target), Data("original".utf8))
    }
}
