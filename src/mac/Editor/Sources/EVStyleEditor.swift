import AppKit
import CViemCore
import ViemAppShell

enum EVStyleEditorTab: Int {
    case character
    case paragraph
}

struct EVStyleEditorInspection: Equatable {
    let targetDocumentIdentity: ObjectIdentifier?
    let targetCoreDocumentID: UInt64?
    let styleSheetRevision: UInt64?
    let selectedStyleID: EVStyleID?
    let selectedStyleKey: EVStyleKey?
    let selectedKind: EVStyleKind?
    let selectedTab: EVStyleEditorTab
    let paragraphTabEnabled: Bool
    let hasDocument: Bool
    let mutationsEnabled: Bool
    let nameEditable: Bool
    let parentValue: String
    let parentChoices: [EVStyleKey]
    let styleCount: Int
    let characterPropertyCount: Int
    let paragraphPropertyCount: Int
    let declaredProperties: Set<EVStyleProperty>
    let hasInvalidDraft: Bool
    let diagnostic: String
    let summary: String
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

    private let replacementDocumentProvider: @MainActor (EVEditorSurfaceController) -> EVEditorSurfaceController?
    private var controller: NSWindowController?
    private weak var target: EVEditorSurfaceController?
    private var contentController: EVStyleEditorViewController?
    private var targetWindowObserver: NSObjectProtocol?
    private var globalSession: EVCodeStyleSession?

    var styleWindow: NSWindow? { controller?.window }
    var inspection: EVStyleEditorInspection? { contentController?.inspection }

    override init() {
        replacementDocumentProvider = { closing in
            Self.frontmostDocumentSurface(excluding: closing)
        }
        super.init()
    }

    init(
        replacementDocumentProvider: @escaping @MainActor (EVEditorSurfaceController) -> EVEditorSurfaceController?
    ) {
        self.replacementDocumentProvider = replacementDocumentProvider
        super.init()
    }

    func show(document: EVEditorSurfaceController, preferredStyle: EVStyleKind, sender: Any?) {
        target = document
        observeTargetWindow(of: document)
        let isNewWindow = prepareWindow()
        (controller?.window as? EVStyleEditorPanel)?.settingsUndoManager = nil
        controller?.window?.title = "Styles"
        contentController?.retarget(document: document, styleKey: preferredStyle.baseKey)
        present(sender: sender, center: isNewWindow)
    }

