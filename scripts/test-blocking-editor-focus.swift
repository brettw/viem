#!/usr/bin/env swift
// Build Viem first, then run: swift scripts/test-blocking-editor-focus.swift [app-bundle]
// Requires an unlocked desktop and Accessibility permission for the invoking
// terminal/runtime. Copies the bundle, isolates settings, and restores focus.
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

func waitFor(_ message: String, timeout: TimeInterval = 15, _ condition: () -> Bool) throws {
    let deadline = Date(timeIntervalSinceNow: timeout)
    while !condition() {
        if Date() >= deadline { throw TestFailure("Timed out: \(message)") }
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
    }
}

func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
    return value
}

func editors(_ element: AXUIElement, depth: Int = 0) -> [AXUIElement] {
    if attribute(element, kAXRoleAttribute) as? String == kAXTextAreaRole,
       attribute(element, kAXDescriptionAttribute) as? String == "Viem editor" {
        return [element]
    }
    guard depth < 20 else { return [] }
    return (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [])
        .flatMap { editors($0, depth: depth + 1) }
}

func button(_ element: AXUIElement, title: String, depth: Int = 0) -> AXUIElement? {
    if attribute(element, kAXRoleAttribute) as? String == kAXButtonRole,
       attribute(element, kAXTitleAttribute) as? String == title { return element }
    guard depth < 20 else { return nil }
    for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] {
        if let found = button(child, title: title, depth: depth + 1) { return found }
    }
    return nil
}

func canonical(_ url: URL) -> URL { url.standardizedFileURL.resolvingSymlinksInPath() }

func bringToFront(_ application: NSRunningApplication) throws {
    guard let bundle = application.bundleURL, !application.isTerminated else {
        throw TestFailure("The application used to restore foreground focus is unavailable.")
    }
    let configuration = NSWorkspace.OpenConfiguration()
    configuration.activates = true
    configuration.createsNewApplicationInstance = false
    configuration.allowsRunningApplicationSubstitution = false
    configuration.addsToRecentItems = false
    configuration.promptsUserIfNeeded = false
    configuration.appleEvent = NSAppleEventDescriptor(
        eventClass: AEEventClass(kAEMiscStandards), eventID: AEEventID(kAEActivate),
        targetDescriptor: nil, returnID: AEReturnID(kAutoGenerateReturnID),
        transactionID: AETransactionID(kAnyTransactionID))
    var completed = false
    var failure: Error?
    NSWorkspace.shared.openApplication(at: bundle, configuration: configuration) { _, error in
        DispatchQueue.main.async {
            failure = error
            completed = true
        }
    }
    try waitFor("activate \(application.localizedName ?? bundle.lastPathComponent)") { completed }
    if let failure { throw failure }
    try waitFor("foreground application changes before the blocking request") {
        NSWorkspace.shared.frontmostApplication?.processIdentifier == application.processIdentifier
    }
}

func stop(_ process: Process) {
    guard process.isRunning else { return }
    process.terminate()
    try? waitFor("test helper exits", timeout: 3) { !process.isRunning }
    if process.isRunning {
        kill(process.processIdentifier, SIGKILL)
        try? waitFor("test helper is killed", timeout: 2) { !process.isRunning }
    }
}

func stop(_ application: NSRunningApplication) {
    guard !application.isTerminated else { return }
    application.terminate()
    try? waitFor("test application exits", timeout: 3) { application.isTerminated }
    if !application.isTerminated {
        application.forceTerminate()
        try? waitFor("test application is killed", timeout: 2) { application.isTerminated }
    }
}

