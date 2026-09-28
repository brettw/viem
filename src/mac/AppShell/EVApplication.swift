import AppKit
import UniformTypeIdentifiers

@MainActor
public enum EVApplication {
    public static func run() {
        // Viem owns argv, including its lazy argument list and Vim flags.
        // Otherwise AppKit also turns those arguments into native open-file
        // callbacks during finishLaunching(), creating one window per file.
        // AppKit expects the string "NO" for this nonpersistent launch default.
        UserDefaults.standard.register(defaults: ["NSTreatUnknownArgumentsAsOpen": "NO"])
        let request = EVInstanceLaunchRequest(
            arguments: Array(CommandLine.arguments.dropFirst()),
            workingDirectory: FileManager.default.currentDirectoryPath
        )
        let instance: EVSingleInstance
        let launchArguments: EVLaunchArguments
        do {
            // Development subprocess tests isolate their IPC endpoint just as
            // VIEM_CONFIG_DIR isolates settings; normal launches share one.
            let instanceDirectory = ProcessInfo.processInfo.environment["VIEM_INSTANCE_DIRECTORY"].map {
                URL(fileURLWithPath: $0, isDirectory: true)
            }
            switch try EVSingleInstance.start(request: request, directory: instanceDirectory) {
            case let .forwarded(response):
                if let error = response.errorMessage {
                    FileHandle.standardError.write(Data("Viem: \(error)\n".utf8))
                    exit(EXIT_FAILURE)
                }
                return
            case let .primary(primary):
                instance = primary
            }
            launchArguments = try parse(request)
        } catch {
            FileHandle.standardError.write(Data("Viem: \(error.localizedDescription)\n".utf8))
            exit(EXIT_FAILURE)
        }
        let application = NSApplication.shared
        let delegate = EVApplicationDelegate(launchArguments: launchArguments,
            launchDirectory: URL(fileURLWithPath: request.workingDirectory, isDirectory: true))

        application.setActivationPolicy(.regular)
        application.delegate = delegate
        withExtendedLifetime((delegate, instance)) {
            // A SwiftPM executable enters AppKit directly rather than through
            // NSApplicationMain, so complete launch explicitly before starting
            // the event loop. This delivers the delegate launch callbacks that
            // install the menu and create the initial untitled document.
            application.finishLaunching()
            instance.setLaunchHandler { request in
                do {
                    let arguments = try parse(request)
                    // Acknowledge acceptance before document recovery or open
                    // errors can enter a modal event loop in the editor.
                    DispatchQueue.main.async {
                        delegate.processLaunchArguments(arguments, workingDirectory:
                            URL(fileURLWithPath: request.workingDirectory, isDirectory: true))
                    }
                    return nil
                } catch { return error.localizedDescription }
            }
            application.activate(ignoringOtherApps: true)
            application.run()
        }
    }

