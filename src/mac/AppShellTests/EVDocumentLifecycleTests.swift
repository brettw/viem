import AppKit
import XCTest

@testable import EvimAppShell

@MainActor
final class EVDocumentLifecycleTests: XCTestCase {
    private final class Surface: EVEditorSurface {
        let viewController: NSViewController = {
            let controller = NSViewController()
            controller.view = NSView(frame: NSRect(x: 0, y: 0, width: 920, height: 655))
            return controller
        }()
        var statusBarState = EVStatusBarState()
        var statusBarStateDidChange: ((EVStatusBarState) -> Void)?

        func perform(menuCommand _: EVMenuCommand, sender _: Any?) {}

        func presentation(for _: EVMenuCommand) -> EVMenuItemPresentation {
            .disabled
        }
    }

    private final class Backend: EVDocumentBackend {
        enum Failure: Error {
            case snapshot
            case acknowledgement
        }

        var sourceDidChange: (() -> Void)?
        var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)?
        var persistenceState = EVDocumentPersistenceState()
        var sourceFormat: EVSourceFormat = .plainText
        var readCalls: [(Data, String)] = []
        var serializationTypes: [String] = []
        var serializedData = Data()
        var saveSnapshots: [EVDocumentSaveSnapshot] = []
        var acknowledgedSnapshots: [EVDocumentSaveSnapshot] = []
        var lifecycleEvents: [String] = []
        var snapshotError: Error?
        var acknowledgementError: Error?
        var onAcknowledge: (() -> Void)?
        var surfaceCount = 0

        func makeEditorSurface() -> any EVEditorSurface {
            surfaceCount += 1
            return Surface()
        }

        func read(source: Data, typeName: String) throws {
            readCalls.append((source, typeName))
            sourceFormat = EVDocument.sourceFormat(forTypeName: typeName) ?? .plainText
        }

        func serializedSource(typeName: String) throws -> Data {
            serializationTypes.append(typeName)
            return serializedData
        }

        func nativeSaveSnapshot(typeName: String) throws -> EVDocumentSaveSnapshot {
            lifecycleEvents.append("snapshot")
            if let snapshotError { throw snapshotError }
            serializationTypes.append(typeName)
            let snapshot = EVDocumentSaveSnapshot(
                data: serializedData,
                documentID: 41,
                documentRevision: 73
            )
            saveSnapshots.append(snapshot)
            return snapshot
        }

        func acknowledgeNativeSave(_ snapshot: EVDocumentSaveSnapshot) throws {
            lifecycleEvents.append("acknowledge")
            acknowledgedSnapshots.append(snapshot)
            onAcknowledge?()
            if let acknowledgementError { throw acknowledgementError }
            persistenceState = EVDocumentPersistenceState(isDirty: false)
            persistenceStateDidChange?(persistenceState)
        }

