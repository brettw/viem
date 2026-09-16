import Darwin
import Foundation
import XCTest
@testable import ViemAppShell

@MainActor
final class EVSingleInstanceTests: XCTestCase {
    private let request = EVInstanceLaunchRequest(arguments: [], workingDirectory: "/tmp")

    private func directory() -> URL {
        // sockaddr_un has a short path limit; the macOS per-process temp path
        // plus a UUID can exceed it.
        let url = URL(fileURLWithPath: "/tmp/viem-instance-test-\(UUID().uuidString)", isDirectory: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    private func primary(at directory: URL) throws -> EVSingleInstance {
        guard case let .primary(primary) = try EVSingleInstance.start(request: request, directory: directory) else {
            throw NSError(domain: "EVSingleInstanceTests", code: 1)
        }
        return primary
    }

    func testLaunchWaitsForHandlerAndPreservesArgumentsAndWorkingDirectory() async throws {
        let directory = directory()
        let primary = try primary(at: directory)
        defer { withExtendedLifetime(primary) {} }
        let forwarded = EVInstanceLaunchRequest(arguments: ["-o2", "+12", "one two.md", "日本語.md"], workingDirectory: "/tmp/invoker")
        let secondary = Task.detached {
            try EVSingleInstance.start(request: forwarded, directory: directory, timeout: 5)
        }
        try await Task.sleep(for: .milliseconds(100))
        var received: [EVInstanceLaunchRequest] = []
        primary.setLaunchHandler { received.append($0); return nil }
        guard case let .forwarded(response) = try await secondary.value else {
            return XCTFail("A second launch must not become the primary.")
        }
        XCTAssertNil(response.errorMessage)
        XCTAssertEqual(received, [forwarded])
    }

    func testHandlerDiagnosticIsAcknowledgedWithoutStartingAnotherInstance() async throws {
        let directory = directory()
        let primary = try primary(at: directory)
        defer { withExtendedLifetime(primary) {} }
        primary.setLaunchHandler { _ in "Unknown option: --invalid" }
        let request = self.request
        let result = try await Task.detached {
            try EVSingleInstance.start(request: request, directory: directory, timeout: 5)
        }.value
        guard case let .forwarded(response) = result else { return XCTFail("Unexpected primary") }
        XCTAssertEqual(response.errorMessage, "Unknown option: --invalid")
    }

    func testSimultaneousLaunchesElectOnePrimaryAndDeliverEveryOtherLaunchOnce() async throws {
        let directory = directory()
        var primary: EVSingleInstance?
        defer { withExtendedLifetime(primary) {} }
        var received: [String] = []
        var primaryCount = 0
        var forwardedCount = 0
        try await withThrowingTaskGroup(of: EVSingleInstance.StartResult.self) { group in
            for index in 0..<12 {
                group.addTask {
                    try EVSingleInstance.start(
                        request: EVInstanceLaunchRequest(arguments: ["file-\(index)"], workingDirectory: "/tmp"),
                        directory: directory, timeout: 5)
                }
            }
            for try await result in group {
                switch result {
                case let .primary(instance):
                    primary = instance
                    primaryCount += 1
                    instance.setLaunchHandler { request in received.append(contentsOf: request.arguments); return nil }
                case let .forwarded(response):
                    XCTAssertNil(response.errorMessage)
                    forwardedCount += 1
                }
            }
        }
        XCTAssertEqual(primaryCount, 1)
        XCTAssertEqual(forwardedCount, 11)
        XCTAssertEqual(received.count, 11)
        XCTAssertEqual(Set(received).count, 11)
    }

    func testCrossProcessStartupRaceHandoffAndStaleSocketRecovery() throws {
        let directory = directory()
        let helper = try python("""
        import fcntl, json, os, socket, struct, sys, time
        socket.setdefaulttimeout(5)
        directory = sys.argv[1]
        os.mkdir(directory, 0o700)
        lock = open(directory + '/instance.lock', 'w')
        fcntl.flock(lock, fcntl.LOCK_EX)
        print('R', end='', flush=True)
        time.sleep(0.15)
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(directory + '/instance.sock')
        listener.listen(1)
        client, _ = listener.accept()
        def read_exact(size):
            data = b''
            while len(data) < size:
                chunk = client.recv(size - len(data))
                if not chunk: raise RuntimeError('unexpected EOF')
                data += chunk
            return data
        size, = struct.unpack('!I', read_exact(4))
        message = json.loads(read_exact(size))
        body = json.dumps({'errorMessage': None}).encode()
        client.sendall(struct.pack('!I', len(body)) + body)
        print(json.dumps(message['request']), flush=True)
        # Deliberately leave the bound socket behind, as a crashed process does.
        """, directory: directory)
        defer { if helper.process.isRunning { helper.process.terminate() } }
        XCTAssertEqual(try helper.output.read(upToCount: 1), Data("R".utf8))
        let forwarded = EVInstanceLaunchRequest(arguments: ["-o", "a b.md", "line\nbreak.md"], workingDirectory: "/tmp/original")
        guard case let .forwarded(response) = try EVSingleInstance.start(request: forwarded, directory: directory, timeout: 5) else {
            return XCTFail("The lock held by another process must win the election.")
        }
        XCTAssertNil(response.errorMessage)
        helper.process.waitUntilExit()
        XCTAssertEqual(helper.process.terminationStatus, 0)
        let received = try JSONDecoder().decode(EVInstanceLaunchRequest.self, from: helper.output.readDataToEndOfFile())
        XCTAssertEqual(received, forwarded)
        let replacement = try primary(at: directory)
        withExtendedLifetime(replacement) {}
    }

    func testMalformedFrameIsRejectedAndListenerStillAcceptsValidLaunch() async throws {
        let directory = directory()
        let primary = try primary(at: directory)
        defer { withExtendedLifetime(primary) {} }
        var received = 0
        primary.setLaunchHandler { _ in received += 1; return nil }
        let helper = try python("""
        import json, socket, struct, sys
        socket.setdefaulttimeout(5)
        client = socket.socket(socket.AF_UNIX)
        client.connect(sys.argv[1] + '/instance.sock')
        client.sendall(struct.pack('!I', 0xffffffff))
        def read_exact(size):
            data = b''
            while len(data) < size:
                chunk = client.recv(size - len(data))
                if not chunk: raise RuntimeError('unexpected EOF')
                data += chunk
            return data
        size, = struct.unpack('!I', read_exact(4))
        response = json.loads(read_exact(size))
        assert response['errorMessage']
        """, directory: directory)
        defer { if helper.process.isRunning { helper.process.terminate() } }
        helper.process.waitUntilExit()
        XCTAssertEqual(helper.process.terminationStatus, 0)
        XCTAssertEqual(received, 0)
        let request = self.request
        let result = try await Task.detached {
            try EVSingleInstance.start(request: request, directory: directory, timeout: 5)
        }.value
        guard case let .forwarded(response) = result else { return XCTFail("Unexpected primary") }
        XCTAssertNil(response.errorMessage)
        XCTAssertEqual(received, 1)
    }

    func testRejectsSharedOrSymlinkedInstanceDirectory() throws {
        let shared = directory()
        try FileManager.default.createDirectory(at: shared, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o755])
        XCTAssertThrowsError(try EVSingleInstance.start(request: request, directory: shared))
        let link = directory()
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: shared)
        XCTAssertThrowsError(try EVSingleInstance.start(request: request, directory: link))
    }

