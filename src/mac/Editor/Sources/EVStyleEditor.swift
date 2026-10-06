import AppKit
import CViemCore
import ViemAppShell

enum EVStyleEditorTab: Int {
    case character
    case paragraph
    case block
}

struct EVStyleEditorInspection: Equatable {
    let targetDocumentIdentity: ObjectIdentifier?
    let targetCoreDocumentID: UInt64?
    let styleSheetRevision: UInt64?
    let selectedStyleID: EVStyleID?
    let selectedStyleKey: EVStyleKey?
    let selectedKind: EVStyleKind?
    let selectedTab: EVStyleEditorTab
    let blockTabEnabled: Bool
    let paragraphTabEnabled: Bool
    let hasDocument: Bool
    let mutationsEnabled: Bool
    let parentValue: String
    let parentChoices: [EVStyleKey]
    let styleCount: Int
    let characterPropertyCount: Int
    let paragraphPropertyCount: Int
    let blockPropertyCount: Int
    let declaredProperties: Set<EVStyleProperty>
    let hasInvalidDraft: Bool
    let diagnostic: String
    let preview: EVCoreTextStylePreviewInspection
}

private final class EVStyleEditorPanel: NSPanel {
    // Use utility chrome without changing the editor's existing main-window
    // responder routing when users work in its controls.
    override var canBecomeMain: Bool { true }
    var settingsUndoManager: UndoManager?
    override var undoManager: UndoManager? { settingsUndoManager ?? super.undoManager }
}

@MainActor
final class EVStyleEditorCoordinator: NSObject, NSWindowDelegate {
    static let shared = EVStyleEditorCoordinator()

    private var controller: NSWindowController?
    private var contentController: EVStyleEditorViewController?
    private var targetWindowObserver: NSObjectProtocol?
    private var activationObservers: [NSObjectProtocol] = []
    private var selectionObserver: NSObjectProtocol?
    private var familyObserver: NSObjectProtocol?
    private var syntaxObserver: NSObjectProtocol?
    private weak var followedDocument: EVEditorSurfaceController?
    private var followsCaretStyle = false
    private var selectionFollowTimer: Timer?
    private(set) var caretFollowQueryCount = 0
    private var globalSession: EVCodeStyleSession?
    private var themeSession: EVThemeStyleSession?
    private var themeSessions: [String: EVThemeStyleSession] = [:]
    private var followsActiveDocuments = false

    var styleWindow: NSWindow? { controller?.window }
    var inspection: EVStyleEditorInspection? { contentController?.inspection }

    func show(document: EVEditorSurfaceController, sender: Any?) {
        followsActiveDocuments = true
        let styleKey = document.currentStyleEditorKey()
        if document.backend.sourceFormat == .code {
            showCode(configuration: document.backend.configuration, preferredStyle: styleKey,
                     following: document, sender: sender)
            return
        }
        do {
            let session = try proseSession(configuration: document.backend.configuration,
                format: document.backend.sourceFormat)
            themeSession = session
            stopFollowingSelection()
            observeTargetWindow(of: document)
            let isNewWindow = prepareWindow()
            (controller?.window as? EVStyleEditorPanel)?.settingsUndoManager = session.undoManager
            controller?.window?.title = session.windowTitle
            contentController?.retarget(settingsSession: session)
            contentController?.followCaretStyle(styleKey)
            followSelection(of: document, globalCode: false)
            present(sender: sender, center: isNewWindow)
        } catch { NSApplication.shared.presentError(error) }
    }

    func showCode(
        configuration: EVConfigurationStore,
        preferredStyle: EVStyleKey = .baseParagraph,
        definingSyntaxName: String? = nil,
        following document: EVEditorSurfaceController? = nil,
        sender: Any?
    ) {
        followsActiveDocuments = document != nil
        do {
            themeSession = nil
            let session = try codeSession(configuration: configuration)
            var selectedStyle = preferredStyle
            if let name = definingSyntaxName {
                // Generating an implicit definition is not a settings edit: it
                // has no undo entry and is never written to a theme file.
                session.endGroup()
                try EVCoreStyleBridge.materializeCodeStyle(name: name)
                if let definition = try session.snapshot().definitions.first(where: {
                    $0.kind == .character && $0.name == name
                }) {
                    selectedStyle = definition.key
                }
            }
            stopFollowingSelection()
            stopObservingTargetWindow()
            let isNewWindow = prepareWindow()
            (controller?.window as? EVStyleEditorPanel)?.settingsUndoManager = session.undoManager
            controller?.window?.title = session.windowTitle
            contentController?.retarget(settingsSession: session)
            contentController?.selectStyle(selectedStyle)
            if let document {
                observeTargetWindow(of: document)
                followSelection(of: document, globalCode: true)
            }
            present(sender: sender, center: isNewWindow)
        } catch {
            EVCodePreferences.shared.reportLoadDiagnostics([error.localizedDescription])
        }
    }

    private func prepareWindow() -> Bool {
        let isNewWindow = controller == nil
        if isNewWindow {
            for name in [NSWindow.didBecomeKeyNotification, .viemActiveEditorSurfaceDidChange] {
                activationObservers.append(NotificationCenter.default.addObserver(forName: name,
                    object: nil, queue: .main) { [weak self] notification in
                    MainActor.assumeIsolated {
                        let surface = (notification.object as? EVEditorSurfaceController)
                            ?? ((notification.object as? NSWindow)?.windowController as? EVDocumentWindowController)?
                                .editorSurface as? EVEditorSurfaceController
                        if let surface { self?.documentDidBecomeActive(surface) }
                    }
                })
            }
            let content = EVStyleEditorViewController()
            content.onClose = { [weak self] in self?.controller?.close() }
            content.onExplicitStyleSelection = { [weak self] in
                self?.followsCaretStyle = false
                self?.cancelPendingSelectionFollow()
            }
            content.onDocumentSelection = { [weak self] format, configuration in
                try self?.selectDocument(format, configuration: configuration)
            }
            let panel = EVStyleEditorPanel(
                contentRect: NSRect(x: 0, y: 0, width: 760, height: 600),
                styleMask: [.titled, .closable, .resizable, .utilityWindow],
                backing: .buffered,
                defer: false
            )
            panel.title = "Styles"
            panel.isFloatingPanel = false
            panel.level = .normal
            panel.hidesOnDeactivate = false
            panel.contentMinSize = NSSize(width: 700, height: 570)
            panel.backgroundColor = .windowBackgroundColor
            panel.isReleasedWhenClosed = false
            panel.collectionBehavior.insert(.fullScreenAuxiliary)
            panel.contentViewController = content
            panel.delegate = self
            panel.setFrameAutosaveName("Viem Style Editor")
            panel.setAccessibilityLabel("Styles")
            controller = NSWindowController(window: panel)
            contentController = content
        }

        return isNewWindow
    }

    private func present(sender: Any?, center: Bool) {
        controller?.showWindow(sender)
        if center { controller?.window?.center() }
        controller?.window?.makeKeyAndOrderFront(sender)
    }

    func documentDidClose(_ document: EVEditorSurfaceController) {
        guard followedDocument === document else { return }
        // Theme settings outlive the source view that selected their family.
        stopFollowingSelection()
        stopObservingTargetWindow()
    }

    func documentDidBecomeActive(_ document: EVEditorSurfaceController) {
        guard controller != nil, followsActiveDocuments, document.session != nil,
              followedDocument !== document else { return }
        // Returning from the inspector to the same view preserves an explicit
        // picker choice. A different document selects its current caret style
        // immediately, without raising the inspector or taking keyboard focus.
        retargetFollowingFamily(of: document)
        observeTargetWindow(of: document)
    }

    func selectStyle(_ id: EVStyleID) { contentController?.selectStyle(id) }
    func selectStyle(_ key: EVStyleKey) { contentController?.selectStyle(key) }
    func selectTab(_ tab: EVStyleEditorTab) { contentController?.selectTab(tab) }
    func close() { controller?.close() }

