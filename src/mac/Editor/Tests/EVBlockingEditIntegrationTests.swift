import AppKit
import CViemCore
import ViemBlockingTransport
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

/// Use the same socket exchange as blocking-viem without parking AppKit's thread.
private final class EVBlockingTestCaller: @unchecked Sendable {
    let completion: ViemBlockingTransport.EVBlockingEditCompletion
    let finished = XCTestExpectation(description: "blocking editor caller finishes")
    private let lock = NSLock()
    private var storedResult: Result<Int32, Error>?

    var result: Result<Int32, Error>? {
        lock.lock()
        defer { lock.unlock() }
        return storedResult
    }

    init() throws {
        let listener = try EVBlockingEditListener()
        completion = try .connect(endpoint: listener.endpoint)
        DispatchQueue.global().async { [self, listener] in
            let result = Result { try listener.waitForCompletion(startupTimeout: 2) }
            lock.lock()
            storedResult = result
            lock.unlock()
            finished.fulfill()
        }
    }
}

@MainActor
final class EVBlockingEditIntegrationTests: XCTestCase {
    @MainActor private final class Fixture {
        let directory: URL
        let delegate: EVApplicationDelegate
        private let originalDocuments: Set<ObjectIdentifier>
        private var callers: [EVBlockingTestCaller] = []

        init(launchArguments: EVLaunchArguments = EVLaunchArguments(),
             initialCaller: EVBlockingTestCaller? = nil) throws {
            EVEditorComposition.install()
            directory = FileManager.default.temporaryDirectory
                .appendingPathComponent("viem-blocking-integration-\(UUID())", isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            originalDocuments = Set(NSDocumentController.shared.documents.map(ObjectIdentifier.init))
            let configuration = EVConfigurationStore(
                directory: directory.appendingPathComponent("profile", isDirectory: true), legacyDefaults: nil)
            delegate = EVApplicationDelegate(configuration: configuration,
                launchArguments: launchArguments, launchDirectory: directory,
                blockingCompletion: initialCaller?.completion)
            delegate.documentFactory = { EVDocument(editorBackend: EVCoreDocumentBackend(configuration: configuration)) }
            delegate.recordRecentDocument = { _ in }
            delegate.terminateApplication = { XCTFail("A native test must not terminate the test application") }
            if let initialCaller { callers.append(initialCaller) }
        }

        var documents: [EVDocument] {
            NSDocumentController.shared.documents.compactMap { document in
                originalDocuments.contains(ObjectIdentifier(document)) ? nil : document as? EVDocument
            }
        }

        func write(_ name: String, text: String = "Original message\n") throws -> URL {
            let url = directory.appendingPathComponent(name)
            try Data(text.utf8).write(to: url)
            return url
        }

        func process(_ path: String, caller: EVBlockingTestCaller? = nil) {
            delegate.processLaunchArguments(EVLaunchArguments(filenames: [path]),
                workingDirectory: directory, blockingCompletion: caller?.completion)
            for document in documents {
                for controller in document.windowControllers.compactMap({ $0 as? EVDocumentWindowController }) {
                    controller.commandDidCloseWindow = {}
                    controller.terminateWithStatus = { _ in XCTFail("Unexpected exit request") }
                }
            }
        }

        func caller(_ path: String) throws -> EVBlockingTestCaller {
            let caller = try EVBlockingTestCaller()
            callers.append(caller)
            process(path, caller: caller)
            return caller
        }

        func document(_ url: URL) throws -> EVDocument {
            try XCTUnwrap(EVDocumentIdentity.existingDocument(at: url))
        }

        func window(_ document: EVDocument) throws -> EVDocumentWindowController {
            try XCTUnwrap(EVDocumentWindowController.windowShowing(document: document)?.windowController
                as? EVDocumentWindowController)
        }

        func close() {
            // Even failed assertions release every background listener before
            // the fixture goes away; never leave an indefinite socket read.
            EVBlockingEditSessions.shared.abort(exitCode: 1)
            for caller in callers { caller.completion.finish(exitCode: 1) }
            for document in documents { document.close() }
            try? FileManager.default.removeItem(at: directory)
        }
    }

    private func surface(_ window: EVDocumentWindowController) throws -> EVEditorSurfaceController {
        try XCTUnwrap(window.editorSurface as? EVEditorSurfaceController)
    }

