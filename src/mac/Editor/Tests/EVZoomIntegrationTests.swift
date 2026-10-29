import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVZoomIntegrationTests: XCTestCase {
    private func configuration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-zoom-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory, legacyDefaults: nil)
    }

    func testMenuStopsSaturateAtTwentyFiveAndFiveHundredPercentWithoutEditing() throws {
        let backend = EVCoreDocumentBackend(configuration: configuration())
        let source = Data("Words for zooming".utf8)
        try backend.readUntitled(source: source, typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        let other = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        other.loadViewIfNeeded()
        let revision = surface.documentState.document_revision
        for percent in [90,80,75,67,50,33,25] {
            surface.perform(menuCommand: .zoomOut, sender: nil)
            XCTAssertEqual(surface.zoomScale, Float(percent) / 100)
        }
        XCTAssertFalse(surface.presentation(for: .zoomOut).isEnabled)
        let minGeneration = surface.viewportState.configuration_generation
        surface.perform(menuCommand: .zoomOut, sender: nil)
        XCTAssertEqual(surface.viewportState.configuration_generation, minGeneration)
        for percent in [33,50,67,75,80,90,100,110,125,150,175,200,250,300,400,500] {
            surface.perform(menuCommand: .zoomIn, sender: nil)
            XCTAssertEqual(surface.zoomScale, Float(percent) / 100)
        }
        XCTAssertFalse(surface.presentation(for: .zoomIn).isEnabled)
        let maxGeneration = surface.viewportState.configuration_generation
        surface.perform(menuCommand: .zoomIn, sender: nil)
        XCTAssertEqual(surface.viewportState.configuration_generation, maxGeneration)
        _ = try session.setScale(1.03)
        surface.refreshPresentation()
        surface.perform(menuCommand: .zoomOut, sender: nil)
        XCTAssertEqual(surface.zoomScale, 1)
        XCTAssertEqual(other.zoomScale, 1)
        XCTAssertEqual(surface.documentState.document_revision, revision)
        XCTAssertEqual(try backend.serializedSource(typeName: "public.plain-text"), source)
        XCTAssertFalse(surface.presentation(for: .undo).isEnabled)
        XCTAssertThrowsError(try session.setScale(5.01))
    }

    func testCommandZoomKeysAndMenuResponderWorkInNormalAndInsertWithoutText() throws {
        let backend = EVCoreDocumentBackend(configuration: configuration())
        let source = Data("Words".utf8)
        try backend.readUntitled(source: source, typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let controller = EVDocumentWindowController(document: EVDocument(), editorSurface: surface)
        let window = try XCTUnwrap(controller.window)
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        let session = try XCTUnwrap(surface.session)
        for insert in [false,true] {
            if insert { _ = try session.sendText("i"); surface.refreshPresentation() }
            let mode = surface.viewPresentation.mode
            for (characters, key, code, scale) in [("=", "=", UInt16(24), Float(1.1)), ("-", "-", UInt16(27), Float(1))] {
                let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
                    modifierFlags: [.command], timestamp: 0, windowNumber: window.windowNumber,
                    context: nil, characters: characters, charactersIgnoringModifiers: key,
                    isARepeat: false, keyCode: code))
                surface.editorView.keyDown(with: event)
                XCTAssertEqual(surface.zoomScale, scale)
                XCTAssertEqual(surface.viewPresentation.mode, mode)
                XCTAssertEqual(try backend.serializedSource(typeName: "public.plain-text"), source)
            }
            let item = NSMenuItem(title: "Zoom In", action: #selector(EVEditorCommandRouting.performEditorMenuCommand(_:)), keyEquivalent: "=")
            item.tag = EVMenuCommand.zoomIn.rawValue
            item.keyEquivalentModifierMask = [.command]
            XCTAssertTrue(surface.editorView.tryToPerform(try XCTUnwrap(item.action), with: item))
            XCTAssertEqual(surface.zoomScale, 1.1)
            surface.perform(menuCommand: .actualSize, sender: nil)
        }
        XCTAssertEqual(try backend.serializedSource(typeName: "public.plain-text"), source)
        withExtendedLifetime(controller) {}
    }

    func testChosenZoomSurvivesRestartAndReplacementPreservesEachLiveView() throws {
        let config = configuration()
        let backend = EVCoreDocumentBackend(configuration: config)
        try backend.readUntitled(source: Data("Words".utf8), typeName: EVDocument.plainTextType)
        let first = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        first.loadViewIfNeeded()
        first.perform(menuCommand: .zoomIn, sender: nil)
        XCTAssertEqual(config.zoomScale, 1.1)
        let second = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        second.loadViewIfNeeded()
        XCTAssertEqual(second.zoomScale, 1.1)
        second.perform(menuCommand: .zoomIn, sender: nil)
        XCTAssertEqual(config.zoomScale, 1.25)
        XCTAssertEqual(first.zoomScale, 1.1)

        // Reloading a buffer copies each live view; neither copy changes the saved default.
        try backend.readUntitled(source: Data("Replacement".utf8), typeName: EVDocument.plainTextType)
        XCTAssertEqual(first.zoomScale, 1.1)
        XCTAssertEqual(second.zoomScale, 1.25)
        let reopened = EVConfigurationStore(directory: config.directory, legacyDefaults: nil)
        XCTAssertEqual(reopened.zoomScale, 1.25)
        let restarted = EVCoreDocumentBackend(configuration: reopened)
        let restored = try XCTUnwrap(restarted.makeEditorSurface() as? EVEditorSurfaceController)
        restored.loadViewIfNeeded()
        XCTAssertEqual(restored.zoomScale, 1.25)
        restored.perform(menuCommand: .actualSize, sender: nil)
        XCTAssertEqual(EVConfigurationStore(directory: config.directory, legacyDefaults: nil).zoomScale, 1)
        try reopened.setZoomScale(2)
        let generation = restored.viewportState.configuration_generation
        restored.perform(menuCommand: .actualSize, sender: nil)
        XCTAssertEqual(EVConfigurationStore(directory: config.directory, legacyDefaults: nil).zoomScale, 1,
            "An explicit Actual Size choice updates the default even in a view already at 100%")
        XCTAssertEqual(restored.viewportState.configuration_generation, generation)
        XCTAssertEqual(first.zoomScale, 1.1)
        XCTAssertEqual(second.zoomScale, 1.25)
    }

    func testSaveFailureKeepsSuccessfulLocalZoomAndReportsIt() throws {
        let config = configuration()
        try config.setZoomScale(1)
        let backend = EVCoreDocumentBackend(configuration: config)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let invalid = Data("invalid external settings".utf8)
        let file = config.directory.appendingPathComponent("config.json")
        try invalid.write(to: file)
        surface.perform(menuCommand: .zoomIn, sender: nil)
        XCTAssertEqual(surface.zoomScale, 1.1)
        XCTAssertFalse(try XCTUnwrap(surface.statusBarState.commandOutput).isEmpty)
        XCTAssertEqual(try Data(contentsOf: file), invalid)
    }
}
