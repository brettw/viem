import AppKit
import Darwin
import XCTest
@testable import ViemAppShell

final class EVRecoveryStoreTests: XCTestCase {
    private func fixture() throws -> (URL, URL) {
        let directory = FileManager.default.temporaryDirectory.resolvingSymlinksInPath().appendingPathComponent("viem-recovery-test-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let target = directory.appendingPathComponent("writing.md")
        try Data("original".utf8).write(to: target)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return (directory, target)
    }
    private func snapshot(_ text: String, revision: UInt64 = 1) -> EVRecoverySnapshot {
        EVRecoverySnapshot(source: Data(text.utf8), format: .markdownSource, encoding: 2, fileFormat: 3, documentID: 19, documentRevision: revision)
    }
    private func staleRecord(for target: URL) -> EVRecoveryRecord {
        XCTAssertEqual(kill(Int32.max, 0), -1)
        XCTAssertEqual(errno, ESRCH)
        return EVRecoveryRecord(version: 1, owner: UUID(), processID: Int32.max,
            host: ProcessInfo.processInfo.hostName, targetPath: target.path,
            created: Date(), updated: Date(), snapshot: snapshot("abandoned contents"))
    }
    private func staleCandidate(for target: URL) throws -> EVRecoveryCandidate {
        let url = target.deletingLastPathComponent().appendingPathComponent(".writing.md.viem.swp")
        try staleRecord(for: target).encoded().write(to: url)
        let candidate = try XCTUnwrap(EVRecoveryStore.candidates(for: target).first { $0.url == url })
        XCTAssertTrue(candidate.canDelete)
        return candidate
    }
    private func inode(of url: URL) throws -> ino_t {
        var attributes = stat()
        guard lstat(url.path, &attributes) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
        return attributes.st_ino
    }

    func testStaleRecoveryRetiresOnlyAfterNewSnapshotCommits() throws {
        let (_, target) = try fixture()
        let old = try staleCandidate(for: target)
        let original = try Data(contentsOf: old.url)
        let store = try EVRecoveryStore.claim(for: target)
        defer { store.closeAndRemove(); store.drainForTesting() }
        store.retireAfterNextCommit([old])
        store.drainForTesting()
        XCTAssertEqual(try Data(contentsOf: old.url), original, "Claiming an empty slot is not a replacement backup")
        let replacement = try XCTUnwrap(old.snapshot)
        let committed = expectation(description: "replacement committed")
        store.write(replacement, completion: { error in XCTAssertNil(error); committed.fulfill() })
        wait(for: [committed], timeout: 3)
        XCTAssertFalse(FileManager.default.fileExists(atPath: old.url.path))
        XCTAssertEqual(EVRecoveryStore.candidates(for: target).first { $0.url == store.url }?.snapshot, replacement)
        XCTAssertEqual(try Data(contentsOf: target), Data("original".utf8))
    }

    func testLiveForeignRemoteInvalidAndSymlinkCandidatesCannotBeDeleted() throws {
        let (directory, target) = try fixture()
        let base = staleRecord(for: target)
        let records = [
            EVRecoveryRecord(version: 1, owner: UUID(), processID: getpid(), host: base.host, targetPath: target.path, created: base.created, updated: base.updated, snapshot: base.snapshot),
            EVRecoveryRecord(version: 1, owner: UUID(), processID: base.processID, host: "another-host", targetPath: target.path, created: base.created, updated: base.updated, snapshot: base.snapshot),
            EVRecoveryRecord(version: 1, owner: UUID(), processID: 0, host: base.host, targetPath: target.path, created: base.created, updated: base.updated, snapshot: base.snapshot),
            EVRecoveryRecord(version: 1, owner: UUID(), processID: -1, host: base.host, targetPath: target.path, created: base.created, updated: base.updated, snapshot: base.snapshot),
            EVRecoveryRecord(version: 999, owner: UUID(), processID: base.processID, host: base.host, targetPath: target.path, created: base.created, updated: base.updated, snapshot: base.snapshot),
            EVRecoveryRecord(version: 1, owner: UUID(), processID: base.processID, host: base.host, targetPath: target.path + ".other", created: base.created, updated: base.updated, snapshot: base.snapshot),
        ]
        var originals: [URL: Data] = [:]
        for (index, record) in records.enumerated() {
            let url = directory.appendingPathComponent(".writing.md.viem.\(index + 1).swp")
            let bytes = try record.encoded()
            try bytes.write(to: url)
            originals[url] = bytes
        }
        let foreign = directory.appendingPathComponent(".writing.md.swp")
        let foreignBytes = Data([0x62, 0x30, 0x56, 0x49, 0x4d, 0xff])
        try foreignBytes.write(to: foreign)
        originals[foreign] = foreignBytes
        let payload = directory.appendingPathComponent("symlink-target")
        let payloadBytes = try base.encoded()
        try payloadBytes.write(to: payload)
        let symlink = directory.appendingPathComponent(".writing.md.viem.7.swp")
        try FileManager.default.createSymbolicLink(at: symlink, withDestinationURL: payload)
        let candidates = EVRecoveryStore.candidates(for: target)
        XCTAssertEqual(candidates.count, originals.count + 1)
        XCTAssertTrue(candidates.allSatisfy { !$0.canDelete })
        let store = try EVRecoveryStore.claim(for: target)
        defer { store.closeAndRemove(); store.drainForTesting() }
        store.retireAfterNextCommit(candidates)
        store.write(snapshot("disk version")); store.drainForTesting()
        for (url, bytes) in originals { XCTAssertEqual(try Data(contentsOf: url), bytes) }
        XCTAssertEqual(try FileManager.default.destinationOfSymbolicLink(atPath: symlink.path), payload.path)
        XCTAssertEqual(try Data(contentsOf: payload), payloadBytes)
    }

    func testIdenticalBytesInReplacementInodeAreNotRetired() throws {
        let (_, target) = try fixture()
        let old = try staleCandidate(for: target)
        let bytes = try Data(contentsOf: old.url)
        let oldInode = try inode(of: old.url)
        let store = try EVRecoveryStore.claim(for: target)
        defer { store.closeAndRemove(); store.drainForTesting() }
        store.retireAfterNextCommit([old])
        try bytes.write(to: old.url, options: .atomic)
        XCTAssertNotEqual(try inode(of: old.url), oldInode)
        let rejected = expectation(description: "replaced inode preserved")
        store.write(snapshot("fresh backup"), completion: { error in XCTAssertNotNil(error); rejected.fulfill() })
        wait(for: [rejected], timeout: 3)
        XCTAssertEqual(try Data(contentsOf: old.url), bytes)
        XCTAssertEqual(EVRecoveryStore.candidates(for: target).first { $0.url == store.url }?.snapshot?.source, Data("fresh backup".utf8))
    }

    func testChangedBytesInSameInodeAreNotRetiredAndFailurePersists() throws {
        let (_, target) = try fixture()
        let old = try staleCandidate(for: target)
        let oldInode = try inode(of: old.url)
        let store = try EVRecoveryStore.claim(for: target)
        defer { store.closeAndRemove(); store.drainForTesting() }
        store.retireAfterNextCommit([old])
        var changed = try XCTUnwrap(EVRecoveryRecord.decode(Data(contentsOf: old.url)))
        changed.snapshot = snapshot("changed after prompt")
        let bytes = try changed.encoded()
        let handle = try FileHandle(forWritingTo: old.url)
        try handle.truncate(atOffset: 0)
        try handle.write(contentsOf: bytes)
        try handle.synchronize()
        try handle.close()
        XCTAssertEqual(try inode(of: old.url), oldInode)
        let rejected = expectation(description: "changed contents preserved")
        store.write(snapshot("fresh backup"), completion: { error in XCTAssertNotNil(error); rejected.fulfill() })
        wait(for: [rejected], timeout: 3)
        XCTAssertEqual(try Data(contentsOf: old.url), bytes)
        let remembered = expectation(description: "cleanup failure retained")
        store.write(snapshot("later backup", revision: 2), completion: { error in XCTAssertNotNil(error); remembered.fulfill() })
        wait(for: [remembered], timeout: 3)
        XCTAssertEqual(try Data(contentsOf: old.url), bytes)
    }

    func testFailedPublicationRetainsCandidateUntilLaterSuccessfulWrite() throws {
        let (directory, target) = try fixture()
        let old = try staleCandidate(for: target)
        let bytes = try Data(contentsOf: old.url)
        let store = try EVRecoveryStore.claim(for: target)
        defer { store.closeAndRemove(); store.drainForTesting() }
        store.retireAfterNextCommit([old])
        let collision = directory.appendingPathComponent(".viem-recovery-\(store.owner.uuidString)-1.tmp")
        try Data().write(to: collision)
        let failed = expectation(description: "replacement staging failed")
        store.write(snapshot("recovered"), completion: { error in XCTAssertNotNil(error); failed.fulfill() })
        wait(for: [failed], timeout: 3)
        XCTAssertEqual(try Data(contentsOf: old.url), bytes)
        let succeeded = expectation(description: "later replacement committed")
        store.write(snapshot("recovered", revision: 2), completion: { error in XCTAssertNil(error); succeeded.fulfill() })
        wait(for: [succeeded], timeout: 3)
        XCTAssertFalse(FileManager.default.fileExists(atPath: old.url.path))
        XCTAssertEqual(EVRecoveryStore.candidates(for: target).first { $0.url == store.url }?.snapshot?.documentRevision, 2)
    }

    func testSupersededWriteCannotRetireCandidate() throws {
        let (_, target) = try fixture()
        let old = try staleCandidate(for: target)
        let bytes = try Data(contentsOf: old.url)
        let store = try EVRecoveryStore.claim(for: target)
        defer { store.closeAndRemove(); store.drainForTesting() }
        let initial = snapshot("before recovery")
        let superseded = snapshot("superseded", revision: 2)
        let skipped = expectation(description: "superseded job finished without publication")
        store.write(initial, completion: { error in
            XCTAssertNil(error)
            store.retireAfterNextCommit([old])
            // This completion runs on the store queue, so the new job cannot
            // begin until after we invalidate it and return.
            store.write(superseded, completion: { error in XCTAssertNil(error); skipped.fulfill() })
            store.invalidatePendingWrites()
        })
        wait(for: [skipped], timeout: 3)
        XCTAssertEqual(try Data(contentsOf: old.url), bytes)
        XCTAssertEqual(EVRecoveryStore.candidates(for: target).first { $0.url == store.url }?.snapshot, initial)
        store.write(snapshot("current", revision: 3)); store.drainForTesting()
        XCTAssertFalse(FileManager.default.fileExists(atPath: old.url.path))
    }

    func testContendedCleanupIsNonblockingAndDoesNotRetryAfterUnlock() throws {
        let (_, target) = try fixture()
        let old = try staleCandidate(for: target)
        let bytes = try Data(contentsOf: old.url)
        let descriptor = open(old.url.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
        XCTAssertGreaterThanOrEqual(descriptor, 0)
        guard descriptor >= 0 else { return }
        defer { _ = flock(descriptor, LOCK_UN); close(descriptor) }
        XCTAssertEqual(flock(descriptor, LOCK_EX | LOCK_NB), 0)
        let store = try EVRecoveryStore.claim(for: target)
        defer { store.closeAndRemove(); store.drainForTesting() }
        store.retireAfterNextCommit([old])
        let contended = expectation(description: "cleanup does not block on another cleaner")
        store.write(snapshot("first backup"), completion: { error in XCTAssertNotNil(error); contended.fulfill() })
        wait(for: [contended], timeout: 3)
        XCTAssertEqual(try Data(contentsOf: old.url), bytes)
        XCTAssertEqual(flock(descriptor, LOCK_UN), 0)
        let later = expectation(description: "cleanup failure remains after unlock")
        store.write(snapshot("later backup", revision: 2), completion: { error in XCTAssertNotNil(error); later.fulfill() })
        wait(for: [later], timeout: 3)
        XCTAssertEqual(try Data(contentsOf: old.url), bytes, "Cleanup is a one-shot action, not an idle retry")
    }

    func testConcurrentCleanersPreserveBothNewSessions() throws {
        let (_, target) = try fixture()
        let old = try staleCandidate(for: target)
        let first = try EVRecoveryStore.claim(for: target)
        let second = try EVRecoveryStore.claim(for: target)
        defer { first.closeAndRemove(); second.closeAndRemove(); first.drainForTesting(); second.drainForTesting() }
        first.retireAfterNextCommit([old]); second.retireAfterNextCommit([old])
        first.write(snapshot("first")); second.write(snapshot("second"))
        first.drainForTesting(); second.drainForTesting()
        XCTAssertFalse(FileManager.default.fileExists(atPath: old.url.path))
        let candidates = EVRecoveryStore.candidates(for: target)
        XCTAssertEqual(candidates.first { $0.url == first.url }?.snapshot?.source, Data("first".utf8))
        XCTAssertEqual(candidates.first { $0.url == second.url }?.snapshot?.source, Data("second".utf8))
    }

    func testExclusiveSlotsPreserveExistingViemAndForeignVimFiles() throws {
        let (directory, target) = try fixture()
        let foreign = directory.appendingPathComponent(".writing.md.swp")
        try Data([0x62, 0x30, 0x56, 0x49, 0x4d, 0xff]).write(to: foreign)
        let first = try EVRecoveryStore.claim(for: target)
        let second = try EVRecoveryStore.claim(for: target)
        XCTAssertNotEqual(first.url, second.url)
        first.write(snapshot("first")); second.write(snapshot("second"))
        first.drainForTesting(); second.drainForTesting()
        let candidates = EVRecoveryStore.candidates(for: target)
        XCTAssertEqual(candidates.count, 3)
        XCTAssertEqual(Set(candidates.compactMap { $0.snapshot?.source }), Set([Data("first".utf8), Data("second".utf8)]))
        XCTAssertNil(candidates.first { $0.url == foreign }?.snapshot)
        XCTAssertEqual(try Data(contentsOf: target), Data("original".utf8))
        first.closeAndRemove(); first.drainForTesting()
        XCTAssertTrue(FileManager.default.fileExists(atPath: second.url.path))
        XCTAssertEqual(try Data(contentsOf: foreign), Data([0x62, 0x30, 0x56, 0x49, 0x4d, 0xff]))
        second.closeAndRemove(); second.drainForTesting()
    }

    func testHardLinkedSnapshotRemainsRecoverableButCannotAuthorizeDeletion() throws {
        let (directory, target) = try fixture()
        let old = try staleCandidate(for: target)
        let alias = directory.appendingPathComponent("linked-recovery")
        try FileManager.default.linkItem(at: old.url, to: alias)
        let candidate = try XCTUnwrap(EVRecoveryStore.candidates(for: target).first { $0.url == old.url })
        XCTAssertNotNil(candidate.snapshot)
        XCTAssertFalse(candidate.canDelete)
        let store = try EVRecoveryStore.claim(for: target)
        defer { store.closeAndRemove(); store.drainForTesting() }
        store.retireAfterNextCommit([old])
        let refused = expectation(description: "new hard link invalidates earlier deletion consent")
        store.write(snapshot("replacement"), completion: { error in XCTAssertNotNil(error); refused.fulfill() })
        wait(for: [refused], timeout: 3)
        XCTAssertTrue(FileManager.default.fileExists(atPath: old.url.path))
        XCTAssertEqual(try Data(contentsOf: old.url), try Data(contentsOf: alias))
    }

    func testZeroByteSlotAndCollidingStagingFileArePreserved() throws {
        let (directory, target) = try fixture()
        let occupied = directory.appendingPathComponent(".writing.md.viem.swp")
        try Data().write(to: occupied)
        let store = try EVRecoveryStore.claim(for: target)
        XCTAssertNotEqual(store.url, occupied)
        store.write(snapshot("last good")); store.drainForTesting()
        let previous = try Data(contentsOf: store.url)
        let staging = directory.appendingPathComponent(".viem-recovery-\(store.owner.uuidString)-2.tmp")
        try Data().write(to: staging)
        let rejected = expectation(description: "staging collision rejected")
        store.write(snapshot("new"), completion: { error in XCTAssertNotNil(error); rejected.fulfill() })
        wait(for: [rejected], timeout: 3)
        XCTAssertEqual(try Data(contentsOf: store.url), previous)
        XCTAssertEqual(try Data(contentsOf: staging), Data())
        XCTAssertEqual(try Data(contentsOf: occupied), Data())
        store.closeAndRemove(); store.drainForTesting()
    }

    func testSnapshotRoundTripsPhysicalBytesAndInterpretationMetadata() throws {
        let (_, target) = try fixture()
        let store = try EVRecoveryStore.claim(for: target)
        var expected = snapshot("unused")
        expected.source = Data([0xff, 0xe9, 0x0d, 0x00, 0x23])
        store.write(expected); store.drainForTesting()
        let recovered = try XCTUnwrap(EVRecoveryStore.candidates(for: target).first?.snapshot)
        XCTAssertEqual(recovered, expected)
        XCTAssertEqual(try FileManager.default.attributesOfItem(atPath: store.url.path)[.posixPermissions] as? Int, 0o600)
        store.closeAndRemove(); store.drainForTesting()
    }

    func testQueuedOlderSnapshotsCannotReplaceLatestAndClosedStoreCannotRecreateSlot() throws {
        let (_, target) = try fixture()
        let store = try EVRecoveryStore.claim(for: target)
        for revision in 1...30 { store.write(snapshot(String(revision), revision: UInt64(revision))) }
        store.drainForTesting()
        XCTAssertEqual(EVRecoveryStore.candidates(for: target).first?.snapshot?.documentRevision, 30)
        store.write(snapshot(String(repeating: "large", count: 100_000), revision: 31))
        store.closeAndRemove(); store.drainForTesting()
        store.write(snapshot("too late", revision: 32)); store.drainForTesting()
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.url.path))
        XCTAssertEqual(try Data(contentsOf: target), Data("original".utf8))
    }

