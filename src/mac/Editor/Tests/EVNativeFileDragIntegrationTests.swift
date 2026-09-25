import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVNativeFileDragIntegrationTests: XCTestCase {
    @MainActor private final class Drag: NSObject, NSDraggingInfo {
        let draggingDestinationWindow: NSWindow?
        let draggingPasteboard: NSPasteboard
        let draggingLocation: NSPoint
        let draggingSourceOperationMask: NSDragOperation = [.copy, .move]
        var draggedImageLocation: NSPoint { draggingLocation }
        nonisolated var draggedImage: NSImage? { nil }
        var draggingSource: Any? { nil }
        var draggingSequenceNumber: Int { 1 }
        var draggingFormation: NSDraggingFormation = .none
        var animatesToDestination = false
        var numberOfValidItemsForDrop = 1
        var springLoadingHighlight: NSSpringLoadingHighlight { .none }

        init(file: URL, target: EVEditorView) throws {
            draggingDestinationWindow = try XCTUnwrap(target.window)
            draggingLocation = target.convert(NSPoint(x: target.bounds.midX, y: target.bounds.midY), to: nil)
            draggingPasteboard = NSPasteboard.withUniqueName()
            super.init()
            XCTAssertTrue(draggingPasteboard.writeObjects([file as NSURL]))
        }
        func slideDraggedImage(to screenPoint: NSPoint) {}
        nonisolated override func namesOfPromisedFilesDropped(atDestination dropDestination: URL) -> [String]? { nil }
        func resetSpringLoading() {}
        func enumerateDraggingItems(options: NSDraggingItemEnumerationOptions, for view: NSView?,
                                    classes: [AnyClass], searchOptions: [NSPasteboard.ReadingOptionKey: Any],
                                    using block: (NSDraggingItem, Int, UnsafeMutablePointer<ObjCBool>) -> Void) {}
    }

    private func waitUntil(_ message: String, _ condition: () -> Bool) {
        let deadline = Date(timeIntervalSinceNow: 5)
        // A synchronous identity lookup may replace the pane immediately;
        // still let the queued host completion finish before the next drag.
        repeat {
            RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.01))
        } while !condition() && Date() < deadline
        XCTAssertTrue(condition(), message)
    }

    private func loadedDocument(_ url: URL, type: String) throws -> (EVDocument, EVCoreDocumentBackend) {
        let backend = EVCoreDocumentBackend()
        let document = EVDocument(editorBackend: backend)
        try document.read(from: url, ofType: type)
        document.fileURL = url
        NSDocumentController.shared.addDocument(document)
        return (document, backend)
    }

    private func drop(_ url: URL, on surface: EVEditorSurfaceController) throws {
        let view = surface.editorView
        let drag = try Drag(file: url, target: view)
        defer { drag.draggingPasteboard.releaseGlobally() }
        XCTAssertTrue(view.window === drag.draggingDestinationWindow)
        XCTAssertTrue(view.registeredDraggedTypes.contains(.fileURL))
        XCTAssertEqual(view.draggingEntered(drag), .copy)
        XCTAssertEqual(view.draggingUpdated(drag), .copy)
        XCTAssertTrue(view.prepareForDragOperation(drag))
        XCTAssertTrue(view.performDragOperation(drag))
        view.concludeDragOperation(drag)
    }

    func testActualDragCallbacksReplaceCleanDocumentThenKeepDirtyTextInItsWindow() throws {
        EVFrontendRegistry.install { EVCoreDocumentBackend() }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-real-drag-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let originalURL = directory.appendingPathComponent("original.txt")
        let firstURL = directory.appendingPathComponent("first file café.txt")
        let secondURL = directory.appendingPathComponent("second.md")
        let originalBytes = Data("Original file stays unchanged".utf8)
        let firstBytes = Data("First dropped file\r\ncafé".utf8)
        let secondBytes = Data("## Second file\n\nParagraph".utf8)
        try originalBytes.write(to: originalURL)
        try firstBytes.write(to: firstURL)
        try secondBytes.write(to: secondURL)
        let (original, originalBackend) = try loadedDocument(originalURL, type: EVDocument.plainTextType)
        let (first, firstBackend) = try loadedDocument(firstURL, type: EVDocument.plainTextType)
        let (second, secondBackend) = try loadedDocument(secondURL, type: EVDocument.markdownType)
        defer {
            for document in [original, first, second] { document.close() }
            try? FileManager.default.removeItem(at: directory)
        }
        original.makeWindowControllers()
        let controller = try XCTUnwrap(original.windowControllers.first as? EVDocumentWindowController)
        controller.showWindow(nil)
        let originalSurface = try XCTUnwrap(controller.editorSurface as? EVEditorSurfaceController)
        XCTAssertTrue(try XCTUnwrap(controller.window).isVisible)
        XCTAssertFalse(originalBackend.persistenceState.isDirty)
        XCTAssertTrue(first.windowControllers.isEmpty)
        XCTAssertTrue(second.windowControllers.isEmpty)

        try drop(firstURL, on: originalSurface)
        waitUntil("The clean target must adopt the dropped document") { controller.activeDocument === first }
        XCTAssertTrue(controller.document === first)
        XCTAssertEqual(first.windowControllers.count, 1)
        let firstSurface = try XCTUnwrap(controller.editorSurface as? EVEditorSurfaceController)
        XCTAssertTrue(firstSurface.backend === firstBackend)
        XCTAssertEqual(firstSurface.formattedText, "First dropped file\ncafé")
        XCTAssertNil(firstSurface.commandOutput)
        XCTAssertEqual(try Data(contentsOf: originalURL), originalBytes)
        XCTAssertEqual(try originalBackend.serializedSource(typeName: EVDocument.plainTextType), originalBytes)

        let session = try XCTUnwrap(firstSurface.session)
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: 105)
        _ = try session.sendText("Unsaved draft: ")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        firstSurface.refreshPresentation()
        let draft = try firstBackend.serializedSource(typeName: EVDocument.plainTextType)
        XCTAssertTrue(firstBackend.persistenceState.isDirty)
        XCTAssertNotEqual(draft, firstBytes)
        try drop(secondURL, on: firstSurface)
        waitUntil("A dirty drop must open another native window") { second.windowControllers.count == 1 }
        XCTAssertTrue(controller.activeDocument === first)
        XCTAssertTrue(controller.editorSurface === firstSurface)
        XCTAssertEqual(try firstBackend.serializedSource(typeName: EVDocument.plainTextType), draft)
        XCTAssertTrue(firstBackend.persistenceState.isDirty)
        XCTAssertNil(firstSurface.commandOutput)
        let secondController = try XCTUnwrap(second.windowControllers.first as? EVDocumentWindowController)
        XCTAssertFalse(secondController === controller)
        XCTAssertTrue(try XCTUnwrap(secondController.window).isVisible)
        let secondSurface = try XCTUnwrap(secondController.editorSurface as? EVEditorSurfaceController)
        XCTAssertTrue(secondSurface.backend === secondBackend)
        // Markdown opens in its source-visible view.
        XCTAssertEqual(secondSurface.formattedText, "## Second file\nParagraph")
        XCTAssertEqual(try Data(contentsOf: firstURL), firstBytes)
        XCTAssertEqual(try Data(contentsOf: secondURL), secondBytes)
        firstSurface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try firstBackend.serializedSource(typeName: EVDocument.plainTextType), firstBytes)
        XCTAssertFalse(firstBackend.persistenceState.isDirty)
        XCTAssertEqual(try secondBackend.serializedSource(typeName: EVDocument.markdownType), secondBytes)
    }
}
