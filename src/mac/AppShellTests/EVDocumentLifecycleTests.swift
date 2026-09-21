import AppKit
import UniformTypeIdentifiers
import XCTest

@testable import ViemAppShell

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
        var persistenceState = EVDocumentPersistenceState(sourceByteCount: 0)
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

    private func preservationFixture(name: String = "ffi.rs") throws -> (URL, Backend, EVDocument) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-preserve-original-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let original = directory.appendingPathComponent(name)
        try Data("original bytes".utf8).write(to: original)
        let backend = Backend()
        let document = EVDocument(editorBackend: backend)
        document.recordRecentDocument = { _ in }
        try document.read(from: original, ofType: EVDocument.plainTextType)
        document.fileURL = original
        backend.serializedData = Data("edited bytes".utf8)
        backend.publishPersistence(.init(isDirty: true))
        return (original, backend, document)
    }

    func testOrdinarySaveKeepsCodeDotfileAndExtensionlessNames() throws {
        for name in ["ffi.rs", ".vimrc", "LICENSE"] {
            let (original, backend, document) = try preservationFixture(name: name)
            defer { document.close() }
            backend.sourceFormat = .code
            backend.publishPersistence(.init(isDirty: true))
            XCTAssertFalse(document.requiresNewFormatDestination)
            XCTAssertEqual(document.fileNameExtension(forType: EVDocument.plainTextType, saveOperation: .saveOperation),
                           original.pathExtension.isEmpty ? nil : original.pathExtension)
            let completion = expectation(description: "ordinary save keeps \(name)")
            document.save(to: original, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
                XCTAssertNil(error); completion.fulfill()
            }
            wait(for: [completion], timeout: 5)
            XCTAssertEqual(document.fileURL, original)
            XCTAssertEqual(try Data(contentsOf: original), backend.serializedData)
            XCTAssertFalse(FileManager.default.fileExists(atPath: original.deletingPathExtension().appendingPathExtension("txt").path))
        }
    }

    func testOrdinarySaveToAnotherURLPreservesOriginalAndAdoptsNewDestination() throws {
        let (original, backend, document) = try preservationFixture()
        defer { document.close() }
        let destination = original.deletingPathExtension().appendingPathExtension("txt")
        let completion = expectation(description: "changed save destination")
        document.save(to: destination, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
            XCTAssertNil(error); completion.fulfill()
        }
        wait(for: [completion], timeout: 5)
        XCTAssertEqual(try Data(contentsOf: original), Data("original bytes".utf8))
        XCTAssertEqual(try Data(contentsOf: destination), backend.serializedData)
        XCTAssertEqual(document.fileURL, destination)
    }

    func testDirectSafeWriteToAnotherURLPreservesOriginalAndDoesNotAdoptCopy() throws {
        let (original, backend, document) = try preservationFixture()
        defer { document.close() }
        let destination = original.deletingPathExtension().appendingPathExtension("txt")
        try document.writeSafely(to: destination, ofType: EVDocument.plainTextType, for: .saveOperation)
        XCTAssertEqual(try Data(contentsOf: original), Data("original bytes".utf8))
        XCTAssertEqual(try Data(contentsOf: destination), backend.serializedData)
        XCTAssertEqual(document.fileURL, original)
    }

    func testOrdinarySaveToAnotherHardLinkPreservesOriginalBytesAndName() throws {
        let (original, backend, document) = try preservationFixture()
        defer { document.close() }
        let destination = original.deletingLastPathComponent().appendingPathComponent("linked.rs")
        try FileManager.default.linkItem(at: original, to: destination)
        XCTAssertTrue(EVDocumentIdentity.sameFile(original, destination))
        let completion = expectation(description: "save to another hard link")
        document.save(to: destination, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
            XCTAssertNil(error); completion.fulfill()
        }
        wait(for: [completion], timeout: 5)
        XCTAssertEqual(try Data(contentsOf: original), Data("original bytes".utf8))
        XCTAssertEqual(try Data(contentsOf: destination), backend.serializedData)
        XCTAssertEqual(document.fileURL, destination)
        XCTAssertFalse(EVDocumentIdentity.sameFile(original, destination))
    }

    func testSaveRejectsDifferentSymlinkSpellingsAndPreservesOriginalBytes() throws {
        let (original, _, document) = try preservationFixture()
        defer { document.close() }
        let alias = original.deletingLastPathComponent().appendingPathComponent("alias.rs")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: original)
        for (bound, destination) in [(original, alias), (alias, original)] {
            document.fileURL = bound
            let completion = expectation(description: "save rejects alias pathname")
            document.save(to: destination, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
                XCTAssertEqual(error as? EVDocumentSerializationError, .destinationAliasesOriginal)
                completion.fulfill()
            }
            wait(for: [completion], timeout: 5)
            XCTAssertEqual(document.fileURL, bound)
            XCTAssertEqual(try Data(contentsOf: original), Data("original bytes".utf8))
            XCTAssertEqual(try Data(contentsOf: alias), Data("original bytes".utf8))
            XCTAssertEqual(try FileManager.default.destinationOfSymbolicLink(atPath: alias.path), original.path)
        }
    }

    func testOrdinarySaveToUnchangedSymlinkNameRemainsSupported() throws {
        let (original, backend, document) = try preservationFixture()
        defer { document.close() }
        let alias = original.deletingLastPathComponent().appendingPathComponent("alias.rs")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: original)
        document.fileURL = alias
        let completion = expectation(description: "ordinary save to bound symlink")
        document.save(to: alias, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
            XCTAssertNil(error); completion.fulfill()
        }
        wait(for: [completion], timeout: 5)
        XCTAssertEqual(try Data(contentsOf: alias), backend.serializedData)
        XCTAssertTrue(FileManager.default.fileExists(atPath: original.path))
        XCTAssertEqual(document.fileURL, alias)
    }

    func testSaveRejectsCaseOnlyAliasOnCaseInsensitiveVolume() throws {
        let (original, _, document) = try preservationFixture()
        defer { document.close() }
        let alias = original.deletingLastPathComponent().appendingPathComponent("FFI.RS")
        guard EVDocumentIdentity.sameFile(original, alias) else { throw XCTSkip("The fixture volume is case-sensitive") }
        let completion = expectation(description: "save rejects case-only alias")
        document.save(to: alias, ofType: EVDocument.plainTextType, for: .saveOperation) { error in
            XCTAssertEqual(error as? EVDocumentSerializationError, .destinationAliasesOriginal)
            completion.fulfill()
        }
        wait(for: [completion], timeout: 5)
        XCTAssertEqual(try Data(contentsOf: original), Data("original bytes".utf8))
        XCTAssertEqual(document.fileURL, original)
    }

    func testChangedSerializationRejectsOriginalAndSaveAsEstablishesNewBaseline() throws {
        let (original, backend, document) = try preservationFixture()
        defer { document.close() }
        backend.sourceFormat = .html
        backend.serializedData = Data("<p>converted bytes</p>".utf8)
        backend.publishPersistence(.init(isDirty: true))
        XCTAssertTrue(document.requiresNewFormatDestination)
        XCTAssertEqual(try Data(contentsOf: original), Data("original bytes".utf8), "Format selection does not write")
        for operation in [NSDocument.SaveOperationType.saveOperation, .saveAsOperation, .saveToOperation] {
            let completion = expectation(description: "format write rejects original")
            document.save(to: original, ofType: EVDocument.htmlType, for: operation) { error in
                XCTAssertEqual(error as? EVDocumentSerializationError, .changedFormatNeedsNewDestination)
                completion.fulfill()
            }
            wait(for: [completion], timeout: 5)
        }
        XCTAssertThrowsError(try document.writeSafely(to: original, ofType: EVDocument.htmlType, for: .saveOperation))
        XCTAssertThrowsError(try document.write(to: original, ofType: EVDocument.htmlType))
        var hostError: Error?
        document.saveHostRevision(documentID: 41, documentRevision: 73, force: true,
                                 to: original, ofType: EVDocument.htmlType, for: .saveOperation) { hostError = $0 }
        XCTAssertEqual(hostError as? EVDocumentSerializationError, .changedFormatNeedsNewDestination)
        XCTAssertEqual(document.fileURL, original)
        XCTAssertEqual(try Data(contentsOf: original), Data("original bytes".utf8))

        let destination = original.deletingPathExtension().appendingPathExtension("html")
        let completion = expectation(description: "converted Save As")
        document.save(to: destination, ofType: EVDocument.htmlType, for: .saveAsOperation) { error in
            XCTAssertNil(error); completion.fulfill()
        }
        wait(for: [completion], timeout: 5)
        XCTAssertEqual(try Data(contentsOf: original), Data("original bytes".utf8))
        XCTAssertEqual(try Data(contentsOf: destination), backend.serializedData)
        XCTAssertEqual(document.fileURL, destination)
        XCTAssertFalse(document.requiresNewFormatDestination)
        XCTAssertNoThrow(try document.validatePreservedOriginal(at: destination))
    }

    func testNativeMoveWritesCopyAndFailedMovePreservesBothOriginalAndBinding() throws {
        let (original, backend, document) = try preservationFixture()
        defer { document.close() }
        let destination = original.deletingLastPathComponent().appendingPathComponent("saved-copy.rs")
        let completion = expectation(description: "non-destructive native move")
        document.move(to: destination) { error in
            XCTAssertNil(error); completion.fulfill()
        }
        wait(for: [completion], timeout: 5)
        XCTAssertEqual(try Data(contentsOf: original), Data("original bytes".utf8))
        XCTAssertEqual(try Data(contentsOf: destination), backend.serializedData)
        XCTAssertEqual(document.fileURL, destination)

        backend.serializedData = Data("new unsaved bytes".utf8)
        backend.publishPersistence(.init(isDirty: true))
        let failure = expectation(description: "failed non-destructive native move")
        let unavailable = destination.deletingLastPathComponent().appendingPathComponent("absent/other.rs")
        document.move(to: unavailable) { error in
            XCTAssertNotNil(error); failure.fulfill()
        }
        wait(for: [failure], timeout: 5)
        XCTAssertEqual(try Data(contentsOf: original), Data("original bytes".utf8))
        XCTAssertEqual(try Data(contentsOf: destination), Data("edited bytes".utf8))
        XCTAssertEqual(document.fileURL, destination)
        XCTAssertTrue(document.isDocumentEdited)
        XCTAssertFalse(FileManager.default.fileExists(atPath: unavailable.path))
    }

    func testSameSerializationPresentationChangesKeepOrdinarySave() throws {
        let (original, backend, document) = try preservationFixture()
        defer { document.close() }
        for (from, to) in [(EVSourceFormat.plainText, EVSourceFormat.code), (.markdown, .markdownSource), (.html, .htmlSource)] {
            backend.sourceFormat = from
            document.recordFileBaseline(Data("original bytes".utf8), at: original)
            backend.sourceFormat = to
            backend.publishPersistence(.init(isDirty: true))
            XCTAssertFalse(document.requiresNewFormatDestination)
            XCTAssertNoThrow(try document.validatePreservedOriginal(at: original))
        }
    }

    func testNativeDocumentRegistrationAcceptsUnknownFilesAsReadableData() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
        let plistURL = root.appendingPathComponent("App/Resources/Info.plist")
        let plist = try XCTUnwrap(PropertyListSerialization.propertyList(
            from: Data(contentsOf: plistURL), format: nil) as? [String: Any])
        let types = try XCTUnwrap(plist["CFBundleDocumentTypes"] as? [[String: Any]])
        let fallback = try XCTUnwrap(types.first {
            ($0["LSItemContentTypes"] as? [String])?.contains(UTType.data.identifier) == true
        })
        XCTAssertEqual(fallback["NSDocumentClass"] as? String, NSStringFromClass(EVDocument.self))
        XCTAssertEqual(fallback["CFBundleTypeRole"] as? String, "Editor")
        XCTAssertTrue(EVDocument.readableTypes.contains(UTType.data.identifier))
        XCTAssertFalse(EVDocument.writableTypes.contains(UTType.data.identifier))
        XCTAssertFalse(EVDocument.isNativeType(UTType.data.identifier))
    }

    func testUnknownReadTypesUseTextWithoutBecomingWritableFormats() throws {
        for type in [UTType.data.identifier, UTType.png.identifier, "com.example.unknown"] {
            let backend = Backend()
            let source = Data([0x73, 0x65, 0x74, 0x20, 0xff, 0x00, 0x0d, 0x0a])
            backend.serializedData = source
            let document = EVDocument(editorBackend: backend)
            defer { document.close() }

            try document.read(from: source, ofType: type)

            XCTAssertEqual(backend.readCalls.count, 1)
            XCTAssertEqual(backend.readCalls.first?.0, source)
            XCTAssertEqual(backend.readCalls.first?.1, EVDocument.plainTextType)
            XCTAssertEqual(backend.sourceFormat, .plainText)
            // AppKit repeats this assignment after its URL initializer reads.
            document.fileType = type
            XCTAssertEqual(document.fileType, EVDocument.plainTextType)
            XCTAssertEqual(document.writableTypes(for: .saveOperation), [EVDocument.plainTextType])
            XCTAssertEqual(try document.data(ofType: EVDocument.plainTextType), source)
            XCTAssertThrowsError(try document.data(ofType: type)) { error in
                XCTAssertEqual(error as? EVDocumentSerializationError, .unsupportedWritableType(type))
            }
        }
    }

    func testNSDocumentDataOverridesDelegateReadAndSameFormatSerializationToSourceBackend() throws {
        let backend = Backend()
        backend.serializedData = Data("serialized source".utf8)
        let document = EVDocument(editorBackend: backend)
        var recent: [URL] = []
        document.recordRecentDocument = { recent.append($0) }

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
        XCTAssertTrue(recent.isEmpty, "In-memory reads and serialization do not open a file")
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
        var recent: [URL] = []
        document.recordRecentDocument = { recent.append($0) }
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-document-tests-\(UUID().uuidString)", isDirectory: true)
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
        XCTAssertEqual(backend.readCalls[0].1, EVDocument.markdownSourceType)
        XCTAssertEqual(backend.serializationTypes, [EVDocument.markdownType])
        XCTAssertEqual(try Data(contentsOf: outputURL), backend.serializedData)
        XCTAssertEqual(recent, [EVDocumentIdentity.canonicalURL(inputURL)],
                       "A successful URL read records once; low-level copy writes do not adopt a document")
    }

    func testFailedURLReadDoesNotRecordARecentDocument() {
        let document = EVDocument(editorBackend: Backend())
        var recent: [URL] = []
        document.recordRecentDocument = { recent.append($0) }
        let missing = FileManager.default.temporaryDirectory.appendingPathComponent("viem-missing-read-\(UUID().uuidString).txt")
        XCTAssertThrowsError(try document.read(from: missing, ofType: EVDocument.plainTextType))
        XCTAssertTrue(recent.isEmpty)
    }

    func testBackendChangeNotificationMarksNativeDocumentEdited() {
        let backend = Backend()
        let document = EVDocument(editorBackend: backend)
        var recent: [URL] = []
        document.recordRecentDocument = { recent.append($0) }
        document.fileURL = FileManager.default.temporaryDirectory.appendingPathComponent("viem-typing-\(UUID().uuidString).txt")
        document.updateChangeCount(.changeCleared)
        XCTAssertFalse(document.isDocumentEdited)

        backend.sourceDidChange?()

        XCTAssertTrue(document.isDocumentEdited)
        XCTAssertTrue(recent.isEmpty, "Typing must not persist recent-file history")
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
        XCTAssertTrue(EVDocumentWindowController.isPristineUntitled(clean))

        clean.fileURL = URL(fileURLWithPath: "/tmp/named.txt")
        XCTAssertFalse(EVDocumentWindowController.isPristineUntitled(clean))

        let nativeDirty = EVDocument(editorBackend: Backend())
        nativeDirty.updateChangeCount(.changeDone)
        XCTAssertFalse(EVDocumentWindowController.isPristineUntitled(nativeDirty))

        let coreDirtyBackend = Backend()
        coreDirtyBackend.persistenceState = .init(isDirty: true)
        let coreDirty = EVDocument(editorBackend: coreDirtyBackend)
        XCTAssertFalse(EVDocumentWindowController.isPristineUntitled(coreDirty))
    }

    func testOpenRecentReplacesThePristineLaunchPlaceholder() throws {
        let documentController = NSDocumentController.shared
        XCTAssertTrue(
            documentController.documents.isEmpty,
            "the document lifecycle test must begin without process-global documents"
        )
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-open-recent-tests-\(UUID().uuidString)", isDirectory: true)
        let inputURL = directory.appendingPathComponent("opened.txt")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data().write(to: inputURL)
        defer {
            for document in documentController.documents { document.close() }
            try? FileManager.default.removeItem(at: directory)
        }

        let delegate = EVApplicationDelegate()
        delegate.recordRecentDocument = { _ in }
        delegate.applicationDidFinishLaunching(
            Notification(name: NSApplication.didFinishLaunchingNotification)
        )
        let placeholder = try XCTUnwrap(documentController.documents.first as? EVDocument)
        let originalController = try XCTUnwrap(placeholder.windowControllers.first)
        let originalWindow = try XCTUnwrap(originalController.window)
        originalWindow.setContentSize(NSSize(width: 730, height: 510))
        let originalFrame = originalWindow.frame
        XCTAssertNil(placeholder.fileURL)

        let item = NSMenuItem()
        item.representedObject = inputURL
        delegate.openRecentDocument(item)

        XCTAssertNotNil(documentController.document(for: inputURL))
        XCTAssertFalse(documentController.documents.contains { $0 === placeholder })
        XCTAssertEqual(documentController.documents.count, 1)
        XCTAssertTrue(documentController.document(for: inputURL)?.windowControllers.first === originalController)
        XCTAssertEqual(originalWindow.frame, originalFrame)
        XCTAssertTrue(originalWindow.isVisible)
    }

    func testNativeSaveAcknowledgesExactSnapshotOnlyAfterSuccessfulWrite() throws {
        let backend = Backend()
        backend.serializedData = Data("exact saved snapshot".utf8)
        backend.persistenceState = .init(isDirty: true)
        let document = EVDocument(editorBackend: backend)
        var recent: [URL] = []
        var bytesSeenWhenRecording: Data?
        document.recordRecentDocument = {
            recent.append($0)
            bytesSeenWhenRecording = try? Data(contentsOf: $0)
        }
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-save-tests-\(UUID().uuidString)", isDirectory: true)
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
        XCTAssertEqual(recent, [EVDocumentIdentity.canonicalURL(outputURL)])
        XCTAssertEqual(bytesSeenWhenRecording, backend.serializedData,
                       "Save As is recorded only after the physical write succeeds")
    }

    func testNativeSaveAsRejectsCrossFormatBeforeSnapshotOrFileMutation() throws {
        let backend = Backend()
        backend.serializedData = Data("# changed source".utf8)
        backend.persistenceState = .init(isDirty: true)
        let document = EVDocument(editorBackend: backend)
        var recent: [URL] = []
        document.recordRecentDocument = { recent.append($0) }
        try document.read(
            from: Data("# original source".utf8),
            ofType: EVDocument.markdownType
        )

        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-cross-format-tests-\(UUID().uuidString)", isDirectory: true)
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
        XCTAssertTrue(recent.isEmpty)
    }

    func testFailedNativeWriteDoesNotAcknowledgeOrClearDirtyState() throws {
        let backend = Backend()
        backend.serializedData = Data("unsaved".utf8)
        backend.persistenceState = .init(isDirty: true)
        let document = EVDocument(editorBackend: backend)
        var recent: [URL] = []
        document.recordRecentDocument = { recent.append($0) }
        let missingParent = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-missing-\(UUID().uuidString)", isDirectory: true)
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
        XCTAssertTrue(recent.isEmpty)
    }

    func testSuccessfulFileWriteRemainsRecentWhenCoreAcknowledgementIsStale() throws {
        let backend = Backend()
        backend.serializedData = Data("written revision".utf8)
        backend.acknowledgementError = Backend.Failure.acknowledgement
        backend.persistenceState = .init(isDirty: true)
        let document = EVDocument(editorBackend: backend)
        var recent: [URL] = []
        document.recordRecentDocument = { recent.append($0) }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-recent-save-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { document.close(); try? FileManager.default.removeItem(at: directory) }
        let target = directory.appendingPathComponent("saved.txt")
        let completion = expectation(description: "written file with stale acknowledgement")
        document.save(to: target, ofType: EVDocument.plainTextType, for: .saveAsOperation) { error in
            XCTAssertNotNil(error)
            completion.fulfill()
        }
        wait(for: [completion], timeout: 5)
        XCTAssertEqual(try Data(contentsOf: target), backend.serializedData)
        XCTAssertEqual(recent, [EVDocumentIdentity.canonicalURL(target)])
        XCTAssertTrue(document.isDocumentEdited)
    }

    func testSaveToWritesCopyWithoutMovingTheCoreSavePoint() throws {
        let backend = Backend()
        backend.serializedData = Data("copy only".utf8)
        backend.persistenceState = .init(isDirty: true)
        let document = EVDocument(editorBackend: backend)
        var recent: [URL] = []
        document.recordRecentDocument = { recent.append($0) }
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-save-to-tests-\(UUID().uuidString)", isDirectory: true)
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
        XCTAssertTrue(recent.isEmpty, "A copy is not a newly opened or adopted document")
    }

    func testSnapshotPreparationFailureDoesNotStartNativeWriteOrAcknowledge() {
        let backend = Backend()
        backend.snapshotError = Backend.Failure.snapshot
        backend.persistenceState = .init(isDirty: true)
        let document = EVDocument(editorBackend: backend)
        var recent: [URL] = []
        document.recordRecentDocument = { recent.append($0) }
        let outputURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-unwritten-\(UUID().uuidString).txt")

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
        XCTAssertTrue(recent.isEmpty)
    }

    func testRecoveryAutosavesDoNotRecordRecentDocumentsOrSaveSource() {
        let backend = Backend()
        let document = EVDocument(editorBackend: backend)
        var recent: [URL] = []
        document.recordRecentDocument = { recent.append($0) }
        let target = FileManager.default.temporaryDirectory.appendingPathComponent("viem-autosave-\(UUID().uuidString).txt")
        document.fileURL = target
        for operation in [NSDocument.SaveOperationType.autosaveElsewhereOperation, .autosaveInPlaceOperation] {
            let completion = expectation(description: "recovery autosave")
            document.save(to: target, ofType: EVDocument.plainTextType, for: operation) { error in
                XCTAssertNil(error)
                completion.fulfill()
            }
            wait(for: [completion], timeout: 2)
        }
        document.flushRecoverySnapshot()
        XCTAssertTrue(recent.isEmpty)
        XCTAssertTrue(backend.saveSnapshots.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: target.path))
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
