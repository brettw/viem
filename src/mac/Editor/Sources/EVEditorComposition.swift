import ViemAppShell

@MainActor
public enum EVEditorComposition {
    public static func install() {
        EVFrontendRegistry.install {
            EVCoreDocumentBackend()
        }
    }
}