    func windowWillClose(_ notification: Notification) {
        guard notification.object as AnyObject? === controller?.window else { return }
        stopObservingTargetWindow()
        stopFollowingSelection()
        contentController?.disableForClosedDocument()
        for observer in activationObservers { NotificationCenter.default.removeObserver(observer) }
        activationObservers.removeAll()
        controller = nil
        contentController = nil
        themeSession = nil
        followsActiveDocuments = false
    }

    private func followSelection(of document: EVEditorSurfaceController, globalCode: Bool) {
        stopFollowingSelection()
        followedDocument = document
        followsCaretStyle = true
        familyObserver = NotificationCenter.default.addObserver(
            forName: .viemCoreDocumentDidChange, object: document.backend, queue: .main
        ) { [weak self, weak document] _ in
            MainActor.assumeIsolated {
                guard let self, let document else { return }
                let family = self.themeSession?.sourceFormat.defaultStyleName ?? "code"
                if family != document.backend.sourceFormat.defaultStyleName {
                    self.retargetFollowingFamily(of: document)
                }
            }
        }
        selectionObserver = NotificationCenter.default.addObserver(
            forName: .viemEditorSelectionDidChange, object: document, queue: .main
        ) { [weak self, weak document] _ in
            guard let self, let document else { return }
            MainActor.assumeIsolated {
                let format = document.backend.sourceFormat
                let currentFamily = self.themeSession?.sourceFormat.defaultStyleName ?? "code"
                if currentFamily != format.defaultStyleName {
                    self.retargetFollowingFamily(of: document)
                    return
                }
                self.followsCaretStyle = true
                self.scheduleSelectionFollow(of: document, globalCode: globalCode)
            }
        }
        syntaxObserver = NotificationCenter.default.addObserver(
            forName: .viemCoreSyntaxDidChange, object: document.backend, queue: .main
        ) { [weak self, weak document] _ in
            MainActor.assumeIsolated {
                guard let self, let document, self.followedDocument === document,
                      self.followsCaretStyle, document.backend.sourceFormat == .code,
                      self.selectionFollowTimer == nil else { return }
                // A mode switch or caret move can precede syntax publication.
                // Finish that lookup when styles arrive, unless the user has
                // since selected a definition explicitly. Do not restart an
                // existing debounce timer for each background publication.
                self.scheduleSelectionFollow(of: document, globalCode: true)
            }
        }
    }

    private func proseSession(configuration: EVConfigurationStore, format: EVSourceFormat) throws -> EVThemeStyleSession {
        let family = format.defaultStyleName
        if let session = themeSessions[family], session.configuration === configuration { return session }
        let session = try EVThemeStyleSession(configuration: configuration, format: format)
        themeSessions[family] = session
        return session
    }

    private func selectDocument(_ format: EVSourceFormat, configuration: EVConfigurationStore) throws {
        let session: any EVStyleSettingsSession
        if format == .code {
            let code = try codeSession(configuration: configuration)
            themeSession = nil
            session = code
        } else {
            let theme = try proseSession(configuration: configuration, format: format)
            themeSession = theme
            session = theme
        }
        // An explicit family choice is independent of the current document.
        // Opening Edit Styles again resumes following the active view.
        followsActiveDocuments = false
        stopFollowingSelection()
        stopObservingTargetWindow()
        (controller?.window as? EVStyleEditorPanel)?.settingsUndoManager = session.undoManager
        contentController?.retarget(settingsSession: session)
    }

    private func codeSession(configuration: EVConfigurationStore) throws -> EVCodeStyleSession {
        if let session = globalSession, session.configuration === configuration { return session }
        let session = try EVCodeStyleSession(configuration: configuration)
        globalSession = session
        return session
    }

    private func retargetFollowingFamily(of document: EVEditorSurfaceController) {
        do {
            let configuration = document.backend.configuration
            let session: any EVStyleSettingsSession
            if document.backend.sourceFormat == .code {
                let code = try codeSession(configuration: configuration)
                themeSession = nil
                session = code
            } else {
                let theme = try proseSession(configuration: configuration, format: document.backend.sourceFormat)
                themeSession = theme
                session = theme
            }
            (controller?.window as? EVStyleEditorPanel)?.settingsUndoManager = session.undoManager
            contentController?.retarget(settingsSession: session)
            contentController?.followCaretStyle(document.currentStyleEditorKey())
            followSelection(of: document, globalCode: document.backend.sourceFormat == .code)
        } catch { NSApplication.shared.presentError(error) }
    }

    private func stopFollowingSelection() {
        cancelPendingSelectionFollow()
        followsCaretStyle = false
        if let syntaxObserver {
            NotificationCenter.default.removeObserver(syntaxObserver)
            self.syntaxObserver = nil
        }
        if let familyObserver {
            NotificationCenter.default.removeObserver(familyObserver)
            self.familyObserver = nil
        }
        if let selectionObserver {
            NotificationCenter.default.removeObserver(selectionObserver)
            self.selectionObserver = nil
        }
        followedDocument = nil
    }

    private func scheduleSelectionFollow(of document: EVEditorSurfaceController, globalCode: Bool) {
        cancelPendingSelectionFollow()
        // Selection notifications filter out unchanged presentations, scrolling,
        // repainting and stylesheet-only revisions. Syntax publication can also
        // complete a lookup that previously had no captures available.
        // Delay the named-style query itself, not only the control refresh.
        let timer = Timer(timeInterval: 0.5, repeats: false) { [weak self, weak document] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.selectionFollowTimer = nil
                guard let document, self.followedDocument === document,
                      (document.backend.sourceFormat == .code) == globalCode else { return }
                self.caretFollowQueryCount += 1
                self.contentController?.followCaretStyle(document.currentStyleEditorKey())
            }
        }
        selectionFollowTimer = timer
        RunLoop.main.add(timer, forMode: .common)
    }

    private func cancelPendingSelectionFollow() {
        selectionFollowTimer?.invalidate()
        selectionFollowTimer = nil
    }

    var selectionFollowScheduledForTesting: Bool { selectionFollowTimer != nil }
    func settleSelectionFollowForTesting() {
        let timer = selectionFollowTimer
        timer?.fire()
        timer?.invalidate()
    }

    private func observeTargetWindow(of document: EVEditorSurfaceController) {
        stopObservingTargetWindow()
        guard document.isViewLoaded, let window = document.view.window else { return }
        targetWindowObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.willCloseNotification,
            object: window,
            queue: .main
        ) { [weak self, weak document] _ in
            guard let self, let document else { return }
            MainActor.assumeIsolated { self.documentDidClose(document) }
        }
    }

    private func stopObservingTargetWindow() {
        if let targetWindowObserver {
            NotificationCenter.default.removeObserver(targetWindowObserver)
            self.targetWindowObserver = nil
        }
    }


}

@MainActor
final class EVStyleEditorViewController: NSViewController {
    var onClose: (() -> Void)?
    var onExplicitStyleSelection: (() -> Void)?
    var onDocumentSelection: ((EVSourceFormat, EVConfigurationStore) throws -> Void)?
    var themeStore = EVThemeStore.shared
    private var themeObserver: NSObjectProtocol?

    private weak var document: EVEditorSurfaceController?
    private var settingsSession: (any EVStyleSettingsSession)?
    private var codeSettingsSession: EVCodeStyleSession? { settingsSession as? EVCodeStyleSession }
    private var hasTarget: Bool { document != nil || settingsSession != nil }
    override var undoManager: UndoManager? { settingsSession?.undoManager ?? super.undoManager }
    private var styleMenuChoices: [EVStyleChoiceIdentity]?
    private var parentMenuChoices: [EVStyleChoiceIdentity]?
    private var snapshot: EVStyleSheetSnapshot?
    private var selectedStyleKey: EVStyleKey = .baseParagraph
    private var documentObserver: NSObjectProtocol?
    private var isUpdatingUI = false
    private var isCommitting = false
    private var refreshAfterCommit = false
    private var diagnosticMessage = ""
    private var activeStyleEditGroup: EVStyleEditGroup?
    private weak var activeStyleEditSession: EVCoreViewSession?

