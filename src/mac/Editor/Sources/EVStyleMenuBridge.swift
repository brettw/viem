import AppKit
import CViemCore
import ViemAppShell

@MainActor
extension EVEditorSurfaceController: EVStyleMenuProviding {
  func listStylePresentation(_ command: EVMenuCommand) -> EVMenuItemPresentation {
    guard ((try? session?.tableContext().flags) ?? 0) & UInt32(VIEM_TABLE_IN_TABLE) == 0,
          let selected = try? session?.selectedNamedStyles() else { return .disabled }
    return EVMenuItemPresentation(isEnabled: true,
      state: command == .bulletedList ? selected.bulletState
        : command == .numberedList ? selected.numberedState : .off)
  }

  func listIndentPresentation(unindent: Bool) -> EVMenuItemPresentation {
    guard ((try? session?.tableContext().flags) ?? 0) & UInt32(VIEM_TABLE_IN_TABLE) == 0, let session,
      let selection = try? session.listSelection(),
      let flags = try? session.listIndentCapabilities(expected: selection)
    else { return .disabled }
    let capability = UInt32(unindent ? VIEM_LIST_CAN_UNINDENT : VIEM_LIST_CAN_INDENT)
    return EVMenuItemPresentation(isEnabled: flags & capability != 0)
  }

  public func currentStyleMenuCatalogue() -> EVStyleMenuCatalogue? {
    if backend.sourceFormat == .code { return currentCodeStyleMenuCatalogue() }
    guard let snapshot = try? backend.styleSheetSnapshot() else { return nil }
    let selection = try? session?.listSelection()
    let selectedStyles = try? session?.selectedNamedStyles()

    return styleMenuCatalogue(snapshot: snapshot, selectedStyles: selectedStyles, selectionAvailable: selection != nil)
  }

  /// Menus and the persistent toolbar share assignment and selection policy.
  /// The toolbar can reuse an exact-revision snapshot while the cursor moves.
  func styleMenuCatalogue(snapshot: EVStyleSheetSnapshot, selectedStyles: EVSelectedNamedStyles?,
                          selectionAvailable: Bool) -> EVStyleMenuCatalogue {
    let inTable = ((try? session?.tableContext().flags) ?? 0) & UInt32(VIEM_TABLE_IN_TABLE) != 0
    var entries = snapshot.definitions.filter {
      !$0.flags.contains(.internalSyntax)
        && (!$0.flags.contains(.internalList)
          || (selectedStyles?.identity == snapshot.identity
            && selectedStyles?.paragraph == $0.key.id))
    }.sorted {
      $0.name.localizedStandardCompare($1.name) == .orderedAscending
    }.map { definition in
      EVStyleMenuEntry(
        role: definition.kind.menuRole,
        stableID: definition.key.id.rawValue,
        displayName: definition.name,
        isBase: definition.flags.isBase,
        presentation: EVMenuItemPresentation(
          isEnabled: selectionAvailable && (!inTable || definition.kind == .character)
            && ((self.standardHeadingLevel(for: definition.key.id.rawValue) != nil
              && definition.kind == .paragraph)
              || (definition.capabilities.contains(.assign)
                && (definition.kind == .paragraph || definition.kind.isContainer
                  || definition.kind == .character))),
          state: selectedStyles?.identity == snapshot.identity
            && ((definition.kind != .character && selectedStyles?.paragraph == definition.key.id)
                || (definition.kind == .character && selectedStyles?.character == definition.key.id))
            ? .on : .off
        )
      )
    }
    entries.insert(EVStyleMenuEntry(
      role: .character, stableID: "", displayName: "Default Paragraph", isBase: true,
      presentation: EVMenuItemPresentation(
        isEnabled: selectionAvailable && [.markdown, .markdownSource].contains(backend.sourceFormat),
        state: selectedStyles?.identity == snapshot.identity
          && selectedStyles?.characterMixed == false && selectedStyles?.character == nil ? .on : .off)
    ), at: 0)
    return EVStyleMenuCatalogue(
      documentID: snapshot.identity.documentID,
      documentRevision: snapshot.identity.documentRevision,
      styleSheetRevision: snapshot.identity.styleSheetRevision,
      entries: entries,
      canEditStyles: session != nil
    )
  }

