import AppKit

/// AppKit may ask a window to constrain itself before it has been assigned a
/// screen during direct SwiftPM application startup. NSWindow's default
/// no-screen result collapses the requested frame to its fitting/minimum size.
/// Preserve the requested frame until a real screen exists, then use AppKit's
/// normal constraints for native moving, tiling, zooming, and resizing.
private final class EVDocumentWindow: NSWindow {
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect {
        guard let screen else { return frameRect }
        return super.constrainFrameRect(frameRect, to: screen)
    }
}

@MainActor
public final class EVDocumentWindowController: NSWindowController, EVDocumentHostEffectHandling {
    static let initialContentSize = NSSize(width: 920, height: 680)
    static let minimumContentSize = NSSize(width: 480, height: 280)

    public let editorSurface: any EVEditorSurface
    let documentContentController: EVDocumentContentViewController
    private weak var hostDocument: EVDocument?
    private var hasPresentedInitialWindow = false
    private var isPerformingDocumentHostEffect = false

    var currentGeometry: EVDocumentWindowGeometry? {
        guard let window, let contentView = synchronizeContentFrame(of: window) else { return nil }
        return documentContentController.geometry(in: contentView)
    }

    public init(document: EVDocument, editorSurface: any EVEditorSurface) {
        self.editorSurface = editorSurface
        hostDocument = document
        documentContentController = EVDocumentContentViewController(editorSurface: editorSurface)

        let window = EVDocumentWindow(
            contentRect: NSRect(origin: .zero, size: Self.initialContentSize),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        let contentView = documentContentController.view
        contentView.frame = NSRect(origin: .zero, size: Self.initialContentSize)
        contentView.translatesAutoresizingMaskIntoConstraints = false
        window.contentView = contentView
        window.contentMinSize = Self.minimumContentSize
        window.setContentSize(Self.initialContentSize)
        // Automatic tabbing can replace a just-created window's requested
        // frame with the geometry of an unrelated existing tab group. eVim's
        // initial UI has document windows, not a tab model, so opt out here.
        window.tabbingMode = .disallowed
        // State restoration needs a restoration class and stable document
        // identity. Advertising restoration without either can resurrect a
        // newly created untitled window in a stale miniaturized state.
        window.isRestorable = false
        window.titleVisibility = .visible

        super.init(window: window)
        // NSDocument.addWindowController(_:) is the sole owner of attaching
        // this controller to its document. Pre-setting `document` here makes
        // AppKit treat the subsequent add as a no-op, leaving the document with
        // no retained window controllers.
        // The initial frame is explicitly centered below. NSWindowController's
        // cascade machinery is useful for nib/restored windows, but can mutate
        // a programmatic document window while it is first being shown.
        shouldCascadeWindows = false
        documentContentController.document = document
        (editorSurface as? any EVDocumentHostAttachable)?.documentHostEffectHandler = self
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }

    public override func windowDidLoad() {
        super.windowDidLoad()
        window?.makeFirstResponder(editorSurface.viewController.view)
    }

    public override func showWindow(_ sender: Any?) {
        guard let window else {
            super.showWindow(sender)
            return
        }

        let isInitialPresentation = !hasPresentedInitialWindow
        if isInitialPresentation {
            prepareInitialFrame(of: window)
        }
        super.showWindow(sender)
        if window.isMiniaturized {
            window.deminiaturize(sender)
        }
        if isInitialPresentation {
            // Apply once more after AppKit has ordered the programmatic window.
            // This defeats any pre-show tiling/restoration candidate without
            // fighting subsequent user resizing.
            prepareInitialFrame(of: window)
            hasPresentedInitialWindow = true
        }
        _ = synchronizeContentFrame(of: window)
        window.makeFirstResponder(editorSurface.viewController.view)
        window.makeKeyAndOrderFront(sender)
    }

    private func prepareInitialFrame(of window: NSWindow) {
        window.setContentSize(Self.initialContentSize)
        // `center()` has surprising destructive behavior before AppKit has
        // assigned a screen: it can collapse the window to its fitting size.
        // This occurs during early launch (and in headless AppKit tests).
        if window.screen != nil {
            window.center()
        }
        _ = synchronizeContentFrame(of: window)
    }

    @discardableResult
    private func synchronizeContentFrame(of window: NSWindow) -> NSView? {
        guard let contentView = window.contentView else { return nil }
        let contentSize = window.contentLayoutRect.size
        if contentView.frame.size != contentSize || contentView.frame.origin != .zero {
            contentView.frame = NSRect(origin: .zero, size: contentSize)
        }
        documentContentController.layoutContent()
        return contentView
    }
}

@MainActor
private extension EVDocumentWindowController {
    func finishDocumentHostEffect(
        _ result: Result<String?, Error>,
        completion: @escaping @MainActor (Result<String?, Error>) -> Void
    ) {
        isPerformingDocumentHostEffect = false
        completion(result)
    }

