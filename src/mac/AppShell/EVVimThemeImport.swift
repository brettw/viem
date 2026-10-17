import AppKit
import UniformTypeIdentifiers

/// A Vim color scheme file, named by its file stem as `:colorscheme` names it.
public struct EVVimColorScheme: Equatable {
  public let name: String
  public let url: URL
}

/// Discovery of the user's Vim color schemes. Evaluation and theme mapping are
/// portable core policy; this only lists and reads files.
@MainActor
public enum EVVimColorSchemes {
  /// `~/.vim/colors`, then Vim 9.1's XDG location `$XDG_CONFIG_HOME/vim/colors`
  /// (`~/.config/vim/colors` by default), in Vim's 'runtimepath' order.
  nonisolated public static func directories(home: URL = FileManager.default.homeDirectoryForCurrentUser,
                                 environment: [String: String] = ProcessInfo.processInfo.environment) -> [URL] {
    let configuration = environment["XDG_CONFIG_HOME"].flatMap { $0.hasPrefix("/") ? URL(fileURLWithPath: $0, isDirectory: true) : nil }
      ?? home.appendingPathComponent(".config", isDirectory: true)
    return [home.appendingPathComponent(".vim/colors", isDirectory: true),
            configuration.appendingPathComponent("vim/colors", isDirectory: true)]
  }

  /// Regular `.vim` files, sorted by name. A name found in an earlier directory
  /// hides later ones, as `:colorscheme` loads the first.
  public static func available(in directories: [URL], manager: FileManager = .default) -> [EVVimColorScheme] {
    var seen = Set<String>()
    var result: [EVVimColorScheme] = []
    for directory in directories {
      let files = (try? manager.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.isRegularFileKey],
        options: [.skipsHiddenFiles])) ?? []
      for file in files.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) where file.pathExtension == "vim" {
        guard (try? file.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true else { continue }
        let name = file.deletingPathExtension().lastPathComponent
        guard !name.isEmpty, seen.insert(name).inserted else { continue }
        result.append(EVVimColorScheme(name: name, url: file))
      }
    }
    return result.sorted {
      let order = $0.name.localizedCaseInsensitiveCompare($1.name)
      return order == .orderedSame ? $0.name < $1.name : order == .orderedAscending
    }
  }

  static func read(_ url: URL) throws -> Data {
    let handle = try FileHandle(forReadingFrom: url)
    defer { try? handle.close() }
    let data = try handle.read(upToCount: EVThemeFile.maximumVimColorSchemeSize + 1) ?? Data()
    guard data.count <= EVThemeFile.maximumVimColorSchemeSize else {
      throw EVConfigurationStore.invalid("The Vim color scheme is larger than 4 MiB")
    }
    return data
  }
}

/// The Import Vim Theme dialog: a list of discovered color schemes with Select
/// File…, Cancel and Import. Importing creates and selects a new theme.
@MainActor
final class EVVimThemeImportController: NSObject, NSTableViewDataSource, NSTableViewDelegate {
  private static var presented: EVVimThemeImportController?

  let panel: NSPanel
  let table = NSTableView()
  let importButton = NSButton(title: "Import", target: nil, action: nil)
  let selectFileButton = NSButton(title: "Select File…", target: nil, action: nil)
  let schemes: [EVVimColorScheme]
  private let store: EVThemeStore
  private let directories: [URL]
  private weak var parent: NSWindow?
  /// Replaceable so tests need not run an open panel.
  var chooseFile: (EVVimThemeImportController) -> Void = { $0.runOpenPanel() }
  var presentError: (Error, NSWindow) -> Void = { error, window in
    NSAlert(error: error).beginSheetModal(for: window)
  }
  var onClose: (() -> Void)?

