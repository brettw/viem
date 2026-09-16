import Darwin
import Foundation

/// A launch is interpreted in the directory of the invoking process, even when
/// an existing editor process handles it.
public struct EVInstanceLaunchRequest: Codable, Equatable, Sendable {
    public let arguments: [String]
    public let workingDirectory: String

    public init(arguments: [String], workingDirectory: String) {
        self.arguments = arguments
        self.workingDirectory = workingDirectory
    }
}

public struct EVInstanceLaunchResponse: Codable, Equatable, Sendable {
    public let errorMessage: String?
    public init(errorMessage: String? = nil) { self.errorMessage = errorMessage }
}

/// Elects the one editor process for this user before AppKit starts. The lock is
/// never unlinked: its inode is the election authority, including across crashes.
/// The socket is replaceable only while holding that lock.
public final class EVSingleInstance: @unchecked Sendable {
    public enum StartResult: Sendable {
        case primary(EVSingleInstance)
        case forwarded(EVInstanceLaunchResponse)
    }

    private struct Message: Codable {
        let version: Int
        let request: EVInstanceLaunchRequest
    }

    private static let maximumMessageSize = 1_048_576
    private let source: DispatchSourceRead
    private let listener: Int32
    @MainActor private var handler: ((EVInstanceLaunchRequest) -> String?)?
    @MainActor private var pending: [(EVInstanceLaunchRequest, Connection)] = []