    private let documentPopup = NSPopUpButton()
    private let documentFormats: [EVSourceFormat] = [.plainText, .markdown, .code]
    private let stylePopup = NSPopUpButton()
    private let restoreDefaultsButton = NSButton(title: "Restore Defaults", target: nil, action: nil)
    private let typeLabel = NSTextField(labelWithString: "")
    private let basedOnPopup = NSPopUpButton()
    private let editParentButton = NSButton()
    private let editNextStyleButton = NSButton()
    private let availabilityLabel = NSTextField(wrappingLabelWithString: "")
    private let tabs = NSSegmentedControl(
        labels: ["Character", "Paragraph", "Block"],
        trackingMode: .selectOne,
        target: nil,
        action: nil
    )
    private let characterControls = NSView()
    private let paragraphControls = NSView()
    private let blockControls = NSView()
    private let preview = EVCoreTextStylePreviewView()
    private let compactControls = EVCompactStyleControls()
    private let nextStyleRow = EVFollowingStyleRow()
    private var formattingHeightConstraint: NSLayoutConstraint?

    private var selectedDefinition: EVStyleDefinition? {
        snapshot?.definition(for: selectedStyleKey)
    }

    var inspection: EVStyleEditorInspection {
        let selectedTab = EVStyleEditorTab(rawValue: tabs.selectedSegment) ?? .character
        let definition = selectedDefinition
        return EVStyleEditorInspection(
            targetDocumentIdentity: document.map { ObjectIdentifier($0.backend) },
            targetCoreDocumentID: snapshot?.identity.documentID,
            styleSheetRevision: snapshot?.identity.styleSheetRevision,
            selectedStyleID: hasTarget ? definition?.key.id : nil,
            selectedStyleKey: hasTarget ? definition?.key : nil,
            selectedKind: definition?.kind,
            selectedTab: selectedTab,
            blockTabEnabled: tabs.isEnabled(forSegment: EVStyleEditorTab.block.rawValue),
            paragraphTabEnabled: tabs.isEnabled(forSegment: EVStyleEditorTab.paragraph.rawValue),
            hasDocument: document != nil,
            mutationsEnabled: definition?.capabilities.contains(.declarations) == true,
            parentValue: basedOnPopup.titleOfSelectedItem ?? "",
            parentChoices: basedOnPopup.itemArray.compactMap { ($0.representedObject as? EVStyleKeyBox)?.key },
            styleCount: snapshot?.definitions.count ?? 0,
            characterPropertyCount: EVStyleProperty.characterProperties.filter { definition?.properties[$0] != nil }.count,
            paragraphPropertyCount: EVStyleProperty.paragraphProperties.filter { definition?.properties[$0] != nil }.count,
            blockPropertyCount: EVStyleProperty.blockProperties.filter { definition?.properties[$0] != nil }.count,
            declaredProperties: Set(definition?.properties.values.compactMap {
                $0.isDeclared ? $0.property : nil
            } ?? []),
            hasInvalidDraft: compactControls.hasInvalidDraft,
            diagnostic: diagnosticMessage,
            preview: preview.inspection()
        )
    }

