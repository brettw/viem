import CryptoKit
import Darwin
import Foundation

/// Full source bytes plus the interpretation required to reopen them exactly.
public struct EVRecoverySnapshot: Codable, Equatable, Sendable {
    public var source: Data
    public var format: EVSourceFormat
    public var encoding: UInt32
    public var fileFormat: UInt32
    public var documentID: UInt64
    public var documentRevision: UInt64

    public init(source: Data, format: EVSourceFormat, encoding: UInt32, fileFormat: UInt32,
                documentID: UInt64, documentRevision: UInt64) {
        self.source = source; self.format = format; self.encoding = encoding
        self.fileFormat = fileFormat; self.documentID = documentID; self.documentRevision = documentRevision
    }
}

struct EVRecoveryRecord: Codable, Equatable, Sendable {
    static let magic = Data("VIEM-RECOVERY\n".utf8)
    let version: Int
    let owner: UUID
    let processID: Int32
    let host: String
    let targetPath: String
    let created: Date
    var updated: Date
    var snapshot: EVRecoverySnapshot?

    func encoded() throws -> Data { Self.magic + (try JSONEncoder().encode(self)) }
    static func decode(_ data: Data) -> Self? {
        guard data.starts(with: magic), let value = try? JSONDecoder().decode(Self.self, from: data.dropFirst(magic.count)), value.version == 1 else { return nil }
        return value
    }
}

public struct EVRecoveryCandidate: Sendable {
    public let url: URL
    public let snapshot: EVRecoverySnapshot?
    public let isViemRecovery: Bool
    public let ownerMayBeRunning: Bool
    public let updated: Date?
    fileprivate let identity: EVRecoveryFileIdentity?
    fileprivate let digest: Data?

    /// Only a verified, unchanged local Viem session whose process has exited
    /// can authorize disposal. Foreign, unreadable and live slots stay intact.
    public var canDelete: Bool {
        isViemRecovery && !ownerMayBeRunning && identity?.owner == getuid() && identity?.links == 1
    }
}

fileprivate struct EVRecoveryFileIdentity: Equatable, Sendable {
    let device: dev_t
    let inode: ino_t
    let generation: UInt32
    let owner: uid_t
    let links: nlink_t
    let size: off_t
    let modifiedSeconds: Int
    let modifiedNanoseconds: Int
    let changedSeconds: Int
    let changedNanoseconds: Int

    init?(_ info: stat) {
        guard info.st_mode & S_IFMT == S_IFREG else { return nil }
        device = info.st_dev; inode = info.st_ino; generation = info.st_gen; size = info.st_size
        owner = info.st_uid; links = info.st_nlink
        modifiedSeconds = info.st_mtimespec.tv_sec; modifiedNanoseconds = info.st_mtimespec.tv_nsec
        changedSeconds = info.st_ctimespec.tv_sec; changedNanoseconds = info.st_ctimespec.tv_nsec
    }
}

enum EVRecoveryError: LocalizedError {
    case ownershipLost
    case noAvailableSlot
    case readOnly
    case backendUnavailable
    case recoveryChanged
    case cannotDeleteRecovery
    case cleanupFailed(String)
    var errorDescription: String? {
        switch self {
        case .ownershipLost: "The recovery file now belongs to another editing session. It was left unchanged."
        case .noAvailableSlot: "Viem could not create a recovery file for this document."
        case .readOnly: "This document was opened read-only. Use :w! or confirm Save Anyway to write it."
        case .backendUnavailable: "This document backend does not support recovery or read-only editing."
        case .recoveryChanged: "The recovery file changed since the recovery prompt. It was left unchanged."
        case .cannotDeleteRecovery: "Only unchanged recovery files from ended local Viem sessions can be deleted."
        case let .cleanupFailed(message): "The document is open, but an old recovery file could not be deleted: \(message)"
        }
    }
}

/// Serial background persistence, with exclusive slot claiming and immutable
/// jobs. A cancelled or superseded job cannot replace a newer recovery state.
final class EVRecoveryStore: @unchecked Sendable {
    let url: URL
    let owner: UUID
    private var record: EVRecoveryRecord
    private let queue = DispatchQueue(label: "com.viem.recovery", qos: .utility)
    private let lock = NSLock()
    private var generation: UInt64 = 0
    private var closed = false
    // Accessed only on queue. A failed/superseded backup cannot consume these;
    // cleanup itself is attempted once and its diagnostic survives later writes.
    private var retiringCandidates: [EVRecoveryCandidate] = []
    private var retirementFailure: Error?

    private init(url: URL, record: EVRecoveryRecord) {
        self.url = url; self.owner = record.owner; self.record = record
    }