    func testUnacknowledgedRequestIsNeverRetriedOrPromotedToPrimary() throws {
        let directory = directory()
        let helper = try python("""
        import fcntl, os, socket, struct, sys
        socket.setdefaulttimeout(5)
        directory = sys.argv[1]
        os.mkdir(directory, 0o700)
        lock = open(directory + '/instance.lock', 'w')
        fcntl.flock(lock, fcntl.LOCK_EX)
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(directory + '/instance.sock')
        listener.listen(2)
        print('R', end='', flush=True)
        client, _ = listener.accept()
        def read_exact(size):
            data = b''
            while len(data) < size:
                chunk = client.recv(size - len(data))
                if not chunk: raise RuntimeError('unexpected EOF')
                data += chunk
            return data
        size, = struct.unpack('!I', read_exact(4))
        read_exact(size)
        # The primary received the launch but its acknowledgment was lost.
        # Retrying could open the file twice, so no second connection is allowed.
        listener.settimeout(0.4)
        try:
            listener.accept()
            raise RuntimeError('launch was retried')
        except socket.timeout:
            pass
        """, directory: directory)
        defer { if helper.process.isRunning { helper.process.terminate() } }
        XCTAssertEqual(try helper.output.read(upToCount: 1), Data("R".utf8))
        XCTAssertThrowsError(try EVSingleInstance.start(request: request, directory: directory, timeout: 0.1))
        helper.process.waitUntilExit()
        XCTAssertEqual(helper.process.terminationStatus, 0)
    }

    private func python(_ script: String, directory: URL) throws -> (process: Process, output: FileHandle) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = ["-c", script, directory.path]
        let output = Pipe()
        process.standardOutput = output
        process.standardError = FileHandle.standardError
        try process.run()
        return (process, output.fileHandleForReading)
    }
}
