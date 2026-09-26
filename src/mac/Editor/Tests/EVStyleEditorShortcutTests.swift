import AppKit
@testable import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVStyleEditorShortcutTests: XCTestCase {
    func testNativeStyleSubmenusValidateWhenTheDocumentFormatChanges() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-style-menu-validation-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data("text".utf8), typeName: EVDocument.plainTextType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let controller = EVDocumentWindowController(document: EVDocument(), editorSurface: surface)
        defer { controller.window?.orderOut(nil) }
        let owner = EVApplicationDelegate(configuration: configuration)
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { surface })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let styles = try XCTUnwrap(main.item(withTitle: "Style")?.submenu)
        let paragraph = try XCTUnwrap(styles.item(withTitle: "Paragraph"))
        let character = try XCTUnwrap(styles.item(withTitle: "Character"))
        let edit = try XCTUnwrap(styles.item(withTitle: "Edit Styles…"))
        // Supply the normal responder-chain target explicitly so native menu
        // validation does not depend on which test window is currently key.
        for item in [paragraph, character, edit] {
            item.target = controller.documentContentController
        }
        for (command, enabled) in [
            (EVMenuCommand.reinterpretAsText, false), (.reinterpretAsCode, false),
            (.reinterpretAsMarkdown, true), (.reinterpretAsHTML, true),
            (.reinterpretAsText, false), (.reinterpretAsMarkdown, true),
        ] {
            surface.perform(menuCommand: command, sender: nil)
            styles.update()
            XCTAssertEqual(paragraph.isEnabled, enabled, "\(command)")
            XCTAssertEqual(character.isEnabled, enabled, "\(command)")
            XCTAssertTrue(edit.isEnabled, "Styles remain editable through the inspector")
        }
        withExtendedLifetime((builder, owner, controller)) {}
    }

    func testBareF8OpensTheModelessStyleEditorInNormalAndInsertWithoutEditing() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-style-shortcut-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let source = Data("# Title `code` tail\n\nBody".utf8)
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
        let styles = try XCTUnwrap(main.item(withTitle: "Style")?.submenu)
        let item = try XCTUnwrap(styles.item(withTitle: "Edit Styles…"))
        XCTAssertEqual(item.keyEquivalent, String(UnicodeScalar(NSF8FunctionKey)!))
        XCTAssertTrue(item.keyEquivalentModifierMask.isEmpty)
        XCTAssertNil(item.target, "The shortcut uses the editor responder chain")
        for title in ["Paragraph", "Character"] {
            let menu = try XCTUnwrap(styles.item(withTitle: title)?.submenu)
            XCTAssertNil(menu.item(withTitle: "Edit Styles…"))
            builder.menuNeedsUpdate(menu)
            XCTAssertNil(menu.item(withTitle: "Edit Styles…"))
        }

        let session = try XCTUnwrap(surface.session)
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 7, length: 0))
        let inlineCode = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Code"))
        XCTAssertEqual(try session.selectedNamedStyles().character, inlineCode.id)
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
            XCTAssertEqual(coordinator.inspection?.selectedStyleKey, inlineCode)
            XCTAssertEqual(surface.viewPresentation.mode, mode)
            XCTAssertEqual(try backend.recoverySnapshot(), original)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
            XCTAssertFalse(surface.canUndo)
            coordinator.close()
        }
        withExtendedLifetime((builder, owner, controller)) {}
    }
}
