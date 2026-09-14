import AppKit
import UniformTypeIdentifiers

@MainActor
public enum EVApplication {
    public static func run() {
        let launchArguments: EVLaunchArguments
        do {
            // Older Launch Services versions add a process serial number. It
            // is native launch metadata rather than part of Vim's argv grammar.
            var arguments = Array(CommandLine.arguments.dropFirst())
            if arguments.first?.hasPrefix("-psn_") == true { arguments.removeFirst() }
            launchArguments = try EVLaunchArguments.parse(arguments)
        } catch {
            FileHandle.standardError.write(Data("Viem: \(error.localizedDescription)\nUsage: Viem [-o[count]] [+line] [--] [file ...]\n".utf8))
            exit(EXIT_FAILURE)
        }
        let application = NSApplication.shared
        let delegate = EVApplicationDelegate(launchArguments: launchArguments)

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
    private let launchArguments: EVLaunchArguments
    private let launchDirectory: URL
    private var hasProcessedLaunchArguments = false
    var documentFactory: () -> EVDocument = { EVDocument() }
    var mainWindow: () -> NSWindow? = { NSApplication.shared.mainWindow }
    var applicationWindows: () -> [NSWindow] = { NSApplication.shared.windows }
    var hasOpenDocumentWindows: () -> Bool = { EVDocumentWindowController.hasOpenDocumentWindows }
    var terminateApplication: () -> Void = { NSApplication.shared.terminate(nil) }
    var recordRecentDocument: (URL) -> Void

    init(configuration: EVConfigurationStore? = nil, launchArguments: EVLaunchArguments = EVLaunchArguments()) {
        let configuration = configuration ?? .shared
        self.configuration = configuration
        self.launchArguments = launchArguments
        self.launchDirectory = URL(fileURLWithPath: FileManager.default.currentDirectoryPath, isDirectory: true)
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
        if !openLaunchArguments() { ensureInitialDocument() }
        NSApplication.shared.activate(ignoringOtherApps: true)
    }

    func applicationShouldOpenUntitledFile(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    /// Only a successful editor quit command requests this check. Defer until
    /// AppKit has finished closing the window and the command has completed.
    /// No flag survives to affect a later stoplight/menu window close.
    func terminateAfterCommandClose() {
        guard hasNoOpenWindows else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self, self.hasNoOpenWindows else { return }
            self.terminateApplication()
        }
    }

    private var hasNoOpenWindows: Bool {
        !hasOpenDocumentWindows() && !applicationWindows().contains {
            !($0 is NSPanel) && ($0.isVisible || $0.isMiniaturized)
        }
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

    @discardableResult
    func openLaunchArguments() -> Bool {
        let hasLaunchOptions = !launchArguments.filenames.isEmpty || launchArguments.splitCount != nil
            || launchArguments.initialLine != nil
        guard !hasProcessedLaunchArguments else { return hasLaunchOptions }
        hasProcessedLaunchArguments = true
        guard hasLaunchOptions else { return false }
        let document = documentFactory()
        document.recordRecentDocument = recordRecentDocument
        NSDocumentController.shared.addDocument(document)
        document.makeWindowControllers()
        launchPlaceholderDocument = document
        guard let controller = document.windowControllers.first as? EVDocumentWindowController else {
            document.showWindows()
            return true
        }
        controller.argumentDocumentOpener = { [weak self] url, _, completion in
            guard let self else { completion(nil, CocoaError(.userCancelled)); return }
            do { completion(try self.loadLaunchDocument(at: url), nil) }
            catch { completion(nil, error) }
        }
        let urls = launchArguments.filenames.map {
            URL(fileURLWithPath: $0, relativeTo: launchDirectory).absoluteURL
        }
        controller.openArgumentList(urls, splitCount: launchArguments.splitCount,
            initialLine: launchArguments.initialLine) { result in
            if case let .failure(error) = result {
                controller.showWindow(nil)
                NSApplication.shared.presentError(error)
            }
        }
        return true
    }

    /// CLI launches also work directly from the executable, where an app
    /// bundle's NSDocumentController type registration may not be installed.
    private func loadLaunchDocument(at url: URL) throws -> EVDocument {
        let url = EVDocumentIdentity.canonicalURL(url)
        if let existing = EVDocumentIdentity.existingDocument(at: url) {
            recordRecentDocument(url)
            return existing
        }
        let document = documentFactory()
        document.recordRecentDocument = recordRecentDocument
        let type = Self.documentType(for: url)
        if FileManager.default.fileExists(atPath: url.path) {
            try document.read(from: url, ofType: type)
        } else {
            try document.read(from: Data(), ofType: type)
            document.configureRecovery(for: url)
        }
        document.fileURL = url
        document.fileType = EVDocument.typeName(for: document.editorBackend.sourceFormat)
        if !document.editorBackend.persistenceState.isDirty { document.updateChangeCount(.changeCleared) }
        NSDocumentController.shared.addDocument(document)
        return document
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