    deinit {
        if let documentObserver { NotificationCenter.default.removeObserver(documentObserver) }
        if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) }
    }

    override func loadView() {
        themeObserver = NotificationCenter.default.addObserver(forName: .viemThemeDidChange, object: nil, queue: .main) { [weak self] notification in
            MainActor.assumeIsolated {
                guard let self, notification.object as AnyObject? === self.themeStore else { return }
                self.compactControls.refreshThemeColors(self.themeStore.theme)
                if let definition = self.selectedDefinition { self.updatePreview(snapshot: self.snapshot, definition: definition) }
            }
        }
        let root = EVStyleEditorBackgroundView(frame: .zero)
        root.setAccessibilityElement(false)
        documentPopup.addItems(withTitles: ["Plain Text", "Markdown", "Code"])
        documentPopup.target = self
        documentPopup.action = #selector(documentChanged(_:))
        documentPopup.setAccessibilityLabel("Document type")
        documentPopup.widthAnchor.constraint(equalToConstant: 140).isActive = true
        stylePopup.target = self
        stylePopup.action = #selector(styleChanged(_:))
        stylePopup.setAccessibilityLabel("Style")
        stylePopup.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        stylePopup.setContentHuggingPriority(.defaultLow, for: .horizontal)
        typeLabel.textColor = .secondaryLabelColor
        typeLabel.setAccessibilityLabel("Style type")
        basedOnPopup.target = self
        basedOnPopup.action = #selector(basedOnChanged(_:))
        basedOnPopup.setAccessibilityLabel("Based on style")
        configureNavigationButton(editParentButton, label: "Edit based on style", action: #selector(editParentStyle(_:)))
        configureNavigationButton(editNextStyleButton, label: "Edit next paragraph style", action: #selector(editNextStyle(_:)))

        let selectors = NSStackView(views: [label("Document type"), documentPopup, label("Style"), stylePopup])
        selectors.orientation = .horizontal
        selectors.alignment = .centerY
        selectors.distribution = .fill
        selectors.spacing = 8
        selectors.setCustomSpacing(20, after: documentPopup)
        let selectorBox = EVStyleEditorSelectorView()
        selectorBox.setAccessibilityLabel("Stylesheet selection")
        selectorBox.addSubview(selectors)
        selectors.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            selectors.leadingAnchor.constraint(equalTo: selectorBox.leadingAnchor, constant: 14),
            selectors.trailingAnchor.constraint(equalTo: selectorBox.trailingAnchor, constant: -14),
            selectors.topAnchor.constraint(equalTo: selectorBox.topAnchor, constant: 12),
            selectors.bottomAnchor.constraint(equalTo: selectorBox.bottomAnchor, constant: -12),
            selectors.heightAnchor.constraint(equalToConstant: 28),
        ])

        let propertiesGrid = NSGridView(views: [
            [label("Style type"), typeLabel],
            [label("Based on"), navigationRow(popup: basedOnPopup, button: editParentButton)],
            [label("Next paragraph"), navigationRow(popup: nextStyleRow.popupForCompactLayout, button: editNextStyleButton)],
        ])
        configurePropertiesGrid(propertiesGrid)
        let propertiesContainer = centeredContainer(
            propertiesGrid,
            maximumWidth: 570,
            horizontalInset: 12
        )
        availabilityLabel.textColor = .secondaryLabelColor
        availabilityLabel.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        availabilityLabel.setAccessibilityLabel("Style editing status")
        tabs.selectedSegment = EVStyleEditorTab.character.rawValue
        tabs.target = self
        tabs.action = #selector(tabChanged(_:))
        tabs.setAccessibilityLabel("Style property category")

        installPropertyControls()
        let controlsContainer = NSView()
        controlsContainer.addSubview(characterControls)
        controlsContainer.addSubview(paragraphControls)
        controlsContainer.addSubview(blockControls)
        characterControls.translatesAutoresizingMaskIntoConstraints = false
        paragraphControls.translatesAutoresizingMaskIntoConstraints = false
        blockControls.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            characterControls.leadingAnchor.constraint(equalTo: controlsContainer.leadingAnchor),
            characterControls.trailingAnchor.constraint(equalTo: controlsContainer.trailingAnchor),
            characterControls.topAnchor.constraint(equalTo: controlsContainer.topAnchor),
            characterControls.bottomAnchor.constraint(equalTo: controlsContainer.bottomAnchor),
            paragraphControls.leadingAnchor.constraint(equalTo: controlsContainer.leadingAnchor),
            paragraphControls.trailingAnchor.constraint(equalTo: controlsContainer.trailingAnchor),
            paragraphControls.topAnchor.constraint(equalTo: controlsContainer.topAnchor),
            paragraphControls.bottomAnchor.constraint(equalTo: controlsContainer.bottomAnchor),
            blockControls.leadingAnchor.constraint(equalTo: controlsContainer.leadingAnchor),
            blockControls.trailingAnchor.constraint(equalTo: controlsContainer.trailingAnchor),
            blockControls.topAnchor.constraint(equalTo: controlsContainer.topAnchor),
            blockControls.bottomAnchor.constraint(equalTo: controlsContainer.bottomAnchor),
        ])

        let formattingBox = EVStyleEditorSectionView()
        formattingBox.addSubview(controlsContainer)
        controlsContainer.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            controlsContainer.leadingAnchor.constraint(equalTo: formattingBox.leadingAnchor, constant: 8),
            controlsContainer.trailingAnchor.constraint(equalTo: formattingBox.trailingAnchor, constant: -8),
            controlsContainer.topAnchor.constraint(equalTo: formattingBox.topAnchor, constant: 7),
            controlsContainer.bottomAnchor.constraint(equalTo: formattingBox.bottomAnchor, constant: -7),
        ])

        let previewBox = EVStyleEditorPreviewContainer()
        previewBox.addSubview(preview)
        preview.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            preview.leadingAnchor.constraint(equalTo: previewBox.leadingAnchor, constant: 12),
            preview.trailingAnchor.constraint(equalTo: previewBox.trailingAnchor, constant: -12),
            preview.topAnchor.constraint(equalTo: previewBox.topAnchor, constant: 10),
            preview.bottomAnchor.constraint(equalTo: previewBox.bottomAnchor, constant: -10),
        ])

        let closeButton = NSButton(title: "Close", target: self, action: #selector(closePressed(_:)))
        closeButton.bezelStyle = .rounded
        closeButton.setAccessibilityLabel("Close style editor")
        closeButton.widthAnchor.constraint(greaterThanOrEqualToConstant: 82).isActive = true
        restoreDefaultsButton.target = self
        restoreDefaultsButton.action = #selector(restoreCodeDefaults(_:))
        restoreDefaultsButton.bezelStyle = .rounded
        restoreDefaultsButton.isHidden = true
        let bottom = NSStackView(views: [restoreDefaultsButton, NSView(), closeButton])
        bottom.orientation = .horizontal
        bottom.alignment = .centerY

        let tabsContainer = centeredContainer(tabs, maximumWidth: 320, horizontalInset: 0)

        let stack = NSStackView(views: [
            selectorBox, propertiesContainer, availabilityLabel,
            separator(), tabsContainer, formattingBox,
            previewBox, bottom,
        ])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        for arranged in stack.arrangedSubviews where !(arranged is NSTextField) {
            arranged.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        }
        stack.setCustomSpacing(10, after: propertiesContainer)
        stack.setCustomSpacing(10, after: selectorBox)
        root.addSubview(stack)
        let preferredFormattingHeight = formattingBox.heightAnchor.constraint(equalToConstant: 196)
        formattingHeightConstraint = preferredFormattingHeight
        let preferredPreviewHeight = previewBox.heightAnchor.constraint(equalToConstant: 140)
        preferredPreviewHeight.priority = .defaultHigh
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 24),
            stack.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -24),
            stack.topAnchor.constraint(equalTo: root.topAnchor, constant: 16),
            stack.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -14),
            preferredFormattingHeight,
            formattingBox.heightAnchor.constraint(greaterThanOrEqualToConstant: 150),
            preferredPreviewHeight,
            previewBox.heightAnchor.constraint(greaterThanOrEqualToConstant: 92),
        ])
        formattingBox.setContentCompressionResistancePriority(.required, for: .vertical)
        previewBox.setContentCompressionResistancePriority(.required, for: .vertical)
        view = root
        renderNoDocument()
    }

    func retarget(document: EVEditorSurfaceController, styleID: EVStyleID) {
        let namespace: EVStyleNamespace = styleID == .defaultParagraph ? .character : .block
        retarget(document: document, styleKey: EVStyleKey(namespace: namespace, id: styleID))
    }

    func retarget(document: EVEditorSurfaceController, styleKey: EVStyleKey) {
        loadViewIfNeeded()
        endContinuousStyleEdit(reportUnexpectedFailure: false)
        stopObservingDocument()
        settingsSession = nil
        restoreDefaultsButton.isHidden = true
        self.document = document
        selectedStyleKey = styleKey
        documentObserver = NotificationCenter.default.addObserver(
            forName: .viemCoreDocumentDidChange,
            object: document.backend,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.targetDocumentDidChange() }
        }
        reloadCommittedStyle(preferredKey: styleKey)
    }

    func retarget(settingsSession: any EVStyleSettingsSession) {
        loadViewIfNeeded()
        endContinuousStyleEdit(reportUnexpectedFailure: false)
        stopObservingDocument()
        document = nil
        self.settingsSession = settingsSession
        restoreDefaultsButton.isHidden = settingsSession.sourceFormat != .code
        selectedStyleKey = .baseParagraph
        diagnosticMessage = settingsSession.lastError ?? ""
        let notification: Notification.Name = settingsSession.sourceFormat == .code ? .viemGlobalCodeStyleDidChange : .viemThemeStyleSessionDidChange
        documentObserver = NotificationCenter.default.addObserver(forName: notification,
            object: settingsSession.sourceFormat == .code ? nil : settingsSession, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.targetDocumentDidChange() }
        }
        reloadCommittedStyle()
    }


    private func targetSnapshot() throws -> EVStyleSheetSnapshot {
        if let settingsSession { return try settingsSession.snapshot() }
        guard let document else { throw EVStyleBridgeError.noEditingView }
        return try document.backend.styleSheetSnapshot()
    }

    func selectStyle(_ id: EVStyleID) {
        guard let snapshot else { return }
        let matching = snapshot.definitions.filter { $0.key.id == id }
        if let sameNamespace = matching.first(where: { $0.key.namespace == selectedStyleKey.namespace }) {
            selectStyle(sameNamespace.key)
        } else if matching.count == 1, let definition = matching.first {
            selectStyle(definition.key)
        }
    }

    func selectStyle(_ key: EVStyleKey) {
        guard hasTarget, snapshot?.definition(for: key) != nil else { return }
        onExplicitStyleSelection?()
        endContinuousStyleEdit(reportUnexpectedFailure: false)
        selectedStyleKey = key
        selectPopupItem(for: key)
        renderCommittedStyle()
    }

    func followCaretStyle(_ key: EVStyleKey) {
        guard hasTarget, !isCommitting, key != selectedStyleKey else { return }
        endContinuousStyleEdit(reportUnexpectedFailure: false)
        diagnosticMessage = ""
        // Source edits can publish the new caret before the stylesheet change
        // notification. Fetch its current definitions before choosing the ID.
        reloadCommittedStyle(preferredKey: key)
    }

    func selectTab(_ tab: EVStyleEditorTab) {
        guard tabs.isEnabled(forSegment: tab.rawValue) else { return }
        tabs.selectedSegment = tab.rawValue
        tabChanged(tabs)
    }

    private func changeStyleCatalogue(_ action: () throws -> Void) -> Bool {
        endContinuousStyleEdit(reportUnexpectedFailure: false)
        isCommitting = true
        defer {
            isCommitting = false
            refreshAfterCommit = false
            reloadCommittedStyle()
        }
        do {
            try action()
            diagnosticMessage = ""
            return true
        } catch {
            diagnosticMessage = error.localizedDescription
            return false
        }
    }

    func disableForClosedDocument() {
        loadViewIfNeeded()
        endContinuousStyleEdit(reportUnexpectedFailure: false)
        stopObservingDocument()
        document = nil
        settingsSession = nil
        restoreDefaultsButton.isHidden = true
        snapshot = nil
        selectedStyleKey = .baseParagraph
        renderNoDocument()
    }

    @discardableResult
    func setPropertyForTesting(_ property: EVStyleProperty, value: EVStyleValue) -> Bool {
        commit(.setDeclaration(property, value))
    }

    @discardableResult
    func useInheritedForTesting(_ property: EVStyleProperty) -> Bool {
        commit(.clearDeclaration(property))
    }

    func beginContinuousStyleEditForTesting() { beginContinuousStyleEdit() }
    func endContinuousStyleEditForTesting() { endContinuousStyleEdit() }
    var hasActiveStyleEditGroupForTesting: Bool { activeStyleEditGroup != nil || settingsSession?.isEditingGroup == true }

    @discardableResult
    func setParentForTesting(_ key: EVStyleKey) -> Bool { commit(key == .defaultParagraph ? .clearParent : .setParent(key.id)) }

    @discardableResult
    func setFollowingStyleForTesting(_ key: EVStyleKey?) -> Bool {
        commit(key.map { .setNextStyle($0.id) } ?? .clearNextStyle)
    }

    @discardableResult
    func submitStaleMutationForTesting(expected: EVStyleSheetIdentity, mutation: EVStyleMutation) -> Bool {
        commit(mutation, forcedIdentity: expected)
    }

    func previewInspectionForTesting(layoutSize: CGSize) -> EVCoreTextStylePreviewInspection {
        preview.inspection(layoutSize: layoutSize)
    }

    var visiblePropertyRowCountForTesting: Int {
        loadViewIfNeeded()
        view.layoutSubtreeIfNeeded()
        let active = paragraphControls.isHidden ? characterControls : paragraphControls
        active.layoutSubtreeIfNeeded()
        return active.subviews.flatMap { ($0 as? NSStackView)?.arrangedSubviews ?? [] }.filter {
            !$0.isHidden && active.bounds.intersects($0.convert($0.bounds, to: active))
        }.count
    }

    var stylePickerFrameForTesting: NSRect {
        loadViewIfNeeded()
        view.layoutSubtreeIfNeeded()
        return stylePopup.convert(stylePopup.bounds, to: view)
    }

    var propertyTabsFrameForTesting: NSRect {
        loadViewIfNeeded()
        view.layoutSubtreeIfNeeded()
        return tabs.convert(tabs.bounds, to: view)
    }

    private func installPropertyControls() {
        compactControls.onMutations = { [weak self] mutations in
            guard let self else { return }
            let opensGroup = !self.hasActiveStyleEditGroupForTesting && mutations.count > 1
            if opensGroup { self.beginContinuousStyleEdit() }
            for mutation in mutations { if !self.commit(mutation) { break } }
            if opensGroup { self.endContinuousStyleEdit() }
        }
        compactControls.onEditBegan = { [weak self] in self?.beginContinuousStyleEdit() }
        compactControls.onEditEnded = { [weak self] in self?.endContinuousStyleEdit() }
        nextStyleRow.onChange = { [weak self] key in
            guard let self else { return }
            _ = self.commit(key.map { .setNextStyle($0.id) } ?? .clearNextStyle)
        }
        for (child, container) in [(compactControls.characterView, characterControls), (compactControls.paragraphView, paragraphControls), (compactControls.blockView, blockControls)] {
            child.translatesAutoresizingMaskIntoConstraints = false
            container.addSubview(child)
            NSLayoutConstraint.activate([
                child.leadingAnchor.constraint(equalTo: container.leadingAnchor),
                child.trailingAnchor.constraint(equalTo: container.trailingAnchor),
                child.topAnchor.constraint(equalTo: container.topAnchor),
                child.bottomAnchor.constraint(lessThanOrEqualTo: container.bottomAnchor),
            ])
        }
    }

    private func reloadCommittedStyle(preferredKey: EVStyleKey? = nil) {
        guard hasTarget else { renderNoDocument(); return }
        do {
            let fresh = try targetSnapshot()
            snapshot = fresh
            let desired = preferredKey ?? selectedStyleKey
            if fresh.definition(for: desired) != nil {
                selectedStyleKey = desired
            } else if let fallback = fresh.fallbackDefinition {
                selectedStyleKey = fallback.key
                diagnosticMessage = "The selected style no longer exists. Base Paragraph was selected."
            }
            renderCommittedStyle()
        } catch {
            diagnosticMessage = error.localizedDescription
            renderUnavailableStyleSheet()
        }
    }

    private func renderCommittedStyle() {
        guard let snapshot, let definition = snapshot.definition(for: selectedStyleKey) else {
            renderUnavailableStyleSheet()
            return
        }
        isUpdatingUI = true
        defer { isUpdatingUI = false }

        if let settingsSession { view.window?.title = settingsSession.windowTitle }
        configureDocumentPopup()
        configureStylePopup(snapshot: snapshot)
        selectPopupItem(for: definition.key)
        typeLabel.stringValue = definition.kind.displayName
        configureBasedOn(snapshot: snapshot, definition: definition)

        availabilityLabel.stringValue = diagnosticMessage
        availabilityLabel.textColor = .systemRed
        availabilityLabel.isHidden = diagnosticMessage.isEmpty

        let sizeBasisKey = definition.kind != .character ? definition.parentKey : .baseParagraph
        let sizeBasis: Float? = {
            guard let sizeBasisKey,
                  case let .float(value)? = snapshot.definition(for: sizeBasisKey)?.properties[.characterSize]?.effective else { return nil }
            return value
        }()
        compactControls.configure(definition, theme: themeStore.theme, documentID: snapshot.identity.documentID, fontSizeBasis: sizeBasis)
        nextStyleRow.configure(
            selected: definition.flags.isBase ? nil
                : definition.nextStyleID.map { EVStyleKey(namespace: .block, id: $0) },
            choices: definition.flags.isBase ? [] : snapshot.compatibleFollowingStyles(for: definition),
            editable: !definition.flags.isBase && settingsSession?.sourceFormat != .code
                && definition.capabilities.contains(.nextStyle)
        )
        configureNavigationTargets(snapshot: snapshot, definition: definition)

        let paragraphEnabled = definition.kind != .character
        tabs.setEnabled(true, forSegment: EVStyleEditorTab.character.rawValue)
        tabs.setEnabled(paragraphEnabled, forSegment: EVStyleEditorTab.paragraph.rawValue)
        tabs.setEnabled(definition.kind != .character, forSegment: EVStyleEditorTab.block.rawValue)
        if !tabs.isEnabled(forSegment: tabs.selectedSegment) {
            let responder = view.window?.firstResponder as? NSView
            let editingControl = (responder as? NSTextView).flatMap {
                $0.isFieldEditor ? $0.delegate as? NSView : nil
            } ?? responder
            if let editingControl, [paragraphControls, blockControls].contains(where: {
                editingControl === $0 || editingControl.isDescendant(of: $0)
            }) {
                view.window?.makeFirstResponder(tabs)
            }
            tabs.selectedSegment = EVStyleEditorTab.character.rawValue
        }
        tabChanged(tabs)
        updatePreview(snapshot: snapshot, definition: definition)
    }

    private func configureStylePopup(snapshot: EVStyleSheetSnapshot) {
        let signature = snapshot.definitions.map {
            EVStyleChoiceIdentity(key: $0.key, name: $0.name, kind: $0.kind, internalSyntax: $0.flags.contains(.internalSyntax))
        }
        stylePopup.isEnabled = true
        if styleMenuChoices == signature { return }
        styleMenuChoices = signature
        stylePopup.menu = Self.makeStyleMenu(definitions: snapshot.definitions)
        stylePopup.target = self
        stylePopup.action = #selector(styleChanged(_:))
    }

    static func makeStyleMenu(definitions: [EVStyleDefinition]) -> NSMenu {
        let menu = NSMenu()
        let definitions = definitions.sorted {
            $0.name.localizedStandardCompare($1.name) == .orderedAscending
        }
        let sections = [
            ("Paragraph", definitions.filter { $0.kind == .paragraph && !$0.flags.contains(.internalSyntax) }),
            ("Character", definitions.filter { $0.kind == .character && !$0.flags.contains(.internalSyntax) }),
            ("Container", definitions.filter { $0.kind.isContainer && !$0.flags.contains(.internalSyntax) }),
            ("Internal", definitions.filter { $0.flags.contains(.internalSyntax) }),
        ].filter { !$0.1.isEmpty }
        for (title, styles) in sections {
            if sections.count > 1 { menu.addItem(.sectionHeader(title: title)) }
            for definition in styles {
                let item = NSMenuItem(title: definition.name, action: nil, keyEquivalent: "")
                item.representedObject = EVStyleKeyBox(definition.key)
                item.toolTip = "Stable ID: \(definition.key.id.rawValue)"
                menu.addItem(item)
            }
        }
        return menu
    }

    private func configureBasedOn(snapshot: EVStyleSheetSnapshot, definition: EVStyleDefinition) {
        let editable = definition.capabilities.contains(.parent)
        var choices: [EVStyleChoiceIdentity] = []
        if editable {
            if definition.kind == .character { choices.append(EVStyleChoiceIdentity(key: .defaultParagraph, name: "Default Paragraph")) }
            choices += snapshot.legalParents(for: definition)
                .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
                .map { EVStyleChoiceIdentity(key: $0.key, name: $0.name) }
        } else {
            let title = definition.parentKey.flatMap { snapshot.definition(for: $0)?.name }
                ?? (definition.kind == .character ? "Default Paragraph" : "None")
            choices = [EVStyleChoiceIdentity(key: definition.parentKey, name: title)]
        }
        if parentMenuChoices != choices {
            basedOnPopup.removeAllItems()
            for choice in choices {
                basedOnPopup.addItem(withTitle: choice.name)
                if let key = choice.key { basedOnPopup.lastItem?.representedObject = EVStyleKeyBox(key) }
            }
            parentMenuChoices = choices
        }
        let selected = definition.parentKey ?? (definition.kind == .character ? .defaultParagraph : nil)
        if let item = basedOnPopup.itemArray.first(where: { ($0.representedObject as? EVStyleKeyBox)?.key == selected }) {
            basedOnPopup.select(item)
        } else { basedOnPopup.select(nil) }
        basedOnPopup.isEnabled = editable && !choices.isEmpty
        basedOnPopup.toolTip = editable ? "Only compatible styles that cannot create an inheritance cycle are shown."
            : (definition.flags.isBase ? "The parent of this distinguished base style is fixed." : "This style's parent is read-only.")
    }

    private func configureNavigationTargets(snapshot: EVStyleSheetSnapshot, definition: EVStyleDefinition) {
        let parent = definition.parentKey.flatMap { snapshot.definition(for: $0) }
        editParentButton.isEnabled = parent != nil && parent?.key != definition.key
        editParentButton.toolTip = parent.map { "Edit \($0.name)" }
            ?? (definition.kind == .character ? "Inherits the current paragraph style." : "This style has no parent.")

        let next = definition.kind == .paragraph && !definition.flags.isBase
            ? definition.nextStyleID.flatMap { snapshot.definition(namespace: .block, id: $0) }
            : nil
        editNextStyleButton.isEnabled = next?.kind == .paragraph && next?.key != definition.key
        editNextStyleButton.toolTip = next.map { "Edit \($0.name)" }
            ?? "Choose a next paragraph style to edit it."
    }

    private func selectPopupItem(for key: EVStyleKey) {
        if let item = stylePopup.itemArray.first(where: {
            ($0.representedObject as? EVStyleKeyBox)?.key == key
        }) {
            stylePopup.select(item)
        }
    }

    private func beginContinuousStyleEdit() {
        if let settingsSession {
            do { try settingsSession.beginGroup() }
            catch { diagnosticMessage = error.localizedDescription; reloadCommittedStyle() }
            return
        }
        guard activeStyleEditGroup == nil,
              let document,
              let session = document.session
        else { return }
        do {
            let latest = try document.backend.styleSheetSnapshot()
            snapshot = latest
            guard latest.definition(for: selectedStyleKey) != nil else {
                reloadCommittedStyle()
                return
            }
            activeStyleEditGroup = try session.beginStyleEditGroup(expected: latest.identity)
            activeStyleEditSession = session
        } catch {
            diagnosticMessage = error.localizedDescription
            reloadCommittedStyle()
        }
    }

    private func endContinuousStyleEdit(reportUnexpectedFailure: Bool = true) {
        settingsSession?.endGroup()
        guard let group = activeStyleEditGroup else { return }
        let session = activeStyleEditSession
        activeStyleEditGroup = nil
        activeStyleEditSession = nil
        do {
            guard let session else { return }
            try session.endStyleEditGroup(group)
        } catch let error as EVStyleBridgeError where error.isEndedStyleEditGroup {
            // An unrelated command is specified to close the successful
            // prefix. The UI token is stale, but there is no user data to roll
            // back and no reason to turn normal focus loss into an alert.
        } catch {
            if reportUnexpectedFailure { diagnosticMessage = error.localizedDescription }
        }
    }

    @discardableResult
    private func commit(_ mutation: EVStyleMutation, forcedIdentity: EVStyleSheetIdentity? = nil) -> Bool {
        guard hasTarget, settingsSession != nil || document?.session != nil else {
            diagnosticMessage = EVStyleBridgeError.noEditingView.localizedDescription
            renderUnavailableStyleSheet()
            return false
        }
        do {
            let latest: EVStyleSheetSnapshot
            if forcedIdentity == nil {
                latest = try targetSnapshot()
                snapshot = latest
                guard latest.definition(for: selectedStyleKey) != nil else {
                    if let fallback = latest.fallbackDefinition { selectedStyleKey = fallback.key }
                    diagnosticMessage = "The selected style no longer exists. Base Paragraph was selected."
                    renderCommittedStyle()
                    return false
                }
            } else if let snapshot {
                latest = snapshot
            } else {
                return false
            }

            isCommitting = true
            defer {
                isCommitting = false
                if refreshAfterCommit {
                    refreshAfterCommit = false
                    reloadCommittedStyle()
                }
            }
            if let settingsSession {
                try settingsSession.edit(key: selectedStyleKey, expected: forcedIdentity ?? latest.identity, mutation: mutation)
            } else if let session = document?.session, let group = activeStyleEditGroup {
                _ = try session.editStyle(
                    key: selectedStyleKey,
                    expected: forcedIdentity ?? latest.identity,
                    mutation: mutation,
                    in: group
                )
            } else if let session = document?.session {
                _ = try session.editStyle(
                    key: selectedStyleKey,
                    expected: forcedIdentity ?? latest.identity,
                    mutation: mutation
                )
            }
            diagnosticMessage = ""
            reloadCommittedStyle()
            return true
        } catch {
            if let styleError = error as? EVStyleBridgeError,
               styleError.isEndedStyleEditGroup
            {
                activeStyleEditGroup = nil
                activeStyleEditSession = nil
            }
            let rejection = error.localizedDescription
            reloadCommittedStyle()
            diagnosticMessage = rejection
            if let definition = selectedDefinition {
                availabilityLabel.stringValue = rejection
                availabilityLabel.isHidden = false
                availabilityLabel.textColor = .systemRed
                updatePreview(snapshot: snapshot, definition: definition)
            }
            return false
        }
    }

    private func targetDocumentDidChange() {
        if isCommitting {
            refreshAfterCommit = true
        } else {
            endContinuousStyleEdit(reportUnexpectedFailure: false)
            diagnosticMessage = ""
            reloadCommittedStyle()
        }
    }

    private func updatePreview(snapshot: EVStyleSheetSnapshot?, definition: EVStyleDefinition) {
        preview.apply(
            kind: definition.kind,
            effectiveValues: definition.properties.reduce(into: [:]) { values, entry in
                if entry.value.usesTextColor { return }
                if entry.value.usesThemeDefault && entry.key == .characterForeground {
                    let color = themeStore.theme.foreground
                    values[entry.key] = .color(EVStyleColor(red: Float(color.red), green: Float(color.green), blue: Float(color.blue), alpha: Float(color.alpha)))
                } else if let effective = entry.value.effective { values[entry.key] = effective }
            },
            canvasBackground: previewCanvasBackground(snapshot: snapshot)
        )

    }

    private func previewCanvasBackground(snapshot: EVStyleSheetSnapshot?) -> EVStyleColor {
        let color = themeStore.theme.background
        return EVStyleColor(red: Float(color.red), green: Float(color.green), blue: Float(color.blue), alpha: Float(color.alpha))
    }

    private func renderNoDocument() {
        configureDocumentPopup()
        isUpdatingUI = true
        defer { isUpdatingUI = false }
        styleMenuChoices = nil; parentMenuChoices = nil
        stylePopup.removeAllItems()
        stylePopup.isEnabled = false
        typeLabel.stringValue = "No document"
        basedOnPopup.removeAllItems()
        basedOnPopup.addItem(withTitle: "Unavailable")
        basedOnPopup.isEnabled = false
        editParentButton.isEnabled = false
        editNextStyleButton.isEnabled = false
        diagnosticMessage = ""
        availabilityLabel.stringValue = ""
        availabilityLabel.isHidden = true
        tabs.selectedSegment = EVStyleEditorTab.character.rawValue
        tabs.setEnabled(false, forSegment: EVStyleEditorTab.character.rawValue)
        tabs.setEnabled(false, forSegment: EVStyleEditorTab.paragraph.rawValue)
        tabs.setEnabled(false, forSegment: EVStyleEditorTab.block.rawValue)
        compactControls.configure(nil, theme: themeStore.theme)
        nextStyleRow.configure(selected: nil, choices: [], editable: false)
        characterControls.isHidden = false
        paragraphControls.isHidden = true
        blockControls.isHidden = true
        preview.showUnavailable()
    }

    private func renderUnavailableStyleSheet() {
        configureDocumentPopup()
        guard hasTarget else { renderNoDocument(); return }
        isUpdatingUI = true
        defer { isUpdatingUI = false }
        stylePopup.isEnabled = false
        basedOnPopup.isEnabled = false
        editParentButton.isEnabled = false
        editNextStyleButton.isEnabled = false
        tabs.setEnabled(false, forSegment: EVStyleEditorTab.character.rawValue)
        tabs.setEnabled(false, forSegment: EVStyleEditorTab.paragraph.rawValue)
        tabs.setEnabled(false, forSegment: EVStyleEditorTab.block.rawValue)
        availabilityLabel.stringValue = diagnosticMessage.isEmpty ? "Styles are temporarily unavailable." : diagnosticMessage
        availabilityLabel.isHidden = false
        availabilityLabel.textColor = .systemRed
        preview.showUnavailable()
    }

    private func stopObservingDocument() {
        if let documentObserver {
            NotificationCenter.default.removeObserver(documentObserver)
            self.documentObserver = nil
        }
    }

    private func configurePropertiesGrid(_ grid: NSGridView) {
        grid.rowSpacing = 8
        grid.columnSpacing = 12
        // Explicit native alignment-rect centering also works for cells that
        // contain stacks. A stack's implicit baseline can otherwise put its
        // label above the popup's text.
        grid.yPlacement = .center
        grid.rowAlignment = .none
        grid.column(at: 0).width = 104
        grid.column(at: 0).xPlacement = .trailing
        grid.column(at: 1).xPlacement = .fill
        for row in 0..<grid.numberOfRows {
            grid.row(at: row).height = 28
            guard let control = grid.cell(atColumnIndex: 1, rowIndex: row).contentView else { continue }
            control.setContentHuggingPriority(.defaultLow, for: .horizontal)
            control.widthAnchor.constraint(greaterThanOrEqualToConstant: 390).isActive = true
        }
    }

    private func configureNavigationButton(_ button: NSButton, label: String, action: Selector) {
        button.bezelStyle = .rounded
        button.image = NSImage(systemSymbolName: "arrow.up.right", accessibilityDescription: nil)
        button.imagePosition = .imageOnly
        button.target = self
        button.action = action
        button.isEnabled = false
        button.setAccessibilityLabel(label)
        button.setContentHuggingPriority(.required, for: .horizontal)
        button.setContentCompressionResistancePriority(.required, for: .horizontal)
        button.widthAnchor.constraint(equalToConstant: 30).isActive = true
    }

    private func navigationRow(popup: NSPopUpButton, button: NSButton) -> NSStackView {
        popup.setContentHuggingPriority(.defaultLow, for: .horizontal)
        popup.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let row = NSStackView(views: [popup, button])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.distribution = .fill
        row.spacing = 6
        return row
    }

    private func centeredContainer(
        _ content: NSView,
        maximumWidth: CGFloat,
        horizontalInset: CGFloat
    ) -> NSView {
        let container = NSView()
        container.addSubview(content)
        content.translatesAutoresizingMaskIntoConstraints = false
        let preferredWidth = content.widthAnchor.constraint(
            equalTo: container.widthAnchor,
            constant: -(horizontalInset * 2)
        )
        preferredWidth.priority = .defaultHigh
        NSLayoutConstraint.activate([
            content.centerXAnchor.constraint(equalTo: container.centerXAnchor),
            content.leadingAnchor.constraint(greaterThanOrEqualTo: container.leadingAnchor, constant: horizontalInset),
            content.trailingAnchor.constraint(lessThanOrEqualTo: container.trailingAnchor, constant: -horizontalInset),
            content.topAnchor.constraint(equalTo: container.topAnchor),
            content.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            content.widthAnchor.constraint(lessThanOrEqualToConstant: maximumWidth),
            preferredWidth,
        ])
        return container
    }

    private func label(_ value: String) -> NSTextField {
        let field = NSTextField(labelWithString: value)
        field.alignment = .right
        return field
    }

    private func separator() -> NSBox {
        let box = NSBox()
        box.boxType = .separator
        return box
    }

    private func configureDocumentPopup() {
        let format = settingsSession?.sourceFormat ?? document?.backend.sourceFormat
        documentPopup.selectItem(at: documentFormats.firstIndex {
            $0.defaultStyleName == format?.defaultStyleName
        } ?? 0)
        documentPopup.isEnabled = settingsSession != nil && onDocumentSelection != nil
    }

    @objc private func documentChanged(_ sender: NSPopUpButton) {
        guard !isUpdatingUI, let settingsSession,
              documentFormats.indices.contains(sender.indexOfSelectedItem) else { return }
        endContinuousStyleEdit(reportUnexpectedFailure: false)
        do {
            try onDocumentSelection?(documentFormats[sender.indexOfSelectedItem], settingsSession.configuration)
        } catch {
            diagnosticMessage = error.localizedDescription
            reloadCommittedStyle()
        }
    }

    @objc private func styleChanged(_ sender: NSPopUpButton) {
        guard let key = (sender.selectedItem?.representedObject as? EVStyleKeyBox)?.key else { return }
        selectStyle(key)
    }

    @objc private func restoreCodeDefaults(_ sender: Any?) {
        guard let settingsSession = codeSettingsSession else { return }
        _ = changeStyleCatalogue { try settingsSession.restoreDefaults() }
    }

    @objc private func basedOnChanged(_ sender: NSPopUpButton) {
        guard !isUpdatingUI, let key = (sender.selectedItem?.representedObject as? EVStyleKeyBox)?.key else { return }
        _ = commit(key == .defaultParagraph ? .clearParent : .setParent(key.id))
    }

    @objc private func editParentStyle(_ sender: Any?) {
        guard editParentButton.isEnabled, let key = selectedDefinition?.parentKey else { return }
        selectStyle(key)
    }

    @objc private func editNextStyle(_ sender: Any?) {
        guard editNextStyleButton.isEnabled, let definition = selectedDefinition,
              definition.kind == .paragraph, let id = definition.nextStyleID else { return }
        selectStyle(EVStyleKey(namespace: .block, id: id))
    }

    @objc private func tabChanged(_ sender: NSSegmentedControl) {
        let showParagraph = sender.selectedSegment == EVStyleEditorTab.paragraph.rawValue
            && sender.isEnabled(forSegment: EVStyleEditorTab.paragraph.rawValue)
        let showBlock = sender.selectedSegment == EVStyleEditorTab.block.rawValue
            && sender.isEnabled(forSegment: EVStyleEditorTab.block.rawValue)
        characterControls.isHidden = showParagraph || showBlock
        paragraphControls.isHidden = !showParagraph
        blockControls.isHidden = !showBlock
        updateFormattingHeight()
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        updateFormattingHeight()
    }

    private func updateFormattingHeight() {
        let controls = !paragraphControls.isHidden ? compactControls.paragraphView
            : !blockControls.isHidden ? compactControls.blockView : compactControls.characterView
        // The old fixed height compressed axis captions before the sliders.
        // Fit the selected tab's controls and keep the preview usable even at
        // the smallest permitted window size.
        let height = max(196, ceil(controls.fittingSize.height) + 14)
        if formattingHeightConstraint?.constant != height {
            formattingHeightConstraint?.constant = height
        }
        guard let window = view.window else { return }
        let oldMinimum = window.contentMinSize.height
        let minimum = 570 + height - 196
        guard abs(oldMinimum - minimum) > 0.5 else { return }
        let current = window.contentLayoutRect.height
        window.contentMinSize = NSSize(width: max(700, window.contentMinSize.width), height: minimum)
        if current < minimum || abs(current - oldMinimum) < 0.5 {
            var frame = window.frame
            frame.origin.y -= minimum - current
            frame.size.height += minimum - current
            window.setFrame(frame, display: true)
        }
    }

    @objc private func closePressed(_ sender: Any?) { onClose?() }
}

