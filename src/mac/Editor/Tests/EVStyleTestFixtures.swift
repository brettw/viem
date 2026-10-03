import ViemAppShell
@testable import ViemEditor

/// Declare the appearance a test needs before creating its views. Shipped themes
/// are user presets, not stable numeric fixtures or part of a test's undo history.
@MainActor
enum EVStyleTestFixtures {
    static func configure(_ configuration: EVConfigurationStore, format: EVSourceFormat = .markdown,
                          declarations: [(EVStyleKey, EVStyleProperty, EVStyleValue?)]) throws {
        let session: any EVStyleSettingsSession = format == .code
            ? try EVCodeStyleSession(configuration: configuration)
            : try EVThemeStyleSession(configuration: configuration, format: format)
        for (key, property, value) in declarations {
            let mutation: EVStyleMutation = value.map { .setDeclaration(property, $0) }
                ?? .clearDeclaration(property)
            try session.edit(key: key, expected: session.snapshot().identity, mutation: mutation)
        }
        session.undoManager.removeAllActions()
    }

    static func block(_ id: String) -> EVStyleKey {
        EVStyleKey(namespace: .block, id: EVStyleID(rawValue: id))
    }

    static func character(_ id: String) -> EVStyleKey {
        EVStyleKey(namespace: .character, id: EVStyleID(rawValue: id))
    }
}
