import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVCodeStyleInheritanceTests: XCTestCase {
    func testLegacyCaptureAliasesCanonicalizeWithoutLosingLocalEditsOrRewritingTheFile() throws {
        let configuration = configuration()
        let parentColor: [String: Float] = ["red": 0.1, "green": 0.5, "blue": 0.3, "alpha": 1]
        let localColor: [String: Float] = ["red": 0.2, "green": 0.3, "blue": 0.8, "alpha": 1]
        let entries: [[String: Any]] = [
                ["id": "syntax:Comment", "name": "Project comments",
                 "properties": ["foreground": parentColor, "size": 21] as [String: Any]],
                ["id": "syntax:@comment", "name": "@comment",
                 "based_on": "syntax:Comment",
                 "properties": ["size": 27] as [String: Any]],
                ["id": "syntax:@comment.documentation", "name": "@comment.documentation",
                 "based_on": "syntax:@comment",
                 "properties": ["foreground": localColor]],
        ]
        let settings: [String: Any] = [
            "version": 2,
            "character_styles": entries,
            "suppressed_character_ids": ["syntax:Todo"],
        ]
        let saved = try JSONSerialization.data(withJSONObject: settings, options: [.sortedKeys])
        try FileManager.default.createDirectory(at: configuration.directory, withIntermediateDirectories: true)
        let file = configuration.directory.appendingPathComponent("code_style.json")
        try saved.write(to: file)
        let session = try EVCodeStyleSession(configuration: configuration)
        XCTAssertNil(session.lastError)
        let snapshot = try session.snapshot()
        let commentKey = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "syntax:Comment"))
        let comment = try XCTUnwrap(snapshot.definition(for: commentKey))
        let documentation = try definition(named: "Comment.documentation", in: snapshot)
        XCTAssertNil(comment.parentKey, "The former capture aliases the existing Comment definition")
        XCTAssertEqual(documentation.parentKey, comment.key)
        XCTAssertEqual(comment.properties[.characterForeground]?.declared,
                       .color(EVStyleColor(red: 0.1, green: 0.5, blue: 0.3, alpha: 1)))
        XCTAssertEqual(comment.properties[.characterSize]?.declared, .float(27),
                       "The former capture's explicit size takes precedence over its parent")
        XCTAssertEqual(documentation.properties[.characterSize]?.effective, .float(27))
        XCTAssertEqual(documentation.properties[.characterForeground]?.declared,
                       .color(EVStyleColor(red: 0.2, green: 0.3, blue: 0.8, alpha: 1)))
        XCTAssertFalse(snapshot.definitions.contains { $0.key.id.rawValue.contains("syntax:@") || $0.name == "Todo" })
        XCTAssertEqual(try Data(contentsOf: file), saved, "Loading the settings file must not rewrite user settings")
        XCTAssertFalse(session.undoManager.canUndo)
    }

    func testCommentParentEditsReachRustCapturesAndChildOverridesSurviveHistoryAndReload() async throws {
        let configuration = configuration()
        let source = Data("// Ordinary comment\n/// Documentation comment\nfn main() {}\n".utf8)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: source, typeName: EVDocument.plainTextType,
                         filename: "comments.rs", allowAutomaticCode: true)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 700, height: 400)
        surface.viewDidLayout()
        let before = try backend.recoverySnapshot()
        let session = try EVCodeStyleSession(configuration: configuration)
        let initial = try session.snapshot()
        let comment = try definition(named: "Comment", in: initial)
        let documentation = try definition(named: "Comment.documentation", in: initial)
        XCTAssertEqual(comment.key.id.rawValue, "syntax:Comment")
        XCTAssertNil(comment.parentKey)
        XCTAssertEqual(documentation.parentKey, comment.key)
        XCTAssertNil(documentation.properties[.characterForeground]?.declared)
        XCTAssertEqual(documentation.properties[.characterForeground]?.contributor, comment.key)
        XCTAssertFalse(initial.definitions.contains { $0.name.hasPrefix("@") })

        let inheritedColor = EVStyleColor(red: 0.15, green: 0.55, blue: 0.25, alpha: 1)
        try editAppearance(session, key: comment.key, size: 23, color: inheritedColor)
        try await waitForComments(backend: backend, surface: surface,
                                  ordinary: (23, inheritedColor), documentation: (23, inheritedColor))
        let inherited = try session.snapshot()
        for key in [comment.key, documentation.key] {
            let style = try XCTUnwrap(inherited.definition(for: key))
            XCTAssertEqual(style.properties[.characterSize]?.effective, .float(23))
            XCTAssertEqual(style.properties[.characterSize]?.contributor, comment.key)
            XCTAssertEqual(style.properties[.characterForeground]?.effective, .color(inheritedColor))
        }

        let localColor = EVStyleColor(red: 0.2, green: 0.3, blue: 0.8, alpha: 1)
        try editAppearance(session, key: documentation.key, size: 31, color: localColor)
        let changedParentColor = EVStyleColor(red: 0.75, green: 0.2, blue: 0.3, alpha: 1)
        try editAppearance(session, key: comment.key, size: 26, color: changedParentColor)
        try await waitForComments(backend: backend, surface: surface,
                                  ordinary: (26, changedParentColor), documentation: (31, localColor))
        XCTAssertEqual(try session.snapshot().definition(for: documentation.key)?
            .properties[.characterSize]?.contributor, documentation.key)
        XCTAssertEqual(try fontSize(at: 46, surface: surface), 14, accuracy: 0.01,
                       "The Rust function below the comments retains its own font size")

        session.undoManager.undo()
        try await waitForComments(backend: backend, surface: surface,
                                  ordinary: (23, inheritedColor), documentation: (31, localColor))
        session.undoManager.redo()
        try await waitForComments(backend: backend, surface: surface,
                                  ordinary: (26, changedParentColor), documentation: (31, localColor))

        // Removing declarations resumes inheritance immediately. The undo
        // restores the local declarations, not a flattened parent appearance.
        try session.beginGroup()
        for property in [EVStyleProperty.characterSize, .characterForeground] {
            try session.edit(key: documentation.key, expected: session.snapshot().identity,
                             mutation: .clearDeclaration(property))
        }
        session.endGroup()
        try await waitForComments(backend: backend, surface: surface,
                                  ordinary: (26, changedParentColor), documentation: (26, changedParentColor))
        session.undoManager.undo()
        try await waitForComments(backend: backend, surface: surface,
                                  ordinary: (26, changedParentColor), documentation: (31, localColor))

        let saved = try XCTUnwrap(configuration.codeStyleSheet())
        XCTAssertEqual(viem_code_replace_style_json(nil, 0), UInt32(VIEM_STATUS_OK))
        XCTAssertEqual(saved.withUnsafeBytes {
            viem_code_replace_style_json($0.bindMemory(to: UInt8.self).baseAddress, UInt64($0.count))
        }, UInt32(VIEM_STATUS_OK))
        let restored = try session.snapshot()
        XCTAssertNil(restored.definition(for: comment.key)?.parentKey)
        XCTAssertEqual(restored.definition(for: documentation.key)?.parentKey, comment.key)
        XCTAssertEqual(restored.definition(for: documentation.key)?.properties[.characterSize]?.declared, .float(31))
        try await waitForComments(backend: backend, surface: surface,
                                  ordinary: (26, changedParentColor), documentation: (31, localColor))
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), source)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(surface.canUndo)
        XCTAssertFalse(surface.canRedo)
    }

    func testBuiltinCodeDeclarationCanBeUncheckedAndStaysClearedAfterReload() throws {
        let configuration = configuration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let initial = try session.snapshot()
        let comment = try definition(named: "Comment", in: initial)
        let documentation = try definition(named: "Comment.documentation", in: initial)
        XCTAssertEqual(documentation.parentKey, comment.key)
        XCTAssertNotNil(comment.properties[.characterForeground]?.declared)
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(codeSession: session)
        editor.selectStyle(comment.key)
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        let checkbox = try XCTUnwrap(descendants(editor.view).compactMap { $0 as? NSButton }
            .first { $0.accessibilityLabel() == "Override foreground" })
        XCTAssertEqual(checkbox.state, .on)
        checkbox.performClick(nil)
        XCTAssertEqual(checkbox.state, .off, editor.inspection.diagnostic)
        let changed = try session.snapshot()
        XCTAssertNil(changed.definition(for: comment.key)?.properties[.characterForeground]?.declared)
        XCTAssertEqual(changed.definition(for: comment.key)?.properties[.characterForeground]?.effective,
                       changed.definition(for: .baseParagraph)?.properties[.characterForeground]?.effective)
        XCTAssertEqual(changed.definition(for: documentation.key)?.properties[.characterForeground]?.effective,
                       changed.definition(for: .baseParagraph)?.properties[.characterForeground]?.effective)
        let saved = try XCTUnwrap(configuration.codeStyleSheet())
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: saved) as? [String: Any])
        XCTAssertEqual(object["version"] as? Int, 3)
        XCTAssertFalse(String(decoding: saved, as: UTF8.self).contains("syntax:@"))
        XCTAssertEqual(viem_code_replace_style_json(nil, 0), UInt32(VIEM_STATUS_OK))
        XCTAssertEqual(saved.withUnsafeBytes {
            viem_code_replace_style_json($0.bindMemory(to: UInt8.self).baseAddress, UInt64($0.count))
        }, UInt32(VIEM_STATUS_OK))
        let restored = try session.snapshot()
        XCTAssertNil(restored.definition(for: comment.key)?.properties[.characterForeground]?.declared)
        XCTAssertEqual(restored.definition(for: documentation.key)?.parentKey, comment.key)
        XCTAssertEqual(restored.definition(for: documentation.key)?.properties[.characterForeground]?.effective,
                       restored.definition(for: .baseParagraph)?.properties[.characterForeground]?.effective)
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

    private func editAppearance(_ session: EVCodeStyleSession, key: EVStyleKey,
                                size: Float, color: EVStyleColor) throws {
        try session.beginGroup()
        defer { session.endGroup() }
        try session.edit(key: key, expected: session.snapshot().identity,
                         mutation: .setDeclaration(.characterSize, .float(size)))
        try session.edit(key: key, expected: session.snapshot().identity,
                         mutation: .setDeclaration(.characterForeground, .color(color)))
    }

    private func fontSize(at position: UInt64, surface: EVEditorSurfaceController) throws -> CGFloat {
        let cluster = try XCTUnwrap(surface.layoutSnapshot?.clusters.first {
            $0.text_start <= position && $0.text_end > position
        })
        let registry = try XCTUnwrap(surface.session).provider.renderRegistry
        return try XCTUnwrap(registry.enAdvance(identifier: cluster.render_run.identifier,
                                               metricsGeneration: cluster.render_run.metrics_generation)) * 2
    }

    private func appearanceMatches(at position: UInt64, size: CGFloat, color: EVStyleColor,
                                   surface: EVEditorSurfaceController) -> Bool {
        guard let actualSize = try? fontSize(at: position, surface: surface),
              abs(actualSize - size) < 0.01,
              let paint = surface.layoutPaint?.runs.first(where: {
                  $0.text_start <= position && $0.text_end > position
              })?.paint,
              paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0 else { return false }
        return abs(paint.foreground.red - color.red) < 0.0001
            && abs(paint.foreground.green - color.green) < 0.0001
            && abs(paint.foreground.blue - color.blue) < 0.0001
            && abs(paint.foreground.alpha - color.alpha) < 0.0001
    }

    private func waitForComments(backend: EVCoreDocumentBackend, surface: EVEditorSurfaceController,
                                 ordinary: (CGFloat, EVStyleColor),
                                 documentation: (CGFloat, EVStyleColor)) async throws {
        for _ in 0..<300 {
            backend.pollSyntax()
            surface.refreshPresentation()
            if appearanceMatches(at: 3, size: ordinary.0, color: ordinary.1, surface: surface),
               appearanceMatches(at: 24, size: documentation.0, color: documentation.1, surface: surface) {
                let names = try backend.syntaxStyleNames()
                XCTAssertTrue(names.contains("Comment"))
                XCTAssertTrue(names.contains("Comment.documentation"))
                let layout = try XCTUnwrap(surface.layoutSnapshot)
                let paint = try XCTUnwrap(surface.layoutPaint)
                XCTAssertTrue(paint.info.identity.isSameLayout(as: layout.info.identity))
                XCTAssertGreaterThan(CGFloat(layout.rows[0].line_advance), ordinary.0)
                XCTAssertGreaterThan(CGFloat(layout.rows[1].line_advance), documentation.0)
                XCTAssertNil(surface.commandOutput, surface.statusBarState.message)
                return
            }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Rust comments did not acquire inherited or overridden glyphs and paint: \(surface.statusBarState.message)")
    }
}