    /// An explicit directory isolates tests from the user's running editor.
    /// A failed or unacknowledged handoff throws; it never starts another editor.
    public static func start(
        request: EVInstanceLaunchRequest,
        directory: URL? = nil,
        timeout: TimeInterval = 30
    ) throws -> StartResult {
        guard timeout.isFinite, timeout > 0 else { throw Failure("The instance launch timeout must be positive.") }
        let directory = directory ?? URL(fileURLWithPath: "/tmp/viem-\(getuid())", isDirectory: true)
        try prepareDirectory(directory.path)
        let lockPath = directory.appendingPathComponent("instance.lock").path
        let socketPath = directory.appendingPathComponent("instance.sock").path
        let lock = Darwin.open(lockPath, O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, 0o600)
        guard lock >= 0 else { throw systemError("Open instance lock") }
        var lockTransferred = false
        defer { if !lockTransferred { Darwin.close(lock) } }
        var info = stat()
        guard fstat(lock, &info) == 0, info.st_uid == getuid(),
              (info.st_mode & S_IFMT) == S_IFREG, info.st_nlink == 1 else {
            throw Failure("The instance lock is not a private regular file.")
        }
        let deadline = Date().addingTimeInterval(timeout)
        while true {
            if flock(lock, LOCK_EX | LOCK_NB) == 0 {
                let instance = try EVSingleInstance(lock: lock, socketPath: socketPath)
                lockTransferred = true
                return .primary(instance)
            }
            guard errno == EWOULDBLOCK || errno == EAGAIN else {
                throw systemError("Acquire instance lock")
            }
            var address = try socketAddress(socketPath)
            let client = try makeSocket()
            let result = withUnsafePointer(to: &address) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.connect(client, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            if result == 0 {
                let connection = Connection(client)
                try requireCurrentUser(client)
                try setTimeout(client, timeout)
                // Once any request bytes may have been sent, never retry or elect
                // a replacement: the primary may already have accepted this launch.
                try connection.write(Message(version: 1, request: request))
                let response: EVInstanceLaunchResponse = try connection.read()
                return .forwarded(response)
            }
            let connectionError = errno
            Darwin.close(client)
            guard connectionError == ENOENT || connectionError == ECONNREFUSED
                    || connectionError == EINTR else {
                throw systemError("Connect to running Viem", code: connectionError)
            }
            guard Date() < deadline else {
                throw Failure("The running Viem instance did not become ready to receive this launch.")
            }
            Thread.sleep(forTimeInterval: 0.02)
        }
    }

    /// Requests received during startup wait here until the application has
    /// installed its common launch handler. Returning acknowledges acceptance;
    /// the handler may queue UI work to avoid waiting for a modal file dialog.
    @MainActor public func setLaunchHandler(_ handler: @escaping (EVInstanceLaunchRequest) -> String?) {
        self.handler = handler
        let requests = pending
        pending.removeAll()
        for (request, connection) in requests {
            respond(to: connection, errorMessage: handler(request))
        }
    }

    private init(lock: Int32, socketPath: String) throws {
        listener = try Self.makeSocket()
        let descriptor = listener
        var ready = false
        defer { if !ready { Darwin.close(descriptor) } }
        var address = try Self.socketAddress(socketPath)
        // A previous process may have crashed. No live peer can own this path
        // because we acquired the process-lifetime lock before touching it.
        if unlink(socketPath) != 0 && errno != ENOENT {
            throw Self.systemError("Remove stale instance socket")
        }
        let bound = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0 else { throw Self.systemError("Bind instance socket") }
        guard chmod(socketPath, 0o600) == 0, Darwin.listen(descriptor, 32) == 0,
              fcntl(descriptor, F_SETFL, O_NONBLOCK) == 0 else {
            throw Self.systemError("Listen for instance launches")
        }
        source = DispatchSource.makeReadSource(fileDescriptor: descriptor, queue: .global(qos: .userInitiated))
        source.setCancelHandler {
            Darwin.close(descriptor)
            unlink(socketPath)
            // Releasing the lock last prevents a replacement from losing its
            // newly bound socket to this endpoint's cleanup.
            Darwin.close(lock)
        }
        source.setEventHandler { [weak self] in self?.acceptRequests() }
        ready = true
        source.resume()
    }

    deinit { source.cancel() }

    private func acceptRequests() {
        while true {
            let client = Darwin.accept(listener, nil, nil)
            if client < 0 {
                if errno == EINTR { continue }
                return
            }
            let connection = Connection(client)
            do {
                try Self.requireCurrentUser(client)
                try Self.configureSocket(client)
                try Self.setTimeout(client, 5)
            } catch {
                continue
            }
            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                do {
                    let message: Message = try connection.read()
                    guard message.version == 1, message.request.workingDirectory.hasPrefix("/"),
                          !message.request.workingDirectory.contains("\0"),
                          !message.request.arguments.contains(where: { $0.contains("\0") }) else {
                        throw Failure("The launch request is invalid or uses an unsupported protocol.")
                    }
                    Task { @MainActor [weak self] in
                        guard let self else {
                            try? connection.write(EVInstanceLaunchResponse(errorMessage: "Viem is shutting down."))
                            return
                        }
                        if let handler = self.handler {
                            self.respond(to: connection, errorMessage: handler(message.request))
                        } else {
                            self.pending.append((message.request, connection))
                        }
                    }
                } catch {
                    try? connection.write(EVInstanceLaunchResponse(errorMessage: error.localizedDescription))
                }
            }
        }
    }

    private func respond(to connection: Connection, errorMessage: String?) {
        DispatchQueue.global(qos: .userInitiated).async {
            try? connection.write(EVInstanceLaunchResponse(errorMessage: errorMessage))
        }
    }

    private static func prepareDirectory(_ path: String) throws {
        if mkdir(path, 0o700) != 0 && errno != EEXIST {
            throw systemError("Create private instance directory")
        }
        var info = stat()
        guard lstat(path, &info) == 0, info.st_uid == getuid(),
              (info.st_mode & S_IFMT) == S_IFDIR, (info.st_mode & 0o077) == 0 else {
            throw Failure("The Viem instance directory must be owned by this user and accessible only to this user.")
        }
    }