    func performDocumentHostRequests(
        _ requests: ArraySlice<EVDocumentHostRequest>,
        messages: [String],
        completion: @escaping @MainActor (Result<String?, Error>) -> Void
    ) {
        guard let request = requests.first else {
            finishDocumentHostEffect(
                .success(messages.isEmpty ? nil : messages.joined(separator: "\n")),
                completion: completion
            )
            return
        }
        performDocumentHostRequest(request) { [weak self] result in
            guard let self else {
                completion(.failure(EVDocumentHostError.unsupportedRequest))
                return
            }
            switch result {
            case let .success(message):
                var nextMessages = messages
                if let message, !message.isEmpty { nextMessages.append(message) }
                self.performDocumentHostRequests(
                    requests.dropFirst(),
                    messages: nextMessages,
                    completion: completion
                )
            case let .failure(error):
                self.finishDocumentHostEffect(.failure(error), completion: completion)
            }
        }
    }

    func performDocumentHostRequest(
        _ request: EVDocumentHostRequest,
        completion: @escaping @MainActor (Result<String?, Error>) -> Void
    ) {
        guard let document = hostDocument else {
            completion(.failure(EVDocumentHostError.unsupportedRequest))
            return
        }
        let persistence = document.editorBackend.persistenceState
        guard persistence.documentID == request.documentID,
              persistence.documentRevision == request.documentRevision
        else {
            completion(.failure(EVDocumentHostError.staleRequest))
            return
        }

        switch request.kind {
        case .write:
            guard request.path == nil, request.hardLineRange == nil else {
                completion(.failure(EVDocumentHostError.preparedWriteUnavailable))
                return
            }
            save(
                document,
                request: request,
                destination: document.fileURL,
                operation: document.fileURL == nil ? .saveAsOperation : .saveOperation,
                completion: completion
            )

        case .saveAs:
            guard request.hardLineRange == nil,
                  let path = request.path,
                  let destination = resolvedFileURL(path, relativeTo: document.fileURL)
            else {
                completion(.failure(EVDocumentHostError.invalidPath(request.path ?? "")))
                return
            }
            save(
                document,
                request: request,
                destination: destination,
                operation: .saveAsOperation,
                completion: completion
            )

        case .writeQuit:
            guard request.path == nil, request.hardLineRange == nil else {
                completion(.failure(EVDocumentHostError.preparedWriteUnavailable))
                return
            }
            save(
                document,
                request: request,
                destination: document.fileURL,
                operation: document.fileURL == nil ? .saveAsOperation : .saveOperation
            ) { [weak self] result in
                guard case .success = result else {
                    completion(result)
                    return
                }
                self?.closeCurrentDocumentOrWindow(document)
                completion(result)
            }

        case .xit:
            guard request.path == nil else {
                completion(.failure(EVDocumentHostError.preparedWriteUnavailable))
                return
            }
            guard persistence.isDirty else {
                closeCurrentDocumentOrWindow(document)
                completion(.success(nil))
                return
            }
            save(
                document,
                request: request,
                destination: document.fileURL,
                operation: document.fileURL == nil ? .saveAsOperation : .saveOperation
            ) { [weak self] result in
                guard case .success = result else {
                    completion(result)
                    return
                }
                self?.closeCurrentDocumentOrWindow(document)
                completion(result)
            }

        case .quit:
            guard request.force || !persistence.isDirty else {
                completion(.failure(EVDocumentHostError.documentModified))
                return
            }
            closeCurrentDocumentOrWindow(document)
            completion(.success(nil))

        case .quitAll:
            var documents = NSDocumentController.shared.documents.compactMap { $0 as? EVDocument }
            if !documents.contains(where: { $0 === document }) { documents.append(document) }
            guard request.force || documents.allSatisfy({ !$0.editorBackend.persistenceState.isDirty }) else {
                completion(.failure(EVDocumentHostError.documentModified))
                return
            }
            for candidate in documents { candidate.close() }
            completion(.success(nil))

        case .writeAll:
            var documents = NSDocumentController.shared.documents.compactMap { $0 as? EVDocument }
            if !documents.contains(where: { $0 === document }) { documents.append(document) }
            saveAll(documents[...], force: request.force, completion: completion)

        case .edit:
            guard request.force || !persistence.isDirty else {
                completion(.failure(EVDocumentHostError.documentModified))
                return
            }
            edit(document, path: request.path, completion: completion)

        case .new:
            guard request.force || !persistence.isDirty else {
                completion(.failure(EVDocumentHostError.documentModified))
                return
            }
            do {
                let newDocument = try NSDocumentController.shared.makeUntitledDocument(
                    ofType: EVDocument.plainTextType
                )
                NSDocumentController.shared.addDocument(newDocument)
                newDocument.makeWindowControllers()
                newDocument.showWindows()
                closeCurrentDocumentOrWindow(document)
                completion(.success(nil))
            } catch {
                completion(.failure(error))
            }
        }
    }

