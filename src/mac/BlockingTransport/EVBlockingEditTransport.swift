import Darwin
import Foundation

/// One invocation owns one endpoint in a random, user-private directory. Only
/// the GUI that actually opens the document connects; EOF is always failure.
public final class EVBlockingEditListener: @unchecked Sendable {
    public let endpoint: String
    private let directory: String
    private let listener: Int32

    public init() throws {
        var template = Array("/tmp/viem-blocking-\(getuid())-XXXXXX".utf8CString)
        guard let created = mkdtemp(&template) else { throw EVBlockingSocket.failure("Create completion directory") }
        directory = String(cString: created)
        endpoint = directory + "/completion.sock"
        var descriptor: Int32 = -1
        do {
            descriptor = try EVBlockingSocket.make()
            var address = try EVBlockingSocket.address(endpoint)
            let result = withUnsafePointer(to: &address) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            guard result == 0, chmod(endpoint, 0o600) == 0,
                  Darwin.listen(descriptor, 1) == 0,
                  fcntl(descriptor, F_SETFL, O_NONBLOCK) == 0 else {
                throw EVBlockingSocket.failure("Listen for editor completion")
            }
            listener = descriptor
        } catch {
            if descriptor >= 0 { Darwin.close(descriptor) }
            unlink(endpoint)
            rmdir(directory)
            throw error
        }
    }

    deinit {
        Darwin.close(listener)
        unlink(endpoint)
        rmdir(directory)
    }

    /// Only startup is bounded. Once connected, the user may edit for any length
    /// of time. A GUI crash closes the connection instead of reporting success.
    public func waitForCompletion(startupTimeout: TimeInterval = 30,
                                  launchFailure: () -> String? = { nil }) throws -> Int32 {
        guard startupTimeout.isFinite, startupTimeout > 0 else {
            throw EVBlockingFailure("The editor startup timeout must be positive.")
        }
        let started = ProcessInfo.processInfo.systemUptime
        var client: Int32 = -1
        while client < 0 {
            let remaining = startupTimeout - (ProcessInfo.processInfo.systemUptime - started)
            guard remaining > 0 else { throw EVBlockingFailure("Viem did not connect before the editor startup timeout.") }
            var descriptor = pollfd(fd: listener, events: Int16(POLLIN), revents: 0)
            let ready = Darwin.poll(&descriptor, 1, Int32(min(100, (remaining * 1_000).rounded(.up))))
            if ready < 0, errno == EINTR { continue }
            guard ready >= 0 else { throw EVBlockingSocket.failure("Wait for Viem") }
            if ready > 0 {
                guard descriptor.revents & Int16(POLLIN) != 0 else {
                    throw EVBlockingFailure("The editor completion listener closed unexpectedly.")
                }
                client = Darwin.accept(listener, nil, nil)
                if client < 0 {
                    if errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK { continue }
                    throw EVBlockingSocket.failure("Accept Viem completion")
                }
            } else if let failure = launchFailure() {
                throw EVBlockingFailure(failure)
            }
        }
        defer { Darwin.close(client) }
        // macOS rejects SO_NOSIGPIPE after a peer has already sent its result
        // and closed. This side only reads, so it never needs that write option.
        try EVBlockingSocket.configure(client, suppressSIGPIPE: false)
        try EVBlockingSocket.requireCurrentUser(client)
        var result = [UInt8](repeating: 0, count: 4)
        try result.withUnsafeMutableBytes { bytes in
            var offset = 0
            while offset < bytes.count {
                let count = Darwin.recv(client, bytes.baseAddress!.advanced(by: offset), bytes.count - offset, 0)
                if count < 0, errno == EINTR { continue }
                guard count > 0 else {
                    if count == 0 { throw EVBlockingFailure("Viem closed before reporting editor completion.") }
                    throw EVBlockingSocket.failure("Read editor completion")
                }
                offset += count
            }
        }
        let bits = UInt32(result[0]) | (UInt32(result[1]) << 8) | (UInt32(result[2]) << 16) | (UInt32(result[3]) << 24)
        return Int32(bitPattern: bits)
    }
}

/// Owned by the GUI for the lifetime of the requested shared document. Dropping
/// an unaccepted request reports failure; normal release explicitly finishes 0.
public final class EVBlockingEditCompletion: @unchecked Sendable {
    private let lock = NSLock()
    private var descriptor: Int32

    private init(descriptor: Int32) { self.descriptor = descriptor }
    deinit { finish(exitCode: 1) }

