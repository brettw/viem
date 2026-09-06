import AppKit
import CEvimCore
import EvimAppShell

@MainActor
extension EVEditorSurfaceController: EVStyleMenuProviding {
    public func currentStyleMenuCatalogue() -> EVStyleMenuCatalogue? {
        guard let snapshot = try? backend.styleSheetSnapshot() else { return nil }
        let selection = try? session?.listSelection()

        let entries = snapshot.definitions.sorted {
            $0.name.localizedStandardCompare($1.name) == .orderedAscending
        }.map { definition in
            EVStyleMenuEntry(
                role: definition.kind.menuRole,
                stableID: definition.key.id.rawValue,
                displayName: definition.name,
                isBase: definition.flags.isBase,
                presentation: EVMenuItemPresentation(
                    isEnabled: selection != nil && (
                        (self.standardHeadingLevel(for: definition.key.id.rawValue) != nil
                            && definition.kind == .paragraph)
                        || (definition.capabilities.contains(.assign)
                            && (definition.kind == .paragraph
                                || (definition.kind == .character
                                    && selection!.text_start < selection!.text_end)))
                    )
                )
            )
        }
        return EVStyleMenuCatalogue(
            documentID: snapshot.identity.documentID,
            documentRevision: snapshot.identity.documentRevision,
            styleSheetRevision: snapshot.identity.styleSheetRevision,
            entries: entries,
            canEditStyles: session != nil
        )
    }

    private func standardHeadingLevel(for id: String) -> UInt32? {
        guard [.markdown, .markdownSource, .html, .rtf].contains(backend.sourceFormat) else { return nil }
        if id == "Paragraph" { return 0 }
        guard id.hasPrefix("Heading"), let level = UInt32(id.dropFirst(7)), (1...6).contains(level) else { return nil }
        return level
    }

    func perform(styleMenuAction action: EVStyleMenuAction, sender: Any?) {
        if action.kind == .assign {
            guard let session,
                  let snapshot = try? backend.styleSheetSnapshot(),
                  snapshot.identity.documentID == action.documentID,
                  snapshot.identity.documentRevision == action.documentRevision,
                  snapshot.identity.styleSheetRevision == action.styleSheetRevision,
                  let definition = snapshot.definition(namespace: action.role.namespace,
                      id: EVStyleID(rawValue: action.stableID)),
                  definition.kind.menuRole == action.role
            else { return }
            let level = action.role == .paragraph ? standardHeadingLevel(for: action.stableID) : nil
            guard level != nil || definition.capabilities.contains(.assign) else { return }
            performInput {
                let selection = try session.listSelection()
                if let level {
                    _ = try session.setParagraphStyle(level: level, expected: selection)
                } else {
                    _ = try session.assignStyle(definition.key, identity: snapshot.identity, expected: selection)
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
        coordinator.show(document: self, preferredStyle: definition.kind, sender: sender)
        coordinator.selectStyle(definition.key)
    }
}

@MainActor
extension EVEditorView: EVStyleMenuActionRouting {
    @objc func performEditorStyleMenuAction(_ sender: Any?) {
        guard let item = sender as? NSMenuItem,
              let action = item.representedObject as? EVStyleMenuAction
        else { return }
        surface?.perform(styleMenuAction: action, sender: sender)
    }
}

private extension EVStyleKind {
    var menuRole: EVStyleMenuRole {
        switch self {
        case .character: .character
        case .paragraph: .paragraph
        case .document: .document
        }
    }
}

private extension EVStyleMenuRole {
    var namespace: EVStyleNamespace {
        switch self {
        case .character: .character
        case .paragraph, .document: .block
        }
    }
}