    func save(
        _ document: EVDocument,
        request: EVDocumentHostRequest,
        destination: URL?,
        operation: NSDocument.SaveOperationType,
        completion: @escaping @MainActor (Result<String?, Error>) -> Void
    ) {
        if let destination {
            let typeName = documentType(for: destination, fallback: document.fileType)
            document.saveHostRevision(
                documentID: request.documentID,
                documentRevision: request.documentRevision,
                to: destination,
                ofType: typeName,
                for: operation
            ) { error in
                if let error {
                    completion(.failure(error))
                } else {
                    completion(.success("\(destination.path) written"))
                }
            }
            return
        }

        let panel = NSSavePanel()
        panel.canCreateDirectories = true
        panel.nameFieldStringValue = document.displayName == "Untitled"
            ? "Untitled.txt"
            : document.displayName
        guard document.prepareSavePanel(panel), let sheetWindow = document.windowForSheet else {
            completion(.failure(EVDocumentHostError.noDocumentURL))
            return
        }
        panel.beginSheetModal(for: sheetWindow) { [weak document] response in
            guard response == .OK, let document, let destination = panel.url else {
                completion(.failure(EVDocumentHostError.saveCancelledOrFailed))
                return
            }
            let typeName = self.documentType(for: destination, fallback: document.fileType)
            document.saveHostRevision(
                documentID: request.documentID,
                documentRevision: request.documentRevision,
                to: destination,
                ofType: typeName,
                for: .saveAsOperation
            ) { error in
                if let error {
                    completion(.failure(error))
                } else {
                    completion(.success("\(destination.path) written"))
                }
            }
        }
    }

