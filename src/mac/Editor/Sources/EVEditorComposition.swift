import ViemAppShell
import Foundation
import CViemCore

@MainActor
public enum EVEditorComposition {
    public static func install() {
        EVLaunchArguments.parse = EVCoreLaunchArguments.parse
        EVCoreArgumentListPolicy.install()
        EVCodePreferences.editStyles = { configuration in
            EVStyleEditorCoordinator.shared.showCode(configuration: configuration, sender: nil)
        }
        EVEditingPreferences.editVisibleWhitespaceStyle = { configuration in
            EVVisibleWhitespaceStyleEditor.show(configuration: configuration)
        }
        EVEditingPreferences.validateWhitespacePresentation = { options in
            guard let data = try? JSONEncoder().encode(options) else { return "Invalid visible whitespace settings." }
            let status = data.withUnsafeBytes { raw in
                viem_validate_whitespace_presentation(raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count))
            }
            return status == UInt32(VIEM_STATUS_OK) ? nil : "Visible whitespace markers must be printable characters occupying one column, with a valid character style."
        }
        EVFrontendRegistry.install {
            EVCoreDocumentBackend()
        }
    }
}