    private func ex(_ command: String, in surface: EVEditorSurfaceController) {
        surface.editorView.insertText(":" + command,
            replacementRange: NSRange(location: NSNotFound, length: 0))
        surface.editorView.doCommand(by: #selector(NSResponder.insertNewline(_:)))
    }

    private func edit(_ text: String, in surface: EVEditorSurfaceController) throws {
        surface.editorView.insertText("i" + text,
            replacementRange: NSRange(location: NSNotFound, length: 0))
        surface.performInput { _ = try XCTUnwrap(surface.session).sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
    }

    private func assertWaiting(_ callers: [EVBlockingTestCaller], file: StaticString = #filePath,
                               line: UInt = #line) async throws {
        try await Task.sleep(for: .milliseconds(40))
        for caller in callers {
            XCTAssertNil(caller.result, "Caller must wait for the last view", file: file, line: line)
        }
    }

    private func assertCompleted(_ callers: [EVBlockingTestCaller], status: Int32,
                                 file: StaticString = #filePath, line: UInt = #line) async throws {
        await fulfillment(of: callers.map(\.finished), timeout: 3)
        for caller in callers {
            XCTAssertEqual(try XCTUnwrap(caller.result, file: file, line: line).get(), status,
                file: file, line: line)
        }
    }

    private func eventually(_ description: String, _ condition: () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail(description)
        throw CocoaError(.userCancelled)
    }

    func testInitialBlockingLaunchActivatesOnceAndWaitsForDocumentClosure() async throws {
        let caller = try EVBlockingTestCaller()
        let f = try Fixture(launchArguments: EVLaunchArguments(filenames: ["COMMIT_EDITMSG"]),
                            initialCaller: caller)
        defer { f.close() }
        let url = try f.write("COMMIT_EDITMSG")
        var activationRequests = 0
        f.delegate.activateForBlockingEdit = { activationRequests += 1 }

        XCTAssertTrue(f.delegate.openLaunchArguments())
        XCTAssertTrue(f.delegate.openLaunchArguments())

        XCTAssertEqual(activationRequests, 1, "Repeated launch callbacks must not replay the focus handoff")
        XCTAssertEqual(f.documents.count, 1)
        let document = try f.document(url)
        XCTAssertEqual(document.windowControllers.count, 1)
        try await assertWaiting([caller])
        document.close()
        try await assertCompleted([caller], status: 0)
    }

    func testConcurrentRelativeAndAliasedCallersReuseExistingDocumentUntilItsLastViewCloses() async throws {
        let f = try Fixture()
        defer { f.close() }
        let url = try f.write("commit message café.txt")
        let alias = f.directory.appendingPathComponent("commit alias.txt")
        let hardlink = f.directory.appendingPathComponent("commit hard link.txt")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: url)
        try FileManager.default.linkItem(at: url, to: hardlink)
        var activationRequests = 0
        f.delegate.activateForBlockingEdit = { activationRequests += 1 }
        f.process(url.path)
        XCTAssertEqual(activationRequests, 0)
        let document = try f.document(url)
        let first = try f.window(document)
        let callers = try ["./" + url.lastPathComponent, alias.path, hardlink.path].map { try f.caller($0) }
        XCTAssertEqual(activationRequests, 3, "Every forwarded blocking request must activate its document owner")
        XCTAssertEqual(f.documents.count, 1)
        XCTAssertTrue(try f.document(alias) === document)
        XCTAssertTrue(try f.document(hardlink) === document)
        XCTAssertEqual(document.windowControllers.count, 1)
        XCTAssertEqual(EVBlockingEditSessions.shared.count, 3)

        document.showAdditionalWindow()
        let second = try XCTUnwrap(document.windowControllers.last as? EVDocumentWindowController)
        second.commandDidCloseWindow = {}
        ex("split", in: try surface(second))
        XCTAssertEqual(second.paneCount, 2, "Views opened after the request also retain its wait")
        ex("q", in: try surface(first))
        XCTAssertEqual(document.windowControllers.count, 1)
        try await assertWaiting(callers)
        ex("q", in: try surface(second))
        XCTAssertEqual(second.paneCount, 1)
        try await assertWaiting(callers)
        ex("q", in: try surface(second))
        try await assertCompleted(callers, status: 0)
        XCTAssertEqual(EVBlockingEditSessions.shared.count, 0)
        XCTAssertNil(EVDocumentIdentity.existingDocument(at: url))
    }

    func testSaveAndReloadKeepWaitingAndCanceledDirtyCloseDoesNotFinish() async throws {
        let f = try Fixture()
        defer { f.close() }
        let url = try f.write("COMMIT_EDITMSG")
        let caller = try f.caller(url.path)
        let document = try f.document(url)
        let window = try f.window(document)
        let originalSurface = try surface(window)
        try edit("Saved ", in: originalSurface)
        ex("w", in: originalSurface)
        try await eventually("Save did not finish") { !document.editorBackend.persistenceState.isDirty }
        XCTAssertEqual(try String(contentsOf: url, encoding: .utf8), "Saved Original message\n")
        try await assertWaiting([caller])

        let identity = document.editorBackend.persistenceState.documentID
        ex("e!", in: originalSurface)
        XCTAssertTrue(try f.document(url) === document)
        XCTAssertNotEqual(document.editorBackend.persistenceState.documentID, identity,
            "Reload replaces the core while retaining the shared native document")
        try await assertWaiting([caller])
        try edit("Unsaved ", in: try surface(window))
        var reviews = 0
        document.closeReviewDecisionHandler = { reply in reviews += 1; reply(false) }
        ex("q", in: try surface(window))
        XCTAssertTrue(window.window?.isVisible == true, "Plain quit refuses the dirty final view")
        window.window?.performClose(nil)
        XCTAssertEqual(reviews, 1)
        XCTAssertTrue(window.window?.isVisible == true)
        XCTAssertTrue(document.editorBackend.persistenceState.isDirty)
        try await assertWaiting([caller])
        ex("q!", in: try surface(window))
        try await assertCompleted([caller], status: 0)
        XCTAssertEqual(try String(contentsOf: url, encoding: .utf8), "Saved Original message\n")
    }

    func testWriteQuitCompletesOnlyRequestedDocumentWhileOtherFilesRemainOpen() async throws {
        let f = try Fixture()
        defer { f.close() }
        let commitURL = try f.write("commit.txt")
        let otherURL = try f.write("unrelated.txt", text: "Other document\n")
        let commitCaller = try f.caller(commitURL.path)
        let otherCaller = try f.caller(otherURL.path)
        let commit = try f.document(commitURL)
        let other = try f.document(otherURL)
        let commitWindow = try f.window(commit)
        let otherWindow = try f.window(other)
        try edit("Accepted ", in: try surface(commitWindow))
        ex("wq", in: try surface(commitWindow))
        try await assertCompleted([commitCaller], status: 0)
        XCTAssertEqual(try String(contentsOf: commitURL, encoding: .utf8), "Accepted Original message\n")
        XCTAssertTrue(otherWindow.window?.isVisible == true)
        XCTAssertTrue(try f.document(otherURL) === other)
        XCTAssertEqual(EVBlockingEditSessions.shared.count, 1)
        try await assertWaiting([otherCaller])
        ex("q", in: try surface(otherWindow))
        try await assertCompleted([otherCaller], status: 0)
    }

    func testOpenFailureReturnsFailureWithoutReleasingAnUnrelatedCaller() async throws {
        let f = try Fixture()
        defer { f.close() }
        let url = try f.write("existing.txt")
        let existingCaller = try f.caller(url.path)
        let invalidURL = f.directory.appendingPathComponent("directory-is-not-a-file", isDirectory: true)
        try FileManager.default.createDirectory(at: invalidURL, withIntermediateDirectories: true)
        let failedCaller = try f.caller(invalidURL.path)
        try await assertCompleted([failedCaller], status: 1)
        XCTAssertEqual(EVBlockingEditSessions.shared.count, 1)
        try await assertWaiting([existingCaller])
        XCTAssertEqual(f.documents.count, 1)
        ex("q", in: try surface(f.window(f.document(url))))
        try await assertCompleted([existingCaller], status: 0)
    }

    func testMissingFirstFileWaitsForItsNamedEmptyDocumentToClose() async throws {
        let f = try Fixture()
        defer { f.close() }
        let url = f.directory.appendingPathComponent("new-message.txt")
        let caller = try f.caller(url.path)
        let document = try f.document(url)
        XCTAssertEqual(f.documents.count, 1)
        XCTAssertEqual(try document.editorBackend.serializedSource(typeName: EVDocument.plainTextType), Data())
        XCTAssertFalse(document.editorBackend.persistenceState.isDirty)
        try await assertWaiting([caller])
        ex("q", in: try surface(f.window(document)))
        try await assertCompleted([caller], status: 0)
        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))
    }