    private static func socketAddress(_ path: String) throws -> sockaddr_un {
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let bytes = Array(path.utf8) + [0]
        guard bytes.count <= MemoryLayout.size(ofValue: address.sun_path) else {
            throw Failure("The instance socket path is too long.")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: bytes) }
        return address
    }

    private static func makeSocket() throws -> Int32 {
        let descriptor = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else { throw systemError("Create instance socket") }
        do { try configureSocket(descriptor) }
        catch { Darwin.close(descriptor); throw error }
        return descriptor
    }

    private static func configureSocket(_ descriptor: Int32) throws {
        var enabled: Int32 = 1
        let flags = fcntl(descriptor, F_GETFL)
        guard fcntl(descriptor, F_SETFD, FD_CLOEXEC) == 0,
              flags >= 0, fcntl(descriptor, F_SETFL, flags & ~O_NONBLOCK) == 0,
              setsockopt(descriptor, SOL_SOCKET, SO_NOSIGPIPE, &enabled, socklen_t(MemoryLayout.size(ofValue: enabled))) == 0 else {
            throw systemError("Configure instance socket")
        }
    }

    private static func requireCurrentUser(_ descriptor: Int32) throws {
        var user: uid_t = 0
        var group: gid_t = 0
        guard getpeereid(descriptor, &user, &group) == 0, user == getuid() else {
            throw Failure("The instance connection does not belong to this user.")
        }
    }

    private static func setTimeout(_ descriptor: Int32, _ seconds: TimeInterval) throws {
        var timeout = timeval(tv_sec: Int(seconds), tv_usec: Int32((seconds - seconds.rounded(.down)) * 1_000_000))
        for option in [SO_RCVTIMEO, SO_SNDTIMEO] {
            guard setsockopt(descriptor, SOL_SOCKET, option, &timeout, socklen_t(MemoryLayout.size(ofValue: timeout))) == 0 else {
                throw systemError("Set instance connection timeout")
            }
        }
    }

    private final class Connection: @unchecked Sendable {
        let descriptor: Int32
        init(_ descriptor: Int32) { self.descriptor = descriptor }
        deinit { Darwin.close(descriptor) }

        func write<T: Encodable>(_ value: T) throws {
            let body = try JSONEncoder().encode(value)
            guard body.count <= maximumMessageSize else { throw Failure("The launch message is too large.") }
            var length = UInt32(body.count).bigEndian
            var data = withUnsafeBytes(of: &length) { Data($0) }
            data.append(body)
            try data.withUnsafeBytes { bytes in
                var offset = 0
                while offset < bytes.count {
                    let count = Darwin.send(descriptor, bytes.baseAddress!.advanced(by: offset), bytes.count - offset, 0)
                    if count < 0 && errno == EINTR { continue }
                    guard count > 0 else { throw systemError("Send instance launch message") }
                    offset += count
                }
            }
        }

        func read<T: Decodable>() throws -> T {
            let header = try readBytes(4)
            let length = header.reduce(0) { ($0 << 8) | Int($1) }
            guard length > 0, length <= maximumMessageSize else { throw Failure("The launch message size is invalid.") }
            return try JSONDecoder().decode(T.self, from: readBytes(length))
        }

        private func readBytes(_ length: Int) throws -> Data {
            var data = Data(count: length)
            try data.withUnsafeMutableBytes { bytes in
                var offset = 0
                while offset < length {
                    let count = Darwin.recv(descriptor, bytes.baseAddress!.advanced(by: offset), length - offset, 0)
                    if count < 0 && errno == EINTR { continue }
                    guard count > 0 else {
                        if count == 0 { throw Failure("The running Viem instance closed the launch connection before acknowledgment.") }
                        throw systemError("Receive instance launch message")
                    }
                    offset += count
                }
            }
            return data
        }
    }

    private struct Failure: LocalizedError {
        let message: String
        init(_ message: String) { self.message = message }
        var errorDescription: String? { message }
    }

    private static func systemError(_ operation: String, code: Int32 = errno) -> Failure {
        Failure("\(operation): \(String(cString: strerror(code))).")
    }
}