private final class EVStyleEditorBackgroundView: NSView {
    override var isOpaque: Bool { true }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }

    override func draw(_ dirtyRect: NSRect) {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            NSColor.windowBackgroundColor.setFill()
            dirtyRect.fill()
        }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        synchronizeWindowBackground()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        synchronizeWindowBackground()
        needsDisplay = true
    }

    private func synchronizeWindowBackground() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            let background = NSColor.windowBackgroundColor.usingColorSpace(.sRGB)
                ?? .windowBackgroundColor
            window?.backgroundColor = background
            layer?.backgroundColor = background.cgColor
        }
    }
}

private class EVStyleEditorBorderedView: NSView {
    let fillColor: NSColor

    init(fillColor: NSColor) {
        self.fillColor = fillColor
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = 6
        layer?.borderWidth = 1
        synchronizeLayerColors()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        synchronizeLayerColors()
    }

    private func synchronizeLayerColors() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            layer?.backgroundColor = (fillColor.usingColorSpace(.sRGB) ?? fillColor).cgColor
            layer?.borderColor = (NSColor.separatorColor.usingColorSpace(.sRGB)
                ?? .separatorColor).cgColor
        }
    }
}

private final class EVStyleEditorSectionView: EVStyleEditorBorderedView {
    init() { super.init(fillColor: .controlBackgroundColor) }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }
}

