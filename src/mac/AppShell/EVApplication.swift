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

    func applicationWillFinishLaunching(_ notification: Notification) {
        let builder = EVMenuBuilder(owner: self)
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
        var openedAny = false
        for filename in filenames {
            openedAny = openDocument(at: URL(fileURLWithPath: filename)) || openedAny
        }
        if openedAny {
            discardPristineLaunchPlaceholder()
        }
        sender.reply(toOpenOrPrint: openedAny ? .success : .failure)
    }

    @objc func newDocument(_ sender: Any?) {
        createUntitledDocument()
    }

    @objc func openDocument(_ sender: Any?) {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            .plainText,
            .html,
            .rtf,
            UTType(filenameExtension: "md") ?? .plainText,
        ]
        panel.begin { [weak self] response in
            guard response == .OK else { return }
            var openedAny = false
            for url in panel.urls {
                openedAny = self?.openDocument(at: url) == true || openedAny
            }
            if openedAny {
                self?.discardPristineLaunchPlaceholder()
            }
        }
    }

    @objc func openRecentDocument(_ sender: Any?) {
        guard let url = (sender as? NSMenuItem)?.representedObject as? URL else { return }
        if openDocument(at: url) {
            discardPristineLaunchPlaceholder()
        }
    }

    @objc func clearRecentDocuments(_ sender: Any?) {
        NSDocumentController.shared.clearRecentDocuments(sender)
    }

    @objc func showSettings(_ sender: Any?) {
        if settingsWindowController == nil {
            settingsWindowController = EVSettingsWindowController()
        }
        settingsWindowController?.showWindow(sender)
        settingsWindowController?.window?.makeKeyAndOrderFront(sender)
    }

    @objc func showHelpItem(_ sender: Any?) {
        let topic = (sender as? NSMenuItem)?.representedObject as? String ?? "eVim Help"
        let alert = NSAlert()
        alert.messageText = topic
        alert.informativeText = "This help topic will be supplied with the eVim documentation bundle."
        alert.alertStyle = .informational
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    @discardableResult
    private func createUntitledDocument() -> EVDocument {
        let document = EVDocument()
        NSDocumentController.shared.addDocument(document)
        document.makeWindowControllers()
        document.showWindows()
        return document
    }

    fileprivate func ensureInitialDocument() {
        guard NSDocumentController.shared.documents.isEmpty else { return }
        launchPlaceholderDocument = createUntitledDocument()
    }

    private func discardPristineLaunchPlaceholder() {
        guard let document = launchPlaceholderDocument else { return }
        launchPlaceholderDocument = nil
        guard Self.isDiscardableLaunchPlaceholder(document) else { return }
        document.close()
    }

    static func isDiscardableLaunchPlaceholder(_ document: EVDocument) -> Bool {
        document.fileURL == nil
            && !document.isDocumentEdited
            && !document.editorBackend.persistenceState.isDirty
    }

    @discardableResult
    private func openDocument(at url: URL) -> Bool {
        let url = EVDocumentIdentity.canonicalURL(url)
        if let existing = EVDocumentIdentity.existingDocument(at: url) {
            if existing.windowControllers.isEmpty { existing.makeWindowControllers() }
            existing.showWindows()
            return true
        }

        do {
            let type = Self.documentType(for: url)
            let document = EVDocument()
            try document.read(from: url, ofType: type)
            document.fileURL = url
            if !document.editorBackend.persistenceState.isDirty {
                document.updateChangeCount(.changeCleared)
            }
            NSDocumentController.shared.addDocument(document)
            NSDocumentController.shared.noteNewRecentDocumentURL(url)
            document.makeWindowControllers()
            document.showWindows()
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
