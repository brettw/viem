import Foundation
import ViemAppShell
import ViemEditor
import ViemCoreTextProvider

@main
struct ViemMain {
    @MainActor
    static func main() {
        if let resources = Bundle.main.resourceURL {
            for failure in EVFontCatalog.registerBundledFonts(in: resources.appendingPathComponent("fonts")) {
                NSLog("Bundled font: %@", failure)
            }
        }
        EVEditorComposition.install()
        EVApplication.run()
    }
}