    func testCanceledInitialRecoveryFailsCallerWithoutCreatingAWindow() async throws {
        let f = try Fixture()
        defer { f.close() }
        let url = try f.write("recoverable.txt")
        let store = try EVRecoveryStore.claim(for: url)
        defer { store.closeAndRemove(); store.drainForTesting() }
        store.write(EVRecoverySnapshot(source: Data("unsaved".utf8), format: .plainText,
            encoding: UInt32(VIEM_ENCODING_UTF8), fileFormat: UInt32(VIEM_FILE_FORMAT_UNIX),
            documentID: 98, documentRevision: 3))
        store.drainForTesting()
        let makeDocument = f.delegate.documentFactory
        var requestedActivation = false
        f.delegate.activateForBlockingEdit = { requestedActivation = true }
        f.delegate.documentFactory = {
            let document = makeDocument()
            document.recoveryDecisionHandler = { _ in
                XCTAssertTrue(requestedActivation, "Recovery must receive focus before waiting for the user")
                return .cancel
            }
            return document
        }

        let caller = try f.caller(url.path)

        try await assertCompleted([caller], status: 1)
        XCTAssertTrue(f.documents.isEmpty)
        XCTAssertEqual(EVBlockingEditSessions.shared.count, 0)
        XCTAssertEqual(try Data(contentsOf: url), Data("Original message\n".utf8))
    }