    static func candidates(for sourceURL: URL) -> [EVRecoveryCandidate] {
        let target = EVDocumentIdentity.canonicalURL(sourceURL)
        var urls = slotURLs(for: target)
        urls.append(target.deletingLastPathComponent().appendingPathComponent(".\(target.lastPathComponent).swp"))
        return urls.compactMap { url in
            var info = stat()
            guard lstat(url.path, &info) == 0 else { return nil }
            // Snapshot and deletion token must describe the same physical read.
            let inspected = inspect(url)
            let value = inspected.flatMap { EVRecoveryRecord.decode($0.bytes) }
            let matching = value.flatMap { $0.targetPath == target.path ? $0 : nil }
            return EVRecoveryCandidate(url: url, snapshot: matching?.snapshot, isViemRecovery: matching != nil,
                ownerMayBeRunning: matching.map { !ownerHasExited($0) } ?? true, updated: matching?.updated,
                identity: inspected?.identity, digest: inspected.map { Data(SHA256.hash(data: $0.bytes)) })
        }.sorted { ($0.updated ?? .distantPast) > ($1.updated ?? .distantPast) }
    }

    private static func ownerHasExited(_ record: EVRecoveryRecord) -> Bool {
        guard record.host == ProcessInfo.processInfo.hostName, record.processID > 0 else { return false }
        return kill(record.processID, 0) == -1 && errno == ESRCH
    }

