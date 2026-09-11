import AppKit
import UniformTypeIdentifiers

@MainActor
public enum EVApplication {
    public static func run() {
        let application = NSApplication.shared
        let delegate = EVApplicationDelegate()

        application.setActivationPolicy(.regular)
        application.delegate = delegate
        withExtendedLifetime(delegate) {
            // A SwiftPM executable enters AppKit directly rather than through
            // NSApplicationMain, so complete launch explicitly before starting
            // the event loop. This delivers the delegate launch callbacks that
            // install the menu and create the initial untitled document.
            application.finishLaunching()
            application.activate(ignoringOtherApps: true)
            application.run()
        }
    }
}

@MainActor
final class EVApplicationDelegate: NSObject,
    NSApplicationDelegate,
    EVApplicationCommandRouting
{
    private var menuBuilder: EVMenuBuilder?
    private var settingsWindowController: EVSettingsWindowController?
    private weak var launchPlaceholderDocument: EVDocument?
    private let configuration: EVConfigurationStore
    var documentFactory: () -> EVDocument = { EVDocument() }
    var mainWindow: () -> NSWindow? = { NSApplication.shared.mainWindow }
    var recordRecentDocument: (URL) -> Void

    init(configuration: EVConfigurationStore? = nil) {
        let configuration = configuration ?? .shared
        self.configuration = configuration
        recordRecentDocument = { try? configuration.recordRecentDocument($0) }
        super.init()
    }

    func applicationWillFinishLaunching(_ notification: Notification) {
        let builder = EVMenuBuilder(owner: self, recentDocumentURLs: { [configuration] in
            configuration.recentDocumentURLs
        })
        NSApplication.shared.mainMenu = builder.buildMainMenu(for: NSApplication.shared)
        menuBuilder = builder
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        ensureInitialDocument()
        NSApplication.shared.activate(ignoringOtherApps: true)
    }

    func applicationShouldOpenUntitledFile(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag {
            if let document = NSDocumentController.shared.documents.first {
                document.showWindows()
            } else {
                createUntitledDocument()
            }
        }
        return true
    }

    func application(_ sender: NSApplication, openFiles filenames: [String]) {
        let replacement = captureUntitledReplacement()
        var openedAny = false
        for filename in filenames {
            openedAny = presentDocument(at: URL(fileURLWithPath: filename), replacing: replacement) || openedAny
        }
        sender.reply(toOpenOrPrint: openedAny ? .success : .failure)
    }

    @objc func newDocument(_ sender: Any?) {
        createUntitledDocument()
    }

    @objc func openDocument(_ sender: Any?) {
        let replacement = captureUntitledReplacement()
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        // Code commonly has unknown or extensionless UTIs. The portable
        // decoder and detection profile decide how to present the bytes.
        panel.allowsOtherFileTypes = true
        panel.begin { [weak self] response in
            guard response == .OK else { return }
            for url in panel.urls {
                _ = self?.presentDocument(at: url, replacing: replacement)
            }
        }
    }

    @objc func openRecentDocument(_ sender: Any?) {
        guard let url = (sender as? NSMenuItem)?.representedObject as? URL else { return }
        _ = presentDocument(at: url, replacing: captureUntitledReplacement())
    }

    @objc func clearRecentDocuments(_ sender: Any?) {
        do { try configuration.clearRecentDocuments() }
        catch { NSApplication.shared.presentError(error) }
    }

    @objc func showSettings(_ sender: Any?) {
        if settingsWindowController == nil {
            settingsWindowController = EVSettingsWindowController()
        }
        settingsWindowController?.showWindow(sender)
        settingsWindowController?.window?.makeKeyAndOrderFront(sender)
    }

    @objc func showHelpItem(_ sender: Any?) {
        let topic = (sender as? NSMenuItem)?.representedObject as? String ?? "Viem Help"
        let alert = NSAlert()
        alert.messageText = topic
        alert.informativeText = "This help topic will be supplied with the Viem documentation bundle."
        alert.alertStyle = .informational
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    @discardableResult
    private func createUntitledDocument() -> EVDocument {
        let document = documentFactory()
        NSDocumentController.shared.addDocument(document)
        document.makeWindowControllers()
        document.showWindows()
        return document
    }

    fileprivate func ensureInitialDocument() {
        guard NSDocumentController.shared.documents.isEmpty else { return }
        launchPlaceholderDocument = createUntitledDocument()
    }

    func captureUntitledReplacement() -> EVDocumentWindowController.UntitledReplacement? {
        if let controller = mainWindow()?.windowController as? EVDocumentWindowController {
            return controller.captureUntitledReplacement()
        }
        return (launchPlaceholderDocument?.windowControllers.first as? EVDocumentWindowController)?
            .captureUntitledReplacement()
    }

    @discardableResult
    func openDocument(
        at url: URL, replacing replacement: EVDocumentWindowController.UntitledReplacement?
    ) throws -> EVDocument {
        let url = EVDocumentIdentity.canonicalURL(url)
        if let existing = EVDocumentIdentity.existingDocument(at: url) {
            recordRecentDocument(url)
            if let window = EVDocumentWindowController.windowShowing(document: existing) {
                window.windowController?.showWindow(nil)
            } else if replacement?.install(existing) != true {
                if existing.windowControllers.isEmpty { existing.makeWindowControllers() }
                existing.showWindows()
            }
            return existing
        }

        let type = Self.documentType(for: url)
        let document = documentFactory()
        document.recordRecentDocument = recordRecentDocument
        try document.read(from: url, ofType: type)
        document.fileURL = url
        if !document.editorBackend.persistenceState.isDirty {
            document.updateChangeCount(.changeCleared)
        }
        NSDocumentController.shared.addDocument(document)
        if replacement?.install(document) != true {
            document.makeWindowControllers()
            document.showWindows()
        }
        return document
    }

    private func presentDocument(
        at url: URL, replacing replacement: EVDocumentWindowController.UntitledReplacement?
    ) -> Bool {
        do {
            try openDocument(at: url, replacing: replacement)
            return true
        } catch {
            NSApplication.shared.presentError(error)
            return false
        }
    }

    private static func documentType(for url: URL) -> String {
        switch url.pathExtension.lowercased() {
        case "md", "markdown", "mdown", "mkd":
            EVDocument.markdownType
        case "html", "htm":
            EVDocument.htmlType
        case "rtf":
            EVDocument.rtfType
        default:
            EVDocument.plainTextType
        }
    }
}