    func testCquitPublishesRequestedStatusBeforeClosingAllUnsavedDocuments() async throws {
        for (command, status): (String, Int32) in [("cq", 1), ("0cq", 0), ("7cq", 7)] {
            let f = try Fixture()
            defer { f.close() }
            let firstURL = try f.write("first.txt")
            let secondURL = try f.write("second.txt", text: "Second original\n")
            let firstCaller = try f.caller(firstURL.path)
            let secondCaller = try f.caller(secondURL.path)
            let first = try f.document(firstURL)
            let second = try f.document(secondURL)
            let firstWindow = try f.window(first)
            let secondWindow = try f.window(second)
            XCTAssertEqual(f.documents.count, NSDocumentController.shared.documents.count,
                "Cquit test must own every open document")
            try edit("Discard ", in: try surface(firstWindow))
            try edit("Discard ", in: try surface(secondWindow))
            var reviews = 0
            first.closeReviewDecisionHandler = { reply in reviews += 1; reply(false) }
            second.closeReviewDecisionHandler = { reply in reviews += 1; reply(false) }
            var pendingWhenClosing: [Int] = []
            let observer = NotificationCenter.default.addObserver(forName: NSWindow.willCloseNotification,
                object: firstWindow.window, queue: .main) { _ in
                MainActor.assumeIsolated { pendingWhenClosing.append(EVBlockingEditSessions.shared.count) }
            }
            defer { NotificationCenter.default.removeObserver(observer) }
            var terminated: Int32?
            firstWindow.terminateWithStatus = { terminated = $0 }
            ex(command, in: try surface(firstWindow))
            XCTAssertEqual(terminated, status, command)
            XCTAssertEqual(pendingWhenClosing, [0], "Completion must be published before ordinary close can publish success")
            XCTAssertEqual(reviews, 0, "Cquit abandons dirty documents without prompting or saving")
            try await assertCompleted([firstCaller, secondCaller], status: status)
            XCTAssertEqual(EVBlockingEditSessions.shared.count, 0)
            XCTAssertTrue(f.documents.isEmpty)
            XCTAssertEqual(try String(contentsOf: firstURL, encoding: .utf8), "Original message\n")
            XCTAssertEqual(try String(contentsOf: secondURL, encoding: .utf8), "Second original\n")
        }
    }
}