        func publishPersistence(_ state: EVDocumentPersistenceState) {
            persistenceState = state
            persistenceStateDidChange?(state)
        }
    }

    func testNativeSavePanelHasCurrentWritableType() throws {
        let backend = Backend()
        let document = EVDocument(editorBackend: backend)
        XCTAssertEqual(document.fileType, EVDocument.plainTextType)
        XCTAssertTrue(EVDocument.isNativeType(EVDocument.plainTextType))
        XCTAssertEqual(document.writableTypes(for: .saveAsOperation), [EVDocument.plainTextType])
        try document.read(from: Data("# text".utf8), ofType: EVDocument.markdownType)
        XCTAssertEqual(document.writableTypes(for: .saveAsOperation), [EVDocument.markdownType])
    }

    func testNSDocumentDataOverridesDelegateReadAndSameFormatSerializationToSourceBackend() throws {
        let backend = Backend()
        backend.serializedData = Data("serialized source".utf8)
        let document = EVDocument(editorBackend: backend)

        let input = Data("input source".utf8)
        try document.read(from: input, ofType: EVDocument.markdownType)
        let output = try document.data(ofType: EVDocument.markdownType)

        XCTAssertEqual(backend.readCalls.count, 1)
        XCTAssertEqual(backend.readCalls[0].0, input)
        XCTAssertEqual(backend.readCalls[0].1, EVDocument.markdownType)
        XCTAssertEqual(backend.serializationTypes, [EVDocument.markdownType])
        XCTAssertEqual(output, backend.serializedData)
        XCTAssertFalse(document.hasUndoManager)
        XCTAssertTrue(backend.saveSnapshots.isEmpty)
        XCTAssertTrue(backend.acknowledgedSnapshots.isEmpty)
    }

    func testNSDocumentDataRejectsUnsafeTypesBeforeBackendSerialization() throws {
        let backend = Backend()
        backend.serializedData = Data("# serialized source".utf8)
        let document = EVDocument(editorBackend: backend)

        try document.read(
            from: Data("# input source".utf8),
            ofType: EVDocument.markdownType
        )

        XCTAssertThrowsError(try document.data(ofType: EVDocument.plainTextType)) { error in
            XCTAssertEqual(
                error as? EVDocumentSerializationError,
                .formatConversionUnavailable(current: .markdown, requested: .plainText)
            )
        }
        XCTAssertThrowsError(try document.data(ofType: "com.example.unsupported")) { error in
            XCTAssertEqual(
                error as? EVDocumentSerializationError,
                .unsupportedWritableType("com.example.unsupported")
            )
        }
        XCTAssertTrue(backend.serializationTypes.isEmpty)
        XCTAssertTrue(backend.saveSnapshots.isEmpty)
    }

    func testNativeURLReadAndWriteFlowThroughNSDocumentDataBoundary() throws {
        let backend = Backend()
        backend.serializedData = Data("saved by backend".utf8)
        let document = EVDocument(editorBackend: backend)
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("evim-document-tests-\(UUID().uuidString)", isDirectory: true)
        let inputURL = directory.appendingPathComponent("input.md")
        let outputURL = directory.appendingPathComponent("output.md")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let input = Data("# Native open\n".utf8)
        try input.write(to: inputURL)

        try document.read(from: inputURL, ofType: EVDocument.markdownType)
        try document.write(to: outputURL, ofType: EVDocument.markdownType)

        XCTAssertEqual(backend.readCalls.count, 1)
        XCTAssertEqual(backend.readCalls[0].0, input)
        XCTAssertEqual(backend.readCalls[0].1, EVDocument.markdownType)
        XCTAssertEqual(backend.serializationTypes, [EVDocument.markdownType])
        XCTAssertEqual(try Data(contentsOf: outputURL), backend.serializedData)
    }

    func testBackendChangeNotificationMarksNativeDocumentEdited() {
        let backend = Backend()
        let document = EVDocument(editorBackend: backend)
        document.updateChangeCount(.changeCleared)
        XCTAssertFalse(document.isDocumentEdited)

        backend.sourceDidChange?()

        XCTAssertTrue(document.isDocumentEdited)
    }

    func testCorePersistenceStateIsTheFinalNativeEditedAuthority() {
        let backend = Backend()
        let document = EVDocument(editorBackend: backend)
        XCTAssertFalse(document.isDocumentEdited)

        backend.sourceDidChange?()
        XCTAssertTrue(document.isDocumentEdited, "source publication creates AppKit's change token")

        backend.publishPersistence(.init(isDirty: false))
        XCTAssertFalse(document.isDocumentEdited, "undoing to the core save point clears that token")

        backend.publishPersistence(.init(isDirty: true))
        XCTAssertTrue(document.isDocumentEdited, "core dirty state can restore native edited chrome")

        backend.publishPersistence(.init(isDirty: false))
        XCTAssertFalse(document.isDocumentEdited)
    }

    func testPrintStaysDisabledUntilThereIsAPaginatedProjection() {
        let document = EVDocument(editorBackend: Backend())
        let printItem = NSMenuItem(
            title: "Print…",
            action: #selector(NSDocument.printDocument(_:)),
            keyEquivalent: "p"
        )

        XCTAssertFalse(document.validateUserInterfaceItem(printItem))
    }

    func testOnlyAPristineUntitledLaunchPlaceholderMayBeDiscardedForAnOpenedFile() {
        let cleanBackend = Backend()
        let clean = EVDocument(editorBackend: cleanBackend)
        XCTAssertTrue(EVApplicationDelegate.isDiscardableLaunchPlaceholder(clean))

        clean.fileURL = URL(fileURLWithPath: "/tmp/named.txt")
        XCTAssertFalse(EVApplicationDelegate.isDiscardableLaunchPlaceholder(clean))

        let nativeDirty = EVDocument(editorBackend: Backend())
        nativeDirty.updateChangeCount(.changeDone)
        XCTAssertFalse(EVApplicationDelegate.isDiscardableLaunchPlaceholder(nativeDirty))

        let coreDirtyBackend = Backend()
        coreDirtyBackend.persistenceState = .init(isDirty: true)
        let coreDirty = EVDocument(editorBackend: coreDirtyBackend)
        XCTAssertFalse(EVApplicationDelegate.isDiscardableLaunchPlaceholder(coreDirty))
    }

    func testOpenRecentReplacesThePristineLaunchPlaceholder() throws {
        let documentController = NSDocumentController.shared
        XCTAssertTrue(
            documentController.documents.isEmpty,
            "the document lifecycle test must begin without process-global documents"
        )
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("evim-open-recent-tests-\(UUID().uuidString)", isDirectory: true)
        let inputURL = directory.appendingPathComponent("opened.txt")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data().write(to: inputURL)
        defer {
            for document in documentController.documents { document.close() }
            try? FileManager.default.removeItem(at: directory)
        }

        let delegate = EVApplicationDelegate()
        delegate.applicationDidFinishLaunching(
            Notification(name: NSApplication.didFinishLaunchingNotification)
        )
        let placeholder = try XCTUnwrap(documentController.documents.first as? EVDocument)
        XCTAssertNil(placeholder.fileURL)

        let item = NSMenuItem()
        item.representedObject = inputURL
        delegate.openRecentDocument(item)

        XCTAssertNotNil(documentController.document(for: inputURL))
        XCTAssertFalse(documentController.documents.contains { $0 === placeholder })
        XCTAssertEqual(documentController.documents.count, 1)
    }

    func testNativeSaveAcknowledgesExactSnapshotOnlyAfterSuccessfulWrite() throws {
        let backend = Backend()
        backend.serializedData = Data("exact saved snapshot".utf8)
        backend.persistenceState = .init(isDirty: true)
        let document = EVDocument(editorBackend: backend)
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("evim-save-tests-\(UUID().uuidString)", isDirectory: true)
        let outputURL = directory.appendingPathComponent("saved.txt")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        var dataSeenAtAcknowledgement: Data?
        backend.onAcknowledge = { dataSeenAtAcknowledgement = try? Data(contentsOf: outputURL) }

        let completion = expectation(description: "native save")
        var completionError: Error?
        document.save(
            to: outputURL,
            ofType: EVDocument.plainTextType,
            for: .saveAsOperation
        ) { error in
            completionError = error
            completion.fulfill()
        }
        wait(for: [completion], timeout: 5)

        XCTAssertNil(completionError)
        XCTAssertEqual(backend.lifecycleEvents, ["snapshot", "acknowledge"])
        XCTAssertEqual(backend.saveSnapshots.count, 1)
        XCTAssertEqual(backend.acknowledgedSnapshots, backend.saveSnapshots)
        XCTAssertEqual(dataSeenAtAcknowledgement, backend.serializedData)
        XCTAssertEqual(try Data(contentsOf: outputURL), backend.serializedData)
        XCTAssertFalse(document.isDocumentEdited)
    }

    func testNativeSaveAsRejectsCrossFormatBeforeSnapshotOrFileMutation() throws {
        let backend = Backend()
        backend.serializedData = Data("# changed source".utf8)
        backend.persistenceState = .init(isDirty: true)
        let document = EVDocument(editorBackend: backend)
        try document.read(
            from: Data("# original source".utf8),
            ofType: EVDocument.markdownType
        )

        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("evim-cross-format-tests-\(UUID().uuidString)", isDirectory: true)
        let originalURL = directory.appendingPathComponent("original.md")
        let destinationURL = directory.appendingPathComponent("converted.txt")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data("# original source".utf8).write(to: originalURL)
        defer { try? FileManager.default.removeItem(at: directory) }
        document.fileURL = originalURL
        document.fileType = EVDocument.markdownType

        let completion = expectation(description: "cross-format save rejected")
        var completionError: Error?
        document.save(
            to: destinationURL,
            ofType: EVDocument.plainTextType,
            for: .saveAsOperation
        ) { error in
            completionError = error
            completion.fulfill()
        }
        wait(for: [completion], timeout: 2)

        XCTAssertEqual(
            completionError as? EVDocumentSerializationError,
            .formatConversionUnavailable(current: .markdown, requested: .plainText)
        )
        XCTAssertTrue(backend.lifecycleEvents.isEmpty)
        XCTAssertTrue(backend.saveSnapshots.isEmpty)
        XCTAssertTrue(backend.acknowledgedSnapshots.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: destinationURL.path))
        XCTAssertEqual(document.fileURL?.standardizedFileURL, originalURL.standardizedFileURL)
        XCTAssertEqual(document.fileType, EVDocument.markdownType)
        XCTAssertEqual(try Data(contentsOf: originalURL), Data("# original source".utf8))
        XCTAssertTrue(document.isDocumentEdited)
    }

    func testFailedNativeWriteDoesNotAcknowledgeOrClearDirtyState() throws {
        let backend = Backend()
        backend.serializedData = Data("unsaved".utf8)
        backend.persistenceState = .init(isDirty: true)
        let document = EVDocument(editorBackend: backend)
        let missingParent = FileManager.default.temporaryDirectory
            .appendingPathComponent("evim-missing-\(UUID().uuidString)", isDirectory: true)
        let outputURL = missingParent.appendingPathComponent("saved.txt")

        let completion = expectation(description: "failed native save")
        var completionError: Error?
        document.save(
            to: outputURL,
            ofType: EVDocument.plainTextType,
            for: .saveAsOperation
        ) { error in
            completionError = error
            completion.fulfill()
        }
        wait(for: [completion], timeout: 5)

        XCTAssertNotNil(completionError)
        XCTAssertEqual(backend.lifecycleEvents, ["snapshot"])
        XCTAssertTrue(backend.acknowledgedSnapshots.isEmpty)
        XCTAssertTrue(document.isDocumentEdited)
    }

    func testSaveToWritesCopyWithoutMovingTheCoreSavePoint() throws {
        let backend = Backend()
        backend.serializedData = Data("copy only".utf8)
        backend.persistenceState = .init(isDirty: true)
        let document = EVDocument(editorBackend: backend)
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("evim-save-to-tests-\(UUID().uuidString)", isDirectory: true)
        let outputURL = directory.appendingPathComponent("copy.txt")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }

        let completion = expectation(description: "native Save To")
        var completionError: Error?
        document.save(
            to: outputURL,
            ofType: EVDocument.plainTextType,
            for: .saveToOperation
        ) { error in
            completionError = error
            completion.fulfill()
        }
        wait(for: [completion], timeout: 5)

        XCTAssertNil(completionError)
        XCTAssertEqual(try Data(contentsOf: outputURL), backend.serializedData)
        XCTAssertEqual(backend.lifecycleEvents, ["snapshot"])
        XCTAssertTrue(backend.acknowledgedSnapshots.isEmpty)
        XCTAssertTrue(document.isDocumentEdited)
    }

    func testSnapshotPreparationFailureDoesNotStartNativeWriteOrAcknowledge() {
        let backend = Backend()
        backend.snapshotError = Backend.Failure.snapshot
        backend.persistenceState = .init(isDirty: true)
        let document = EVDocument(editorBackend: backend)
        let outputURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("evim-unwritten-\(UUID().uuidString).txt")

        let completion = expectation(description: "rejected native save")
        var completionError: Error?
        document.save(
            to: outputURL,
            ofType: EVDocument.plainTextType,
            for: .saveAsOperation
        ) { error in
            completionError = error
            completion.fulfill()
        }
        wait(for: [completion], timeout: 2)

        XCTAssertNotNil(completionError)
        XCTAssertEqual(backend.lifecycleEvents, ["snapshot"])
        XCTAssertTrue(backend.saveSnapshots.isEmpty)
        XCTAssertTrue(backend.acknowledgedSnapshots.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: outputURL.path))
        XCTAssertTrue(document.isDocumentEdited)
    }

    func testMakeWindowControllersCreatesOneSurfacePerNativeDocumentWindow() {
        let backend = Backend()
        let document = EVDocument(editorBackend: backend)
        defer { document.close() }

        document.makeWindowControllers()
        XCTAssertEqual(document.windowControllers.count, 1)
        XCTAssertEqual(backend.surfaceCount, 1)

        document.showAdditionalWindow()
        XCTAssertEqual(document.windowControllers.count, 2)
        XCTAssertEqual(backend.surfaceCount, 2)
        XCTAssertFalse(document.windowControllers[0] === document.windowControllers[1])
        XCTAssertTrue(document.windowControllers[1].window?.isVisible ?? false)
    }
}
