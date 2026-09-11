import ViemAppShell

@MainActor
public enum EVEditorComposition {
    public static func install() {
        EVCodePreferences.editStyles = { configuration in
            EVStyleEditorCoordinator.shared.showCode(configuration: configuration, sender: nil)
        }
        EVFrontendRegistry.install {
            EVCoreDocumentBackend()
        }
    }
}