    func saveAll(
        _ documents: ArraySlice<EVDocument>,
        force: Bool,
        completion: @escaping @MainActor (Result<String?, Error>) -> Void
    ) {
        guard let document = documents.first else {
            completion(.success(nil))
            return
        }
        let state = document.editorBackend.persistenceState
        guard state.isDirty else {
            saveAll(documents.dropFirst(), force: force, completion: completion)
            return
        }
        let request = EVDocumentHostRequest(
            kind: .write,
            documentID: state.documentID,
            documentRevision: state.documentRevision,
            force: force
        )
        save(
            document,
            request: request,
            destination: document.fileURL,
            operation: document.fileURL == nil ? .saveAsOperation : .saveOperation
        ) { result in
            switch result {
            case .success:
                self.saveAll(documents.dropFirst(), force: force, completion: completion)
            case .failure:
                completion(result)
            }
        }
    }

    func edit(
        _ document: EVDocument,
        path: String?,
        completion: @escaping @MainActor (Result<String?, Error>) -> Void
    ) {
        if let path {
            guard let url = resolvedFileURL(path, relativeTo: document.fileURL) else {
                completion(.failure(EVDocumentHostError.invalidPath(path)))
                return
            }
            if url.standardizedFileURL == document.fileURL?.standardizedFileURL {
                do {
                    try document.revert(
                        toContentsOf: url,
                        ofType: documentType(for: url, fallback: document.fileType)
                    )
                    completion(.success(nil))
                } catch {
                    completion(.failure(error))
                }
                return
            }
            NSDocumentController.shared.openDocument(withContentsOf: url, display: true) {
                [weak self] openedDocument, _, error in
                if let error {
                    completion(.failure(error))
                } else if openedDocument != nil {
                    self?.closeCurrentDocumentOrWindow(document)
                    completion(.success(nil))
                } else {
                    completion(.failure(EVDocumentHostError.unsupportedRequest))
                }
            }
            return
        }

        guard let url = document.fileURL else {
            completion(.failure(EVDocumentHostError.noDocumentURL))
            return
        }
        do {
            try document.revert(
                toContentsOf: url,
                ofType: documentType(for: url, fallback: document.fileType)
            )
            completion(.success(nil))
        } catch {
            completion(.failure(error))
        }
    }

    func closeCurrentDocumentOrWindow(_ document: EVDocument) {
        if document.windowControllers.count > 1 {
            close()
            document.removeWindowController(self)
        } else {
            document.close()
        }
    }

    func resolvedFileURL(_ path: String, relativeTo currentURL: URL?) -> URL? {
        guard !path.isEmpty, !path.utf8.contains(0) else { return nil }
        let expanded = (path as NSString).expandingTildeInPath
        if expanded.hasPrefix("/") {
            return URL(fileURLWithPath: expanded).standardizedFileURL
        }
        let base = currentURL?.deletingLastPathComponent()
            ?? URL(fileURLWithPath: FileManager.default.currentDirectoryPath, isDirectory: true)
        return URL(fileURLWithPath: expanded, relativeTo: base).standardizedFileURL
    }

