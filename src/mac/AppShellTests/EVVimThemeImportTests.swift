import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
final class EVVimThemeImportTests: XCTestCase {
  private let night = Data("set background=dark\nhi clear\nlet g:colors_name = 'night'\nhi Normal guifg=#c0c0c0 guibg=#101010\nhi Comment guifg=#ff0000\n".utf8)

  private func directory() -> URL {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-vim-themes-\(UUID().uuidString)")
    try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    return directory
  }

  private func object(_ file: URL) throws -> [String: Any] {
    try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
  }

  func testImportCreatesSelectedThemesNamedByStemAndNumbersCollisions() throws {
    let profile = directory()
    let store = EVConfigurationStore(directory: profile, bundleResourceURL: nil)
    XCTAssertEqual(try store.importVimTheme(night, named: "night"), "night")
    XCTAssertEqual(store.currentThemeName, "night")
    XCTAssertEqual(try object(profile.appendingPathComponent("config.json"))["selectedTheme"] as? String, "night")
    let theme = try object(store.themesDirectory.appendingPathComponent("night.json"))
    let canvas = try XCTUnwrap((theme["theme"] as? [String: Any])?["background"] as? [String: Double])
    XCTAssertEqual(canvas["red"] ?? 0, 16.0 / 255, accuracy: 0.0001)
    XCTAssertEqual(try store.importVimTheme(night, named: "Night"), "Night (2)")
    XCTAssertEqual(store.currentThemeName, "Night (2)")
    // Default is reserved for the built-in theme.
    XCTAssertEqual(try store.importVimTheme(night, named: "default"), "default (2)")
    let before = store.availableThemeNames
    XCTAssertThrowsError(try store.importVimTheme(Data("plain prose\n".utf8), named: "prose")) { error in
      XCTAssertEqual(error.localizedDescription, "The file contains no Vim highlight commands")
    }
    XCTAssertEqual(store.availableThemeNames, before)
    XCTAssertEqual(store.currentThemeName, "default (2)")
  }

  func testColorSchemesAreDiscoveredInVimRuntimeOrderByStem() throws {
    let home = directory()
    let vim = home.appendingPathComponent(".vim/colors", isDirectory: true)
    let xdg = home.appendingPathComponent(".config/vim/colors", isDirectory: true)
    for folder in [vim, xdg, vim.appendingPathComponent("folder.vim")] {
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    }
    for file in [vim.appendingPathComponent("desert.vim"), vim.appendingPathComponent(".hidden.vim"),
                 vim.appendingPathComponent("notes.txt"), xdg.appendingPathComponent("desert.vim"),
                 xdg.appendingPathComponent("Blue.vim")] {
      try night.write(to: file)
    }
    let directories = EVVimColorSchemes.directories(home: home, environment: [:])
    XCTAssertEqual(directories.map(\.path), [vim.path, xdg.path])
    XCTAssertEqual(EVVimColorSchemes.directories(home: home, environment: ["XDG_CONFIG_HOME": "/tmp/xdg"])[1].path, "/tmp/xdg/vim/colors")
    let schemes = EVVimColorSchemes.available(in: directories)
    XCTAssertEqual(schemes.map(\.name), ["Blue", "desert"])
    XCTAssertEqual(schemes.last?.url.deletingLastPathComponent().resolvingSymlinksInPath().path,
      vim.resolvingSymlinksInPath().path)
    XCTAssertTrue(EVVimColorSchemes.available(in: [home.appendingPathComponent("missing")]).isEmpty)
  }

  func testDialogImportsTheSelectedSchemeOrAChosenFileAndReportsFailures() throws {
    let configuration = EVConfigurationStore(directory: directory(), bundleResourceURL: nil)
    let store = EVThemeStore(configuration: configuration)
    let colors = directory()
    try night.write(to: colors.appendingPathComponent("dusk.vim"))
    try night.write(to: colors.appendingPathComponent("zeta.vim"))
    let dialog = EVVimThemeImportController(store: store, directories: [colors])
    XCTAssertEqual(dialog.panel.title, "Import Vim Theme")
    XCTAssertEqual(dialog.schemes.map(\.name), ["dusk", "zeta"])
    let buttons = Set(descendants(try XCTUnwrap(dialog.panel.contentView)).compactMap { ($0 as? NSButton)?.title })
    XCTAssertEqual(buttons, ["Select File…", "Cancel", "Import"])
    XCTAssertTrue(dialog.importButton.isEnabled)
    // Names sit in the middle of their rows.
    let cell = try XCTUnwrap(dialog.tableView(dialog.table, viewFor: nil, row: 0) as? NSTableCellView)
    cell.frame = NSRect(x: 0, y: 0, width: 300, height: dialog.table.rowHeight)
    cell.layoutSubtreeIfNeeded()
    let label = try XCTUnwrap(cell.textField)
    XCTAssertEqual(label.stringValue, "dusk")
    XCTAssertEqual(label.frame.midY, cell.bounds.midY, accuracy: 0.5)
    var closed = false
    dialog.onClose = { closed = true }
    dialog.table.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)
    dialog.importButton.performClick(nil)
    XCTAssertTrue(closed)
    XCTAssertEqual(configuration.currentThemeName, "zeta")

    let custom = directory().appendingPathComponent("custom.vim")
    try night.write(to: custom)
    let chooser = EVVimThemeImportController(store: store, directories: [colors])
    chooser.chooseFile = { $0.importScheme(at: custom) }
    chooser.selectFileButton.performClick(nil)
    XCTAssertEqual(configuration.currentThemeName, "custom")

    let prose = directory().appendingPathComponent("prose.vim")
    try Data("prose\n".utf8).write(to: prose)
    let failing = EVVimThemeImportController(store: store, directories: [directory()])
    XCTAssertFalse(failing.importButton.isEnabled)
    var errors: [String] = []
    var failedClose = false
    failing.onClose = { failedClose = true }
    failing.presentError = { error, _ in errors.append(error.localizedDescription) }
    failing.importScheme(at: prose)
    XCTAssertEqual(errors, ["The file contains no Vim highlight commands"])
    XCTAssertFalse(failedClose)
    XCTAssertEqual(configuration.currentThemeName, "custom")
  }

  func testThemeSettingsImportButtonFollowsRevertAndOpensTheDialog() throws {
    let configuration = EVConfigurationStore(directory: directory())
    let controller = EVSettingsWindowController(store: EVThemeStore(configuration: configuration))
    defer { controller.close() }
    let root = try XCTUnwrap(controller.window?.contentView)
    let row = try XCTUnwrap(descendants(root).compactMap { $0 as? NSStackView }.first {
      $0.arrangedSubviews.contains { ($0 as? NSButton)?.title == "Import…" }
    })
    let titles = row.arrangedSubviews.compactMap { $0 as? NSButton }.filter { !($0 is NSPopUpButton) }.map(\.title)
    XCTAssertEqual(titles, ["New Theme…", "Revert", "Import…"])
    var presented: NSWindow?
    controller.themeActions.presentVimImport = { window, _ in presented = window }
    try XCTUnwrap(row.arrangedSubviews.compactMap { $0 as? NSButton }.first { $0.title == "Import…" }).performClick(nil)
    XCTAssertTrue(presented === controller.window)
  }

  private func descendants(_ view: NSView) -> [NSView] {
    view.subviews + view.subviews.flatMap(descendants)
  }
}