  init(store: EVThemeStore, directories: [URL] = EVVimColorSchemes.directories()) {
    self.store = store
    self.directories = directories
    schemes = EVVimColorSchemes.available(in: directories)
    panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 440, height: 380),
      styleMask: [.titled, .resizable], backing: .buffered, defer: false)
    super.init()
    build()
  }

  /// Show the dialog as a sheet on `window`, or as a modal panel without one.
  static func present(for window: NSWindow?, store: EVThemeStore) {
    let controller = EVVimThemeImportController(store: store)
    presented = controller
    controller.onClose = { presented = nil }
    controller.begin(for: window)
  }

  func begin(for window: NSWindow?) {
    parent = window
    if let window {
      window.beginSheet(panel)
    } else {
      panel.center()
      NSApp.runModal(for: panel)
    }
  }

  private func build() {
    panel.title = "Import Vim Theme"
    panel.contentMinSize = NSSize(width: 380, height: 300)
    panel.isReleasedWhenClosed = false
    let home = FileManager.default.homeDirectoryForCurrentUser.path
    let places = directories.map { $0.path.hasPrefix(home + "/") ? "~" + $0.path.dropFirst(home.count) : $0.path }
    let message = NSTextField(wrappingLabelWithString: schemes.isEmpty
      ? "No Vim color schemes were found in \(places.joined(separator: " or ")). Select a .vim file to import."
      : "Choose a color scheme from \(places.joined(separator: " or ")), or select another .vim file. Its GUI colors become a new theme.")
    message.textColor = .secondaryLabelColor

    let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("name"))
    column.title = "Color scheme"
    table.addTableColumn(column)
    table.headerView = nil
    table.usesAlternatingRowBackgroundColors = true
    table.allowsEmptySelection = true
    table.dataSource = self
    table.delegate = self
    table.target = self
    table.doubleAction = #selector(importSelection(_:))
    table.setAccessibilityLabel("Vim color schemes")
    let scroll = NSScrollView()
    scroll.documentView = table
    scroll.hasVerticalScroller = true
    scroll.borderType = .bezelBorder
    scroll.translatesAutoresizingMaskIntoConstraints = false

    selectFileButton.target = self
    selectFileButton.action = #selector(selectFile(_:))
    let cancel = NSButton(title: "Cancel", target: self, action: #selector(cancel(_:)))
    cancel.keyEquivalent = "\u{1b}"
    importButton.target = self
    importButton.action = #selector(importSelection(_:))
    importButton.keyEquivalent = "\r"
    let spacer = NSView()
    spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
    let buttons = NSStackView(views: [selectFileButton, spacer, cancel, importButton])
    buttons.spacing = 8

    let stack = NSStackView(views: [message, scroll, buttons])
    stack.orientation = .vertical
    stack.alignment = .leading
    stack.spacing = 12
    stack.edgeInsets = NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
    stack.translatesAutoresizingMaskIntoConstraints = false
    let content = NSView()
    content.addSubview(stack)
    panel.contentView = content
    NSLayoutConstraint.activate([
      stack.leadingAnchor.constraint(equalTo: content.leadingAnchor),
      stack.trailingAnchor.constraint(equalTo: content.trailingAnchor),
      stack.topAnchor.constraint(equalTo: content.topAnchor),
      stack.bottomAnchor.constraint(equalTo: content.bottomAnchor),
      message.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -40),
      scroll.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -40),
      buttons.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -40),
      scroll.heightAnchor.constraint(greaterThanOrEqualToConstant: 180),
    ])
    if !schemes.isEmpty { table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false) }
    updateButtons()
    panel.initialFirstResponder = table
  }

  func numberOfRows(in tableView: NSTableView) -> Int { schemes.count }

  func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
    let cell = NSTextField(labelWithString: schemes[row].name)
    cell.lineBreakMode = .byTruncatingTail
    cell.toolTip = schemes[row].url.path
    return cell
  }

  func tableViewSelectionDidChange(_ notification: Notification) { updateButtons() }

  private func updateButtons() { importButton.isEnabled = table.selectedRow >= 0 }

  @objc func importSelection(_ sender: Any?) {
    guard schemes.indices.contains(table.selectedRow) else { return }
    importScheme(at: schemes[table.selectedRow].url)
  }

  @objc func selectFile(_ sender: Any?) { chooseFile(self) }

  @objc func cancel(_ sender: Any?) { close() }

  /// Import immediately once a file is chosen, as the list's Import does.
  func importScheme(at url: URL) {
    do {
      let data = try EVVimColorSchemes.read(url)
      try store.importVimTheme(data, named: url.deletingPathExtension().lastPathComponent)
      close()
    } catch {
      presentError(error, panel)
    }
  }

  private func runOpenPanel() {
    let open = NSOpenPanel()
    open.title = "Select a Vim Color Scheme"
    open.prompt = "Import"
    open.canChooseFiles = true
    open.canChooseDirectories = false
    open.allowsMultipleSelection = false
    open.allowedContentTypes = [UTType(filenameExtension: "vim") ?? .plainText]
    open.directoryURL = directories.first { FileManager.default.fileExists(atPath: $0.path) }
      ?? FileManager.default.homeDirectoryForCurrentUser
    open.beginSheetModal(for: panel) { [weak self] response in
      guard response == .OK, let url = open.url else { return }
      MainActor.assumeIsolated { self?.importScheme(at: url) }
    }
  }

  private func close() {
    if let parent, panel.sheetParent === parent {
      parent.endSheet(panel)
    } else if NSApp.modalWindow === panel {
      NSApp.stopModal()
    }
    panel.orderOut(nil)
    onClose?()
  }
}