private final class EVStyleEditorSelectorView: EVStyleEditorBorderedView {
    init() {
        super.init(fillColor: NSColor(name: nil) { appearance in
            var color = NSColor.controlBackgroundColor
            appearance.performAsCurrentDrawingAppearance {
                color = NSColor.windowBackgroundColor.blended(withFraction: 0.09, of: .white)
                    ?? .controlBackgroundColor
            }
            return color
        })
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
}

private final class EVStyleEditorPreviewContainer: EVStyleEditorBorderedView {
    init() { super.init(fillColor: .textBackgroundColor) }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }
}

private final class EVStyleKeyBox: NSObject {
    let key: EVStyleKey
    init(_ key: EVStyleKey) { self.key = key }
}

private struct EVStyleChoiceIdentity: Equatable {
    let key: EVStyleKey?
    let name: String
    var kind: EVStyleKind? = nil
    var internalSyntax: Bool = false
}

@MainActor
private final class EVFollowingStyleRow: NSObject {
    var popupForCompactLayout: NSPopUpButton { popup }
    let view: NSView
    var onChange: ((EVStyleKey?) -> Void)?
    private let popup = NSPopUpButton()
    private var isConfiguring = false
    private var menuChoices: [EVStyleChoiceIdentity]?

    override init() {
        let title = NSTextField(labelWithString: "Following paragraph style")
        title.alignment = .right
        title.widthAnchor.constraint(equalToConstant: 155).isActive = true
        popup.setAccessibilityLabel("Following paragraph style")
        let row = NSStackView(views: [title])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = 10
        view = row
        super.init()
        popup.target = self
        popup.action = #selector(changed(_:))
    }

