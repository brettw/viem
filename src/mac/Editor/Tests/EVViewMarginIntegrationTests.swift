import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVViewMarginIntegrationTests: XCTestCase {
    func testViewMarginNotificationUpdatesOpenViewsWithoutChangingTheirDocuments() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-view-margins-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory)
        let preferences = EVViewPreferences(configuration: configuration)
        let original = preferences.margins
        var backends: [EVCoreDocumentBackend] = []
        var surfaces: [EVEditorSurfaceController] = []
        for _ in 0..<2 {
            let backend = EVCoreDocumentBackend(configuration: configuration)
            try backend.read(source: Data("A paragraph with words to wrap in this view.".utf8), typeName: EVDocument.markdownType)
            let surface = EVEditorSurfaceController(backend: backend, viewPreferences: preferences)
            surface.loadViewIfNeeded()
            surface.view.frame = NSRect(x: 0, y: 0, width: 500, height: 300)
            surface.viewDidLayout()
            backends.append(backend)
            surfaces.append(surface)
        }
        let before = try backends.map { try $0.recoverySnapshot() }
        let margins = EVViewMargins(top: original.top == 41 ? 42 : 41, left: 51, bottom: 31, right: 61)
        XCTAssertTrue(preferences.setMargins(margins))
        for (index, surface) in surfaces.enumerated() {
            let insets = try XCTUnwrap(surface.layoutSnapshot).info.content_insets
            XCTAssertEqual(insets.top, Float(margins.top))
            XCTAssertEqual(insets.left, Float(margins.left))
            XCTAssertEqual(insets.bottom, Float(margins.bottom))
            XCTAssertEqual(insets.right, Float(margins.right))
            XCTAssertEqual(try backends[index].recoverySnapshot(), before[index])
            XCTAssertFalse(surface.canUndo)
            XCTAssertFalse(backends[index].persistenceState.isDirty)
        }
    }
}
