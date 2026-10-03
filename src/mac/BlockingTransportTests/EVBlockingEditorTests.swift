import Foundation
import XCTest
@testable import ViemBlockingTransport

final class EVBlockingEditorTests: XCTestCase {
    func testLaunchPreservesRelativeUnicodeSpacedAndOptionLikePathsThroughSymlink() throws {
        let directory = try directory()
        let bin = directory.appendingPathComponent("application with spaces", isDirectory: true)
        try FileManager.default.createDirectory(at: bin, withIntermediateDirectories: true)
        let executable = bin.appendingPathComponent("blocking-viem")
        try Data().write(to: executable)
        let link = directory.appendingPathComponent("editor-link")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: executable)
        try gui(in: bin, body: """
        import json, os, socket, struct, sys
        assert sys.argv[1] == '--blocking-edit'
        with open('received.json', 'w') as file:
            json.dump({'path': sys.argv[3], 'cwd': os.getcwd()}, file)
        connection = socket.socket(socket.AF_UNIX)
        connection.connect(sys.argv[2])
        connection.sendall(struct.pack('<i', 7))
        """)
        let name = "--日本語 commit message"
        let status = try EVBlockingEditor.run(file: name, workingDirectory: directory.path,
                                              wrapperExecutable: link, startupTimeout: 2)
        XCTAssertEqual(status, 7)
        let result = try JSONDecoder().decode([String: String].self,
            from: Data(contentsOf: directory.appendingPathComponent("received.json")))
        XCTAssertEqual(result["path"], directory.appendingPathComponent(name).path)
        let receivedDirectory = URL(fileURLWithPath: try XCTUnwrap(result["cwd"])).resolvingSymlinksInPath()
        XCTAssertEqual(receivedDirectory.path, directory.resolvingSymlinksInPath().path)
    }

    func testForwarderMayExitBeforeGUIConnects() throws {
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
        XCTAssertEqual(try EVBlockingEditor.run(file: "COMMIT_EDITMSG", workingDirectory: directory.path,
            wrapperExecutable: directory.appendingPathComponent("blocking-viem"), startupTimeout: 2), 0)
    }

    func testMissingGUIFailedLaunchAndSilentForwarderAreFailures() throws {
        let directory = try directory()
        let wrapper = directory.appendingPathComponent("blocking-viem")
        XCTAssertThrowsError(try EVBlockingEditor.run(file: "file", workingDirectory: directory.path,
                                                     wrapperExecutable: wrapper, startupTimeout: 1))
        try gui(in: directory, body: "import sys; sys.exit(7)")
        XCTAssertThrowsError(try EVBlockingEditor.run(file: "file", workingDirectory: directory.path,
                                                     wrapperExecutable: wrapper, startupTimeout: 2)) { error in
            XCTAssertTrue(error.localizedDescription.contains("launch status 7"), error.localizedDescription)
        }
        try gui(in: directory, body: "import sys; sys.exit(0)")
        XCTAssertThrowsError(try EVBlockingEditor.run(file: "file", workingDirectory: directory.path,
                                                     wrapperExecutable: wrapper, startupTimeout: 0.2))
    }

    func testRejectsDirectoryAndInvalidFileArgumentsBeforeLaunch() throws {
        let directory = try directory()
        for file in [directory.path, "", "bad\0file"] {
            XCTAssertThrowsError(try EVBlockingEditor.run(file: file, workingDirectory: directory.path,
                wrapperExecutable: directory.appendingPathComponent("blocking-viem"), startupTimeout: 1)) { error in
                XCTAssertTrue(error.localizedDescription.contains("Expected a file"), error.localizedDescription)
            }
        }
    }

    private func directory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-blocking-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return directory
    }

    private func gui(in directory: URL, body: String) throws {
        let file = directory.appendingPathComponent("Viem")
        try ("#!/usr/bin/python3\n" + body + "\n").write(to: file, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: file.path)
    }
}
