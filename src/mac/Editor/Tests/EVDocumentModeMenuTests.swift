import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVDocumentModeMenuTests: XCTestCase {
    private final class Owner: NSObject, EVApplicationCommandRouting {
        func newDocument(_ sender: Any?) {}
        func openDocument(_ sender: Any?) {}
        func openRecentDocument(_ sender: Any?) {}
        func clearRecentDocuments(_ sender: Any?) {}
        func showSettings(_ sender: Any?) {}
        func showThemeSettings(_ sender: Any?) {}
        func showHelpItem(_ sender: Any?) {}
    }
    private func surface(_ source: String, filename: String, type: String? = nil) throws -> EVEditorSurfaceController {
        let profile = FileManager.default.temporaryDirectory.appendingPathComponent("viem-mode-menu-\(UUID())")
        addTeardownBlock { try? FileManager.default.removeItem(at: profile) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: profile))
        try backend.read(source: Data(source.utf8), typeName: type ?? EVDocument.plainTextType,
                         filename: filename, allowAutomaticCode: type == nil)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        return surface
    }
    private func choose(_ item: NSMenuItem, builder: EVMenuBuilder) throws {
        XCTAssertTrue(builder.validateMenuItem(item))
        XCTAssertTrue(NSApplication.shared.sendAction(try XCTUnwrap(item.action), to: item.target, from: item))
    }

    func testMenusShowDetectedLanguageAlignChecksAndApplyOverridesWithoutChangingSource() throws {
        let source = "# Heading\r\n\r\n**words**\r\n"
        let surface = try surface(source, filename: "notes.rs")
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner, documentModeProvider: { surface })
        let main = builder.buildMainMenu(for: .shared)
        let menu = try XCTUnwrap(main.item(withTitle: "View")?.submenu)
        XCTAssertEqual(Array(menu.items.prefix(4)).map { $0.isSeparatorItem ? "-" : $0.title }, ["Plain text", "Markdown", "Code", "-"])
        XCTAssertTrue(menu.showsStateColumn)
        let code = try XCTUnwrap(menu.item(withTitle: "Code"))
        let languages = try XCTUnwrap(code.submenu)
        XCTAssertTrue(languages.showsStateColumn)
        XCTAssertEqual(languages.items[0].title, "Auto (Rust)")
        XCTAssertTrue(languages.items[1].isSeparatorItem)
        XCTAssertEqual(Array(languages.items.dropFirst(2)).map(\.title), EVCodeLanguage.all.map(\.name))
        XCTAssertEqual(code.state, .on)
        XCTAssertEqual(languages.items[0].state, .on)
        try choose(try XCTUnwrap(menu.item(withTitle: "Plain text")), builder: builder)
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(surface.backend.sourceFormat, .plainText)
        XCTAssertEqual(menu.item(withTitle: "Plain text")?.state, .on)
        XCTAssertEqual(code.state, .off)
        XCTAssertEqual(languages.items[0].state, .off)
        try choose(try XCTUnwrap(languages.item(withTitle: "Python")), builder: builder)
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(surface.backend.sourceFormat, .code)
        XCTAssertEqual(code.state, .on)
        XCTAssertEqual(languages.item(withTitle: "Python")?.state, .on)
        XCTAssertEqual(languages.items[0].title, "Auto (Rust)")
        try choose(languages.items[0], builder: builder)
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(languages.item(withTitle: "Python")?.state, .off)
        XCTAssertEqual(languages.items[0].state, .on)
        XCTAssertFalse(surface.backend.persistenceState.isDirty)
        XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.codeType), Data(source.utf8))
        XCTAssertEqual(languages.item(withTitle: "Rust")?.state, .off, "Auto is the selection, not a forced Rust override")
    }

    func testAutoFallbackAndMarkdownCodeDistinctionAndTrackedItemIdentity() throws {
        let surface = try surface("# Heading", filename: "notes.md", type: EVDocument.markdownSourceType)
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner, documentModeProvider: { surface })
        let menu = try XCTUnwrap(builder.buildMainMenu(for: .shared).item(withTitle: "View")?.submenu)
        let code = try XCTUnwrap(menu.item(withTitle: "Code"))
        let languages = try XCTUnwrap(code.submenu)
        XCTAssertEqual(menu.item(withTitle: "Markdown")?.state, .on)
        try choose(languages.items[0], builder: builder)
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(surface.backend.sourceFormat, .code)
        XCTAssertEqual(languages.items[0].title, "Auto (Markdown)")
        XCTAssertEqual(code.state, .on)
        try surface.backend.configuration.setMarkdownFormattedView(true)
        try choose(try XCTUnwrap(menu.item(withTitle: "Markdown")), builder: builder)
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(surface.backend.sourceFormat, .markdown)
        XCTAssertEqual(try surface.backend.formattedText(), "Heading")
        builder.menuWillOpen(languages)
        let items = languages.items
        builder.menuNeedsUpdate(languages)
        XCTAssertTrue(zip(items, languages.items).allSatisfy { $0 === $1 })
        builder.menuDidClose(languages)

        surface.backend.updateFilename("notes.unknown")
        builder.menuNeedsUpdate(menu)
        try choose(languages.items[0], builder: builder)
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(surface.backend.sourceFormat, .plainText)
        XCTAssertEqual(menu.item(withTitle: "Plain text")?.state, .on)
        XCTAssertEqual(code.state, .off)
        XCTAssertEqual(languages.items[0].title, "Auto (Plain Text)")
        XCTAssertEqual(languages.items[0].state, .on)
    }
}
