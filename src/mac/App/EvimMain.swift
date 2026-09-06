import EvimAppShell
import EvimEditor

@main
struct EvimMain {
    @MainActor
    static func main() {
        EVEditorComposition.install()
        EVApplication.run()
    }
}
