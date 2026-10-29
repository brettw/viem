import AppKit
@testable import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVStyleEditorShortcutTests: XCTestCase {
    @MainActor
    private final class HeadingShortcutDelegate: NSObject, NSMenuDelegate {
        let builder: EVMenuBuilder
        weak var editor: EVEditorView?

        init(builder: EVMenuBuilder, editor: EVEditorView) {
            self.builder = builder
            self.editor = editor
        }

        func menuNeedsUpdate(_ menu: NSMenu) {
            builder.menuNeedsUpdate(menu)
            // XCTest has no application-global key-window responder target.
            // Keep native matching/validation, supplying its editor explicitly.
            for item in menu.items { item.target = editor }
        }
    }

    func testNativeHeadingSubmenuAndFormatStyleEditorValidateWhenDocumentFormatChanges() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-style-menu-validation-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data("text".utf8), typeName: EVDocument.codeType, origin: .file(path: "notes.md", automaticCode: false))
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let controller = EVDocumentWindowController(document: EVDocument(), editorSurface: surface)
        defer { controller.window?.orderOut(nil) }
        let owner = EVApplicationDelegate(configuration: configuration)
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { surface })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let insert = try XCTUnwrap(main.item(withTitle: "Insert")?.submenu)
        let format = try XCTUnwrap(main.item(withTitle: "Format")?.submenu)
        let heading = try XCTUnwrap(insert.item(withTitle: "Heading"))
        let edit = try XCTUnwrap(format.item(withTitle: "Edit Styles…"))
        for item in [heading, edit] { item.target = controller.documentContentController }
        for preview in [true, false, true] {
            surface.setMarkdownPreview(preview)
            insert.update()
            format.update()
            XCTAssertEqual(heading.isEnabled, preview)
            XCTAssertTrue(edit.isEnabled, "Styles remain editable through the inspector")
        }
        withExtendedLifetime((builder, owner, controller)) {}
    }

    func testInsertHeadingMatchesParagraphPopupAndRefreshesKeyboardActions() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-insert-heading-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let original = Data("Words".utf8)
        try backend.readUntitled(source: original, typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let controller = EVDocumentWindowController(document: EVDocument(editorBackend: backend), editorSurface: surface)
        defer { controller.close() }
        let window = try XCTUnwrap(controller.window)
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
        let menu = try XCTUnwrap(main.item(withTitle: "Insert")?.submenu?.item(withTitle: "Heading")?.submenu)
        let toolbar = surface.formattingToolbar
        for level in 1...6 {
            builder.menuNeedsUpdate(menu)
            toolbar.refresh()
            let item = try XCTUnwrap(menu.item(withTag: EVMenuCommand.heading0.rawValue + level))
            let action = try XCTUnwrap(item.representedObject as? EVStyleMenuAction)
            let otherItem = try XCTUnwrap(menu.item(withTag: EVMenuCommand.heading0.rawValue + (level == 1 ? 2 : 1)))
            let popupItem = try XCTUnwrap(toolbar.paragraphStyle.itemArray.first {
                ($0.representedObject as? EVStyleMenuAction)?.stableID == "Heading\(level)"
            })
            let popupAction = try XCTUnwrap(popupItem.representedObject as? EVStyleMenuAction)
            XCTAssertEqual(action.kind, popupAction.kind)
            XCTAssertEqual(action.role, popupAction.role)
            XCTAssertEqual(action.stableID, popupAction.stableID)
            XCTAssertEqual(action.documentID, popupAction.documentID)
            XCTAssertEqual(action.documentRevision, popupAction.documentRevision)
            XCTAssertEqual(action.styleSheetRevision, popupAction.styleSheetRevision)
            XCTAssertEqual(item.isEnabled, popupItem.isEnabled)
            XCTAssertTrue(item.isEnabled)
            surface.editorView.performEditorStyleMenuAction(item)
            let expected = Data((String(repeating: "#", count: level) + " Words").utf8)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), expected)
            XCTAssertNotEqual(try backend.styleSheetSnapshot().identity.documentRevision, action.documentRevision)
            surface.editorView.performEditorStyleMenuAction(otherItem)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), expected,
                           "a menu item captured before the edit cannot change its newer revision")
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), original)
            toolbar.refresh()
            let currentPopupItem = try XCTUnwrap(toolbar.paragraphStyle.itemArray.first {
                ($0.representedObject as? EVStyleMenuAction)?.stableID == "Heading\(level)"
            })
            toolbar.paragraphStyle.select(currentPopupItem)
            toolbar.chooseStyle(toolbar.paragraphStyle)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), expected)
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), original)
        }
        let keyboardDelegate = HeadingShortcutDelegate(builder: builder, editor: surface.editorView)
        menu.delegate = keyboardDelegate
        keyboardDelegate.menuNeedsUpdate(menu)
        defer { withExtendedLifetime(keyboardDelegate) {} }
        // AppKit refreshes the submenu while matching accelerators, even when
        // it has not been opened since the document revision changed.
        for level in [1, 2] {
            let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
                modifierFlags: [.command], timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, characters: String(level),
                charactersIgnoringModifiers: String(level), isARepeat: false, keyCode: level == 1 ? 18 : 19))
            XCTAssertTrue(menu.performKeyEquivalent(with: event))
            let expected = Data((String(repeating: "#", count: level) + " Words").utf8)
            // Native menu highlighting may defer action delivery until the next
            // run-loop turn. Wait for that action, without refreshing the menu.
            let deadline = Date().addingTimeInterval(1)
            while try backend.serializedSource(typeName: EVDocument.markdownType) != expected && Date() < deadline {
                RunLoop.main.run(until: Date().addingTimeInterval(0.01))
            }
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), expected)
        }
        XCTAssertNil(surface.commandOutput)
    }

    func testBareF8OpensTheModelessStyleEditorInNormalAndInsertWithoutEditing() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-style-shortcut-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let source = Data("# Title `code` tail\n\nBody".utf8)
        try backend.readUntitled(source: source, typeName: EVDocument.markdownType)
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
        let styles = try XCTUnwrap(main.item(withTitle: "Format")?.submenu)
        let item = try XCTUnwrap(styles.item(withTitle: "Edit Styles…"))
        XCTAssertEqual(item.keyEquivalent, String(UnicodeScalar(NSF8FunctionKey)!))
        XCTAssertTrue(item.keyEquivalentModifierMask.isEmpty)
        XCTAssertNil(item.target, "The shortcut uses the editor responder chain")
        XCTAssertNil(main.item(withTitle: "Style"))
        XCTAssertNil(styles.item(withTitle: "Paragraph"))
        XCTAssertNil(styles.item(withTitle: "Character"))

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
            XCTAssertNil(coordinator.inspection?.targetDocumentIdentity)
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
