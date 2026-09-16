import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVLaunchArgumentsTests: XCTestCase {
    @MainActor private final class LaunchFixture {
        let directory: URL
        let delegate: EVApplicationDelegate
        private let originalDocuments: Set<ObjectIdentifier>

        init() throws {
            EVEditorComposition.install()
            directory = FileManager.default.temporaryDirectory
                .appendingPathComponent("viem-forwarded-launch-\(UUID())", isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            originalDocuments = Set(NSDocumentController.shared.documents.map(ObjectIdentifier.init))
            delegate = EVApplicationDelegate()
            delegate.recordRecentDocument = { _ in }
        }

        var documents: [EVDocument] {
            NSDocumentController.shared.documents.compactMap { document in
                originalDocuments.contains(ObjectIdentifier(document)) ? nil : document as? EVDocument
            }
        }

        func write(_ filename: String, text: String = "first\nsecond\nthird") throws -> URL {
            let url = directory.appendingPathComponent(filename)
            try Data(text.utf8).write(to: url)
            return url
        }

        func process(_ arguments: EVLaunchArguments = EVLaunchArguments()) {
            delegate.processLaunchArguments(arguments, workingDirectory: directory)
        }

        func document(at url: URL) throws -> EVDocument {
            try XCTUnwrap(EVDocumentIdentity.existingDocument(at: url))
        }

        func window(for document: EVDocument) throws -> EVDocumentWindowController {
            try XCTUnwrap(EVDocumentWindowController.windowShowing(document: document)?.windowController
                as? EVDocumentWindowController)
        }

        func close() {
            for document in documents { document.close() }
            try? FileManager.default.removeItem(at: directory)
        }
    }

    func testPortableParserSurvivesSwiftABIAndPreservesLiteralFilenames() throws {
        EVEditorComposition.install()
        XCTAssertEqual(try EVLaunchArguments.parse(["first file.md", "-o2", "+123", "β.txt"]),
            EVLaunchArguments(filenames: ["first file.md", "β.txt"], splitCount: 2, initialLine: 123))
        XCTAssertEqual(try EVLaunchArguments.parse(["-o", "+", "--", "+123", "-draft"]),
            EVLaunchArguments(filenames: ["+123", "-draft"], splitCount: 0, initialLine: .max))
    }

    func testMalformedFlagsProduceUserFacingErrors() throws {
        for arguments in [["-unknown"], ["-o-1"], ["+abc"], ["+18446744073709551616"]] {
            XCTAssertThrowsError(try EVCoreLaunchArguments.parse(arguments)) { error in
                XCTAssertNotNil(error as? EVLaunchArgumentError)
                XCTAssertFalse(error.localizedDescription.isEmpty)
            }
        }
    }

    func testInitialLineUsesLogicalLinesAndClampsWithoutEditing() throws {
        let source = "first\n  βeta\nlast"
        for (line, expected) in [(UInt64(2), UInt64(8)), (0, 0), (999, 14), (UInt64.max, 14)] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data(source.utf8), typeName: EVDocument.plainTextType)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            surface.goToLine(line)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, expected, "line \(line)")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
            XCTAssertFalse(backend.persistenceState.isDirty)
            XCTAssertEqual(surface.statusBarState.message, "")
        }
    }

    func testInitialLineCancelsExistingInputWithoutChangingSource() throws {
        for pendingInput in ["d", "2d", "v", "vg", "v:", "iTyped ", "3iTyped ", "RTyped ", "iLiteral "] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data("first\nsecond\nthird".utf8), typeName: EVDocument.plainTextType)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            surface.goToLine(1)
            let session = try XCTUnwrap(surface.session)
            _ = try session.sendText(pendingInput)
            if pendingInput == "iLiteral " {
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 118)
            }
            let before = try backend.serializedSource(typeName: EVDocument.plainTextType)
            let dirty = backend.persistenceState.isDirty
            surface.goToLine(2)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), before, pendingInput)
            XCTAssertEqual(backend.persistenceState.isDirty, dirty, pendingInput)
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL), pendingInput)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset,
                UInt64(try XCTUnwrap(before.firstIndex(of: 10)) + 1), pendingInput)
            XCTAssertEqual(surface.statusBarState.message, "", pendingInput)
        }
    }

    func testLaunchOpensFirstMissingFilenameAsNamedEmptyBufferAndKeepsRemainingArguments() throws {
        EVEditorComposition.install()
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-launch-\(UUID())", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let urls = [directory.appendingPathComponent("first.md"), directory.appendingPathComponent("second.txt")]
        let delegate = EVApplicationDelegate(launchArguments: EVLaunchArguments(filenames: urls.map(\.path)))
        delegate.recordRecentDocument = { _ in }
        XCTAssertTrue(delegate.openLaunchArguments())
        let first = try XCTUnwrap(EVDocumentIdentity.existingDocument(at: urls[0]))
        defer { first.close() }
        let controller = try XCTUnwrap(first.windowControllers.first as? EVDocumentWindowController)
        XCTAssertEqual(controller.paneCount, 1)
        XCTAssertEqual(controller.activeDocument?.fileURL, EVDocumentIdentity.canonicalURL(urls[0]))
        XCTAssertEqual(try first.editorBackend.serializedSource(typeName: EVDocument.markdownType), Data())
        XCTAssertFalse(first.editorBackend.persistenceState.isDirty)
        XCTAssertNil(EVDocumentIdentity.existingDocument(at: urls[1]))
        XCTAssertFalse(FileManager.default.fileExists(atPath: urls[0].path))
        XCTAssertTrue(delegate.openLaunchArguments(), "launch argv is consumed only once")
        XCTAssertEqual(first.windowControllers.count, 1)
    }

    func testLaunchSplitsExistingFilesAndPositionsOnlyTheFirstFile() throws {
        EVEditorComposition.install()
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-launch-splits-\(UUID())", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let urls = [directory.appendingPathComponent("first.txt"), directory.appendingPathComponent("second.txt")]
        for url in urls { try Data("first\nsecond\nthird".utf8).write(to: url) }
        let delegate = EVApplicationDelegate(launchArguments: EVLaunchArguments(
            filenames: urls.map(\.path), splitCount: 0, initialLine: 2))
        delegate.recordRecentDocument = { _ in }
        XCTAssertTrue(delegate.openLaunchArguments())
        let first = try XCTUnwrap(EVDocumentIdentity.existingDocument(at: urls[0]))
        let second = try XCTUnwrap(EVDocumentIdentity.existingDocument(at: urls[1]))
        defer { first.close(); second.close() }
        let controller = try XCTUnwrap(first.windowControllers.first as? EVDocumentWindowController)
        XCTAssertEqual(controller.paneCount, 2)
        XCTAssertTrue(controller.activeDocument === first)
        let surface = try XCTUnwrap(controller.editorSurface as? EVEditorSurfaceController)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 6)
        XCTAssertFalse(first.editorBackend.persistenceState.isDirty)
        XCTAssertFalse(second.editorBackend.persistenceState.isDirty)
        XCTAssertEqual(try Data(contentsOf: urls[0]), Data("first\nsecond\nthird".utf8))
        controller.close()
    }

    func testEmptyForwardedLaunchReusesWindowAndCreatesBlankAfterLastWindowCloses() throws {
        let f = try LaunchFixture()
        defer { f.close() }
        XCTAssertTrue(NSDocumentController.shared.documents.isEmpty)
        f.process()
        let original = try XCTUnwrap(f.documents.first)
        let controller = try f.window(for: original)
        XCTAssertNil(original.fileURL)
        XCTAssertTrue(try XCTUnwrap(controller.window).isVisible)

        f.process()
        XCTAssertEqual(f.documents.count, 1)
        XCTAssertTrue(f.documents.first === original)
        XCTAssertTrue(try f.window(for: original) === controller)

        controller.close()
        f.process()
        let replacement = try XCTUnwrap(f.documents.first)
        XCTAssertEqual(f.documents.count, 1)
        XCTAssertFalse(replacement === original)
        XCTAssertNil(replacement.fileURL)
        XCTAssertTrue(try XCTUnwrap(f.window(for: replacement).window).isVisible)
    }

    func testForwardedRelativeFilenamesUseSenderDirectoryAndRetainLazyArgumentList() throws {
        let f = try LaunchFixture()
        defer { f.close() }
        let firstURL = try f.write("first file café.txt")
        let secondURL = try f.write("second.txt")
        f.process(EVLaunchArguments(filenames: [firstURL.lastPathComponent, secondURL.lastPathComponent], initialLine: 2))
        let first = try f.document(at: firstURL)
        let controller = try f.window(for: first)
        let surface = try XCTUnwrap(controller.editorSurface as? EVEditorSurfaceController)
        XCTAssertEqual(controller.paneCount, 1)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 6)
        XCTAssertNil(EVDocumentIdentity.existingDocument(at: secondURL))

        let state = first.editorBackend.persistenceState
        var navigationResult: Result<String?, Error>?
        controller.perform(documentHostRequests: [.init(kind: .navigateArgument,
            documentID: state.documentID, documentRevision: state.documentRevision,
            argumentNavigation: .init(target: .next))]) { navigationResult = $0 }
        _ = try XCTUnwrap(navigationResult).get()
        XCTAssertEqual(controller.activeDocument?.fileURL, EVDocumentIdentity.canonicalURL(secondURL))
    }

    func testForwardedExistingFirstFileDoesNotEagerlyOpenLaterArguments() throws {
        let f = try LaunchFixture()
        defer { f.close() }
        let firstURL = try f.write("first.txt")
        let secondURL = try f.write("second.txt")
        f.process(EVLaunchArguments(filenames: [firstURL.path]))
        let first = try f.document(at: firstURL)
        let controller = try f.window(for: first)
        f.process(EVLaunchArguments(filenames: [firstURL.path, secondURL.path]))
        XCTAssertEqual(f.documents.count, 1)
        XCTAssertTrue(try f.window(for: first) === controller)
        XCTAssertEqual(controller.paneCount, 1)
        XCTAssertNil(EVDocumentIdentity.existingDocument(at: secondURL))

        let state = first.editorBackend.persistenceState
        var result: Result<String?, Error>?
        controller.perform(documentHostRequests: [.init(kind: .navigateArgument,
            documentID: state.documentID, documentRevision: state.documentRevision,
            argumentNavigation: .init(target: .next))]) { result = $0 }
        _ = try XCTUnwrap(result).get()
        XCTAssertEqual(controller.activeDocument?.fileURL, EVDocumentIdentity.canonicalURL(secondURL))
    }

    func testForwardedExistingFileKeepsUnsavedInsertTextAndNavigatesInSameView() throws {
        let f = try LaunchFixture()
        defer { f.close() }
        let url = try f.write("draft.txt")
        f.process(EVLaunchArguments(filenames: [url.path]))
        let document = try f.document(at: url)
        let controller = try f.window(for: document)
        let surface = try XCTUnwrap(controller.editorSurface as? EVEditorSurfaceController)
        let session = try XCTUnwrap(surface.session)
        _ = try session.sendText("iUnsaved ")
        let expected = try document.editorBackend.serializedSource(typeName: EVDocument.plainTextType)
        XCTAssertTrue(document.editorBackend.persistenceState.isDirty)
        f.process(EVLaunchArguments(filenames: [url.path], initialLine: 2))
        XCTAssertEqual(f.documents.count, 1)
        XCTAssertEqual(document.windowControllers.count, 1)
        XCTAssertTrue(try f.window(for: document) === controller)
        XCTAssertTrue(controller.editorSurface === surface)
        XCTAssertEqual(try document.editorBackend.serializedSource(typeName: EVDocument.plainTextType), expected)
        XCTAssertEqual(try Data(contentsOf: url), Data("first\nsecond\nthird".utf8))
        XCTAssertTrue(document.editorBackend.persistenceState.isDirty)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 14)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
    }

    func testForwardedExistingSplitFileFocusesItsPaneWithoutDuplicatingIt() throws {
        let f = try LaunchFixture()
        defer { f.close() }
        let firstURL = try f.write("first.txt")
        let secondURL = try f.write("second.txt")
        f.process(EVLaunchArguments(filenames: [firstURL.path, secondURL.path], splitCount: 0))
        let first = try f.document(at: firstURL)
        let second = try f.document(at: secondURL)
        let controller = try f.window(for: first)
        XCTAssertTrue(controller.activeDocument === first)

        f.process(EVLaunchArguments(filenames: [secondURL.path], initialLine: 3))
        XCTAssertEqual(f.documents.count, 2)
        XCTAssertEqual(controller.paneCount, 2)
        XCTAssertTrue(try f.window(for: second) === controller)
        XCTAssertTrue(controller.activeDocument === second)
        let surface = try XCTUnwrap(controller.editorSurface as? EVEditorSurfaceController)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 13)
        XCTAssertFalse(first.editorBackend.persistenceState.isDirty)
        XCTAssertFalse(second.editorBackend.persistenceState.isDirty)
    }

    func testForwardedMixedSplitLaunchReusesExistingFileAndOpensOnlyNewFilesTogether() throws {
        let f = try LaunchFixture()
        defer { f.close() }
        let oldURL = try f.write("old.txt")
        let newURLs = try ["new-first.txt", "new-second.txt"].map { try f.write($0) }
        f.process(EVLaunchArguments(filenames: [oldURL.path]))
        let old = try f.document(at: oldURL)
        let oldController = try f.window(for: old)
        f.process(EVLaunchArguments(filenames: [oldURL.path] + newURLs.map(\.path), splitCount: 0, initialLine: 2))

        let newFirst = try f.document(at: newURLs[0])
        let newSecond = try f.document(at: newURLs[1])
        let newController = try f.window(for: newFirst)
        XCTAssertEqual(f.documents.count, 3)
        XCTAssertTrue(try f.window(for: old) === oldController)
        XCTAssertEqual(oldController.paneCount, 1)
        XCTAssertFalse(newController === oldController)
        XCTAssertTrue(try f.window(for: newSecond) === newController)
        XCTAssertEqual(newController.paneCount, 2)
        XCTAssertEqual((oldController.editorSurface as? EVEditorSurfaceController)?.viewPresentation.cursor_utf8_offset, 6)
        XCTAssertEqual((newController.editorSurface as? EVEditorSurfaceController)?.viewPresentation.cursor_utf8_offset, 0)
    }

    func testRepeatedCanonicalFileArgumentsNeverCreateDuplicatePanes() throws {
        let f = try LaunchFixture()
        defer { f.close() }
        let url = try f.write("original.txt")
        let alias = f.directory.appendingPathComponent("alias.txt")
        let hardlink = f.directory.appendingPathComponent("hardlink.txt")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: url)
        try FileManager.default.linkItem(at: url, to: hardlink)
        f.process(EVLaunchArguments(filenames: [url.path, "./original.txt", alias.path, hardlink.path], splitCount: 0))
        let document = try f.document(at: url)
        let controller = try f.window(for: document)
        XCTAssertEqual(f.documents.count, 1)
        XCTAssertEqual(document.windowControllers.count, 1)
        XCTAssertEqual(controller.paneCount, 1)
        f.process(EVLaunchArguments(filenames: [alias.path, url.path, hardlink.path], splitCount: 0))
        XCTAssertEqual(f.documents.count, 1)
        XCTAssertEqual(document.windowControllers.count, 1)
        XCTAssertEqual(controller.paneCount, 1)
    }

    func testRepeatedExistingArgumentsDoNotConsumeSpaceForNewSplitFiles() throws {
        let f = try LaunchFixture()
        defer { f.close() }
        let existingURL = try f.write("existing.txt")
        let newURL = try f.write("new.txt")
        f.process(EVLaunchArguments(filenames: [existingURL.path]))
        let existing = try f.document(at: existingURL)
        let existingController = try f.window(for: existing)
        let arguments = Array(repeating: existingURL.path, count: 10_000) + [newURL.path]
        f.process(EVLaunchArguments(filenames: arguments, splitCount: 0))
        let opened = try f.document(at: newURL)
        let newController = try f.window(for: opened)
        XCTAssertEqual(f.documents.count, 2)
        XCTAssertEqual(existingController.paneCount, 1)
        XCTAssertEqual(newController.paneCount, 1)
        XCTAssertFalse(existingController === newController)
    }

    func testExplicitSplitCountLimitsArgumentSlotsBeforeDuplicateSuppression() throws {
        let f = try LaunchFixture()
        defer { f.close() }
        let firstURL = try f.write("first.txt")
        let laterURL = try f.write("later.txt")
        f.process(EVLaunchArguments(filenames: [firstURL.path, firstURL.path, laterURL.path], splitCount: 2))
        let first = try f.document(at: firstURL)
        XCTAssertEqual(f.documents.count, 1)
        XCTAssertEqual(try f.window(for: first).paneCount, 1)
        XCTAssertNil(EVDocumentIdentity.existingDocument(at: laterURL))
    }

    func testForwardedSplitOptionsWithoutFilenamesCreateRequestedBlankPanes() throws {
        let f = try LaunchFixture()
        defer { f.close() }
        f.process()
        let old = try XCTUnwrap(f.documents.first)
        let oldController = try f.window(for: old)
        f.process(EVLaunchArguments(splitCount: 3))
        let blank = try XCTUnwrap(f.documents.first { $0 !== old })
        let newController = try f.window(for: blank)
        XCTAssertFalse(oldController === newController)
        XCTAssertEqual(oldController.paneCount, 1)
        XCTAssertEqual(newController.paneCount, 3)
        XCTAssertEqual(f.documents.count, 4)
        XCTAssertTrue(f.documents.allSatisfy { $0.fileURL == nil && !$0.editorBackend.persistenceState.isDirty })
    }
}
