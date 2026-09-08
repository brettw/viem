import AppKit
import XCTest

@testable import ViemAppShell

@MainActor
final class EVDocumentHostEffectsTests: XCTestCase {
    private final class Surface: EVEditorSurface, EVDocumentHostAttachable {
        let viewController: NSViewController = {
            let controller = NSViewController()
            controller.view = NSView(frame: NSRect(x: 0, y: 0, width: 520, height: 320))
            return controller
        }()
        var statusBarState = EVStatusBarState()
        var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
        weak var documentHostEffectHandler: (any EVDocumentHostEffectHandling)?

        func perform(menuCommand _: EVMenuCommand, sender _: Any?) {}
        func presentation(for _: EVMenuCommand) -> EVMenuItemPresentation { .disabled }
    }

    private final class Backend: EVDocumentBackend {
        var sourceDidChange: (() -> Void)?
        var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)?
        var persistenceState: EVDocumentPersistenceState
        var sourceFormat: EVSourceFormat = .plainText
        var serializedData: Data
        var snapshots: [EVDocumentSaveSnapshot] = []
        var acknowledgements: [EVDocumentSaveSnapshot] = []
        var events: [String] = []
        var rangedData: Data?
        var requestedRanges: [ClosedRange<UInt64>] = []
        var onAcknowledge: (() -> Void)?

        init(
            data: Data,
            dirty: Bool = true,
            documentID: UInt64 = 41,
            revision: UInt64 = 73
        ) {
            serializedData = data
            persistenceState = EVDocumentPersistenceState(
                isDirty: dirty,
                documentID: documentID,
                documentRevision: revision
            )
        }

