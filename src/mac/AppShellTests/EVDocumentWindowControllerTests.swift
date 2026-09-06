import AppKit
import XCTest

@testable import EvimAppShell

@MainActor
final class EVDocumentWindowControllerTests: XCTestCase {
    private final class Surface: EVEditorSurface {
        let viewController: NSViewController = {
            let controller = NSViewController()
            controller.view = NSView(frame: NSRect(x: 0, y: 0, width: 920, height: 655))
            return controller
        }()
        var statusBarState = EVStatusBarState()
        var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
        var performedCommands: [EVMenuCommand] = []
        var presentations: [EVMenuCommand: EVMenuItemPresentation] = [:]

        func perform(menuCommand: EVMenuCommand, sender _: Any?) {
            performedCommands.append(menuCommand)
        }

        func presentation(for command: EVMenuCommand) -> EVMenuItemPresentation {
            presentations[command] ?? .enabled
        }
    }

    private final class Backend: EVDocumentBackend {
        var sourceDidChange: (() -> Void)?
        var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)?
        var persistenceState = EVDocumentPersistenceState()
        var sourceFormat: EVSourceFormat = .plainText
        var surfaces: [Surface] = []

        func makeEditorSurface() -> any EVEditorSurface {
            let surface = Surface()
            surfaces.append(surface)
            return surface
        }

