import AppKit

struct EVDiffCommit: Decodable, Sendable {
  let hash: String
  let short: String
  let title: String
}

@MainActor
final class EVDiffCommitPicker: NSObject, NSTableViewDataSource, NSTableViewDelegate, NSTextFieldDelegate {
  let alert = NSAlert()
  let input = NSTextField()
  private let table = NSTableView()
  private let commits: [EVDiffCommit]

  init(_ commits: [EVDiffCommit]) {
    self.commits = commits
    super.init()
    alert.messageText = "Choose a commit"
    alert.addButton(withTitle: "Compare")
    alert.addButton(withTitle: "Cancel")
    let content = NSView(frame: NSRect(x: 0, y: 0, width: 500, height: 290))
    input.frame = NSRect(x: 0, y: 262, width: 500, height: 24)
    input.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
    input.placeholderString = "Commit hash"
    input.setAccessibilityLabel("Commit hash")
    input.delegate = self
    content.addSubview(input)

    let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 500, height: 254))
    scroll.hasVerticalScroller = true
    scroll.borderType = .bezelBorder
    table.headerView = nil
    table.setAccessibilityLabel("Recent commits")
    let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("commit"))
    column.width = scroll.contentSize.width
    table.addTableColumn(column)
    table.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
    table.dataSource = self
    table.delegate = self
    scroll.documentView = table
    content.addSubview(scroll)
    alert.accessoryView = content
    alert.window.initialFirstResponder = input
  }

  func numberOfRows(in tableView: NSTableView) -> Int { commits.count }

  func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
    let commit = commits[row]
    return NSTextField(labelWithString: "\(commit.short)  \(commit.title)")
  }

  func tableViewSelectionDidChange(_ notification: Notification) {
    guard commits.indices.contains(table.selectedRow) else { return }
    input.stringValue = commits[table.selectedRow].hash
    alert.buttons[0].isEnabled = true
  }

  func controlTextDidChange(_ notification: Notification) {
    alert.buttons[0].isEnabled = !hash.isEmpty
    if commits.indices.contains(table.selectedRow), hash != commits[table.selectedRow].hash { table.deselectAll(nil) }
  }

  private var hash: String { input.stringValue.trimmingCharacters(in: .whitespacesAndNewlines) }

  func pick(in window: NSWindow) async -> String? {
    alert.buttons[0].isEnabled = !hash.isEmpty
    let response = await withCheckedContinuation { continuation in
      alert.beginSheetModal(for: window) { continuation.resume(returning: $0) }
    }
    return response == .alertFirstButtonReturn ? hash : nil
  }
}
