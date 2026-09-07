import AppKit
import CEvimCore
import EvimAppShell
import XCTest

@testable import EvimEditor

@MainActor
final class EVFileDropTests: XCTestCase {
    private final class Host: EVDocumentHostEffectHandling {
        var files: [[URL]] = []
        weak var target: (any EVEditorSurface)?
        var result: Result<Void, Error> = .success(())
        var onDrop: (() -> Void)?

        func perform(documentHostRequests: [EVDocumentHostRequest],
                     completion: @escaping @MainActor (Result<String?, Error>) -> Void) {
            XCTFail("File drops are native open requests, not editor commands")
            completion(.failure(EVDocumentHostError.unsupportedRequest))
        }

        func openDroppedFiles(_ urls: [URL], in targetSurface: any EVEditorSurface,
                              completion: @escaping @MainActor (Result<Void, Error>) -> Void) {
            files.append(urls)
            target = targetSurface
            onDrop?()
            completion(result)
        }
    }

    private func makeSurface(_ text: String = "Current document") throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController, Host) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(text.utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let host = Host()
        surface.documentHostEffectHandler = host
        return (backend, surface, host)
    }

    private func pasteboard(_ urls: [URL]) -> NSPasteboard {
        let board = NSPasteboard.withUniqueName()
        XCTAssertTrue(board.writeObjects(urls.map { $0 as NSURL }))
        return board
    }

    func testFileURLsKeepOrderAndUnicodeNamesWithoutTreatingStringsAsPaths() {
        let urls = [
            URL(fileURLWithPath: "/tmp/a file % café #1.txt"),
            URL(fileURLWithPath: "/tmp/second\nfile.md"),
        ]
        let board = pasteboard(urls)
        defer { board.releaseGlobally() }
        XCTAssertEqual(EVFileDrop.fileURLs(on: board), urls)

        board.clearContents()
        board.setString(urls[0].path, forType: .string)
        XCTAssertTrue(EVFileDrop.fileURLs(on: board).isEmpty)
        board.clearContents()
        XCTAssertTrue(board.writeObjects([NSURL(string: "https://example.com/document.txt")!]))
        XCTAssertTrue(EVFileDrop.fileURLs(on: board).isEmpty)
    }

    func testFinderDropRoutesToItsExactSurfaceWithoutInsertingPathsOrEditingSource() throws {
        let (backend, surface, host) = try makeSurface()
        let before = try backend.serializedSource(typeName: EVDocument.plainTextType)
        let state = backend.persistenceState
        let urls = [URL(fileURLWithPath: "/tmp/one.txt"), URL(fileURLWithPath: "/tmp/two.md")]
        let board = pasteboard(urls)
        defer { board.releaseGlobally() }
        XCTAssertTrue(surface.editorView.registeredDraggedTypes.contains(.fileURL))
        XCTAssertEqual(surface.editorView.fileDropOperation(on: board, sourceMask: [.copy, .move]), .copy)
        XCTAssertEqual(surface.editorView.fileDropOperation(on: board, sourceMask: [.move]), [])
        XCTAssertTrue(surface.editorView.performFileDrop(on: board))
        XCTAssertEqual(host.files, [urls])
        XCTAssertTrue(host.target === surface)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), before)
        XCTAssertEqual(backend.persistenceState, state)
    }

    func testDropsRequireAnAttachedHostAndIgnoreFoldersAndText() throws {
        let (_, surface, host) = try makeSurface()
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: folder) }
        let board = pasteboard([folder])
        defer { board.releaseGlobally() }
        XCTAssertEqual(surface.editorView.fileDropOperation(on: board, sourceMask: .every), [])
        XCTAssertFalse(surface.editorView.performFileDrop(on: board))
        board.clearContents()
        board.setString("/tmp/file.txt", forType: .string)
        XCTAssertFalse(surface.editorView.performFileDrop(on: board))
        XCTAssertTrue(host.files.isEmpty)

        board.clearContents()
        XCTAssertTrue(board.writeObjects([URL(fileURLWithPath: "/tmp/file.txt") as NSURL]))
        surface.documentHostEffectHandler = nil
        XCTAssertFalse(surface.editorView.performFileDrop(on: board))
        surface.documentHostEffectHandler = host
        surface.detachFromCore()
        XCTAssertEqual(surface.editorView.fileDropOperation(on: board, sourceMask: .every), [])
        XCTAssertFalse(surface.editorView.performFileDrop(on: board))
    }

    func testOpenFailureIsShownWithoutChangingTheOriginalDocument() throws {
        let (backend, surface, host) = try makeSurface("Unsaved text stays here")
        let before = try backend.serializedSource(typeName: EVDocument.plainTextType)
        host.result = .failure(EVDocumentHostError.invalidPath("missing.txt"))
        let board = pasteboard([URL(fileURLWithPath: "/tmp/missing.txt")])
        defer { board.releaseGlobally() }
        XCTAssertTrue(surface.editorView.performFileDrop(on: board))
        XCTAssertEqual(surface.commandOutput, "The file path is invalid: missing.txt")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), before)
    }

    func testMarkedInputIsCommittedBeforeTheHostChecksWhetherReplacementIsSafe() throws {
        let (backend, surface, host) = try makeSurface("")
        let session = try XCTUnwrap(surface.session)
        _ = try session.sendKey(kind: UInt32(EVIM_KEY_CHARACTER), codepoint: 105)
        surface.refreshPresentation()
        surface.editorView.setMarkedText("draft", selectedRange: NSRange(location: 5, length: 0),
                                         replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(surface.editorView.hasMarkedText())
        XCTAssertFalse(backend.persistenceState.isDirty)
        host.onDrop = {
            XCTAssertFalse(surface.editorView.hasMarkedText())
            XCTAssertTrue(backend.persistenceState.isDirty)
            XCTAssertEqual(try? backend.serializedSource(typeName: EVDocument.plainTextType), Data("draft".utf8))
        }
        let board = pasteboard([URL(fileURLWithPath: "/tmp/dropped.txt")])
        defer { board.releaseGlobally() }
        XCTAssertTrue(surface.editorView.performFileDrop(on: board))
        XCTAssertEqual(host.files.count, 1)
    }
}
