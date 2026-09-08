import ViemAppShell
import ViemEditor

@main
struct ViemMain {
    @MainActor
    static func main() {
        EVEditorComposition.install()
        EVApplication.run()
    }
}