  private func currentCodeStyleMenuCatalogue() -> EVStyleMenuCatalogue? {
    guard let snapshot = try? EVCoreStyleBridge.copyStyleSheet(core: nil),
      let state = try? backend.documentState()
    else { return nil }
    let selectedStyles = try? session?.selectedNamedStyles()
    // The Code authority has document ID zero; the selection still belongs to
    // this buffer and the exact global stylesheet installed in its projection.
    let selectionMatches = selectedStyles?.identity.documentID == state.document_id
      && selectedStyles?.identity.documentRevision == state.document_revision
      && selectedStyles?.identity.styleSheetRevision == snapshot.identity.styleSheetRevision
    var entries = snapshot.definitions.filter { !$0.flags.contains(.internalSyntax) }.map { definition in
      EVStyleMenuEntry(
        role: definition.kind.menuRole,
        stableID: definition.key.id.rawValue,
        displayName: definition.name,
        isBase: definition.flags.isBase,
        presentation: EVMenuItemPresentation(
          isEnabled: true,
          state: selectionMatches
            && ((definition.kind != .character && selectedStyles?.paragraph == definition.key.id)
              || (definition.kind == .character && selectedStyles?.character == definition.key.id))
            ? .on : .off
        ),
        actionKind: .edit
      )
    }
    entries.insert(EVStyleMenuEntry(
      role: .character, stableID: "", displayName: "Default Paragraph", isBase: true,
      presentation: EVMenuItemPresentation(isEnabled: true,
        state: selectionMatches && selectedStyles?.characterMixed == false && selectedStyles?.character == nil ? .on : .off),
      actionKind: .edit
    ), at: 0)
    // A referenced name is normally generated when its highlighting is
    // published. Until then, choosing it generates and opens its definition.
    let definedNames = Set(snapshot.definitions.filter { $0.kind == .character }.map(\.name))
    for name in (try? backend.syntaxStyleNames()) ?? [] where !definedNames.contains(name) {
      entries.append(EVStyleMenuEntry(
        role: .character,
        stableID: "",
        displayName: name,
        isBase: false,
        presentation: .enabled,
        actionKind: .defineSyntax,
        syntaxName: name
      ))
    }
    entries.sort {
      $0.displayName.localizedStandardCompare($1.displayName) == .orderedAscending
    }
    return EVStyleMenuCatalogue(
      documentID: state.document_id,
      documentRevision: state.document_revision,
      styleSheetRevision: snapshot.identity.styleSheetRevision,
      entries: entries,
      canEditStyles: true
    )
  }

  func performHeadingShortcut(level: UInt32) {
    guard level <= 6, let session, headingShortcutPresentation(level: level).isEnabled else {
      return
    }
    performInput {
      _ = try session.setParagraphStyle(level: level, expected: session.listSelection())
    }
  }

  func headingShortcutPresentation(level: UInt32) -> EVMenuItemPresentation {
    let id = level == 0 ? "Paragraph" : "Heading\(level)"
    let snapshot = try? backend.styleSheetSnapshot()
    let selectedStyles = try? session?.selectedNamedStyles()
    return EVMenuItemPresentation(
      isEnabled: level <= 6 && session != nil
        && standardHeadingLevel(for: id) != nil
        && snapshot?.definition(namespace: .block, id: EVStyleID(rawValue: id)) != nil,
      state: selectedStyles?.identity == snapshot?.identity
        && selectedStyles?.paragraph?.rawValue == id ? .on : .off
    )
  }

  private func standardHeadingLevel(for id: String) -> UInt32? {
    guard [.markdown, .markdownSource].contains(backend.sourceFormat) else {
      return nil
    }
    if id == "Paragraph" { return 0 }
    guard id.hasPrefix("Heading"), let level = UInt32(id.dropFirst(7)), (1...6).contains(level)
    else { return nil }
    return level
  }

  func perform(styleMenuAction action: EVStyleMenuAction, sender: Any?) {
    if action.kind == .editCurrent {
      guard session != nil, let state = try? backend.documentState(),
        state.document_id == action.documentID else { return }
      EVStyleEditorCoordinator.shared.show(document: self, sender: sender)
      return
    }
    if backend.sourceFormat == .code {
      performCodeStyleMenuAction(action, sender: sender)
      return
    }
    if action.kind == .assign {
      guard let session,
        let snapshot = try? backend.styleSheetSnapshot(),
        snapshot.identity.documentID == action.documentID,
        snapshot.identity.documentRevision == action.documentRevision,
        snapshot.identity.styleSheetRevision == action.styleSheetRevision
      else { return }
      let key = EVStyleKey(namespace: action.role.namespace, id: EVStyleID(rawValue: action.stableID))
      let clearCharacter = key == .defaultParagraph
      let definition = snapshot.definition(for: key)
      if !clearCharacter {
        guard let definition, definition.kind.menuRole == action.role,
          !definition.flags.contains(.internalSyntax) else { return }
      }
      let level = action.role == .paragraph ? standardHeadingLevel(for: action.stableID) : nil
      guard clearCharacter || level != nil || definition?.capabilities.contains(.assign) == true else { return }
      performInput {
        let selection = try session.listSelection()
        if let level {
          _ = try session.setParagraphStyle(level: level, expected: selection)
        } else {
          _ = try session.assignStyle(key, identity: snapshot.identity, expected: selection)
        }
      }
      return
    }
    guard action.kind == .edit,
      let snapshot = try? backend.styleSheetSnapshot(),
      snapshot.identity.documentID == action.documentID,
      let definition = snapshot.definition(
        namespace: action.role.namespace,
        id: EVStyleID(rawValue: action.stableID)
      ),
      definition.kind.menuRole == action.role
    else {
      // A stale or malformed action never guesses another style target.
      return
    }

    let coordinator = EVStyleEditorCoordinator.shared
    coordinator.show(document: self, sender: sender)
    coordinator.selectStyle(definition.key)
  }