    func documentType(for url: URL, fallback: String?) -> String {
        switch url.pathExtension.lowercased() {
        case "md", "markdown", "mdown": EVDocument.markdownType
        case "txt", "text": EVDocument.plainTextType
        default: fallback ?? EVDocument.plainTextType
        }
    }
}

@MainActor
public extension EVDocumentWindowController {
    func perform(
        documentHostRequests: [EVDocumentHostRequest],
        completion: @escaping @MainActor (Result<String?, Error>) -> Void
    ) {
        guard !isPerformingDocumentHostEffect else {
            completion(.failure(EVDocumentHostError.operationAlreadyInProgress))
            return
        }
        isPerformingDocumentHostEffect = true
        performDocumentHostRequests(
            documentHostRequests[...],
            messages: [],
            completion: completion
        )
    }
}

struct EVDocumentWindowGeometry: Equatable {
    let content: NSRect
    let editor: NSRect
    let statusBar: NSRect
    let statusBarIsVisible: Bool
}

@MainActor
final class EVDocumentContentViewController: NSViewController,
    EVEditorCommandRouting,
    NSMenuItemValidation
{
    weak var document: EVDocument?

    private let editorSurface: any EVEditorSurface
    private let statusBar = EVStatusBarView()
    private var showsStatusBar: Bool

    init(editorSurface: any EVEditorSurface) {
        self.editorSurface = editorSurface
        showsStatusBar = UserDefaults.standard.object(forKey: "EVShowStatusBar") as? Bool ?? true
        super.init(nibName: nil, bundle: nil)

        editorSurface.statusBarStateDidChange = { [weak self] state in
            self?.statusBar.apply(state)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }

    override func loadView() {
        let root = NSView()
        // Programmatic content views do not receive the nib loader's default
        // width/height autoresizing mask. Keep the controller root matched to
        // the window's content rect as the window is shown and resized.
        root.autoresizingMask = [.width, .height]

        addChild(editorSurface.viewController)
        let editorView = editorSurface.viewController.view
        editorView.translatesAutoresizingMaskIntoConstraints = true
        editorView.autoresizingMask = [.width, .height]
        statusBar.translatesAutoresizingMaskIntoConstraints = true
        statusBar.autoresizingMask = [.width, .maxYMargin]
        root.addSubview(editorView)
        root.addSubview(statusBar)

        statusBar.apply(editorSurface.statusBarState)
        statusBar.isHidden = !showsStatusBar
        view = root
        layoutContent()
    }

    func geometry(in root: NSView) -> EVDocumentWindowGeometry {
        loadViewIfNeeded()
        layoutContent()
        let editorView = editorSurface.viewController.view
        return EVDocumentWindowGeometry(
            content: root.bounds,
            editor: editorView.convert(editorView.bounds, to: root),
            statusBar: statusBar.convert(statusBar.bounds, to: root),
            statusBarIsVisible: !statusBar.isHidden
        )
    }

    func layoutContent() {
        loadViewIfNeeded()
        let bounds = view.bounds
        let statusHeight = showsStatusBar ? EVStatusBarView.preferredHeight : 0
        let editorView = editorSurface.viewController.view
        statusBar.frame = NSRect(
            x: bounds.minX,
            y: bounds.minY,
            width: bounds.width,
            height: statusHeight
        )
        editorView.frame = NSRect(
            x: bounds.minX,
            y: bounds.minY + statusHeight,
            width: bounds.width,
            height: max(0, bounds.height - statusHeight)
        )
        statusBar.layoutSubtreeIfNeeded()
        editorView.layoutSubtreeIfNeeded()
    }

    @objc func performEditorMenuCommand(_ sender: Any?) {
        guard
            let menuItem = sender as? NSMenuItem,
            let command = EVMenuCommand(rawValue: menuItem.tag)
        else { return }

        if command == .newWindowForDocument {
            document?.showAdditionalWindow()
        } else {
            editorSurface.perform(menuCommand: command, sender: sender)
        }
    }

    @objc func toggleStatusBar(_ sender: Any?) {
        showsStatusBar.toggle()
        statusBar.isHidden = !showsStatusBar
        layoutContent()
        UserDefaults.standard.set(showsStatusBar, forKey: "EVShowStatusBar")
    }

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if menuItem.action == #selector(toggleStatusBar(_:)) {
            menuItem.state = showsStatusBar ? .on : .off
            return true
        }

        guard
            menuItem.action == #selector(performEditorMenuCommand(_:)),
            let command = EVMenuCommand(rawValue: menuItem.tag)
        else { return true }

        if command == .newWindowForDocument {
            return document != nil
        }

        let presentation = editorSurface.presentation(for: command)
        menuItem.state = presentation.state
        if let title = presentation.title {
            menuItem.title = title
        }
        return presentation.isEnabled
    }
}
