import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVLaunchArgumentsTests: XCTestCase {
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
}