    func configure(selected: EVStyleKey?, choices: [EVStyleDefinition], editable: Bool) {
        isConfiguring = true
        defer { isConfiguring = false }
        let signature = choices.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
            .map { EVStyleChoiceIdentity(key: $0.key, name: $0.name) }
        if menuChoices != signature {
            popup.removeAllItems()
            popup.addItem(withTitle: "Same Style")
            for choice in signature {
                popup.addItem(withTitle: choice.name)
                if let key = choice.key { popup.lastItem?.representedObject = EVStyleKeyBox(key) }
            }
            menuChoices = signature
        }
        if let selected,
           let item = popup.itemArray.first(where: { ($0.representedObject as? EVStyleKeyBox)?.key == selected })
        {
            popup.select(item)
        } else { popup.selectItem(at: 0) }
        popup.isEnabled = editable
    }

    @objc private func changed(_ sender: NSPopUpButton) {
        guard !isConfiguring, sender.isEnabled else { return }
        onChange?((sender.selectedItem?.representedObject as? EVStyleKeyBox)?.key)
    }
}

private func format(_ value: EVStyleValue?) -> String {
    switch value {
    case let .float(number): "\(formatNumber(number)) pt"
    case let .percentage(number): "\(number)%"
    case let .unsigned(number): String(number)
    case let .boolean(enabled): enabled ? "On" : "Off"
    case let .color(color): colorHex(color)
    case let .string(value): value
    case let .stringList(values): values.isEmpty ? "None" : values.joined(separator: " → ")
    case let .fontSlant(value):
        value == UInt32(VIEM_FONT_SLANT_ITALIC) ? "Italic" : (value == UInt32(VIEM_FONT_SLANT_OBLIQUE) ? "Oblique" : "Upright")
    case let .writingDirection(value):
        value == UInt32(VIEM_TEXT_DIRECTION_LEFT_TO_RIGHT) ? "Left to right"
            : (value == UInt32(VIEM_TEXT_DIRECTION_RIGHT_TO_LEFT) ? "Right to left" : "Natural")
    case let .openTypeFeatures(values):
        values.isEmpty ? "Default feature set" : values.map { "\($0.tag)=\($0.setting)" }.joined(separator: ", ")
    case let .lineSpacing(spacing):
        switch spacing.kind {
        case UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER): "\(formatNumber(spacing.value))×"
        case UInt32(VIEM_STYLE_LINE_SPACING_AT_LEAST): "At least \(formatNumber(spacing.value)) pt"
        case UInt32(VIEM_STYLE_LINE_SPACING_EXACT): "Exactly \(formatNumber(spacing.value)) pt"
        default: "Normal"
        }
    case let .paragraphAlignment(value):
        value == UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER) ? "Center"
            : (value == UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_END) ? "End" : "Start")
    case nil: "None"
    }
}

private func formatNumber(_ value: Float) -> String {
    if value.rounded() == value,
       value >= Float(Int.min), value <= Float(Int.max)
    {
        return String(Int(value))
    }
    return String(format: "%.3g", value)
}

private func colorHex(_ color: EVStyleColor) -> String {
    func component(_ value: Float) -> UInt32 { UInt32((max(0, min(1, value)) * 255).rounded()) }
    return String(
        format: "#%02X%02X%02X%02X",
        component(color.red), component(color.green), component(color.blue), component(color.alpha)
    )
}
