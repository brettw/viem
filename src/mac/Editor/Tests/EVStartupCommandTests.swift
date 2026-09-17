import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVStartupCommandTests: XCTestCase {
    private func configuration(_ startup: String) throws -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-startup-commands-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data(startup.utf8).write(to: directory.appendingPathComponent("startup.viem"))
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory)
    }

    private func surface(_ configuration: EVConfigurationStore, text: String = "alpha beta\nnext") throws
        -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(text.utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 300)
        surface.viewDidLayout()
        return (backend, surface)
    }

    private func type(_ keys: String, in surface: EVEditorSurfaceController) {
        surface.editorView.insertText(keys, replacementRange: NSRange(location: NSNotFound, length: 0))
    }

    func testStartupYankMappingUsesNamedRegisterWithoutChangingSource() throws {
        let (backend, surface) = try surface(configuration("map Y y$\n"))
        type("\"aY", in: surface)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(surface.canUndo)
        XCTAssertEqual(try backend.formattedText(), "alpha beta\nnext")
        type("j\"ap", in: surface)
        XCTAssertEqual(try backend.formattedText(), "alpha beta\nnalpha betaext")
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.formattedText(), "alpha beta\nnext")
        XCTAssertNil(surface.commandOutput)
    }

    func testStartupSettingsAndMappingsApplyToEveryDocumentAndNewView() throws {
        let config = try configuration("set nowrap\nmap Q l\n")
        let (first, a) = try surface(config)
        // The already-read startup snapshot remains authoritative this launch.
        try Data("map Q h\n".utf8).write(to: config.directory.appendingPathComponent("startup.viem"))
        let (second, b) = try surface(config)
        let c = try XCTUnwrap(first.makeEditorSurface() as? EVEditorSurfaceController)
        c.loadViewIfNeeded()
        for view in [a, b, c] {
            XCTAssertEqual(view.presentation(for: .wordWrap).state, .off)
            type("Q", in: view)
            XCTAssertEqual(view.viewPresentation.cursor_utf8_offset, 1)
            XCTAssertNil(view.commandOutput)
        }
        XCTAssertFalse(first.persistenceState.isDirty)
        XCTAssertFalse(second.persistenceState.isDirty)
    }

    func testInvalidStartupLineIsReportedAndLaterSettingsStillApply() throws {
        let config = try configuration("\" comment\n\nnotacommand\nmap Q l\n")
        let (backend, surface) = try surface(config)
        let warning = try XCTUnwrap(backend.configurationWarning)
        XCTAssertTrue(warning.contains("startup.viem:3:"), warning)
        XCTAssertTrue(surface.commandOutput?.contains("startup.viem:3:") == true)
        type("Q", in: surface)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 1)
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    func testMappingTimeoutUsesTheShorterCompleteMapping() throws {
        let (_, surface) = try surface(configuration("map g l\nmap gg $\n"))
        let session = try XCTUnwrap(surface.session)
        type("g", in: surface)
        XCTAssertTrue(try session.hasPendingMapping())
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 0)
        let deadline = Date().addingTimeInterval(2)
        while try session.hasPendingMapping(), Date() < deadline {
            RunLoop.main.run(until: Date().addingTimeInterval(0.02))
        }
        XCTAssertFalse(try session.hasPendingMapping())
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 1)
        XCTAssertNil(surface.commandOutput)
    }
}
