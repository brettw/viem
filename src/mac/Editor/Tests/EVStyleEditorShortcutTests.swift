import AppKit
@testable import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVStyleEditorShortcutTests: XCTestCase {
    func testBareF8OpensTheModelessStyleEditorInNormalAndInsertWithoutEditing() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-style-shortcut-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let source = Data("# Heading\n\nBody".utf8)
        try backend.read(source: source, typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let controller = EVDocumentWindowController(document: EVDocument(), editorSurface: surface)
        let window = try XCTUnwrap(controller.window)
        defer { window.orderOut(nil) }
        let coordinator = EVStyleEditorCoordinator.shared
        coordinator.close()
        defer { coordinator.close() }

        let application = NSApplication.shared
        let previousMain = application.mainMenu
        let previousWindows = application.windowsMenu
        let previousHelp = application.helpMenu
        let previousServices = application.servicesMenu
        defer {
            application.mainMenu = previousMain
            application.windowsMenu = previousWindows
            application.helpMenu = previousHelp
            application.servicesMenu = previousServices
        }
        let owner = EVApplicationDelegate(configuration: configuration)
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { surface })
        let main = builder.buildMainMenu(for: application)
        application.mainMenu = main
        let format = try XCTUnwrap(main.item(withTitle: "Format")?.submenu)
        let styles = try XCTUnwrap(format.item(withTitle: "Style")?.submenu)
        let item = try XCTUnwrap(styles.item(withTitle: "Edit Styles…"))
        XCTAssertEqual(item.keyEquivalent, String(UnicodeScalar(NSF8FunctionKey)!))
        XCTAssertTrue(item.keyEquivalentModifierMask.isEmpty)
        XCTAssertNil(item.target, "The shortcut uses the editor responder chain")

        let session = try XCTUnwrap(surface.session)
        let original = try backend.recoverySnapshot()
        for insert in [false, true] {
            if insert { _ = try session.sendText("i"); surface.refreshPresentation() }
            let mode = surface.viewPresentation.mode
            window.makeKeyAndOrderFront(nil)
            XCTAssertTrue(window.makeFirstResponder(surface.editorView))
            XCTAssertTrue(controller.documentContentController.validateMenuItem(item))
            let key = String(UnicodeScalar(NSF8FunctionKey)!)
            let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
                modifierFlags: [.function], timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, characters: key,
                charactersIgnoringModifiers: key, isARepeat: false, keyCode: 100))
            window.sendEvent(event)
            let panel = try XCTUnwrap(coordinator.styleWindow)
            XCTAssertTrue(panel.isVisible)
            XCTAssertNil(application.modalWindow)
            XCTAssertEqual(coordinator.inspection?.targetDocumentIdentity, ObjectIdentifier(backend))
            XCTAssertEqual(surface.viewPresentation.mode, mode)
            XCTAssertEqual(try backend.recoverySnapshot(), original)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
            XCTAssertFalse(surface.canUndo)
            coordinator.close()
        }
        withExtendedLifetime((builder, owner, controller)) {}
    }
}