        func makeEditorSurface() -> any EVEditorSurface { Surface() }
        func read(source: Data, typeName: String) throws {
            serializedData = source
            sourceFormat = EVDocument.sourceFormat(forTypeName: typeName) ?? .plainText
        }
        func serializedSource(typeName _: String) throws -> Data { serializedData }
        func nativeSaveSnapshot(typeName _: String) throws -> EVDocumentSaveSnapshot {
            events.append("snapshot")
            let snapshot = EVDocumentSaveSnapshot(
                data: serializedData,
                documentID: persistenceState.documentID,
                documentRevision: persistenceState.documentRevision
            )
            snapshots.append(snapshot)
            return snapshot
        }
        func nativeSaveSnapshot(typeName: String, hardLineRange: ClosedRange<UInt64>) throws -> EVDocumentSaveSnapshot {
            guard let data = rangedData else { throw EVDocumentHostError.preparedWriteUnavailable }
            events.append("ranged snapshot")
            requestedRanges.append(hardLineRange)
            let snapshot = EVDocumentSaveSnapshot(data: data,
                documentID: persistenceState.documentID,
                documentRevision: persistenceState.documentRevision,
                isCompleteSource: false)
            snapshots.append(snapshot)
            return snapshot
        }
        func acknowledgeNativeSave(_ snapshot: EVDocumentSaveSnapshot) throws {
            events.append("acknowledge")
            acknowledgements.append(snapshot)
            onAcknowledge?()
            persistenceState.isDirty = false
            persistenceStateDidChange?(persistenceState)
        }
    }

    func testWindowInstallsItselfAsTypedDocumentHost() throws {
        let backend = Backend(data: Data())
        let document = EVDocument(editorBackend: backend)
        let surface = Surface()
        let controller = EVDocumentWindowController(document: document, editorSurface: surface)
        defer { controller.close() }

        XCTAssertTrue((surface.documentHostEffectHandler as AnyObject?) === controller)
    }

    func testCurrentFullWriteUsesNativeSaveAndAcknowledgesExactRevision() throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("document.txt")
        try Data("old contents".utf8).write(to: url)
        let backend = Backend(data: Data("native source bytes".utf8))
        let (document, controller) = makeController(backend: backend, fileURL: url)
        defer { controller.close() }

        let completion = expectation(description: "write")
        var result: Result<String?, Error>?
        controller.perform(documentHostRequests: [request(.write, backend: backend)]) {
            result = $0
            completion.fulfill()
        }
        wait(for: [completion], timeout: 5)

        XCTAssertNoThrow(try result?.get())
        XCTAssertEqual(try Data(contentsOf: url), backend.serializedData)
        XCTAssertEqual(backend.events, ["snapshot", "acknowledge"])
        XCTAssertEqual(backend.acknowledgements, backend.snapshots)
        XCTAssertFalse(document.isDocumentEdited)
    }

    func testFailedWriteDoesNotAcknowledgeOrCloseWriteQuitWindow() throws {
        let missingDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-host-missing-\(UUID().uuidString)", isDirectory: true)
        let url = missingDirectory.appendingPathComponent("document.txt")
        let backend = Backend(data: Data("unsaved".utf8))
        let (_, controller) = makeController(backend: backend, fileURL: url)
        controller.showWindow(nil)
        defer { controller.close() }

        let completion = expectation(description: "failed wq")
        var result: Result<String?, Error>?
        controller.perform(documentHostRequests: [request(.writeQuit, backend: backend)]) {
            result = $0
            completion.fulfill()
        }
        wait(for: [completion], timeout: 5)

        XCTAssertThrowsError(try result?.get())
        XCTAssertEqual(backend.events, ["snapshot"])
        XCTAssertTrue(backend.acknowledgements.isEmpty)
        XCTAssertTrue(controller.window?.isVisible ?? false)
    }

    func testWriteQuitClosesOnlyAfterSuccessfulAcknowledgement() throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("document.txt")
        try Data("old contents".utf8).write(to: url)
        let backend = Backend(data: Data("saved".utf8))
        let (document, controller) = makeController(backend: backend, fileURL: url)
        controller.showWindow(nil)
        var visibleWhenAcknowledged = false
        backend.onAcknowledge = { visibleWhenAcknowledged = controller.window?.isVisible ?? false }

        let completion = expectation(description: "successful wq")
        var result: Result<String?, Error>?
        controller.perform(documentHostRequests: [request(.writeQuit, backend: backend)]) {
            result = $0
            completion.fulfill()
        }
        wait(for: [completion], timeout: 5)

        XCTAssertNoThrow(try result?.get())
        XCTAssertTrue(visibleWhenAcknowledged, "save acknowledgement precedes the close")
        XCTAssertFalse(controller.window?.isVisible ?? true)
        XCTAssertFalse(NSDocumentController.shared.documents.contains { $0 === document })
    }

    func testSaveAsUsesExplicitPathAndMovesNativeDocumentOnlyAfterSuccess() throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let original = directory.appendingPathComponent("original.txt")
        let destination = directory.appendingPathComponent("renamed.text")
        let backend = Backend(data: Data("source".utf8))
        let (document, controller) = makeController(backend: backend, fileURL: original)
        defer { controller.close() }
        let request = EVDocumentHostRequest(
            kind: .saveAs,
            documentID: backend.persistenceState.documentID,
            documentRevision: backend.persistenceState.documentRevision,
            force: true,
            path: destination.path
        )

        let completion = expectation(description: "saveas")
        var result: Result<String?, Error>?
        controller.perform(documentHostRequests: [request]) {
            result = $0
            completion.fulfill()
        }
        wait(for: [completion], timeout: 5)

        XCTAssertNoThrow(try result?.get())
        XCTAssertEqual(document.fileURL?.standardizedFileURL, destination.standardizedFileURL)
        XCTAssertEqual(document.fileType, EVDocument.plainTextType)
        XCTAssertEqual(try Data(contentsOf: destination), backend.serializedData)
        XCTAssertEqual(backend.events, ["snapshot", "acknowledge"])
    }

    func testSaveAsPreservesSourceFormatAndBytesRegardlessOfFilenameExtension() throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let original = directory.appendingPathComponent("original.txt")
        let destination = directory.appendingPathComponent("renamed.md")
        try Data("original source".utf8).write(to: original)
        let backend = Backend(data: Data("changed source".utf8))
        let (document, controller) = makeController(backend: backend, fileURL: original)
        defer { controller.close() }
        let request = EVDocumentHostRequest(
            kind: .saveAs,
            documentID: backend.persistenceState.documentID,
            documentRevision: backend.persistenceState.documentRevision,
            force: true,
            path: destination.path
        )

        let completion = expectation(description: "source-preserving saveas")
        var result: Result<String?, Error>?
        controller.perform(documentHostRequests: [request]) {
            result = $0
            completion.fulfill()
        }
        wait(for: [completion], timeout: 2)

        XCTAssertNoThrow(try result?.get())
        XCTAssertEqual(document.fileURL?.standardizedFileURL, destination.standardizedFileURL)
        XCTAssertEqual(document.fileType, EVDocument.plainTextType)
        XCTAssertEqual(backend.sourceFormat, .plainText)
        XCTAssertEqual(try Data(contentsOf: original), Data("original source".utf8))
        XCTAssertEqual(try Data(contentsOf: destination), backend.serializedData)
        XCTAssertEqual(backend.events, ["snapshot", "acknowledge"])
        XCTAssertEqual(backend.acknowledgements, backend.snapshots)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(document.isDocumentEdited)
    }

    func testXitSkipsWriteWhenCleanAndSavesBeforeClosingWhenDirty() throws {
        do {
            let backend = Backend(data: Data("clean".utf8), dirty: false)
            let (_, controller) = makeController(backend: backend)
            controller.showWindow(nil)

            let completion = expectation(description: "clean xit")
            controller.perform(documentHostRequests: [request(.xit, backend: backend)]) { result in
                if case let .failure(error) = result { XCTFail("unexpected xit failure: \(error)") }
                completion.fulfill()
            }
            wait(for: [completion], timeout: 1)
            XCTAssertTrue(backend.events.isEmpty)
            XCTAssertFalse(controller.window?.isVisible ?? true)
        }

        do {
            let directory = temporaryDirectory()
            defer { try? FileManager.default.removeItem(at: directory) }
            let url = directory.appendingPathComponent("dirty.txt")
            try Data("old contents".utf8).write(to: url)
            let backend = Backend(data: Data("dirty".utf8))
            let (_, controller) = makeController(backend: backend, fileURL: url)
            controller.showWindow(nil)

            let completion = expectation(description: "dirty xit")
            controller.perform(documentHostRequests: [request(.xit, backend: backend)]) { result in
                if case let .failure(error) = result { XCTFail("unexpected xit failure: \(error)") }
                completion.fulfill()
            }
            wait(for: [completion], timeout: 5)
            XCTAssertEqual(backend.events, ["snapshot", "acknowledge"])
            XCTAssertEqual(try Data(contentsOf: url), backend.serializedData)
            XCTAssertFalse(controller.window?.isVisible ?? true)
        }
    }

    func testDirtyEditNewAndQuitAllRespectForceGuardBeforeNativeMutation() throws {
        let backend = Backend(data: Data("dirty".utf8))
        let (_, controller) = makeController(backend: backend)
        defer { controller.close() }

        for request in [
            EVDocumentHostRequest(
                kind: .edit,
                documentID: backend.persistenceState.documentID,
                documentRevision: backend.persistenceState.documentRevision,
                path: "/tmp/other.txt"
            ),
            self.request(.new, backend: backend),
            self.request(.quitAll, backend: backend),
        ] {
            assertFailure(.documentModified, from: controller, request: request)
        }
        XCTAssertTrue(backend.events.isEmpty)
    }

    func testWriteAllIncludesCurrentUnregisteredDocumentAndUsesExactSave() throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("all.txt")
        try Data("old contents".utf8).write(to: url)
        let backend = Backend(data: Data("all".utf8))
        let (_, controller) = makeController(backend: backend, fileURL: url)
        defer { controller.close() }

        let completion = expectation(description: "wall")
        var result: Result<String?, Error>?
        controller.perform(documentHostRequests: [request(.writeAll, backend: backend)]) {
            result = $0
            completion.fulfill()
        }
        wait(for: [completion], timeout: 5)

        XCTAssertNoThrow(try result?.get())
        XCTAssertEqual(backend.events, ["snapshot", "acknowledge"])
        XCTAssertEqual(try Data(contentsOf: url), backend.serializedData)
    }

    func testDirtyQuitIsRejectedButForcedQuitClosesWithoutSaving() throws {
        let backend = Backend(data: Data("dirty".utf8))
        let (_, controller) = makeController(backend: backend)
        controller.showWindow(nil)

        let rejected = expectation(description: "dirty quit rejected")
        var rejectedResult: Result<String?, Error>?
        controller.perform(documentHostRequests: [request(.quit, backend: backend)]) { result in
            rejectedResult = result
            rejected.fulfill()
        }
        wait(for: [rejected], timeout: 1)
        XCTAssertThrowsError(try rejectedResult?.get()) { error in
            XCTAssertEqual(error as? EVDocumentHostError, .documentModified)
        }
        XCTAssertTrue(controller.window?.isVisible ?? false)
        XCTAssertTrue(backend.events.isEmpty)

        let forced = expectation(description: "forced quit")
        var forcedRequest = request(.quit, backend: backend)
        forcedRequest = EVDocumentHostRequest(
            kind: forcedRequest.kind,
            documentID: forcedRequest.documentID,
            documentRevision: forcedRequest.documentRevision,
            force: true
        )
        var forcedResult: Result<String?, Error>?
        controller.perform(documentHostRequests: [forcedRequest]) { result in
            forcedResult = result
            forced.fulfill()
        }
        wait(for: [forced], timeout: 1)
        XCTAssertNoThrow(try forcedResult?.get())
        XCTAssertFalse(controller.window?.isVisible ?? true)
        XCTAssertTrue(backend.events.isEmpty)
    }

    func testStaleWritesFailAndAlternateAndRangedWritesPreserveBindingAndSavePoint() throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let original = directory.appendingPathComponent("original.txt")
        let originalBytes = Data("original source".utf8)
        try originalBytes.write(to: original)
        let backend = Backend(data: Data("first\r\nsecond\r\nthird".utf8))
        backend.rangedData = Data("second\r\n".utf8)
        let (document, controller) = makeController(backend: backend, fileURL: original)
        defer { controller.close() }

        let stale = EVDocumentHostRequest(
            kind: .write,
            documentID: backend.persistenceState.documentID,
            documentRevision: backend.persistenceState.documentRevision - 1
        )
        assertFailure(.staleRequest, from: controller, request: stale)
        XCTAssertTrue(backend.events.isEmpty)

        let alternateURL = directory.appendingPathComponent("alternate.txt")
        let alternate = EVDocumentHostRequest(
            kind: .write,
            documentID: backend.persistenceState.documentID,
            documentRevision: backend.persistenceState.documentRevision,
            path: alternateURL.path
        )
        let alternateCompletion = expectation(description: "alternate write completed")
        var alternateResult: Result<String?, Error>?
        controller.perform(documentHostRequests: [alternate]) {
            alternateResult = $0
            alternateCompletion.fulfill()
        }
        wait(for: [alternateCompletion], timeout: 5)
        XCTAssertNoThrow(try alternateResult?.get())
        XCTAssertEqual(try Data(contentsOf: alternateURL), backend.serializedData)
        XCTAssertEqual(backend.events, ["snapshot"])

        let rangedURL = directory.appendingPathComponent("range.txt")
        let ranged = EVDocumentHostRequest(
            kind: .write,
            documentID: backend.persistenceState.documentID,
            documentRevision: backend.persistenceState.documentRevision,
            path: rangedURL.path,
            hardLineRange: 1 ... 1
        )
        let rangedCompletion = expectation(description: "ranged write completed")
        var rangedResult: Result<String?, Error>?
        controller.perform(documentHostRequests: [ranged]) {
            rangedResult = $0
            rangedCompletion.fulfill()
        }
        wait(for: [rangedCompletion], timeout: 5)
        XCTAssertNoThrow(try rangedResult?.get())
        XCTAssertEqual(try Data(contentsOf: rangedURL), backend.rangedData)
        XCTAssertEqual(backend.requestedRanges, [1 ... 1])
        XCTAssertEqual(backend.events, ["snapshot", "ranged snapshot"])
        XCTAssertEqual(try Data(contentsOf: original), originalBytes)
        XCTAssertEqual(document.fileURL?.standardizedFileURL, original.standardizedFileURL)
        XCTAssertTrue(backend.acknowledgements.isEmpty)
        XCTAssertTrue(backend.persistenceState.isDirty)
        XCTAssertTrue(document.isDocumentEdited)
    }

    private func makeController(
        backend: Backend,
        fileURL: URL? = nil
    ) -> (EVDocument, EVDocumentWindowController) {
        let document = EVDocument(editorBackend: backend)
        document.fileURL = fileURL
        document.fileType = EVDocument.plainTextType
        let controller = EVDocumentWindowController(document: document, editorSurface: Surface())
        document.addWindowController(controller)
        return (document, controller)
    }

    private func request(
        _ kind: EVDocumentHostRequest.Kind,
        backend: Backend
    ) -> EVDocumentHostRequest {
        EVDocumentHostRequest(
            kind: kind,
            documentID: backend.persistenceState.documentID,
            documentRevision: backend.persistenceState.documentRevision
        )
    }

    private func assertFailure(
        _ expectedError: EVDocumentHostError,
        from controller: EVDocumentWindowController,
        request: EVDocumentHostRequest,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        var captured: Error?
        controller.perform(documentHostRequests: [request]) { result in
            if case let .failure(error) = result { captured = error }
        }
        XCTAssertEqual(captured as? EVDocumentHostError, expectedError, file: file, line: line)
    }

    private func temporaryDirectory() -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-host-effects-\(UUID().uuidString)", isDirectory: true)
        try! FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }
}