        func read(source _: Data, typeName _: String) throws {}
        func serializedSource(typeName _: String) throws -> Data { Data() }
        func nativeSaveSnapshot(typeName _: String) throws -> EVDocumentSaveSnapshot {
            EVDocumentSaveSnapshot(data: Data(), documentID: 1, documentRevision: 1)
        }
        func acknowledgeNativeSave(_: EVDocumentSaveSnapshot) throws {}
    }

    private func preservingStatusBarDefault(_ body: () throws -> Void) rethrows {
        let defaults = UserDefaults.standard
        let previous = defaults.object(forKey: "EVShowStatusBar")
        defaults.set(true, forKey: "EVShowStatusBar")
        defer {
            if let previous {
                defaults.set(previous, forKey: "EVShowStatusBar")
            } else {
                defaults.removeObject(forKey: "EVShowStatusBar")
            }
        }
        try body()
    }

    func testInitialShowUsesRequestedContentSizeAndFillsCanvasAboveStatusBar() throws {
        try preservingStatusBarDefault {
            let document = EVDocument()
            let surface = Surface()
            let controller = EVDocumentWindowController(document: document, editorSurface: surface)
            defer { controller.close() }

            XCTAssertFalse(controller.window?.isVisible ?? true)
            controller.showWindow(nil)

            let window = try XCTUnwrap(controller.window)
            let geometry = try XCTUnwrap(controller.currentGeometry)
            XCTAssertEqual(window.contentLayoutRect.width, 920, accuracy: 0.5)
            XCTAssertEqual(window.contentLayoutRect.height, 680, accuracy: 0.5)
            XCTAssertEqual(window.contentMinSize, NSSize(width: 480, height: 280))
            XCTAssertFalse(window.isMiniaturized)
            XCTAssertTrue(window.isVisible)

            XCTAssertTrue(geometry.statusBarIsVisible)
            XCTAssertEqual(geometry.content.width, 920, accuracy: 0.5)
            XCTAssertEqual(geometry.content.height, 680, accuracy: 0.5)
            XCTAssertEqual(geometry.editor.width, geometry.content.width, accuracy: 0.5)
            XCTAssertEqual(geometry.statusBar.width, geometry.content.width, accuracy: 0.5)
            XCTAssertEqual(geometry.statusBar.height, EVStatusBarView.preferredHeight, accuracy: 0.5)
            XCTAssertEqual(
                geometry.editor.height,
                geometry.content.height - EVStatusBarView.preferredHeight,
                accuracy: 0.5
            )
            XCTAssertEqual(geometry.statusBar.minY, geometry.content.minY, accuracy: 0.5)
            XCTAssertEqual(geometry.editor.minY, geometry.statusBar.maxY, accuracy: 0.5)
            XCTAssertEqual(geometry.editor.maxY, geometry.content.maxY, accuracy: 0.5)
        }
    }

    func testInitialShowOverridesPreShowGeometryWithoutRestorationOrTabbing() throws {
        let document = EVDocument()
        let controller = EVDocumentWindowController(document: document, editorSurface: Surface())
        defer { controller.close() }

        let window = try XCTUnwrap(controller.window)
        window.setContentSize(NSSize(width: 500, height: 900))
        controller.showWindow(nil)

        XCTAssertEqual(window.contentLayoutRect.width, 920, accuracy: 0.5)
        XCTAssertEqual(window.contentLayoutRect.height, 680, accuracy: 0.5)
        XCTAssertFalse(window.isRestorable)
        XCTAssertEqual(window.tabbingMode, .disallowed)
    }

    func testLaterShowsPreserveAUserResize() throws {
        let document = EVDocument()
        let controller = EVDocumentWindowController(document: document, editorSurface: Surface())
        defer { controller.close() }
        controller.showWindow(nil)

        let window = try XCTUnwrap(controller.window)
        window.setContentSize(NSSize(width: 700, height: 500))
        XCTAssertEqual(controller.editorSurface.viewController.view.frame.width, 700, accuracy: 0.5)
        XCTAssertEqual(controller.editorSurface.viewController.view.frame.height, 475, accuracy: 0.5)
        window.orderOut(nil)
        controller.showWindow(nil)

        XCTAssertEqual(window.contentLayoutRect.width, 700, accuracy: 0.5)
        XCTAssertEqual(window.contentLayoutRect.height, 500, accuracy: 0.5)
        let geometry = try XCTUnwrap(controller.currentGeometry)
        XCTAssertEqual(geometry.editor.width, 700, accuracy: 0.5)
        XCTAssertEqual(geometry.editor.height, 475, accuracy: 0.5)
    }

    func testStatusBarMenuValidationToggleAndLayoutStayWindowLocal() throws {
        try preservingStatusBarDefault {
            let document = EVDocument()
            let controller = EVDocumentWindowController(document: document, editorSurface: Surface())
            defer { controller.close() }
            controller.showWindow(nil)
            let contentController = controller.documentContentController
            let item = NSMenuItem(
                title: "Show Status Bar",
                action: #selector(contentController.toggleStatusBar(_:)),
                keyEquivalent: ""
            )

            XCTAssertTrue(contentController.validateMenuItem(item))
            XCTAssertEqual(item.state, .on)
            contentController.toggleStatusBar(item)

            var geometry = try XCTUnwrap(controller.currentGeometry)
            XCTAssertFalse(geometry.statusBarIsVisible)
            XCTAssertEqual(geometry.statusBar.height, 0, accuracy: 0.5)
            XCTAssertEqual(geometry.editor, geometry.content)
            XCTAssertEqual(UserDefaults.standard.bool(forKey: "EVShowStatusBar"), false)
            XCTAssertTrue(contentController.validateMenuItem(item))
            XCTAssertEqual(item.state, .off)

            contentController.toggleStatusBar(item)
            geometry = try XCTUnwrap(controller.currentGeometry)
            XCTAssertTrue(geometry.statusBarIsVisible)
            XCTAssertEqual(geometry.statusBar.height, EVStatusBarView.preferredHeight, accuracy: 0.5)
            XCTAssertEqual(
                geometry.editor.height,
                geometry.content.height - EVStatusBarView.preferredHeight,
                accuracy: 0.5
            )
        }
    }

    func testNewWindowCommandRoutesToDocumentWhileOtherCommandsStaySurfaceOwned() throws {
        let backend = Backend()
        let document = EVDocument(editorBackend: backend)
        document.makeWindowControllers()
        defer { document.close() }
        let first = try XCTUnwrap(document.windowControllers.first as? EVDocumentWindowController)
        let newWindowItem = NSMenuItem(
            title: "New Window for Document",
            action: #selector(first.documentContentController.performEditorMenuCommand(_:)),
            keyEquivalent: ""
        )
        newWindowItem.tag = EVMenuCommand.newWindowForDocument.rawValue

        XCTAssertTrue(first.documentContentController.validateMenuItem(newWindowItem))
        first.documentContentController.performEditorMenuCommand(newWindowItem)

        XCTAssertEqual(document.windowControllers.count, 2)
        XCTAssertEqual(backend.surfaces.count, 2)
        XCTAssertTrue(document.windowControllers.last?.window?.isVisible ?? false)
        XCTAssertTrue(backend.surfaces[0].performedCommands.isEmpty)

        let unsupported = NSMenuItem(
            title: "Bold",
            action: #selector(first.documentContentController.performEditorMenuCommand(_:)),
            keyEquivalent: ""
        )
        unsupported.tag = EVMenuCommand.bold.rawValue
        backend.surfaces[0].presentations[.bold] = EVMenuItemPresentation(
            isEnabled: false,
            state: .mixed,
            title: "Bold (Unavailable)"
        )

        XCTAssertFalse(first.documentContentController.validateMenuItem(unsupported))
        XCTAssertEqual(unsupported.state, .mixed)
        XCTAssertEqual(unsupported.title, "Bold (Unavailable)")
        first.documentContentController.performEditorMenuCommand(unsupported)
        XCTAssertEqual(backend.surfaces[0].performedCommands, [.bold])

        let undo = NSMenuItem(
            title: "Undo",
            action: #selector(first.documentContentController.performEditorMenuCommand(_:)),
            keyEquivalent: "z"
        )
        undo.tag = EVMenuCommand.undo.rawValue
        backend.surfaces[0].presentations[.undo] = EVMenuItemPresentation(
            isEnabled: true,
            title: "Undo Insert Text"
        )
        XCTAssertTrue(first.documentContentController.validateMenuItem(undo))
        XCTAssertEqual(undo.title, "Undo Insert Text")
    }
}
