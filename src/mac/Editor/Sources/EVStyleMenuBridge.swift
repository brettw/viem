import AppKit
import EvimAppShell

@MainActor
extension EVEditorSurfaceController: EVStyleMenuProviding {
    public func currentStyleMenuCatalogue() -> EVStyleMenuCatalogue? {
        guard let snapshot = try? backend.styleSheetSnapshot() else { return nil }

        let entries = snapshot.definitions.map { definition in
            EVStyleMenuEntry(
                role: definition.kind.menuRole,
                stableID: definition.key.id.rawValue,
                displayName: definition.name,
                isBase: definition.flags.isBase,
                // No current format adapter exposes source-backed named-style
                // assignment through the C ABI. Showing the catalogue is
                // still useful, but enabling these entries would promise an
                // edit that the projection cannot round-trip.
                presentation: .disabled
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

    func perform(styleMenuAction action: EVStyleMenuAction, sender: Any?) {
        guard action.kind == .edit,
              let snapshot = try? backend.styleSheetSnapshot(),
              snapshot.identity.documentID == action.documentID,
              let definition = snapshot.definition(
                  namespace: action.role.namespace,
                  id: EVStyleID(rawValue: action.stableID)
              ),
              definition.kind.menuRole == action.role
        else {
            // Assign is deliberately unavailable until an adapter exposes a
            // reversible source-backed intention. A stale/malformed Edit
            // Styles payload likewise performs no guessed operation.
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
