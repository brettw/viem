import Darwin
import Foundation
import XCTest
@testable import ViemBlockingTransport

final class EVCommandLineLauncherTests: XCTestCase {
    func testLaunchPreservesArgumentsAndWorkingDirectoryThroughExternalSymlink() throws {
        let directory = try directory()
        let bin = directory.appendingPathComponent("application with spaces %00", isDirectory: true)
        try FileManager.default.createDirectory(at: bin, withIntermediateDirectories: true)
        let executable = bin.appendingPathComponent("viem-cli")
        try Data().write(to: executable)
        let link = directory.appendingPathComponent("viem")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: executable)
        try gui(in: bin, body: """
        import json, os, socket, stat, struct, sys
        assert sys.argv[1] == '--cli-launch'
        null = os.stat(os.devnull)
        detached = all(stat.S_ISCHR(os.fstat(fd).st_mode) and os.fstat(fd).st_rdev == null.st_rdev
                       for fd in range(3))
        with open('received.json', 'w') as file:
            json.dump({'arguments': sys.argv[3:], 'cwd': os.getcwd(), 'detached': detached}, file)
        connection = socket.socket(socket.AF_UNIX)
        connection.connect(sys.argv[2])
        connection.sendall(struct.pack('<i', 0))
        """)
        let arguments = ["-O2", "+42", "--", "日本語 one.md", "--help", "$(touch unwanted)", "", "a\nb.md"]
        XCTAssertEqual(try EVCommandLineLauncher.run(arguments: arguments, workingDirectory: directory.path,
            wrapperExecutable: link, startupTimeout: 2, completionTimeout: 1), 0)
        let result = try JSONDecoder().decode(Invocation.self,
            from: Data(contentsOf: directory.appendingPathComponent("received.json")))
        XCTAssertEqual(result.arguments, arguments)
        XCTAssertEqual(URL(fileURLWithPath: result.cwd).resolvingSymlinksInPath().path,
                       directory.resolvingSymlinksInPath().path)
        XCTAssertTrue(result.detached)
        XCTAssertFalse(FileManager.default.fileExists(atPath: directory.appendingPathComponent("unwanted").path))
    }

    func testNoArgumentsReturnsOnAcceptanceWhileGUIStaysAlive() throws {
        let directory = try directory()
        try gui(in: directory, body: """
        import os, socket, struct, sys, time
        assert sys.argv[3:] == []
        with open('gui.pid', 'w') as file:
            file.write(str(os.getpid()))
        connection = socket.socket(socket.AF_UNIX)
        connection.connect(sys.argv[2])
        connection.sendall(struct.pack('<i', 0))
        time.sleep(10)
        """)
        defer { stopGUI(in: directory) }
        let started = ProcessInfo.processInfo.systemUptime
        XCTAssertEqual(try launch(in: directory), 0)
        XCTAssertLessThan(ProcessInfo.processInfo.systemUptime - started, 3)
        XCTAssertEqual(kill(try guiPID(in: directory), 0), 0)
    }

    func testForwarderMayExitBeforeOwningGUIAcceptsRequest() throws {
        let directory = try directory()
        try gui(in: directory, body: """
        import os, socket, struct, sys, time
        if os.fork() != 0:
            os._exit(0)
        time.sleep(0.15)
        connection = socket.socket(socket.AF_UNIX)
        connection.connect(sys.argv[2])
        connection.sendall(struct.pack('<i', 0))
        os._exit(0)
        """)
        XCTAssertEqual(try launch(in: directory), 0)
    }

    func testMissingGUIFailedLaunchAndMissingAcknowledgmentFail() throws {
        let directory = try directory()
        XCTAssertThrowsError(try launch(in: directory))
        try gui(in: directory, body: "import sys; sys.exit(7)")
        XCTAssertThrowsError(try launch(in: directory)) { error in
            XCTAssertTrue(error.localizedDescription.contains("launch status 7"), error.localizedDescription)
        }
        try gui(in: directory, body: "import sys; sys.exit(0)")
        XCTAssertThrowsError(try launch(in: directory, startupTimeout: 0.1))
        try gui(in: directory, body: "import sys; sys.exit(0)")
        try FileManager.default.setAttributes([.posixPermissions: 0o600],
            ofItemAtPath: directory.appendingPathComponent("Viem").path)
        XCTAssertThrowsError(try launch(in: directory))
    }

    func testConnectedSilentAndPartialAcknowledgmentsTimeOut() throws {
        for count in [0, 1, 3] {
            let directory = try directory()
            try gui(in: directory, body: """
            import os, socket, sys, time
            with open('gui.pid', 'w') as file:
                file.write(str(os.getpid()))
            connection = socket.socket(socket.AF_UNIX)
            connection.connect(sys.argv[2])
            connection.sendall(bytes(\(count)))
            time.sleep(10)
            """)
            defer { stopGUI(in: directory) }
            let started = ProcessInfo.processInfo.systemUptime
            XCTAssertThrowsError(try launch(in: directory, completionTimeout: 0.08)) { error in
                XCTAssertTrue(error.localizedDescription.contains("launch timeout"), error.localizedDescription)
            }
            XCTAssertLessThan(ProcessInfo.processInfo.systemUptime - started, 3)
        }
    }

    func testClosedAndPartialAcknowledgmentsNeverReportSuccess() throws {
        for count in [0, 1, 3] {
            let directory = try directory()
            try gui(in: directory, body: """
            import os, socket, sys
            connection = socket.socket(socket.AF_UNIX)
            connection.connect(sys.argv[2])
            connection.sendall(bytes(\(count)))
            os._exit(0)
            """)
            XCTAssertThrowsError(try launch(in: directory))
        }
    }

    func testRejectionForwardsNonzeroAcceptanceStatus() throws {
        let directory = try directory()
        try gui(in: directory, body: """
        import socket, struct, sys
        connection = socket.socket(socket.AF_UNIX)
        connection.connect(sys.argv[2])
        connection.sendall(struct.pack('<i', 1))
        """)
        XCTAssertEqual(try launch(in: directory), 1)
    }

    func testInvalidArgumentsAndDirectoriesFailBeforeLaunch() throws {
        let directory = try directory()
        try gui(in: directory, body: "open('launched', 'w').close()")
        let wrapper = directory.appendingPathComponent("viem-cli")
        XCTAssertThrowsError(try EVCommandLineLauncher.run(arguments: ["bad\0argument"],
            workingDirectory: directory.path, wrapperExecutable: wrapper,
            startupTimeout: 0.1, completionTimeout: 0.1)) { error in
            XCTAssertTrue(error.localizedDescription.contains("null character"), error.localizedDescription)
        }
        for workingDirectory in ["relative", "", directory.path + "\0bad", directory.path + "/missing",
                                 directory.appendingPathComponent("Viem").path] {
            XCTAssertThrowsError(try EVCommandLineLauncher.run(arguments: [],
                workingDirectory: workingDirectory, wrapperExecutable: wrapper,
                startupTimeout: 0.1, completionTimeout: 0.1)) { error in
                XCTAssertTrue(error.localizedDescription.contains("working directory"), error.localizedDescription)
            }
        }
        for executable in [URL(string: "https://example.invalid/viem-cli")!,
                           URL(fileURLWithPath: directory.path + "/bad\0launcher")] {
            XCTAssertThrowsError(try EVCommandLineLauncher.run(arguments: [],
                workingDirectory: directory.path, wrapperExecutable: executable,
                startupTimeout: 0.1, completionTimeout: 0.1)) { error in
                XCTAssertTrue(error.localizedDescription.contains("locate the Viem launcher"), error.localizedDescription)
            }
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: directory.appendingPathComponent("launched").path))
    }

    private struct Invocation: Decodable {
        let arguments: [String]
        let cwd: String
        let detached: Bool
    }

    private func launch(in directory: URL, startupTimeout: TimeInterval = 2,
                        completionTimeout: TimeInterval = 1) throws -> Int32 {
        try EVCommandLineLauncher.run(arguments: [], workingDirectory: directory.path,
            wrapperExecutable: directory.appendingPathComponent("viem-cli"),
            startupTimeout: startupTimeout, completionTimeout: completionTimeout)
    }

    private func directory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-cli-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return directory
    }

    private func gui(in directory: URL, body: String) throws {
        let file = directory.appendingPathComponent("Viem")
        try ("#!/usr/bin/python3\n" + body + "\n").write(to: file, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: file.path)
    }

    private func guiPID(in directory: URL) throws -> pid_t {
        try XCTUnwrap(pid_t(String(contentsOf: directory.appendingPathComponent("gui.pid"), encoding: .utf8)))
    }

    private func stopGUI(in directory: URL) {
        if let pid = try? guiPID(in: directory) { _ = kill(pid, SIGTERM) }
    }
}
