import AppKit
import XCTest
@testable import EvimAppShell

final class EVRecoveryStoreTests: XCTestCase {
    private func fixture() throws -> (URL, URL) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("evim-recovery-test-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let target = directory.appendingPathComponent("writing.md")
        try Data("original".utf8).write(to: target)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return (directory, target)
    }
    private func snapshot(_ text: String, revision: UInt64 = 1) -> EVRecoverySnapshot {
        EVRecoverySnapshot(source: Data(text.utf8), format: .markdownSource, encoding: 2, fileFormat: 3, documentID: 19, documentRevision: revision)
    }

    func testExclusiveSlotsPreserveExistingEVimAndForeignVimFiles() throws {
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

    func testZeroByteSlotAndCollidingStagingFileArePreserved() throws {
        let (directory, target) = try fixture()
        let occupied = directory.appendingPathComponent(".writing.md.evim.swp")
        try Data().write(to: occupied)
        let store = try EVRecoveryStore.claim(for: target)
        XCTAssertNotEqual(store.url, occupied)
        store.write(snapshot("last good")); store.drainForTesting()
        let previous = try Data(contentsOf: store.url)
        let staging = directory.appendingPathComponent(".evim-recovery-\(store.owner.uuidString)-2.tmp")
        try Data().write(to: staging)
        let rejected = expectation(description: "staging collision rejected")
        store.write(snapshot("new")) { error in XCTAssertNotNil(error); rejected.fulfill() }
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
        store.write(snapshot("ours")) { error in XCTAssertNotNil(error); failed.fulfill() }
        wait(for: [failed], timeout: 2)
        store.closeAndRemove(); store.drainForTesting()
        XCTAssertEqual(try Data(contentsOf: store.url), bytes)
    }

    func testSymlinkUsesTargetRecoveryIdentityAndUnknownVersionIsNotRecoverable() throws {
        let (directory, target) = try fixture()
        let alias = directory.appendingPathComponent("alias.md")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: target)
        let store = try EVRecoveryStore.claim(for: alias)
        XCTAssertEqual(store.url.lastPathComponent, ".writing.md.evim.swp")
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