    private static func inspect(_ url: URL) -> (bytes: Data, identity: EVRecoveryFileIdentity)? {
        let descriptor = open(url.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
        guard descriptor >= 0 else { return nil }
        defer { close(descriptor) }
        return try? inspect(descriptor, at: url)
    }

    private static func inspect(_ descriptor: Int32, at url: URL) throws -> (bytes: Data, identity: EVRecoveryFileIdentity) {
        var before = stat(), after = stat(), path = stat()
        guard fstat(descriptor, &before) == 0, let identity = EVRecoveryFileIdentity(before) else {
            throw EVRecoveryError.cannotDeleteRecovery
        }
        let bytes = try FileHandle(fileDescriptor: descriptor, closeOnDealloc: false).readToEnd() ?? Data()
        guard fstat(descriptor, &after) == 0, lstat(url.path, &path) == 0,
              EVRecoveryFileIdentity(after) == identity, EVRecoveryFileIdentity(path) == identity else {
            throw EVRecoveryError.recoveryChanged
        }
        return (bytes, identity)
    }

    /// Register the prompt's exact candidates before the first backup write.
    /// An ordinary close before any successful write leaves them recoverable.
    func retireAfterNextCommit(_ candidates: [EVRecoveryCandidate]) {
        queue.async { [self] in retiringCandidates = candidates }
    }

    private static func removeUnchangedRecovery(_ candidate: EVRecoveryCandidate) throws {
        guard candidate.canDelete, let expected = candidate.identity, let digest = candidate.digest else {
            throw EVRecoveryError.cannotDeleteRecovery
        }
        let descriptor = open(candidate.url.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
        if descriptor < 0, errno == ENOENT { return }
        guard descriptor >= 0 else { throw EVRecoveryError.recoveryChanged }
        defer { close(descriptor) }
        // Serialize competing cleaners of this inode through the last identity
        // check and unlink. A new session can claim the name only afterward.
        guard flock(descriptor, LOCK_EX | LOCK_NB) == 0 else { throw EVRecoveryError.recoveryChanged }
        defer { _ = flock(descriptor, LOCK_UN) }
        let current = try inspect(descriptor, at: candidate.url)
        guard current.identity == expected, Data(SHA256.hash(data: current.bytes)) == digest,
              let record = EVRecoveryRecord.decode(current.bytes), ownerHasExited(record) else {
            throw EVRecoveryError.recoveryChanged
        }
        var final = stat()
        guard lstat(candidate.url.path, &final) == 0, EVRecoveryFileIdentity(final) == expected else {
            throw EVRecoveryError.recoveryChanged
        }
        guard unlink(candidate.url.path) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
        try synchronizeDirectory(containing: candidate.url)
    }

    private func retireCommittedCandidates() {
        let candidates = retiringCandidates
        retiringCandidates.removeAll()
        var failures: [String] = []
        for candidate in candidates {
            do { try Self.removeUnchangedRecovery(candidate) }
            catch { if failures.count < 3 { failures.append("\(candidate.url.lastPathComponent): \(error.localizedDescription)") } }
        }
        if !failures.isEmpty { retirementFailure = EVRecoveryError.cleanupFailed(failures.joined(separator: " ")) }
    }

    static func claim(for sourceURL: URL) throws -> EVRecoveryStore {
        let target = EVDocumentIdentity.canonicalURL(sourceURL)
        let record = EVRecoveryRecord(version: 1, owner: UUID(), processID: getpid(), host: ProcessInfo.processInfo.hostName,
                                      targetPath: target.path, created: Date(), updated: Date(), snapshot: nil)
        let bytes = try record.encoded()
        for url in slotURLs(for: target) {
            if url.deletingLastPathComponent() != target.deletingLastPathComponent() {
                try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            }
            do {
                _ = try writeExclusive(bytes, to: url)
                return EVRecoveryStore(url: url, record: record)
            } catch { continue }

        }
        throw EVRecoveryError.noAvailableSlot
    }

    private static func slotURLs(for target: URL) -> [URL] {
        let name = ".\(target.lastPathComponent).viem"
        let local = (0..<100).map { index in target.deletingLastPathComponent().appendingPathComponent(index == 0 ? "\(name).swp" : "\(name).\(index).swp") }
        let hash = SHA256.hash(data: Data(target.path.utf8)).map { String(format: "%02x", $0) }.joined()
        let fallback = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
            .appendingPathComponent("Viem/Recovery", isDirectory: true)
        return local + (0..<100).map { fallback.appendingPathComponent("\(hash).\($0).swp") }
    }

    func write(_ snapshot: EVRecoverySnapshot, didCommit: (@Sendable () -> Void)? = nil, completion: (@Sendable (Error?) -> Void)? = nil) {
        lock.lock(); generation &+= 1; let requested = generation; let stopped = closed; lock.unlock()
        guard !stopped else { completion?(nil); return }
        queue.async { [self] in
            do {
                lock.lock(); let current = !closed && generation == requested; lock.unlock()
                guard current else { completion?(nil); return }
                var candidate = record
                candidate.updated = Date(); candidate.snapshot = snapshot
                let bytes = try candidate.encoded()
                let temporary = url.deletingLastPathComponent().appendingPathComponent(".viem-recovery-\(owner.uuidString)-\(requested).tmp")
                let temporaryIdentity = try Self.writeExclusive(bytes, to: temporary)
                defer { Self.remove(temporary, ifIdentityMatches: temporaryIdentity) }
                let committed = try lock.withLock {
                    guard !closed && generation == requested else { return false }
                    guard ownsCurrentFile(), Self.identity(of: temporary) == temporaryIdentity else { throw EVRecoveryError.ownershipLost }
                    // Persist staging bytes before replacing the last good snapshot.
                    guard rename(temporary.path, url.path) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
                    record = candidate
                    try Self.synchronizeDirectory(containing: url)
                    return true
                }
                if committed {
                    retireCommittedCandidates()
                    didCommit?()
                }
                completion?(retirementFailure)
            } catch { completion?(error) }
        }
    }

    func invalidatePendingWrites() { lock.lock(); generation &+= 1; lock.unlock() }

    /// Enqueued after prior writes; ordinary close removes only our slot.
    func closeAndRemove(completion: (@Sendable () -> Void)? = nil) {
        lock.lock(); closed = true; generation &+= 1; lock.unlock()
        queue.async { [self] in
            if ownsCurrentFile() { try? FileManager.default.removeItem(at: url) }
            completion?()
        }
    }

    func waitForPendingOperations() { queue.sync {} }
    func drainForTesting() { waitForPendingOperations() }

    private struct FileIdentity: Equatable {
        let device: dev_t
        let inode: ino_t
    }

    private static func identity(of url: URL) -> FileIdentity? {
        var value = stat()
        guard lstat(url.path, &value) == 0, value.st_mode & S_IFMT == S_IFREG else { return nil }
        return FileIdentity(device: value.st_dev, inode: value.st_ino)
    }

    private static func remove(_ url: URL, ifIdentityMatches expected: FileIdentity) {
        guard identity(of: url) == expected else { return }
        _ = unlink(url.path)
    }

    /// Both lock claims and staging files start private and exclusive. Cleanup
    /// after a partial write is confined to the inode this invocation created.
    private static func writeExclusive(_ bytes: Data, to url: URL) throws -> FileIdentity {
        let descriptor = open(url.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, S_IRUSR | S_IWUSR)
        guard descriptor >= 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
        defer { close(descriptor) }
        var attributes = stat()
        guard fstat(descriptor, &attributes) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
        let created = FileIdentity(device: attributes.st_dev, inode: attributes.st_ino)
        do {
            try bytes.withUnsafeBytes { buffer in
                var offset = 0
                while offset < buffer.count {
                    let written = Darwin.write(descriptor, buffer.baseAddress!.advanced(by: offset), buffer.count - offset)
                    if written < 0 && errno == EINTR { continue }
                    guard written > 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
                    offset += written
                }
            }
            guard fsync(descriptor) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
            return created
        } catch {
            remove(url, ifIdentityMatches: created)
            throw error
        }
    }

    private static func synchronizeDirectory(containing url: URL) throws {
        let descriptor = open(url.deletingLastPathComponent().path, O_RDONLY | O_DIRECTORY | O_CLOEXEC)
        guard descriptor >= 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
        defer { close(descriptor) }
        guard fsync(descriptor) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
    }

    private func ownsCurrentFile() -> Bool {
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
              attributes[.type] as? FileAttributeType == .typeRegular,
              let bytes = try? Data(contentsOf: url), let current = EVRecoveryRecord.decode(bytes)
        else { return false }
        return current.owner == owner && current.targetPath == record.targetPath
    }
}
