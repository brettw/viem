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
        let menu = try XCTUnwrap(main.item(withTitle: "Format")?.submenu)
        XCTAssertEqual(Array(menu.items.prefix(4)).map { $0.isSeparatorItem ? "-" : $0.title }, ["Plain text", "Markdown Preview", "Code", "-"])
        XCTAssertTrue(menu.showsStateColumn)
        let code = try XCTUnwrap(menu.item(withTitle: "Code"))
        XCTAssertNil(code.submenu)
        let languages = try XCTUnwrap(main.item(withTitle: "File")?.submenu?.item(withTitle: "Code language")?.submenu)
        XCTAssertTrue(languages.showsStateColumn)
        XCTAssertEqual(languages.items[0].title, "Auto (Rust)")
        XCTAssertEqual(languages.items[1].title, "Obscure languages")
        let obscure = try XCTUnwrap(languages.items[1].submenu)
        XCTAssertTrue(obscure.showsStateColumn)
        XCTAssertTrue(languages.items[2].isSeparatorItem)
        XCTAssertEqual(Array(languages.items.dropFirst(3)).map(\.title), EVCodeLanguage.primaryLanguages.map(\.name))
        XCTAssertEqual(obscure.items.map(\.title), EVCodeLanguage.obscureLanguages.map(\.name))
        XCTAssertNotNil(languages.item(withTitle: "Objective-C"))
        XCTAssertNil(languages.item(withTitle: "Zsh"))
        XCTAssertNotNil(obscure.item(withTitle: "Zsh"))
        XCTAssertNil(languages.item(withTitle: "Zig"))
        XCTAssertNotNil(obscure.item(withTitle: "Zig"))
        XCTAssertEqual(code.state, .on)
        XCTAssertEqual(languages.items[0].state, .on)
        try choose(try XCTUnwrap(menu.item(withTitle: "Plain text")), builder: builder)
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(surface.backend.sourceFormat, .plainText)
        XCTAssertEqual(menu.item(withTitle: "Plain text")?.state, .on)
        XCTAssertEqual(code.state, .off)
        XCTAssertEqual(languages.items[0].state, .off)
        try choose(try XCTUnwrap(obscure.item(withTitle: "Zig")), builder: builder)
        builder.menuNeedsUpdate(obscure)
        XCTAssertEqual(surface.backend.sourceFormat, .code)
        XCTAssertEqual(obscure.item(withTitle: "Zig")?.state, .on)
        XCTAssertEqual(languages.items[1].state, .on)
        try choose(try XCTUnwrap(languages.item(withTitle: "Python")), builder: builder)
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(surface.backend.sourceFormat, .code)
        XCTAssertEqual(code.state, .on)
        XCTAssertEqual(languages.item(withTitle: "Python")?.state, .on)
        XCTAssertEqual(obscure.item(withTitle: "Zig")?.state, .off)
        XCTAssertEqual(languages.items[1].state, .off)
        XCTAssertEqual(languages.items[0].title, "Auto (Rust)")
        try choose(try XCTUnwrap(menu.item(withTitle: "Plain text")), builder: builder)
        builder.menuNeedsUpdate(menu)
        try choose(code, builder: builder)
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(surface.backend.sourceFormat, .code)
        XCTAssertEqual(languages.item(withTitle: "Python")?.state, .on,
                       "Selecting the Code document type retains the language choice")
        try choose(languages.items[0], builder: builder)
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(languages.item(withTitle: "Python")?.state, .off)
        XCTAssertEqual(languages.items[0].state, .on)
        XCTAssertFalse(surface.backend.persistenceState.isDirty)
        XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.codeType), Data(source.utf8))
        XCTAssertEqual(languages.item(withTitle: "Rust")?.state, .off, "Auto is the selection, not a forced Rust override")
    }

    func testAutoFallbackAndMarkdownCodeDistinctionAndTrackedItemIdentity() throws {
        let surface = try surface("# Heading", filename: "notes.md", type: EVDocument.markdownType)
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner, documentModeProvider: { surface })
        let main = builder.buildMainMenu(for: .shared)
        let menu = try XCTUnwrap(main.item(withTitle: "Format")?.submenu)
        let code = try XCTUnwrap(menu.item(withTitle: "Code"))
        let languages = try XCTUnwrap(main.item(withTitle: "File")?.submenu?.item(withTitle: "Code language")?.submenu)
        XCTAssertEqual(menu.item(withTitle: "Markdown Preview")?.state, .on)
        try choose(languages.items[0], builder: builder)
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(surface.backend.sourceFormat, .code)
        XCTAssertEqual(languages.items[0].title, "Auto (Markdown)")
        XCTAssertEqual(code.state, .on)
        try surface.backend.configuration.setMarkdownFormattedView(true)
        try choose(try XCTUnwrap(menu.item(withTitle: "Markdown Preview")), builder: builder)
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
        try choose(code, builder: builder)
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(surface.backend.sourceFormat, .code,
                       "Code is a document type even when no language is detected")
        XCTAssertEqual(code.state, .on)
        XCTAssertEqual(languages.items[0].state, .on)
        XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.codeType), Data("# Heading".utf8))
    }

    func testFlyoutRetainsCapturedRevisionWhileParentTracksAndDisablesWithoutDocument() throws {
        final class Provider: EVDocumentModeMenuProviding {
            var state: EVDocumentModeState?
            var selected: EVDocumentModeChoice?
            var expected: EVDocumentModeState?
            func currentDocumentMode() -> EVDocumentModeState? { state }
            func selectDocumentMode(_ choice: EVDocumentModeChoice, expected: EVDocumentModeState) {
                selected = choice
                self.expected = expected
            }
        }
        let provider = Provider()
        provider.state = EVDocumentModeState(documentId: 7, documentRevision: 1, format: "code",
            automatic: false, language: "zig", detectedLanguage: "zig", detectedName: "Zig")
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner, documentModeProvider: { provider })
        let main = builder.buildMainMenu(for: .shared)
        let file = try XCTUnwrap(main.item(withTitle: "File")?.submenu)
        let format = try XCTUnwrap(main.item(withTitle: "Format")?.submenu)
        let languages = try XCTUnwrap(file.item(withTitle: "Code language")?.submenu)
        let obscureItem = try XCTUnwrap(languages.item(withTitle: "Obscure languages"))
        let obscure = try XCTUnwrap(obscureItem.submenu)
        let zig = try XCTUnwrap(obscure.item(withTitle: "Zig"))
        builder.menuWillOpen(file)
        provider.state = EVDocumentModeState(documentId: 7, documentRevision: 2, format: "code",
            automatic: false, language: "python", detectedLanguage: "zig", detectedName: "Zig")
        builder.menuWillOpen(languages)
        builder.menuWillOpen(obscure)
        builder.menuNeedsUpdate(obscure)
        XCTAssertEqual(zig.state, .on)
        XCTAssertEqual(obscureItem.state, .on)
        try choose(zig, builder: builder)
        XCTAssertEqual(provider.selected, .code("zig"))
        XCTAssertEqual(provider.expected?.documentRevision, 1)
        builder.menuDidClose(obscure)
        builder.menuDidClose(languages)
        builder.menuDidClose(file)
        builder.menuNeedsUpdate(obscure)
        XCTAssertEqual(zig.state, .off)
        XCTAssertEqual(obscureItem.state, .off)
        try choose(zig, builder: builder)
        XCTAssertEqual(provider.expected?.documentRevision, 2)
        provider.state = nil
        builder.menuNeedsUpdate(format)
        XCTAssertTrue(format.items.prefix(3).allSatisfy { !$0.isEnabled && !builder.validateMenuItem($0) })
        XCTAssertFalse(try XCTUnwrap(file.item(withTitle: "Code language")).isEnabled)
        XCTAssertFalse(obscureItem.isEnabled)
        XCTAssertTrue(obscure.items.allSatisfy { !$0.isEnabled && !builder.validateMenuItem($0) })
        XCTAssertTrue(languages.items.dropFirst(3).allSatisfy { !$0.isEnabled && !builder.validateMenuItem($0) })
    }
}
