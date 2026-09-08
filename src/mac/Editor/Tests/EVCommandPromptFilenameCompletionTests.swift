import AppKit
import CEvimCore
import EvimAppShell
import XCTest
@testable import EvimEditor

@MainActor final class EVCommandPromptFilenameCompletionTests: XCTestCase {
    private func makeSurface() throws -> (URL, EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession, NSWindow) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("evim-completion-\(UUID())", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("document".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = surface
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 900, height: 400)
        surface.viewDidLayout()
        window.makeFirstResponder(surface.editorView)
        return (directory, backend, surface, try XCTUnwrap(surface.session), window)
    }

    private func key(_ code: UInt16, modifiers: NSEvent.ModifierFlags = [], characters: String = "") throws -> NSEvent {
        try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: 0, windowNumber: 0, context: nil, characters: characters, charactersIgnoringModifiers: characters, isARepeat: false, keyCode: code))
    }

    func testNativeTabAndBacktabCycleCaseInsensitiveFilesAndDirectories() throws {
        let (directory, backend, surface, session, window) = try makeSurface()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        for name in ["Alpha.txt", "alpha TWO.txt", "Beta.txt"] {
            try Data().write(to: directory.appendingPathComponent(name))
        }
        try FileManager.default.createDirectory(at: directory.appendingPathComponent("aFolder"), withIntermediateDirectories: false)
        let base = "e \(directory.path)/"
        let revision = backend.persistenceState.documentRevision
        surface.performInput { _ = try session.sendText(":\(base)a") }

        var candidates: [String] = []
        for _ in 0..<3 {
            surface.editorView.keyDown(with: try key(48))
            candidates.append(try XCTUnwrap(surface.commandLine?.text))
        }
        XCTAssertEqual(candidates, ["\(base)aFolder/", "\(base)alpha TWO.txt", "\(base)Alpha.txt"])
        surface.editorView.keyDown(with: try key(48))
        XCTAssertEqual(surface.commandLine?.text, candidates[0])
        surface.editorView.keyDown(with: try key(48, modifiers: .shift))
        XCTAssertEqual(surface.commandLine?.text, candidates[2])
        surface.editorView.doCommand(by: #selector(NSResponder.insertBacktab(_:)))
        XCTAssertEqual(surface.commandLine?.text, candidates[1])
        surface.editorView.doCommand(by: #selector(NSResponder.insertTab(_:)))
        XCTAssertEqual(surface.commandLine?.text, candidates[2])
        surface.editorView.keyDown(with: try key(16, modifiers: .control, characters: "y"))
        surface.editorView.keyDown(with: try key(14, modifiers: .control, characters: "e"))
        XCTAssertEqual(surface.commandLine?.text, candidates[2])
        XCTAssertNil(surface.commandOutput)
        XCTAssertEqual(try backend.formattedText(), "document")
        XCTAssertEqual(backend.persistenceState.documentRevision, revision)
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    func testNativeSlashAcceptsDirectoryWithoutDuplicationAndCompletesItsChild() throws {
        let (directory, backend, surface, session, window) = try makeSurface()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        let nested = directory.appendingPathComponent("Nested", isDirectory: true)
        try FileManager.default.createDirectory(at: nested, withIntermediateDirectories: false)
        try Data().write(to: nested.appendingPathComponent("Final.txt"))
        surface.performInput { _ = try session.sendText(":edit \(directory.path)/ne") }
        surface.editorView.keyDown(with: try key(48))
        XCTAssertEqual(surface.commandLine?.text, "edit \(nested.path)/")
        surface.editorView.insertText("/", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(surface.commandLine?.text, "edit \(nested.path)/")
        surface.editorView.insertText("fi", replacementRange: NSRange(location: NSNotFound, length: 0))
        surface.editorView.keyDown(with: try key(48))
        XCTAssertEqual(surface.commandLine?.text, "edit \(nested.path)/Final.txt")
        XCTAssertEqual(try backend.formattedText(), "document")
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    func testNativeTypingAndCaretMovementAcceptCompletionAndControlECancelsOnlyActiveCycle() throws {
        let (directory, backend, surface, session, window) = try makeSurface()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        try Data().write(to: directory.appendingPathComponent("Alpha.txt"))
        let prefix = "e \(directory.path)/al"
        let completed = "e \(directory.path)/Alpha.txt"
        surface.performInput { _ = try session.sendText(":\(prefix)") }
        surface.editorView.keyDown(with: try key(48))
        XCTAssertEqual(surface.commandLine?.text, completed)
        surface.editorView.keyDown(with: try key(14, modifiers: .control, characters: "e"))
        XCTAssertEqual(surface.commandLine?.text, prefix)
        surface.editorView.keyDown(with: try key(48))
        surface.editorView.keyDown(with: try key(123))
        XCTAssertEqual(surface.commandLine?.info.cursor_utf8_offset, UInt64(completed.utf8.count - 1))
        surface.editorView.insertText("!", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(surface.commandLine?.text, String(completed.dropLast()) + "!t")
        surface.editorView.keyDown(with: try key(14, modifiers: .control, characters: "e"))
        XCTAssertEqual(surface.commandLine?.text, String(completed.dropLast()) + "!t")
        XCTAssertEqual(try backend.formattedText(), "document")
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    func testNativeTextAcceptsCompletedFilenameAndNoMatchLeavesPromptUnchanged() throws {
        let (directory, _, surface, session, window) = try makeSurface()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        try Data().write(to: directory.appendingPathComponent("Alpha.txt"))
        let completed = "write \(directory.path)/Alpha.txt"
        surface.performInput { _ = try session.sendText(":write \(directory.path)/al") }
        surface.editorView.keyDown(with: try key(48))
        surface.editorView.insertText("!", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(surface.commandLine?.text, completed + "!")
        surface.editorView.keyDown(with: try key(14, modifiers: .control, characters: "e"))
        XCTAssertEqual(surface.commandLine?.text, completed + "!")
        surface.editorView.keyDown(with: try key(48))
        XCTAssertEqual(surface.commandLine?.text, completed + "!")
        XCTAssertNil(surface.commandOutput)
    }

    func testNativeShiftTabInInsertModeRetainsTabInsertion() throws {
        let (directory, backend, surface, session, window) = try makeSurface()
        defer { window.close(); try? FileManager.default.removeItem(at: directory) }
        surface.performInput { _ = try session.sendText("i") }
        XCTAssertNil(surface.commandLine?.prompt)
        surface.editorView.keyDown(with: try key(48, modifiers: .shift))
        XCTAssertEqual(try backend.formattedText(), "\tdocument")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(EVIM_MODE_INSERT))
    }

}