func run() throws {
    try require(AXIsProcessTrusted(),
        "Accessibility permission is required for the terminal/runtime running this GUI test.")
    let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
    let sourceBundle = CommandLine.arguments.dropFirst().first.map { URL(fileURLWithPath: $0) }
        ?? root.appendingPathComponent(".build/Viem.app")
    for executable in ["Viem", "blocking-viem"] {
        try require(FileManager.default.isExecutableFile(atPath:
            sourceBundle.appendingPathComponent("Contents/MacOS/\(executable)").path),
            "Build Viem and blocking-viem before running this test.")
    }
    let previousApplication = NSWorkspace.shared.frontmostApplication
    guard let backgroundApplication = NSRunningApplication.runningApplications(
        withBundleIdentifier: "com.apple.finder").first ?? previousApplication else {
        throw TestFailure("An existing application is required to test the focus handoff.")
    }
    // Keep the Unix-domain socket path below sockaddr_un's macOS limit.
    let temporary = URL(fileURLWithPath: "/tmp/viem-focus-\(UUID().uuidString.prefix(8))", isDirectory: true)
    try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true,
        attributes: [.posixPermissions: 0o700])
    defer { try? FileManager.default.removeItem(at: temporary) }
    let bundle = temporary.appendingPathComponent("Viem.app", isDirectory: true)
    try FileManager.default.copyItem(at: sourceBundle, to: bundle)
    let executable = bundle.appendingPathComponent("Contents/MacOS/Viem")
    let helper = bundle.appendingPathComponent("Contents/MacOS/blocking-viem")
    // Match the unique copied executable, never another user's running Viem bundle.
    func testApplications() -> [NSRunningApplication] {
        NSWorkspace.shared.runningApplications.filter {
            $0.executableURL.map(canonical) == canonical(executable)
        }
    }
    var processes: [Process] = []
    defer {
        processes.forEach(stop)
        testApplications().forEach(stop)
        if let previousApplication, !previousApplication.isTerminated {
            do { try bringToFront(previousApplication) }
            catch { FileHandle.standardError.write(Data("Could not restore foreground focus: \(error)\n".utf8)) }
        }
    }
    let coldName = "COMMIT_EDITMSG"
    let coldText = "Cold blocking editor focus\n"
    let warmName = "warm message.txt"
    let warmText = "Warm blocking editor focus\n"
    let decoyName = "other.txt"
    let decoyText = "The other pane must not receive the blocking request.\n"
    for (filename, text) in [(coldName, coldText), (warmName, warmText), (decoyName, decoyText)] {
        try Data(text.utf8).write(to: temporary.appendingPathComponent(filename))
    }
    try FileManager.default.createSymbolicLink(atPath: temporary.appendingPathComponent("alias.txt").path,
        withDestinationPath: warmName)
    var environment = ProcessInfo.processInfo.environment
    environment["VIEM_INSTANCE_DIRECTORY"] = temporary.appendingPathComponent("instance").path
    environment["VIEM_CONFIG_DIR"] = temporary.appendingPathComponent("config").path
    let logURL = temporary.appendingPathComponent("process.log")
    FileManager.default.createFile(atPath: logURL.path, contents: nil)
    let log = try FileHandle(forWritingTo: logURL)
    defer { try? log.close() }

    func launch(_ program: URL, _ arguments: [String]) throws -> Process {
        let process = Process()
        process.executableURL = program
        process.arguments = arguments
        process.currentDirectoryURL = temporary
        process.environment = environment
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = log
        process.standardError = log
        try process.run()
        processes.append(process)
        return process
    }
    func requireWaiting(_ callers: [Process]) throws {
        let deadline = Date(timeIntervalSinceNow: 0.3)
        repeat {
            try require(callers.allSatisfy(\.isRunning),
                "A blocking helper returned before its document closed.")
            RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
        } while Date() < deadline
    }
    func requireSuccess(_ process: Process) throws {
        try waitFor("blocking helper completes after its document closes") { !process.isRunning }
        try require(process.terminationStatus == 0,
            "Helper failed: \((try? String(contentsOf: logURL, encoding: .utf8)) ?? "")")
    }

    try require(testApplications().isEmpty, "The cold test must start without its isolated GUI process.")
    try bringToFront(backgroundApplication)
    let cold = try launch(helper, [coldName])
    try waitFor("cold blocking request starts its isolated application") { testApplications().count == 1 }
    guard let primary = testApplications().first else {
        throw TestFailure("The isolated GUI exited during startup.")
    }
    let application = AXUIElementCreateApplication(primary.processIdentifier)
    func windows() -> [AXUIElement] {
        (attribute(application, kAXWindowsAttribute) as? [AXUIElement] ?? [])
            .filter { !editors($0).isEmpty }
    }
    func focused(_ text: String, filename: String, paneCount: Int) -> Bool {
        let documentWindows = windows()
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == primary.processIdentifier,
              documentWindows.count == 1, editors(documentWindows[0]).count == paneCount,
              let documentPath = attribute(documentWindows[0], kAXDocumentAttribute) as? String,
              let documentURL = URL(string: documentPath),
              canonical(documentURL) == canonical(temporary.appendingPathComponent(filename)),
              let focusedWindow = attribute(application, kAXFocusedWindowAttribute),
              CFEqual(focusedWindow, documentWindows[0]),
              let focusedEditor = attribute(application, kAXFocusedUIElementAttribute),
              let expected = editors(documentWindows[0]).first(where: {
                  attribute($0, kAXValueAttribute) as? String == text
              }) else { return false }
        return CFEqual(focusedEditor, expected)
    }
    func closeDocumentWindow() throws {
        guard windows().count == 1, let close = attribute(windows()[0], kAXCloseButtonAttribute) else {
            throw TestFailure("Expected one document window with an accessible close button.")
        }
        try require(AXUIElementPerformAction(close as! AXUIElement, kAXPressAction as CFString) == .success,
            "Could not close the test document window.")
        try waitFor("document window closes while its GUI process remains") {
            windows().isEmpty && !primary.isTerminated
        }
    }

    try waitFor("cold request makes its exact document and GUI frontmost without a blank window") {
        focused(coldText, filename: coldName, paneCount: 1)
    }
    try requireWaiting([cold])
    try closeDocumentWindow()
    try requireSuccess(cold)

    let split = try launch(executable, ["-o", decoyName, warmName])
    try waitFor("warm setup forwards to the existing application") { !split.isRunning }
    try require(split.terminationStatus == 0, "Could not prepare the warm split window.")
    try waitFor("warm setup has two distinct panes with the first selected") {
        windows().count == 1 && editors(windows()[0]).count == 2
            && editors(windows()[0]).contains { attribute($0, kAXValueAttribute) as? String == warmText }
            && attribute(application, kAXFocusedUIElementAttribute).map {
                attribute($0 as! AXUIElement, kAXValueAttribute) as? String == decoyText
            } == true
    }
    try bringToFront(backgroundApplication)
    let warm = try launch(helper, [warmName])
    try waitFor("warm request reuses and focuses the requested pane in the original GUI") {
        focused(warmText, filename: warmName, paneCount: 2)
            && testApplications().map(\.processIdentifier) == [primary.processIdentifier]
    }
    try requireWaiting([warm])
    try bringToFront(backgroundApplication)
    let alias = try launch(helper, ["alias.txt"])
    try waitFor("alias request restores focus without another window, pane, or GUI process") {
        focused(warmText, filename: warmName, paneCount: 2)
            && testApplications().map(\.processIdentifier) == [primary.processIdentifier]
    }
    try requireWaiting([warm, alias])
    try closeDocumentWindow()
    try requireSuccess(warm)
    try requireSuccess(alias)

    let recoveryFile = temporary.appendingPathComponent("recovery.txt")
    let swapFile = temporary.appendingPathComponent(".recovery.txt.swp")
    let recoveryBytes = Data("Recovery focus fixture\n".utf8)
    let foreignSwapBytes = Data("Foreign editor swap fixture".utf8)
    try recoveryBytes.write(to: recoveryFile)
    try foreignSwapBytes.write(to: swapFile)
    func recoveryCancelButton() -> AXUIElement? {
        for window in attribute(application, kAXWindowsAttribute) as? [AXUIElement] ?? [] {
            if button(window, title: "Open Read-Only") != nil {
                return button(window, title: "Cancel")
            }
        }
        return nil
    }
    try bringToFront(backgroundApplication)
    let recovery = try launch(helper, [recoveryFile.lastPathComponent])
    try waitFor("recovery decision becomes foreground before its modal loop completes") {
        NSWorkspace.shared.frontmostApplication?.processIdentifier == primary.processIdentifier
            && recoveryCancelButton() != nil && windows().isEmpty
            && testApplications().map(\.processIdentifier) == [primary.processIdentifier]
    }
    try requireWaiting([recovery])
    guard let cancel = recoveryCancelButton() else { throw TestFailure("Recovery dialog disappeared.") }
    try require(AXUIElementPerformAction(cancel, kAXPressAction as CFString) == .success,
        "Could not cancel the recovery dialog.")
    try waitFor("cancelled recovery returns failure without opening another document") {
        !recovery.isRunning && recoveryCancelButton() == nil && windows().isEmpty && !primary.isTerminated
    }
    try require(recovery.terminationStatus != 0, "Cancelling recovery must fail the blocking request.")
    try require(try Data(contentsOf: recoveryFile) == recoveryBytes && Data(contentsOf: swapFile) == foreignSwapBytes,
        "Cancelling recovery must preserve both the document and foreign swap file.")
    print("Blocking-editor cold, warm, alias, recovery focus and wait checks passed.")
}

do { try run() }
catch {
    FileHandle.standardError.write(Data("Blocking-editor focus test failed: \(error)\n".utf8))
    exit(EXIT_FAILURE)
}
