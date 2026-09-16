#!/usr/bin/env swift
// Build Viem first, then run: swift scripts/test-single-instance.swift [executable]
// The invoking terminal/runtime needs Accessibility permission. The test uses
// private instance/config directories and terminates only processes it starts.
import AppKit
import ApplicationServices
import Darwin
import Foundation

struct TestFailure: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

func require(_ condition: Bool, _ message: String) throws {
    if !condition { throw TestFailure(message) }
}

func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
    return value
}

func children(_ element: AXUIElement) -> [AXUIElement] {
    attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []
}

func editors(_ element: AXUIElement, depth: Int = 0) -> [AXUIElement] {
    if attribute(element, kAXRoleAttribute) as? String == kAXTextAreaRole,
       attribute(element, kAXDescriptionAttribute) as? String == "Viem editor" {
        return [element]
    }
    guard depth < 20 else { return [] }
    return children(element).flatMap { editors($0, depth: depth + 1) }
}

func waitFor(_ message: String, timeout: TimeInterval = 10, _ condition: () -> Bool) throws {
    let deadline = Date(timeIntervalSinceNow: timeout)
    while !condition() {
        if Date() >= deadline { throw TestFailure("Timed out: \(message)") }
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
    }
}

func selectedOffset(_ editor: AXUIElement) -> Int? {
    guard let value = attribute(editor, kAXSelectedTextRangeAttribute),
          CFGetTypeID(value) == AXValueGetTypeID() else { return nil }
    var range = CFRange()
    guard AXValueGetValue(value as! AXValue, .cfRange, &range) else { return nil }
    return range.location
}

func stop(_ process: Process) {
    guard process.isRunning else { return }
    process.terminate()
    var deadline = Date(timeIntervalSinceNow: 3)
    while process.isRunning && Date() < deadline {
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
    }
    if process.isRunning {
        kill(process.processIdentifier, SIGKILL)
        deadline = Date(timeIntervalSinceNow: 2)
        while process.isRunning && Date() < deadline {
            RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
        }
    }
}