  private func performCodeStyleMenuAction(_ action: EVStyleMenuAction, sender: Any?) {
    guard let state = try? backend.documentState(), action.documentID == state.document_id else { return }
    if action.kind == .defineSyntax {
      guard action.role == .character, let name = action.syntaxName,
        let catalogue = currentCodeStyleMenuCatalogue(),
        catalogue.documentRevision == action.documentRevision,
        catalogue.styleSheetRevision == action.styleSheetRevision,
        catalogue.entries.contains(where: { $0.actionKind == .defineSyntax && $0.syntaxName == name })
      else { return }
      EVStyleEditorCoordinator.shared.showCode(
        configuration: backend.configuration, definingSyntaxName: name, following: self, sender: sender)
      return
    }
    if action.kind == .edit, action.role == .character, action.stableID.isEmpty {
      EVStyleEditorCoordinator.shared.showCode(
        configuration: backend.configuration,
        preferredStyle: .baseParagraph, following: self, sender: sender)
      return
    }
    guard action.kind == .edit,
      let snapshot = try? EVCoreStyleBridge.copyStyleSheet(core: nil),
      let definition = snapshot.definition(namespace: action.role.namespace, id: EVStyleID(rawValue: action.stableID)),
      definition.kind.menuRole == action.role
    else { return }
    EVStyleEditorCoordinator.shared.showCode(
      configuration: backend.configuration, preferredStyle: definition.key, following: self, sender: sender)
  }
}

@MainActor
extension EVEditorView: EVStyleMenuActionRouting, NSMenuItemValidation {
  func validateMenuItem(_ item: NSMenuItem) -> Bool {
    if item.action == #selector(openLink(_:)) {
      guard let target = item.representedObject as? EVLinkMenuTarget,
            let surface else { return false }
      return (try? surface.backend.linkDestination(at: target)) != nil
    }
    if let command = EVMenuCommand.nativeEditCommand(for: item.action), let surface {
      if let enabled = commandLineMenuEnabled(command) { return enabled }
      let presentation = surface.presentation(for: command)
      item.state = presentation.state
      if let title = presentation.title { item.title = title }
      return presentation.isEnabled
    }
    item.state = .off
    guard let action = item.representedObject as? EVStyleMenuAction,
      let catalogue = surface?.currentStyleMenuCatalogue(),
      catalogue.documentID == action.documentID
    else { return false }
    if action.kind == .editCurrent { return catalogue.canEditStyles }
    if action.kind == .edit {
      guard catalogue.canEditStyles, let entry = catalogue.entries.first(where: {
        $0.role == action.role && $0.stableID == action.stableID && $0.actionKind == .edit
      }) else { return false }
      item.state = action.marksCurrentStyle ? entry.presentation.state : .off
      return true
    }
    guard catalogue.documentRevision == action.documentRevision,
      catalogue.styleSheetRevision == action.styleSheetRevision,
      let entry = catalogue.entries.first(where: {
        $0.role == action.role && $0.stableID == action.stableID
          && $0.actionKind == action.kind && $0.syntaxName == action.syntaxName
      })
    else { return false }
    item.state = action.marksCurrentStyle ? entry.presentation.state : .off
    return entry.presentation.isEnabled
  }

  @objc func performEditorStyleMenuAction(_ sender: Any?) {
    guard let item = sender as? NSMenuItem,
      let action = item.representedObject as? EVStyleMenuAction
    else { return }
    surface?.perform(styleMenuAction: action, sender: sender)
  }
}

extension EVStyleKind {
  fileprivate var menuRole: EVStyleMenuRole {
    switch self {
    case .character: .character
    case .paragraph, .quote, .codeBlock, .list, .listItem, .table: .paragraph
    }
  }
}

extension EVStyleMenuRole {
  fileprivate var namespace: EVStyleNamespace {
    switch self {
    case .character: .character
    case .paragraph: .block
    }
  }
}
