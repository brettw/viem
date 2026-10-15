import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVCodeStyleInheritanceTests: XCTestCase {
    func testSavedCanonicalStylesPreserveInheritanceWithoutRewritingTheFile() throws {
        let configuration = configuration()
        let parentColor: [String: Float] = ["red": 0.1, "green": 0.5, "blue": 0.3, "alpha": 1]
        let localColor: [String: Float] = ["red": 0.2, "green": 0.3, "blue": 0.8, "alpha": 1]
        let entries: [[String: Any]] = [
                ["id": "syntax:Comment", "name": "Project comments",
                 "properties": ["foreground": parentColor, "size": 27] as [String: Any]],
                ["id": "syntax:Comment.documentation", "name": "Comment.documentation",
                 "based_on": "syntax:Comment",
                 "properties": ["foreground": localColor]],
        ]
        let settings: [String: Any] = [
            "version": 3,
            "character_styles": entries,
            "suppressed_character_ids": ["syntax:Todo"],
        ]
        let saved = try JSONSerialization.data(withJSONObject: settings, options: [.sortedKeys])
        try configuration.saveCodeStyleSheet(saved)
        let file = try XCTUnwrap(configuration.selectedThemeURL)
        let savedTheme = try Data(contentsOf: file)
        let session = try EVCodeStyleSession(configuration: configuration)
        XCTAssertNil(session.lastError)
        let snapshot = try session.snapshot()
        let commentKey = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "syntax:Comment"))
        let comment = try XCTUnwrap(snapshot.definition(for: commentKey))
        let documentation = try definition(named: "Comment.documentation", in: snapshot)
        XCTAssertNil(comment.parentKey)
        XCTAssertEqual(documentation.parentKey, comment.key)
        XCTAssertEqual(comment.properties[.characterForeground]?.declared,
                       .color(EVStyleColor(red: 0.1, green: 0.5, blue: 0.3, alpha: 1)))
        XCTAssertEqual(comment.properties[.characterSize]?.declared, .float(27))
        XCTAssertEqual(documentation.properties[.characterSize]?.effective, .float(27))
        XCTAssertEqual(documentation.properties[.characterForeground]?.declared,
                       .color(EVStyleColor(red: 0.2, green: 0.3, blue: 0.8, alpha: 1)))
        XCTAssertFalse(snapshot.definitions.contains { $0.key.id.rawValue.contains("syntax:@") || $0.name == "Todo" })
        XCTAssertEqual(try Data(contentsOf: file), savedTheme, "Loading the settings file must not rewrite user settings")
        XCTAssertFalse(session.undoManager.canUndo)
    }

    private func configuration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-code-style-inheritance-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory, legacyDefaults: nil)
    }

    private func definition(named name: String, in snapshot: EVStyleSheetSnapshot) throws -> EVStyleDefinition {
        try XCTUnwrap(snapshot.definitions.first { $0.name == name })
    }

}