func run() throws {
    try require(AXIsProcessTrusted(),
        "Accessibility permission is required for the terminal/runtime running this GUI test.")
    let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
    let executable = CommandLine.arguments.dropFirst().first.map { URL(fileURLWithPath: $0) }
        ?? root.appendingPathComponent(".build/Viem.app/Contents/MacOS/Viem")
    try require(FileManager.default.isExecutableFile(atPath: executable.path), "Build Viem before running this test.")
    // Keep the Unix-domain socket path below sockaddr_un's macOS limit.
    let temporary = URL(fileURLWithPath: "/tmp/viem-gui-\(UUID().uuidString.prefix(8))", isDirectory: true)
    try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true,
        attributes: [.posixPermissions: 0o700])
    defer { try? FileManager.default.removeItem(at: temporary) }
    let firstDirectory = temporary.appendingPathComponent("first", isDirectory: true)
    let senderDirectory = temporary.appendingPathComponent("sender", isDirectory: true)
    for directory in [firstDirectory, senderDirectory] {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }
    let initialText = "first\nsecond\nthird"
    let newFirstText = "alpha\nbeta\ngamma"
    let newSecondText = "north\nsouth\nwest"
    for (directory, filename, content) in [
        (firstDirectory, "startup.txt", initialText),
        (firstDirectory, "lazy.txt", "Lazy file must stay unopened"),
        (senderDirectory, "new first.txt", newFirstText),
        (senderDirectory, "new-second.txt", newSecondText),
    ] {
        try Data(content.utf8).write(to: directory.appendingPathComponent(filename))
    }
    var environment = ProcessInfo.processInfo.environment
    environment["VIEM_INSTANCE_DIRECTORY"] = temporary.appendingPathComponent("instance").path
    environment["VIEM_CONFIG_DIR"] = temporary.appendingPathComponent("config").path
    let logURL = temporary.appendingPathComponent("process.log")
    FileManager.default.createFile(atPath: logURL.path, contents: nil)
    let log = try FileHandle(forWritingTo: logURL)
    defer { try? log.close() }
    var processes: [Process] = []
    defer {
        for process in processes { stop(process) }
    }

    func launch(_ arguments: [String], in directory: URL) throws -> Process {
        let process = Process()
        process.executableURL = executable
        process.arguments = arguments
        process.currentDirectoryURL = directory
        process.environment = environment
        process.standardOutput = log
        process.standardError = log
        try process.run()
        processes.append(process)
        return process
    }

    let primary = try launch(["+2", "startup.txt", "lazy.txt"], in: firstDirectory)
    let application = AXUIElementCreateApplication(primary.processIdentifier)
    func windows() -> [AXUIElement] {
        (attribute(application, kAXWindowsAttribute) as? [AXUIElement] ?? []).filter { !editors($0).isEmpty }
    }
    func allEditors() -> [AXUIElement] { windows().flatMap { editors($0) } }
    func editor(containing text: String) -> AXUIElement? {
        allEditors().first { attribute($0, kAXValueAttribute) as? String == text }
    }
    func request(_ arguments: [String], in directory: URL = senderDirectory) throws {
        let process = try launch(arguments, in: directory)
        try waitFor("forwarded process should exit", timeout: 15) { !process.isRunning }
        try require(process.terminationStatus == 0, "Forwarded process failed; log: \((try? String(contentsOf: logURL, encoding: .utf8)) ?? "")")
        try require(primary.isRunning, "The original process must stay running.")
    }

    try waitFor("startup uses +line and lazily opens only the first argument") {
        windows().count == 1 && allEditors().count == 1
            && editor(containing: initialText).flatMap(selectedOffset) == 6
    }
    try request([])
    try waitFor("empty invocation activates the existing window") {
        windows().count == 1 && allEditors().count == 1
            && (attribute(application, kAXFrontmostAttribute) as? Bool) == true
    }
    try request(["-o", "+2", "new first.txt", "new-second.txt"])
    try waitFor("relative forwarded files open together with +line only on the first") {
        windows().count == 2 && allEditors().count == 3
            && editor(containing: newFirstText).flatMap(selectedOffset) == 6
            && editor(containing: newSecondText).flatMap(selectedOffset) == 0
    }
    try request(["+3", "new-second.txt"])
    try waitFor("existing split document is reused and positioned") {
        windows().count == 2 && allEditors().count == 3
            && editor(containing: newSecondText).flatMap(selectedOffset) == 12
    }
    try request(["-o", "new first.txt", "./new first.txt"])
    try waitFor("duplicate arguments focus the existing pane without creating another") {
        guard let focused = attribute(application, kAXFocusedUIElementAttribute) else { return false }
        return windows().count == 2 && allEditors().count == 3
            && attribute(focused as! AXUIElement, kAXValueAttribute) as? String == newFirstText
    }
    let malformed = try launch(["-unknown"], in: senderDirectory)
    try waitFor("malformed launch reports an error") { !malformed.isRunning }
    try require(malformed.terminationStatus != 0, "Malformed arguments must fail.")
    try require((try String(contentsOf: logURL, encoding: .utf8)).contains("Usage: Viem"),
        "Malformed arguments must report command-line usage.")
    try require(primary.isRunning && windows().count == 2 && allEditors().count == 3,
        "Malformed arguments must preserve the original process and its windows.")

    for window in windows() {
        guard let close = attribute(window, kAXCloseButtonAttribute) else {
            throw TestFailure("Document window has no accessible close button.")
        }
        try require(AXUIElementPerformAction(close as! AXUIElement, kAXPressAction as CFString) == .success,
            "Could not close the test document window.")
    }
    try waitFor("all document windows close while original process remains") { windows().isEmpty && primary.isRunning }
    try request([])
    try waitFor("empty invocation recreates a blank window in the original process") {
        windows().count == 1 && allEditors().count == 1 && editor(containing: "") != nil
    }

    stop(primary)
    try require(!primary.isRunning, "The first isolated test instance must terminate before the next startup test.")
    environment["VIEM_INSTANCE_DIRECTORY"] = temporary.appendingPathComponent("split-instance").path
    let splitPrimary = try launch(["-o", "+2", "new first.txt", "new-second.txt"], in: senderDirectory)
    let splitApplication = AXUIElementCreateApplication(splitPrimary.processIdentifier)
    try waitFor("fresh-process -o startup creates one window with the requested panes") {
        let splitWindows = (attribute(splitApplication, kAXWindowsAttribute) as? [AXUIElement] ?? [])
            .filter { !editors($0).isEmpty }
        let splitEditors = splitWindows.flatMap { editors($0) }
        return splitWindows.count == 1 && splitEditors.count == 2
            && splitEditors.first { attribute($0, kAXValueAttribute) as? String == newFirstText }
                .flatMap(selectedOffset) == 6
            && splitEditors.first { attribute($0, kAXValueAttribute) as? String == newSecondText }
                .flatMap(selectedOffset) == 0
    }
    print("Single-instance GUI launch checks passed.")
}

do { try run() }
catch {
    FileHandle.standardError.write(Data("Single-instance GUI test failed: \(error)\n".utf8))
    exit(EXIT_FAILURE)
}