    func testReplacementByAnotherOwnerIsNeverOverwrittenOrDeleted() throws {
        let (_, target) = try fixture()
        let store = try EVRecoveryStore.claim(for: target)
        let foreign = EVRecoveryRecord(version: 1, owner: UUID(), processID: 1, host: "different-host", targetPath: target.path,
                                       created: Date(), updated: Date(), snapshot: snapshot("other owner"))
        let bytes = try foreign.encoded()
        try bytes.write(to: store.url, options: .atomic)
        let failed = expectation(description: "ownership rejected")
        store.write(snapshot("ours"), completion: { error in XCTAssertNotNil(error); failed.fulfill() })
        wait(for: [failed], timeout: 2)
        store.closeAndRemove(); store.drainForTesting()
        XCTAssertEqual(try Data(contentsOf: store.url), bytes)
    }

    func testSymlinkUsesTargetRecoveryIdentityAndUnknownVersionIsNotRecoverable() throws {
        let (directory, target) = try fixture()
        let alias = directory.appendingPathComponent("alias.md")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: target)
        let store = try EVRecoveryStore.claim(for: alias)
        XCTAssertEqual(store.url.lastPathComponent, ".writing.md.viem.swp")
        store.write(snapshot("through alias")); store.drainForTesting()
        XCTAssertEqual(EVRecoveryStore.candidates(for: target).first?.snapshot?.source, Data("through alias".utf8))
        var bytes = try Data(contentsOf: store.url)
        let source = try XCTUnwrap(String(data: bytes, encoding: .utf8))
        bytes = Data(source.replacingOccurrences(of: "\"version\":1", with: "\"version\":999").utf8)
        try bytes.write(to: store.url)
        XCTAssertNil(EVRecoveryStore.candidates(for: target).first?.snapshot)
        store.closeAndRemove(); store.drainForTesting()
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.url.path), "Unknown/newer format is never adopted for deletion")
    }
}
