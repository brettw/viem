import Darwin
import Foundation
import XCTest
@testable import ViemBlockingTransport

final class EVBlockingEditTransportTests: XCTestCase {
    func testCompletionForwardsExactStatusAndFinishesOnlyOnce() throws {
        for status: Int32 in [0, 1, 7, 255, -1, .max] {
            let listener = try EVBlockingEditListener()
            let completion = try EVBlockingEditCompletion.connect(endpoint: listener.endpoint)
            completion.finish(exitCode: status)
            completion.finish(exitCode: 99)
            XCTAssertEqual(try listener.waitForCompletion(startupTimeout: 1), status)
        }
    }

    func testDroppingUnacceptedCompletionReportsFailure() throws {
        let listener = try EVBlockingEditListener()
        var completion: EVBlockingEditCompletion? = try .connect(endpoint: listener.endpoint)
        XCTAssertNotNil(completion)
        completion = nil
        XCTAssertEqual(try listener.waitForCompletion(startupTimeout: 1), 1)
    }

    func testEditingDoesNotInheritStartupTimeout() async throws {
        let listener = try EVBlockingEditListener()
        let completion = try EVBlockingEditCompletion.connect(endpoint: listener.endpoint)
        let wait = Task.detached { try listener.waitForCompletion(startupTimeout: 0.05) }
        try await Task.sleep(for: .milliseconds(150))
        completion.finish(exitCode: 7)
        let status = try await wait.value
        XCTAssertEqual(status, 7)
    }

    func testStartupTimeoutAndFailedLaunchAreFailures() throws {
        let listener = try EVBlockingEditListener()
        XCTAssertThrowsError(try listener.waitForCompletion(startupTimeout: 0.03))
        XCTAssertThrowsError(try listener.waitForCompletion(startupTimeout: 1) { "launch failed" }) { error in
            XCTAssertEqual(error.localizedDescription, "launch failed")
        }
        XCTAssertThrowsError(try listener.waitForCompletion(startupTimeout: .infinity))
        XCTAssertThrowsError(try listener.waitForCompletion(startupTimeout: 0))
    }

    func testCrashAndPartialStatusNeverReportSuccessfulEditing() throws {
        for length in [0, 1, 3] {
            let listener = try EVBlockingEditListener()
            let child = try python("""
            import os, socket, sys
            connection = socket.socket(socket.AF_UNIX)
            connection.connect(sys.argv[1])
            connection.sendall(bytes(int(sys.argv[2])))
            os._exit(123)
            """, arguments: [listener.endpoint, String(length)])
            defer { if child.isRunning { child.terminate() } }
            XCTAssertThrowsError(try listener.waitForCompletion(startupTimeout: 2))
            child.waitUntilExit()
            XCTAssertEqual(child.terminationStatus, 123)
        }
    }

    func testEndpointsArePrivateAndRemovedAfterUse() throws {
        var listener: EVBlockingEditListener? = try .init()
        let endpoint = try XCTUnwrap(listener?.endpoint)
        let directory = URL(fileURLWithPath: endpoint).deletingLastPathComponent().path
        var info = stat()
        XCTAssertEqual(lstat(directory, &info), 0)
        XCTAssertEqual(info.st_mode & 0o777, 0o700)
        XCTAssertEqual(info.st_uid, getuid())
        XCTAssertEqual(lstat(endpoint, &info), 0)
        XCTAssertEqual(info.st_mode & 0o777, 0o600)
        listener = nil
        XCTAssertFalse(FileManager.default.fileExists(atPath: endpoint))
        XCTAssertFalse(FileManager.default.fileExists(atPath: directory))
    }

    func testRejectsMalformedSharedAndSymlinkedCompletionEndpoints() throws {
        for endpoint in ["", "/tmp/sock", "/tmp/viem-blocking-\(getuid())-../completion.sock",
                         "/tmp/viem-blocking-\(getuid())-AAAAAA/nested/completion.sock"] {
            XCTAssertThrowsError(try EVBlockingEditCompletion.connect(endpoint: endpoint))
        }
        let listener = try EVBlockingEditListener()
        let directory = URL(fileURLWithPath: listener.endpoint).deletingLastPathComponent().path
        XCTAssertEqual(chmod(directory, 0o755), 0)
        XCTAssertThrowsError(try EVBlockingEditCompletion.connect(endpoint: listener.endpoint))
        XCTAssertEqual(chmod(directory, 0o700), 0)
        XCTAssertEqual(chmod(listener.endpoint, 0o666), 0)
        XCTAssertThrowsError(try EVBlockingEditCompletion.connect(endpoint: listener.endpoint))
        XCTAssertEqual(unlink(listener.endpoint), 0)
        XCTAssertEqual(symlink("/tmp/unrelated-completion.sock", listener.endpoint), 0)
        XCTAssertThrowsError(try EVBlockingEditCompletion.connect(endpoint: listener.endpoint))
    }

    func testDeadCallerCannotRaiseSIGPIPEInGUI() throws {
        let output = Pipe()
        let child = try python("""
        import os, secrets, socket, sys
        target = '/tmp/viem-blocking-' + str(os.getuid()) + '-' + secrets.token_hex(3)
        os.mkdir(target, 0o700)
        path = target + '/completion.sock'
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(path)
        os.chmod(path, 0o600)
        listener.listen(1)
        print(path, flush=True)
        connection, _ = listener.accept()
        connection.close()
        listener.close()
        os.unlink(path)
        os.rmdir(target)
        """, output: output)
        defer { if child.isRunning { child.terminate() } }
        let endpoint = try readLine(output.fileHandleForReading)
        let completion = try EVBlockingEditCompletion.connect(endpoint: endpoint)
        child.waitUntilExit()
        XCTAssertEqual(child.terminationStatus, 0)
        completion.finish(exitCode: 7)
        completion.finish(exitCode: 0)
    }

    private func python(_ script: String, arguments: [String] = [], output: Pipe? = nil) throws -> Process {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = ["-c", script] + arguments
        process.standardOutput = output ?? Pipe()
        process.standardError = FileHandle.standardError
        try process.run()
        return process
    }

    private func readLine(_ handle: FileHandle) throws -> String {
        var line = Data()
        while let byte = try handle.read(upToCount: 1), !byte.isEmpty {
            if byte == Data([10]) { break }
            line.append(byte)
        }
        return String(decoding: line, as: UTF8.self)
    }
}