    func showCode(
        configuration: EVConfigurationStore,
        preferredStyle: EVStyleKey = .baseDocument,
        definingSyntaxName: String? = nil,
        sender: Any?
    ) {
        do {
            let session: EVCodeStyleSession
            if let current = globalSession, current.configuration.directory.standardizedFileURL == configuration.directory.standardizedFileURL {
                session = current
            } else {
                session = try EVCodeStyleSession(configuration: configuration)
                globalSession = session
            }
            var selectedStyle = preferredStyle
            if let name = definingSyntaxName {
                // Explicit creation is its own settings undo action, even when
                // invoked while another style control has an open edit group.
                session.endGroup()
                let latest = try session.snapshot()
                if let existing = latest.definitions.first(where: { $0.kind == .character && $0.name == name }) {
                    selectedStyle = existing.key
                } else {
                    let key = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: UUID().uuidString.lowercased()))
                    try session.create(key: key, name: name, expected: latest.identity)
                    selectedStyle = key
                }
            }
            target = nil
            stopObservingTargetWindow()
            let isNewWindow = prepareWindow()
            (controller?.window as? EVStyleEditorPanel)?.settingsUndoManager = session.undoManager
            controller?.window?.title = "Code Styles"
            contentController?.retarget(codeSession: session)
            contentController?.selectStyle(selectedStyle)
            present(sender: sender, center: isNewWindow)
        } catch {
            EVCodePreferences.shared.reportLoadDiagnostics([error.localizedDescription])
        }
    }

    private func prepareWindow() -> Bool {
        let isNewWindow = controller == nil
        if isNewWindow {
            let content = EVStyleEditorViewController()
            content.onClose = { [weak self] in self?.controller?.close() }
            let panel = EVStyleEditorPanel(
                contentRect: NSRect(x: 0, y: 0, width: 760, height: 770),
                styleMask: [.titled, .closable, .resizable, .utilityWindow],
                backing: .buffered,
                defer: false
            )
            panel.title = "Styles"
            panel.isFloatingPanel = false
            panel.level = .normal
            panel.hidesOnDeactivate = false
            panel.contentMinSize = NSSize(width: 700, height: 720)
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
        guard target === document else { return }
        if let alternate = document.backend.alternateStyleEditorSurface(excluding: document) {
            let retainedSelection = contentController?.inspection.selectedStyleKey ?? .baseParagraph
            target = alternate
            observeTargetWindow(of: alternate)
            contentController?.retarget(document: alternate, styleKey: retainedSelection)
            return
        }
        if let replacement = replacementDocumentProvider(document) {
            target = replacement
            observeTargetWindow(of: replacement)
            contentController?.retarget(document: replacement, styleKey: .baseParagraph)
            return
        }
        stopObservingTargetWindow()
        target = nil
        contentController?.disableForClosedDocument()
    }

    func selectStyle(_ id: EVStyleID) { contentController?.selectStyle(id) }
    func selectStyle(_ key: EVStyleKey) { contentController?.selectStyle(key) }
    func selectTab(_ tab: EVStyleEditorTab) { contentController?.selectTab(tab) }
    func close() { controller?.close() }

    func windowWillClose(_ notification: Notification) {
        guard notification.object as AnyObject? === controller?.window else { return }
        stopObservingTargetWindow()
        contentController?.disableForClosedDocument()
        controller = nil
        contentController = nil
        target = nil
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

    private static func frontmostDocumentSurface(
        excluding closing: EVEditorSurfaceController
    ) -> EVEditorSurfaceController? {
        var windows: [NSWindow] = []
        if let key = NSApplication.shared.keyWindow { windows.append(key) }
        if let main = NSApplication.shared.mainWindow { windows.append(main) }
        windows.append(contentsOf: NSApplication.shared.orderedWindows)
        var seen = Set<ObjectIdentifier>()
        for window in windows where seen.insert(ObjectIdentifier(window)).inserted {
            guard let controller = window.windowController as? EVDocumentWindowController,
                  let surface = controller.editorSurface as? EVEditorSurfaceController,
                  surface !== closing,
                  surface.session != nil
            else { continue }
            return surface
        }
        return nil
    }
}

@MainActor
final class EVStyleEditorViewController: NSViewController, NSTextFieldDelegate {
    var onClose: (() -> Void)?
    var themeStore = EVThemeStore.shared
    private var themeObserver: NSObjectProtocol?

    private weak var document: EVEditorSurfaceController?
    private var codeSession: EVCodeStyleSession?
    private var hasTarget: Bool { document != nil || codeSession != nil }
    override var undoManager: UndoManager? { codeSession?.undoManager ?? super.undoManager }
    private var snapshot: EVStyleSheetSnapshot?
    private var selectedStyleKey: EVStyleKey = .baseParagraph
    private var documentObserver: NSObjectProtocol?
    private var isUpdatingUI = false
    private var isCommitting = false
    private var refreshAfterCommit = false
    private var diagnosticMessage = ""
    private var nameDraftIsInvalid = false
    private var activeStyleEditGroup: EVStyleEditGroup?
    private weak var activeStyleEditSession: EVCoreViewSession?

    private let stylePopup = NSPopUpButton()
    private let newStylePopup = NSPopUpButton(frame: .zero, pullsDown: true)
    private let deleteStyleButton = NSButton()
    private let restoreDefaultsButton = NSButton(title: "Restore Defaults", target: nil, action: nil)
    private let nameField = NSTextField()
    private let typeLabel = NSTextField(labelWithString: "")
    private let basedOnPopup = NSPopUpButton()
    private let editParentButton = NSButton()
    private let editNextStyleButton = NSButton()
    private let availabilityLabel = NSTextField(wrappingLabelWithString: "")
    private let tabs = NSSegmentedControl(
        labels: ["Character", "Paragraph"],
        trackingMode: .selectOne,
        target: nil,
        action: nil
    )
    private let characterControls = NSView()
    private let paragraphControls = NSView()
    private let preview = EVCoreTextStylePreviewView()
    private let summary = NSTextView()
    private let compactControls = EVCompactStyleControls()
    private let nextStyleRow = EVFollowingStyleRow()

    private var selectedDefinition: EVStyleDefinition? {
        snapshot?.definition(for: selectedStyleKey)
    }

    var inspection: EVStyleEditorInspection {
        let selectedTab: EVStyleEditorTab = tabs.selectedSegment == EVStyleEditorTab.paragraph.rawValue
            ? .paragraph
            : .character
        let definition = selectedDefinition
        return EVStyleEditorInspection(
            targetDocumentIdentity: document.map { ObjectIdentifier($0.backend) },
            targetCoreDocumentID: snapshot?.identity.documentID,
            styleSheetRevision: snapshot?.identity.styleSheetRevision,
            selectedStyleID: hasTarget ? definition?.key.id : nil,
            selectedStyleKey: hasTarget ? definition?.key : nil,
            selectedKind: definition?.kind,
            selectedTab: selectedTab,
            paragraphTabEnabled: tabs.isEnabled(forSegment: EVStyleEditorTab.paragraph.rawValue),
            hasDocument: document != nil,
            mutationsEnabled: definition?.capabilities.contains(.declarations) == true,
            nameEditable: nameField.isEditable && nameField.isEnabled,
            parentValue: basedOnPopup.titleOfSelectedItem ?? "",
            parentChoices: basedOnPopup.itemArray.compactMap { ($0.representedObject as? EVStyleKeyBox)?.key },
            styleCount: snapshot?.definitions.count ?? 0,
            characterPropertyCount: EVStyleProperty.characterProperties.filter { definition?.properties[$0] != nil }.count,
            paragraphPropertyCount: EVStyleProperty.paragraphProperties.filter { definition?.properties[$0] != nil }.count,
            declaredProperties: Set(definition?.properties.values.compactMap {
                $0.isDeclared ? $0.property : nil
            } ?? []),
            hasInvalidDraft: nameDraftIsInvalid || compactControls.hasInvalidDraft,
            diagnostic: diagnosticMessage,
            summary: summary.string,
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
                if let definition = self.selectedDefinition { self.updatePreviewAndSummary(snapshot: self.snapshot, definition: definition) }
            }
        }
        let root = EVStyleEditorBackgroundView(frame: .zero)
        root.setAccessibilityElement(false)
        stylePopup.target = self
        stylePopup.action = #selector(styleChanged(_:))
        stylePopup.setAccessibilityLabel("Style")
        stylePopup.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        stylePopup.setContentHuggingPriority(.defaultLow, for: .horizontal)
        newStylePopup.addItems(withTitles: ["New", "New Paragraph Style", "New Character Style"])
        newStylePopup.target = self
        newStylePopup.action = #selector(newStylePressed(_:))
        newStylePopup.setAccessibilityLabel("Create style")
        newStylePopup.widthAnchor.constraint(equalToConstant: 66).isActive = true
        newStylePopup.setContentCompressionResistancePriority(.required, for: .horizontal)
        deleteStyleButton.title = "Delete"
        deleteStyleButton.bezelStyle = .rounded
        deleteStyleButton.target = self
        deleteStyleButton.action = #selector(deleteStylePressed(_:))
        deleteStyleButton.setAccessibilityLabel("Delete selected style")
        deleteStyleButton.widthAnchor.constraint(equalToConstant: 64).isActive = true
        deleteStyleButton.setContentCompressionResistancePriority(.required, for: .horizontal)
        let stylePicker = NSStackView(views: [stylePopup, newStylePopup, deleteStyleButton])
        stylePicker.orientation = .horizontal
        stylePicker.alignment = .centerY
        stylePicker.distribution = .fill
        stylePicker.spacing = 6
        nameField.placeholderString = "Style name"
        nameField.delegate = self
        nameField.setAccessibilityLabel("Style name")
        typeLabel.textColor = .secondaryLabelColor
        typeLabel.setAccessibilityLabel("Style type")
        basedOnPopup.target = self
        basedOnPopup.action = #selector(basedOnChanged(_:))
        basedOnPopup.setAccessibilityLabel("Based on style")
        configureNavigationButton(editParentButton, label: "Edit based on style", action: #selector(editParentStyle(_:)))
        configureNavigationButton(editNextStyleButton, label: "Edit next paragraph style", action: #selector(editNextStyle(_:)))

        let propertiesGrid = NSGridView(views: [
            [label("Style"), stylePicker],
            [label("Name"), nameField],
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
        availabilityLabel.setAccessibilityLabel("Style editing availability")
        tabs.selectedSegment = EVStyleEditorTab.character.rawValue
        tabs.target = self
        tabs.action = #selector(tabChanged(_:))
        tabs.setAccessibilityLabel("Style property category")

        installPropertyControls()
        let controlsContainer = NSView()
        controlsContainer.addSubview(characterControls)
        controlsContainer.addSubview(paragraphControls)
        characterControls.translatesAutoresizingMaskIntoConstraints = false
        paragraphControls.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            characterControls.leadingAnchor.constraint(equalTo: controlsContainer.leadingAnchor),
            characterControls.trailingAnchor.constraint(equalTo: controlsContainer.trailingAnchor),
            characterControls.topAnchor.constraint(equalTo: controlsContainer.topAnchor),
            characterControls.bottomAnchor.constraint(equalTo: controlsContainer.bottomAnchor),
            paragraphControls.leadingAnchor.constraint(equalTo: controlsContainer.leadingAnchor),
            paragraphControls.trailingAnchor.constraint(equalTo: controlsContainer.trailingAnchor),
            paragraphControls.topAnchor.constraint(equalTo: controlsContainer.topAnchor),
            paragraphControls.bottomAnchor.constraint(equalTo: controlsContainer.bottomAnchor),
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

        summary.isEditable = false
        summary.isSelectable = true
        summary.drawsBackground = true
        summary.backgroundColor = .textBackgroundColor
        summary.font = .systemFont(ofSize: 12)
        summary.textContainerInset = NSSize(width: 9, height: 8)
        summary.setAccessibilityLabel("Resolved style summary")
        let summaryScroll = NSScrollView()
        summaryScroll.borderType = .bezelBorder
        summaryScroll.hasVerticalScroller = true
        summaryScroll.autohidesScrollers = true
        summaryScroll.documentView = summary

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

        let tabsContainer = centeredContainer(tabs, maximumWidth: 260, horizontalInset: 0)

        let stack = NSStackView(views: [
            sectionTitle("Properties"), propertiesContainer, availabilityLabel,
            separator(), tabsContainer, formattingBox,
            previewBox, summaryScroll, bottom,
        ])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        for arranged in stack.arrangedSubviews where !(arranged is NSTextField) {
            arranged.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        }
        stack.setCustomSpacing(10, after: propertiesContainer)
        root.addSubview(stack)
        let preferredFormattingHeight = formattingBox.heightAnchor.constraint(equalToConstant: 196)
        preferredFormattingHeight.priority = .defaultHigh
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
            summaryScroll.heightAnchor.constraint(greaterThanOrEqualToConstant: 70),
        ])
        formattingBox.setContentCompressionResistancePriority(.required, for: .vertical)
        previewBox.setContentCompressionResistancePriority(.required, for: .vertical)
        summaryScroll.setContentHuggingPriority(.defaultLow, for: .vertical)
        view = root
        renderNoDocument()
    }

    func retarget(document: EVEditorSurfaceController, styleID: EVStyleID) {
        let namespace: EVStyleNamespace = styleID == .baseCharacter ? .character : .block
        retarget(document: document, styleKey: EVStyleKey(namespace: namespace, id: styleID))
    }

    func retarget(document: EVEditorSurfaceController, styleKey: EVStyleKey) {
        loadViewIfNeeded()
        endContinuousStyleEdit(reportUnexpectedFailure: false)
        stopObservingDocument()
        codeSession = nil
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

    func retarget(codeSession: EVCodeStyleSession) {
        loadViewIfNeeded()
        endContinuousStyleEdit(reportUnexpectedFailure: false)
        stopObservingDocument()
        document = nil
        self.codeSession = codeSession
        restoreDefaultsButton.isHidden = false
        selectedStyleKey = .baseDocument
        diagnosticMessage = codeSession.lastError ?? ""
        documentObserver = NotificationCenter.default.addObserver(forName: .viemGlobalCodeStyleDidChange, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.targetDocumentDidChange() }
        }
        reloadCommittedStyle()
    }

    private func targetSnapshot() throws -> EVStyleSheetSnapshot {
        if let codeSession { return try codeSession.snapshot() }
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
        endContinuousStyleEdit(reportUnexpectedFailure: false)
        selectedStyleKey = key
        selectPopupItem(for: key)
        renderCommittedStyle()
    }

    func selectTab(_ tab: EVStyleEditorTab) {
        guard tabs.isEnabled(forSegment: tab.rawValue) else { return }
        tabs.selectedSegment = tab.rawValue
        tabChanged(tabs)
    }

    @discardableResult
    func createStyle(kind: EVStyleKind) -> Bool {
        if let codeSession {
            guard kind == .character else { return false }
            return changeStyleCatalogue {
                let latest = try codeSession.snapshot()
                let key = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: UUID().uuidString.lowercased()))
                var name = "New Syntax Style"
                var suffix = 2
                while latest.definitions.contains(where: { $0.name == name }) {
                    name = "New Syntax Style \(suffix)"
                    suffix += 1
                }
                try codeSession.create(key: key, name: name, expected: latest.identity)
                self.selectedStyleKey = key
            }
        }
        guard kind != .document, let document, let session = document.session,
              [.html, .htmlSource, .rtf].contains(document.backend.sourceFormat) else { return false }
        return changeStyleCatalogue {
            let latest = try document.backend.styleSheetSnapshot()
            let prefix = kind == .paragraph ? "RtfP" : "RtfC"
            let id: String
            if document.backend.sourceFormat == .rtf {
                let handles = Set(latest.definitions.compactMap { definition -> UInt32? in
                    let id = definition.key.id.rawValue
                    guard id.hasPrefix("RtfP") || id.hasPrefix("RtfC") else { return nil }
                    return UInt32(id.dropFirst(4))
                })
                var handle: UInt32 = 1
                while handles.contains(handle) { handle += 1 }
                id = "\(prefix)\(handle)"
            } else { id = UUID().uuidString.lowercased() }
            let key = EVStyleKey(namespace: kind == .paragraph ? .block : .character,
                                 id: EVStyleID(rawValue: id))
            let baseName = "New \(kind.displayName) Style"
            var name = baseName
            var suffix = 2
            while latest.definitions.contains(where: { $0.name == name }) {
                name = "\(baseName) \(suffix)"
                suffix += 1
            }
            _ = try session.createStyle(key, name: name, identity: latest.identity)
            self.selectedStyleKey = key
        }
    }

    @discardableResult
    func deleteSelectedStyle() -> Bool {
        if let codeSession {
            guard selectedDefinition?.capabilities.contains(.delete) == true else { return false }
            return changeStyleCatalogue {
                let latest = try codeSession.snapshot()
                try codeSession.delete(key: self.selectedStyleKey, expected: latest.identity)
                self.selectedStyleKey = .baseParagraph
            }
        }
        guard let document, let session = document.session,
              selectedDefinition?.capabilities.contains(.delete) == true else { return false }
        return changeStyleCatalogue {
            let latest = try document.backend.styleSheetSnapshot()
            _ = try session.deleteStyle(self.selectedStyleKey, identity: latest.identity)
            self.selectedStyleKey = .baseParagraph
        }
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
        codeSession = nil
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

    @discardableResult
    func renameForTesting(_ name: String) -> Bool {
        guard isValidStyleName(name) else { return false }
        return commit(.setDisplayName(name))
    }

    func enterNameDraftForTesting(_ name: String) {
        nameField.stringValue = name
        controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: nameField))
    }

    func beginContinuousStyleEditForTesting() { beginContinuousStyleEdit() }
    func endContinuousStyleEditForTesting() { endContinuousStyleEdit() }
    var hasActiveStyleEditGroupForTesting: Bool { activeStyleEditGroup != nil || codeSession?.isEditingGroup == true }

    @discardableResult
    func setParentForTesting(_ key: EVStyleKey) -> Bool { commit(.setParent(key.id)) }

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
        for (child, container) in [(compactControls.characterView, characterControls), (compactControls.paragraphView, paragraphControls)] {
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

        configureStylePopup(snapshot: snapshot)
        selectPopupItem(for: definition.key)
        newStylePopup.isEnabled = codeSession != nil || (document.map { [.html, .htmlSource, .rtf].contains($0.backend.sourceFormat) } ?? false)
        newStylePopup.item(at: 1)?.isHidden = codeSession != nil
        deleteStyleButton.isEnabled = definition.capabilities.contains(.delete)
        nameField.stringValue = definition.name
        nameDraftIsInvalid = false
        nameField.backgroundColor = .textBackgroundColor
        nameField.isEditable = definition.capabilities.contains(.displayName)
        nameField.isSelectable = true
        nameField.isEnabled = definition.capabilities.contains(.displayName)
        nameField.toolTip = definition.capabilities.contains(.displayName)
            ? "Changes are applied immediately to the core-owned style."
            : "This style's display name is read-only."
        typeLabel.stringValue = definition.kind.displayName
        configureBasedOn(snapshot: snapshot, definition: definition)

        if diagnosticMessage.isEmpty {
            if definition.capabilities.isEmpty {
                availabilityLabel.stringValue = "This \(definition.origin.displayName) style is read-only. Effective and inherited values remain available for inspection."
            } else {
                availabilityLabel.stringValue = codeSession != nil ? "Shared by every Code document. Changes are saved to code_style.json." : ""
            }
            availabilityLabel.textColor = .secondaryLabelColor
        } else {
            availabilityLabel.stringValue = diagnosticMessage
            availabilityLabel.textColor = .systemRed
        }
        availabilityLabel.isHidden = availabilityLabel.stringValue.isEmpty

        compactControls.configure(definition, theme: themeStore.theme, sourceFormat: codeSession != nil ? .code : (document?.backend.sourceFormat ?? .plainText), documentID: snapshot.identity.documentID)
        nextStyleRow.configure(
            selected: definition.nextStyleID.map { EVStyleKey(namespace: .block, id: $0) },
            choices: snapshot.compatibleFollowingStyles(for: definition),
            editable: codeSession == nil && definition.capabilities.contains(.nextStyle)
        )
        configureNavigationTargets(snapshot: snapshot, definition: definition)

        let paragraphEnabled = definition.kind == .paragraph
        tabs.setEnabled(true, forSegment: EVStyleEditorTab.character.rawValue)
        tabs.setEnabled(paragraphEnabled, forSegment: EVStyleEditorTab.paragraph.rawValue)
        if !paragraphEnabled, tabs.selectedSegment == EVStyleEditorTab.paragraph.rawValue {
            if let firstResponder = view.window?.firstResponder as? NSView,
               firstResponder === paragraphControls || firstResponder.isDescendant(of: paragraphControls)
            {
                view.window?.makeFirstResponder(tabs)
            }
            tabs.selectedSegment = EVStyleEditorTab.character.rawValue
        }
        tabChanged(tabs)
        updatePreviewAndSummary(snapshot: snapshot, definition: definition)
    }

    private func configureStylePopup(snapshot: EVStyleSheetSnapshot) {
        let menu = NSMenu()
        for kind in EVStyleKind.allCases {
            menu.addItem(.sectionHeader(title: kind.displayName))
            for definition in snapshot.definitions
                .filter({ $0.kind == kind })
                .sorted(by: { $0.name.localizedStandardCompare($1.name) == .orderedAscending })
            {
                let item = NSMenuItem(title: definition.name, action: nil, keyEquivalent: "")
                item.representedObject = EVStyleKeyBox(definition.key)
                item.toolTip = "Stable ID: \(definition.key.id.rawValue)"
                menu.addItem(item)
            }
        }
        stylePopup.menu = menu
        stylePopup.target = self
        stylePopup.action = #selector(styleChanged(_:))
        stylePopup.isEnabled = true
    }

    private func configureBasedOn(snapshot: EVStyleSheetSnapshot, definition: EVStyleDefinition) {
        basedOnPopup.removeAllItems()
        if definition.capabilities.contains(.parent) {
            for candidate in snapshot.legalParents(for: definition)
                .sorted(by: { $0.name.localizedStandardCompare($1.name) == .orderedAscending })
            {
                basedOnPopup.addItem(withTitle: candidate.name)
                basedOnPopup.lastItem?.representedObject = EVStyleKeyBox(candidate.key)
            }
            if let parentKey = definition.parentKey,
               let item = basedOnPopup.itemArray.first(where: {
                   ($0.representedObject as? EVStyleKeyBox)?.key == parentKey
               })
            {
                basedOnPopup.select(item)
            }
            basedOnPopup.isEnabled = !basedOnPopup.itemArray.isEmpty
            basedOnPopup.toolTip = "Only compatible styles that cannot create an inheritance cycle are shown."
        } else {
            let title = definition.parentKey.flatMap { snapshot.definition(for: $0)?.name } ?? "None"
            basedOnPopup.addItem(withTitle: title)
            basedOnPopup.isEnabled = false
            basedOnPopup.toolTip = definition.flags.isBase
                ? "The parent of this distinguished base style is fixed."
                : "This style's parent is read-only."
        }
    }

    private func configureNavigationTargets(snapshot: EVStyleSheetSnapshot, definition: EVStyleDefinition) {
        let parent = definition.parentKey.flatMap { snapshot.definition(for: $0) }
        editParentButton.isEnabled = parent != nil && parent?.key != definition.key
        editParentButton.toolTip = parent.map { "Edit \($0.name)" } ?? "This style has no parent."

        let next = definition.kind == .paragraph
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

    private func contributorName(for property: EVResolvedStyleProperty?) -> String {
        guard let property else { return "Unavailable" }
        if property.isDeclared { return "Explicit" }
        if let contributor = property.contributor {
            let name = snapshot?.definition(for: contributor)?.name ?? contributor.id.rawValue
            return "Inherited · \(name)"
        }
        if property.contributorKind == UInt32(VIEM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY) {
            return "Default · engine"
        }
        return "Inherited"
    }

    private func beginContinuousStyleEdit() {
        if let codeSession {
            do { try codeSession.beginGroup() }
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
        codeSession?.endGroup()
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
        guard hasTarget, codeSession != nil || document?.session != nil else {
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
            if let codeSession {
                try codeSession.edit(key: selectedStyleKey, expected: forcedIdentity ?? latest.identity, mutation: mutation)
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
                updatePreviewAndSummary(snapshot: snapshot, definition: definition)
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

    private func updatePreviewAndSummary(snapshot: EVStyleSheetSnapshot?, definition: EVStyleDefinition) {
        preview.apply(
            kind: definition.kind,
            effectiveValues: definition.properties.reduce(into: [:]) { values, entry in
                if entry.value.usesThemeDefault && entry.key == .characterForeground {
                    let color = themeStore.theme.foreground
                    values[entry.key] = .color(EVStyleColor(red: Float(color.red), green: Float(color.green), blue: Float(color.blue), alpha: Float(color.alpha)))
                } else if let effective = entry.value.effective { values[entry.key] = effective }
            },
            canvasBackground: previewCanvasBackground(snapshot: snapshot)
        )

        let parent = definition.parentKey.flatMap { snapshot?.definition(for: $0)?.name } ?? "None"
        let visible = EVStyleProperty.characterProperties
            + (definition.kind == .paragraph ? EVStyleProperty.paragraphProperties : [])
        let declared = visible.compactMap { definition.properties[$0] }.filter(\.isDeclared)
        var lines = ["Style: \(definition.name)  ·  Based on: \(parent)"]
        if declared.isEmpty { lines.append("All formatting is inherited.") }
        else { lines.append(declared.map { "\($0.property.displayName): \(format($0.declared))" }.joined(separator: "  ·  ")) }
        let inherited = visible.compactMap { definition.properties[$0] }.filter { !$0.isDeclared }
        if !inherited.isEmpty {
            lines.append("Inherited contributions")
            lines.append(inherited.map { $0.usesThemeDefault ? "\($0.property.displayName): Default (theme foreground)" : "\($0.property.displayName): \(format($0.effective)) — \(contributorName(for: $0))" }.joined(separator: "  ·  "))
        }
        if definition.kind == .paragraph {
            let nextName = definition.nextStyleID.flatMap {
                snapshot?.definition(namespace: .block, id: $0)?.name
            } ?? "Same Style"
            lines.append("  Following paragraph style: \(nextName)")
        }
        if !diagnosticMessage.isEmpty { lines.append(contentsOf: ["", "Last edit: \(diagnosticMessage)"]) }
        summary.string = lines.joined(separator: "\n")
    }

    private func previewCanvasBackground(snapshot: EVStyleSheetSnapshot?) -> EVStyleColor {
        if let resolved = snapshot?.definition(for: .baseDocument)?.properties[.canvasBackground],
           !resolved.usesThemeDefault, case let .color(background)? = resolved.effective { return background }
        let color = themeStore.theme.background
        return EVStyleColor(red: Float(color.red), green: Float(color.green), blue: Float(color.blue), alpha: Float(color.alpha))
    }

    private func renderNoDocument() {
        isUpdatingUI = true
        defer { isUpdatingUI = false }
        stylePopup.removeAllItems()
        stylePopup.isEnabled = false
        newStylePopup.isEnabled = false
        deleteStyleButton.isEnabled = false
        nameDraftIsInvalid = false
        nameField.stringValue = ""
        nameField.isEditable = false
        nameField.isEnabled = false
        typeLabel.stringValue = "No document"
        basedOnPopup.removeAllItems()
        basedOnPopup.addItem(withTitle: "Unavailable")
        basedOnPopup.isEnabled = false
        editParentButton.isEnabled = false
        editNextStyleButton.isEnabled = false
        diagnosticMessage = ""
        availabilityLabel.stringValue = "No target document. Choose Edit Styles… from a document to retarget this window."
        availabilityLabel.isHidden = false
        availabilityLabel.textColor = .secondaryLabelColor
        tabs.selectedSegment = EVStyleEditorTab.character.rawValue
        tabs.setEnabled(false, forSegment: EVStyleEditorTab.character.rawValue)
        tabs.setEnabled(false, forSegment: EVStyleEditorTab.paragraph.rawValue)
        compactControls.configure(nil, theme: themeStore.theme)
        nextStyleRow.configure(selected: nil, choices: [], editable: false)
        characterControls.isHidden = false
        paragraphControls.isHidden = true
        preview.showUnavailable()
        summary.string = "No document is currently targeted. No style changes can be issued."
    }

    private func renderUnavailableStyleSheet() {
        guard hasTarget else { renderNoDocument(); return }
        isUpdatingUI = true
        defer { isUpdatingUI = false }
        stylePopup.isEnabled = false
        newStylePopup.isEnabled = false
        deleteStyleButton.isEnabled = false
        nameField.isEnabled = false
        basedOnPopup.isEnabled = false
        editParentButton.isEnabled = false
        editNextStyleButton.isEnabled = false
        tabs.setEnabled(false, forSegment: EVStyleEditorTab.character.rawValue)
        tabs.setEnabled(false, forSegment: EVStyleEditorTab.paragraph.rawValue)
        availabilityLabel.stringValue = diagnosticMessage.isEmpty ? "Styles are temporarily unavailable." : diagnosticMessage
        availabilityLabel.isHidden = false
        availabilityLabel.textColor = .systemRed
        preview.showUnavailable()
        summary.string = availabilityLabel.stringValue
    }

    func controlTextDidChange(_ notification: Notification) {
        guard !isUpdatingUI, notification.object as AnyObject? === nameField else { return }
        let candidate = nameField.stringValue
        guard isValidStyleName(candidate) else {
            nameDraftIsInvalid = true
            nameField.backgroundColor = NSColor.systemRed.withAlphaComponent(0.12)
            nameField.toolTip = "A style name cannot be blank or contain a null character."
            return
        }
        nameDraftIsInvalid = false
        nameField.backgroundColor = .textBackgroundColor
        if candidate != selectedDefinition?.name { _ = commit(.setDisplayName(candidate)) }
    }

    func controlTextDidBeginEditing(_ notification: Notification) {
        guard notification.object as AnyObject? === nameField else { return }
        beginContinuousStyleEdit()
    }

    func controlTextDidEndEditing(_ notification: Notification) {
        guard notification.object as AnyObject? === nameField else { return }
        endContinuousStyleEdit()
    }

    private func isValidStyleName(_ value: String) -> Bool {
        !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !value.contains("\0")
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

    private func sectionTitle(_ value: String) -> NSTextField {
        let field = NSTextField(labelWithString: value)
        field.font = .systemFont(ofSize: 13, weight: .semibold)
        field.setAccessibilityRoleDescription("heading")
        return field
    }

    private func separator() -> NSBox {
        let box = NSBox()
        box.boxType = .separator
        return box
    }

    @objc private func styleChanged(_ sender: NSPopUpButton) {
        guard let key = (sender.selectedItem?.representedObject as? EVStyleKeyBox)?.key else { return }
        selectStyle(key)
    }

    @objc private func newStylePressed(_ sender: NSPopUpButton) {
        let kind: EVStyleKind = sender.indexOfSelectedItem == 2 ? .character : .paragraph
        if createStyle(kind: kind) { view.window?.makeFirstResponder(nameField) }
    }

    @objc private func restoreCodeDefaults(_ sender: Any?) {
        guard let codeSession else { return }
        _ = changeStyleCatalogue { try codeSession.restoreDefaults() }
    }

    @objc private func deleteStylePressed(_ sender: Any?) { _ = deleteSelectedStyle() }

    @objc private func basedOnChanged(_ sender: NSPopUpButton) {
        guard !isUpdatingUI, let key = (sender.selectedItem?.representedObject as? EVStyleKeyBox)?.key else { return }
        _ = commit(.setParent(key.id))
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
        characterControls.isHidden = showParagraph
        paragraphControls.isHidden = !showParagraph
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

@MainActor
private final class EVFollowingStyleRow: NSObject {
    var popupForCompactLayout: NSPopUpButton { popup }
    let view: NSView
    var onChange: ((EVStyleKey?) -> Void)?
    private let popup = NSPopUpButton()
    private var isConfiguring = false

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
        popup.removeAllItems()
        popup.addItem(withTitle: "Same Style")
        for choice in choices.sorted(by: { $0.name.localizedStandardCompare($1.name) == .orderedAscending }) {
            popup.addItem(withTitle: choice.name)
            popup.lastItem?.representedObject = EVStyleKeyBox(choice.key)
        }
        if let selected,
           let item = popup.itemArray.first(where: { ($0.representedObject as? EVStyleKeyBox)?.key == selected })
        {
            popup.select(item)
        } else { popup.selectItem(at: 0) }
        popup.isEnabled = editable
    }

    @objc private func changed(_ sender: NSPopUpButton) {
        guard !isConfiguring else { return }
        onChange?((sender.selectedItem?.representedObject as? EVStyleKeyBox)?.key)
    }
}

private func format(_ value: EVStyleValue?) -> String {
    switch value {
    case let .float(number): "\(formatNumber(number)) pt"
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