    private static func parse(_ request: EVInstanceLaunchRequest) throws -> EVLaunchArguments {
        // Older Launch Services versions add native process metadata to argv.
        var arguments = request.arguments
        if arguments.first?.hasPrefix("-psn_") == true { arguments.removeFirst() }
        do { return try EVLaunchArguments.parse(arguments) }
        catch {
            throw EVLaunchArgumentError("\(error.localizedDescription)\nUsage: Viem [-o[count]] [+line] [--] [file ...]")
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
    private lazy var themeStore = EVThemeStore(configuration: configuration)
    private weak var launchPlaceholderDocument: EVDocument?
    private let configuration: EVConfigurationStore
    private let launchArguments: EVLaunchArguments
    private let launchDirectory: URL
    private var hasProcessedLaunchArguments = false
    private var pendingLaunches: [(EVLaunchArguments, URL)] = []
    private var isProcessingLaunch = false
    var documentFactory: () -> EVDocument = { EVDocument() }
    var mainWindow: () -> NSWindow? = { NSApplication.shared.mainWindow }
    var applicationWindows: () -> [NSWindow] = { NSApplication.shared.windows }
    var hasOpenDocumentWindows: () -> Bool = { EVDocumentWindowController.hasOpenDocumentWindows }
    var terminateApplication: () -> Void = { NSApplication.shared.terminate(nil) }
    var recordRecentDocument: (URL) -> Void

    init(configuration: EVConfigurationStore? = nil, launchArguments: EVLaunchArguments = EVLaunchArguments(),
         launchDirectory: URL = URL(fileURLWithPath: FileManager.default.currentDirectoryPath, isDirectory: true)) {
        let configuration = configuration ?? .shared
        self.configuration = configuration
        self.launchArguments = launchArguments
        self.launchDirectory = launchDirectory
        recordRecentDocument = { try? configuration.recordRecentDocument($0) }
        super.init()
    }

    func applicationWillFinishLaunching(_ notification: Notification) {
        let builder = EVMenuBuilder(owner: self, recentDocumentURLs: { [configuration] in
            configuration.recentDocumentURLs
        }, themeStore: themeStore)
        NSApplication.shared.mainMenu = builder.buildMainMenu(for: NSApplication.shared)
        menuBuilder = builder
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        openLaunchArguments()
        NSApplication.shared.activate(ignoringOtherApps: true)
    }

    func applicationDidBecomeActive(_ notification: Notification) {
        try? configuration.ensureCurrentThemeExists()
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
        processLaunchArguments(EVLaunchArguments(), workingDirectory: launchDirectory)
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
            settingsWindowController = EVSettingsWindowController(store: themeStore)
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

    @discardableResult
    func openLaunchArguments() -> Bool {
        let hasLaunchOptions = !launchArguments.isEmpty
        guard !hasProcessedLaunchArguments else { return hasLaunchOptions }
        hasProcessedLaunchArguments = true
        processLaunchArguments(launchArguments, workingDirectory: launchDirectory)
        return hasLaunchOptions
    }

    /// Startup and later executable invocations share this dispatch path.
    /// Serializing requests also prevents nested recovery panels from opening
    /// the same file twice before the first request has installed its window.
    func processLaunchArguments(_ arguments: EVLaunchArguments, workingDirectory: URL) {
        pendingLaunches.append((arguments, workingDirectory))
        processNextLaunch()
    }

    private func processNextLaunch() {
        guard !isProcessingLaunch, !pendingLaunches.isEmpty else { return }
        isProcessingLaunch = true
        let (arguments, workingDirectory) = pendingLaunches.removeFirst()
        guard !arguments.isEmpty else {
            if hasNoOpenWindows {
                launchPlaceholderDocument = createUntitledDocument()
            } else if let window = mainWindow() ?? applicationWindows().first(where: {
                !($0 is NSPanel) && ($0.isVisible || $0.isMiniaturized)
            }) {
                if window.isMiniaturized { window.deminiaturize(nil) }
                window.makeKeyAndOrderFront(nil)
            }
            finishLaunch()
            return
        }
        let document = documentFactory()
        document.recordRecentDocument = recordRecentDocument
        NSDocumentController.shared.addDocument(document)
        document.makeWindowControllers()
        launchPlaceholderDocument = document
        guard let controller = document.windowControllers.first as? EVDocumentWindowController else {
            document.showWindows()
            finishLaunch()
            return
        }
        controller.argumentDocumentOpener = { [weak self] url, _, completion in
            guard let self else { completion(nil, CocoaError(.userCancelled)); return }
            do { completion(try self.loadLaunchDocument(at: url), nil) }
            catch { completion(nil, error) }
        }
        let urls = arguments.filenames.map {
            URL(fileURLWithPath: $0, relativeTo: workingDirectory).absoluteURL
        }
        controller.openArgumentList(urls, splitCount: arguments.splitCount,
            initialLine: arguments.initialLine) { [self] result in
            if case let .failure(error) = result {
                NSApplication.shared.presentError(error)
            }
            finishLaunch()
        }
    }

    private func finishLaunch() {
        NSApplication.shared.activate(ignoringOtherApps: true)
        isProcessingLaunch = false
        processNextLaunch()
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
            try document.read(
                from: Data(),
                ofType: EVDocument.defaultOpeningType(for: url, nativeType: type)
            )
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
        case "html", "htm", "xhtml":
            EVDocument.codeType
        default:
            EVDocument.plainTextType
        }
    }
}
