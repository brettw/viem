import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVIncrementalMatchStyleTests: XCTestCase {
    private let key = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "* Incremental match"))

    private func configuration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-match-style-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory)
    }

    func testInternalStyleIsEditableButAbsentFromApplicationMenusForEveryFormat() throws {
        for (type, source) in [
            (EVDocument.plainTextType, "one two"),
            (EVDocument.markdownType, "one **two**"),
            (EVDocument.markdownSourceType, "one **two**"),
            (EVDocument.rtfType, #"{\rtf1 one {\b two}}"#),
            (EVDocument.codeType, "one two"),
        ] {
            let configuration = configuration()
            let backend = EVCoreDocumentBackend(configuration: configuration)
            let original = Data(source.utf8)
            try backend.read(source: original, typeName: type)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            let snapshot = try backend.styleSheetSnapshot()
            let definition = try XCTUnwrap(snapshot.definition(for: key))
            XCTAssertEqual(definition.name, "Incremental match")
            XCTAssertTrue(definition.flags.contains(.internalSyntax))
            XCTAssertTrue(definition.capabilities.contains(.declarations))
            XCTAssertFalse(definition.capabilities.contains(.assign))
            let menu = try XCTUnwrap(surface.currentStyleMenuCatalogue())
            XCTAssertFalse(menu.entries.contains { $0.stableID == key.id.rawValue })

            let editor = EVStyleEditorViewController()
            if type == EVDocument.codeType {
                editor.retarget(settingsSession: try EVCodeStyleSession(configuration: configuration))
                editor.selectStyle(key)
            } else {
                editor.retarget(document: surface, styleKey: key)
            }
            XCTAssertEqual(editor.inspection.selectedStyleKey, key)
            let color = EVStyleColor(red: 0.3, green: 0.5, blue: 0.9, alpha: 0.7)
            XCTAssertTrue(editor.setPropertyForTesting(.characterBackground, value: .color(color)), editor.inspection.diagnostic)
            XCTAssertEqual(try backend.serializedSource(typeName: type), original)
            XCTAssertFalse(backend.persistenceState.isDirty)
            let current = try XCTUnwrap(surface.currentStyleMenuCatalogue())
            surface.perform(styleMenuAction: EVStyleMenuAction(
                kind: .assign, role: .character, stableID: key.id.rawValue,
                documentID: current.documentID, documentRevision: current.documentRevision,
                styleSheetRevision: current.styleSheetRevision), sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), original)
            XCTAssertFalse(backend.persistenceState.isDirty)

            if type != EVDocument.codeType {
                try configuration.saveStyleDefaults(backend.exportStyleDefaults(), named: backend.sourceFormat.defaultStyleName)
            }
            let reopened = EVCoreDocumentBackend(configuration: configuration)
            try reopened.read(source: original, typeName: type)
            XCTAssertEqual(try reopened.styleSheetSnapshot().definition(for: key)?.properties[.characterBackground]?.effective, .color(color))
            XCTAssertEqual(try reopened.serializedSource(typeName: type), original)
            XCTAssertFalse(reopened.persistenceState.isDirty)
        }
    }
}