    public static func connect(endpoint: String) throws -> EVBlockingEditCompletion {
        try EVBlockingSocket.validateEndpoint(endpoint)
        let descriptor = try EVBlockingSocket.make()
        do {
            // Even a malformed/full same-user endpoint must not stall the GUI.
            guard fcntl(descriptor, F_SETFL, O_NONBLOCK) == 0 else {
                throw EVBlockingSocket.failure("Prepare editor completion connection")
            }
            var address = try EVBlockingSocket.address(endpoint)
            let result = withUnsafePointer(to: &address) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.connect(descriptor, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            if result != 0 {
                guard errno == EINPROGRESS || errno == EINTR else {
                    throw EVBlockingSocket.failure("Connect editor completion")
                }
                let started = ProcessInfo.processInfo.systemUptime
                while true {
                    let remaining = 5 - (ProcessInfo.processInfo.systemUptime - started)
                    guard remaining > 0 else { throw EVBlockingFailure("The editor completion connection timed out.") }
                    var pollDescriptor = pollfd(fd: descriptor, events: Int16(POLLOUT), revents: 0)
                    let ready = Darwin.poll(&pollDescriptor, 1, Int32((remaining * 1_000).rounded(.up)))
                    if ready < 0, errno == EINTR { continue }
                    guard ready > 0 else { throw EVBlockingFailure("The editor completion connection timed out.") }
                    var error: Int32 = 0
                    var length = socklen_t(MemoryLayout.size(ofValue: error))
                    guard getsockopt(descriptor, SOL_SOCKET, SO_ERROR, &error, &length) == 0 else {
                        throw EVBlockingSocket.failure("Check editor completion connection")
                    }
                    guard error == 0 else { throw EVBlockingSocket.failure("Connect editor completion", code: error) }
                    break
                }
            }
            try EVBlockingSocket.configure(descriptor)
            try EVBlockingSocket.requireCurrentUser(descriptor)
            // Four bytes fit immediately in an empty socket, but a lost caller
            // must never block AppKit shutdown. Do not wait indefinitely on I/O.
            var timeout = timeval(tv_sec: 1, tv_usec: 0)
            guard setsockopt(descriptor, SOL_SOCKET, SO_SNDTIMEO, &timeout,
                             socklen_t(MemoryLayout.size(ofValue: timeout))) == 0 else {
                throw EVBlockingSocket.failure("Set completion timeout")
            }
            return EVBlockingEditCompletion(descriptor: descriptor)
        } catch {
            Darwin.close(descriptor)
            throw error
        }
    }

    public func finish(exitCode: Int32) {
        lock.lock()
        let owned = descriptor
        descriptor = -1
        lock.unlock()
        guard owned >= 0 else { return }
        defer { Darwin.close(owned) }
        var result = exitCode.littleEndian
        withUnsafeBytes(of: &result) { bytes in
            var offset = 0
            while offset < bytes.count {
                let count = Darwin.send(owned, bytes.baseAddress!.advanced(by: offset), bytes.count - offset, 0)
                if count < 0, errno == EINTR { continue }
                guard count > 0 else { return }
                offset += count
            }
        }
    }
}

private enum EVBlockingSocket {
    static func validateEndpoint(_ endpoint: String) throws {
        let prefix = "/tmp/viem-blocking-\(getuid())-"
        let suffix = "/completion.sock"
        guard endpoint.hasPrefix(prefix), endpoint.hasSuffix(suffix) else {
            throw EVBlockingFailure("Invalid blocking editor completion endpoint.")
        }
        let token = endpoint.dropFirst(prefix.count).dropLast(suffix.count)
        guard token.count == 6, token.utf8.allSatisfy({ byte in
            (48...57).contains(byte) || (65...90).contains(byte) || (97...122).contains(byte)
        }) else { throw EVBlockingFailure("Invalid blocking editor completion endpoint.") }
        let directory = String(endpoint.dropLast(suffix.count))
        var info = stat()
        guard lstat(directory, &info) == 0, info.st_uid == getuid(),
              info.st_mode & S_IFMT == S_IFDIR, info.st_mode & 0o077 == 0,
              lstat(endpoint, &info) == 0, info.st_uid == getuid(),
              info.st_mode & S_IFMT == S_IFSOCK, info.st_mode & 0o077 == 0 else {
            throw EVBlockingFailure("The editor completion endpoint must be private and owned by this user.")
        }
    }

    static func address(_ path: String) throws -> sockaddr_un {
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let bytes = Array(path.utf8) + [0]
        guard bytes.count <= MemoryLayout.size(ofValue: address.sun_path) else {
            throw EVBlockingFailure("The editor completion socket path is too long.")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: bytes) }
        return address
    }

    static func make() throws -> Int32 {
        let descriptor = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else { throw failure("Create editor completion socket") }
        do { try configure(descriptor) }
        catch { Darwin.close(descriptor); throw error }
        return descriptor
    }

    static func configure(_ descriptor: Int32, suppressSIGPIPE: Bool = true) throws {
        var enabled: Int32 = 1
        let flags = fcntl(descriptor, F_GETFL)
        guard fcntl(descriptor, F_SETFD, FD_CLOEXEC) == 0,
              flags >= 0, fcntl(descriptor, F_SETFL, flags & ~O_NONBLOCK) == 0 else {
            throw failure("Configure editor completion socket")
        }
        if suppressSIGPIPE, setsockopt(descriptor, SOL_SOCKET, SO_NOSIGPIPE, &enabled,
                                      socklen_t(MemoryLayout.size(ofValue: enabled))) != 0 {
            throw failure("Suppress editor completion SIGPIPE")
        }
    }

    static func requireCurrentUser(_ descriptor: Int32) throws {
        var user: uid_t = 0
        var group: gid_t = 0
        guard getpeereid(descriptor, &user, &group) == 0, user == getuid() else {
            throw EVBlockingFailure("The editor completion connection does not belong to this user.")
        }
    }

    static func failure(_ operation: String, code: Int32 = errno) -> EVBlockingFailure {
        EVBlockingFailure("\(operation): \(String(cString: strerror(code))).")
    }
}

struct EVBlockingFailure: LocalizedError {
    let message: String
    init(_ message: String) { self.message = message }
    var errorDescription: String? { message }
}
