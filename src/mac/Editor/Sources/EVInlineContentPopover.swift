import AppKit
import CViemCore
import ViemAppShell

@MainActor
private final class EVLinkPanel: NSPanel {
    var takesKeyboardFocus = false
    override var canBecomeKey: Bool { takesKeyboardFocus }
    override var canBecomeMain: Bool { false }
}

/// Transient controls retain an exact core selection; the document owns all edits.
@MainActor
final class EVInlineContentPopoverController: NSObject, NSTextFieldDelegate, NSComboBoxDelegate {
    private weak var surface: EVEditorSurfaceController?
    private let kind: EVInlineContentKind
    private let panel = EVLinkPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel],
                                    backing: .buffered, defer: false)
    private let content = NSVisualEffectView()
    private let summary = NSView()
    private let form = NSStackView()
    let destinationButton = NSButton()
    let copyButton = NSButton()
    let editButton = NSButton()
    let reloadButton = NSButton()
    let removeButton = NSButton()
    let textField = NSTextField()
    let destinationField: NSTextField
    let chooseImageButton = NSButton()
    let applyButton = NSButton(title: "Apply", target: nil, action: nil)
    private let cancelButton = NSButton(title: "Cancel", target: nil, action: nil)
    private let errorLabel = NSTextField(wrappingLabelWithString: "")
    private var context: EVInlineContentContext?
    private var expected: ViemLogicalSelectionIdentityV1?
    private var suppressed: ViemLogicalSelectionIdentityV1?
    private var cachedSelection: ViemLogicalSelectionIdentityV1?
    private var cachedContext: EVInlineContentContext?
    private var cachedReadOnly: Bool?
    private weak var originWindow: NSWindow?
    private var eventMonitor: Any?
    private var observers: [NSObjectProtocol] = []
    private(set) var isOpen = false
    private(set) var isEditing = false
    private var applying = false
    private var anchor = NSRect.zero
    private var headingChoices: [EVLinkHeadingList.Heading] = []
    private var selectingDestination = false
    private var destinationDraft: NSObject?
    private var scheduledDestinationDraft: NSObject?
    private var pendingDestination: (index: Int, text: String, destination: String)?
    private var imageFilePicker: NSOpenPanel?
    var imageFilePanelPresenter: ((NSOpenPanel, NSWindow, @escaping @MainActor (URL?) -> Void) -> Void)?
    var popupFrame: NSRect { panel.frame }

    init(surface: EVEditorSurfaceController, kind: EVInlineContentKind) {
        self.surface = surface
        self.kind = kind
        destinationField = kind == .link ? NSComboBox() : NSTextField()
        super.init()
        panel.isReleasedWhenClosed = false
        panel.hasShadow = true
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.level = .floating
        content.material = .popover
        content.blendingMode = .behindWindow
        content.state = .active
        content.wantsLayer = true
        content.layer?.cornerRadius = 9
        content.layer?.masksToBounds = true
        panel.contentView = content
        content.addSubview(summary)
        content.addSubview(form)
        destinationButton.isBordered = false
        destinationButton.font = .systemFont(ofSize: 12)
        destinationButton.contentTintColor = .linkColor
        destinationButton.alignment = .left
        destinationButton.cell?.lineBreakMode = .byTruncatingMiddle
        destinationButton.setAccessibilityLabel(kind == .image ? "Open image location" : "Open link")
        destinationButton.target = self
        destinationButton.action = #selector(openDestination)
        summary.addSubview(destinationButton)
        configure(copyButton, title: "Copy destination", symbol: "square.on.square", action: #selector(copyDestination))
        configure(editButton, title: kind == .image ? "Edit image" : "Edit link", symbol: "pencil", action: #selector(edit))
        configure(reloadButton, title: "Reload image from disk", symbol: "arrow.clockwise", action: #selector(reload))
        reloadButton.isHidden = true
        configure(removeButton, title: kind == .image ? "Delete image" : "Remove link", symbol: kind == .image ? "trash" : "link.badge.minus", action: #selector(remove))
        // Draw the requested struck chain rather than relying on OS symbol availability.
        if kind == .link { removeButton.image = Self.unlinkImage() }
        for button in [copyButton, editButton, reloadButton, removeButton] { summary.addSubview(button) }
        // Native two-column form: regular labels, intrinsic control heights,
        // and a footer immediately below the fields. Validation has no empty slot.
        form.orientation = .vertical
        form.alignment = .trailing
        form.spacing = 12
        form.detachesHiddenViews = true
        form.widthAnchor.constraint(equalToConstant: 360).isActive = true
        configure(chooseImageButton, title: "Choose image file", symbol: "folder", action: #selector(chooseImage))
        chooseImageButton.refusesFirstResponder = false
        chooseImageButton.isHidden = kind != .image
        chooseImageButton.widthAnchor.constraint(equalToConstant: 28).isActive = true
        let rows = [(kind == .image ? "Alt text" : "Text", textField), (kind == .image ? "Location" : "Destination", destinationField)].map { label, field -> [NSView] in
            let title = NSTextField(labelWithString: label + ":")
            title.font = .systemFont(ofSize: NSFont.systemFontSize)
            title.alignment = .right
            field.controlSize = .regular
            field.font = .systemFont(ofSize: NSFont.systemFontSize)
            field.isBezeled = true
            field.bezelStyle = .squareBezel
            field.delegate = self
            field.setAccessibilityLabel(label)
            field.setContentHuggingPriority(.defaultLow, for: .horizontal)
            field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            if kind == .image, field === destinationField {
                let location = NSStackView(views: [field, chooseImageButton])
                location.orientation = .horizontal
                location.alignment = .centerY
                location.spacing = 6
                return [title, location]
            }
            return [title, field]
        }
        let grid = NSGridView(views: rows)
        grid.rowSpacing = 8
        grid.columnSpacing = 8
        grid.yPlacement = .center
        grid.column(at: 0).xPlacement = .trailing
        grid.column(at: 1).xPlacement = .fill
        grid.widthAnchor.constraint(equalToConstant: 360).isActive = true
        form.addArrangedSubview(grid)
        destinationField.placeholderString = kind == .image ? "image.png or https://…" : "https://…, document.md, or #heading"
        if let combo = destinationField as? NSComboBox {
            combo.completes = false
            combo.numberOfVisibleItems = 12
            combo.hasVerticalScroller = true
        }
        errorLabel.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        errorLabel.textColor = .systemRed
        errorLabel.maximumNumberOfLines = 2
        errorLabel.widthAnchor.constraint(equalToConstant: 360).isActive = true
        errorLabel.isHidden = true
        form.addArrangedSubview(errorLabel)
        let buttons = NSStackView(views: [cancelButton, applyButton])
        buttons.orientation = .horizontal
        buttons.alignment = .centerY
        buttons.spacing = 8
        buttons.setHuggingPriority(.required, for: .horizontal)
        for button in [applyButton, cancelButton] {
            button.bezelStyle = .rounded
            button.controlSize = .regular
            button.font = .systemFont(ofSize: NSFont.systemFontSize)
            button.widthAnchor.constraint(greaterThanOrEqualToConstant: 70).isActive = true
            button.target = self
        }
        form.addArrangedSubview(buttons)
        applyButton.action = #selector(apply)
        applyButton.keyEquivalent = "\r"
        cancelButton.action = #selector(cancel)
        textField.nextKeyView = destinationField
        destinationField.nextKeyView = kind == .image ? chooseImageButton : applyButton
        chooseImageButton.nextKeyView = applyButton
        applyButton.nextKeyView = cancelButton
        cancelButton.nextKeyView = textField
    }

    private func configure(_ button: NSButton, title: String, symbol: String, action: Selector) {
        button.title = title
        button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: title)
        button.imagePosition = .imageOnly
        button.bezelStyle = .accessoryBarAction
        button.controlSize = .small
        button.refusesFirstResponder = true
        button.toolTip = title
        button.setAccessibilityLabel(title)
        button.target = self
        button.action = action
    }

    private static func unlinkImage() -> NSImage {
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: false) { _ in
            NSImage(systemSymbolName: "link", accessibilityDescription: nil)?.draw(in: NSRect(x: 1, y: 1, width: 16, height: 16))
            NSColor.black.setStroke()
            let slash = NSBezierPath()
            slash.move(to: NSPoint(x: 2, y: 16)); slash.line(to: NSPoint(x: 16, y: 2))
            slash.lineWidth = 1.5; slash.stroke()
            return true
        }
        image.isTemplate = true
        return image
    }

    private func readContext(session: EVCoreViewSession, selection: ViemLogicalSelectionIdentityV1) throws -> EVInlineContentContext {
        // Pending link choices change without a source or selection revision.
        // Image context has no typing override and can reuse this identity.
        let readOnly = kind == .image
            ? try session.document.documentState().flags & UInt32(VIEM_DOCUMENT_STATE_READ_ONLY) != 0 : nil
        if kind == .image, let cachedSelection,
           cachedSelection.isSameSelection(as: selection), cachedReadOnly == readOnly,
           let cachedContext { return cachedContext }
        let result = try session.inlineContentContext(kind)
        cachedSelection = selection; cachedContext = result; cachedReadOnly = readOnly
        return result
    }

    var toolbarPresentation: EVMenuItemPresentation {
        guard let surface, surface.commandLine?.prompt == nil, let session = surface.session,
              !session.hasActiveComposition, let selection = try? session.listSelection(),
              let context = try? readContext(session: session, selection: selection) else { return .disabled }
        if kind == .link {
            return EVMenuItemPresentation(isEnabled: context.linked
                ? (selection.text_start == selection.text_end ? context.canExitLink : context.canRemoveSelection)
                : context.canInsert, state: context.linked ? .on : .off)
        }
        let active = context.item.map { image in
            (surface.viewPresentation.mode == UInt32(VIEM_MODE_NORMAL) && selection.text_start == selection.text_end)
                || (selection.text_start == image.start && selection.text_end == image.end)
        } ?? false
        return EVMenuItemPresentation(isEnabled: !active && context.canInsert, state: active ? .on : .off)
    }

    var editorPresentation: EVMenuItemPresentation {
        guard let surface, surface.commandLine?.prompt == nil, let session = surface.session,
              surface.documentState.flags & UInt32(VIEM_DOCUMENT_STATE_READ_ONLY) == 0,
              !session.hasActiveComposition, let selection = try? session.listSelection(),
              let context = try? readContext(session: session, selection: selection) else { return .disabled }
        return EVMenuItemPresentation(isEnabled: context.item?.editable == true || context.canInsert,
            state: context.linked ? .on : .off)
    }

    func performToolbarAction() {
        guard toolbarPresentation.isEnabled, let surface, let session = surface.session else { return }
        do {
            let selection = try session.listSelection()
            let context = try readContext(session: session, selection: selection)
            if kind == .link, context.linked {
                close()
                surface.performInput {
                    _ = try session.editInlineContent(.link,
                        action: selection.text_start == selection.text_end ? 3 : 4,
                        context: context, expected: selection)
                }
                cachedSelection = nil; cachedContext = nil
                surface.formattingToolbar.refresh()
                surface.editorView.window?.makeFirstResponder(surface.editorView)
            } else { openEditor(inserting: true) }
        } catch { surface.report(error) }
    }

    func refresh() {
        guard !applying else { return }
        guard let surface, let session = surface.session, let window = surface.editorView.window,
              surface.backend.sourceFormat == .markdown || surface.backend.sourceFormat == .markdownSource,
              !session.hasActiveComposition, surface.commandLine?.prompt == nil,
              let selection = try? session.listSelection() else { close(); return }
        if kind == .link, !isEditing, (try? session.inlineContentContext(.image).item) != nil { close(); return }
        if kind == .image ? surface.linkPopover.isEditing : surface.imagePopover.isEditing { close(); return }
        if let suppressed, !suppressed.isSameSelection(as: selection) { self.suppressed = nil }
        if isEditing {
            guard let expected, expected.isSameSelection(as: selection), window === originWindow,
                  anchorRect(for: context?.item) != nil else { close(); return }
            position(animated: false)
            return
        }
        guard window.isKeyWindow, window.firstResponder === surface.editorView,
              suppressed?.isSameSelection(as: selection) != true,
              let next = try? readContext(session: session, selection: selection), let link = next.item,
              (!surface.hasSelection || (kind == .image && selection.text_start == link.start && selection.text_end == link.end)),
              let rect = anchorRect(for: link) else { close(); return }
        context = next; expected = selection; anchor = rect
        let label = kind == .link ? link.text : link.destination
        let empty = label.isEmpty
        destinationButton.attributedTitle = NSAttributedString(string: empty ? "empty" : label, attributes: [
            .font: empty ? NSFontManager.shared.convert(NSFont.systemFont(ofSize: 12), toHaveTrait: .italicFontMask) : NSFont.systemFont(ofSize: 12)
        ])
        destinationButton.toolTip = link.destination
        editButton.isEnabled = link.editable
        removeButton.isEnabled = link.editable
        show(editing: false)
    }

    func openEditor(inserting: Bool = false) {
        guard let surface, surface.commandLine?.prompt == nil, surface.acceptCompletionForNativeInput(), let session = surface.session,
              !session.hasActiveComposition else { return }
        do {
            let selection = try session.listSelection()
            let current = try readContext(session: session, selection: selection)
            let next = inserting ? current.forInsertion : current
            guard inserting ? next.canInsert : (next.item?.editable == true || next.canInsert),
                  let rect = anchorRect(for: next.item) else { return }
            context = next; expected = selection; suppressed = nil; anchor = rect
            beginEditing()
        } catch { surface.report(error) }
    }

    private func beginEditing() {
        guard let context else { return }
        destinationDraft = NSObject()
        pendingDestination = nil
        if let combo = destinationField as? NSComboBox, combo.indexOfSelectedItem >= 0 {
            combo.deselectItem(at: combo.indexOfSelectedItem)
        }
        textField.stringValue = context.item?.text ?? context.text
        destinationField.stringValue = context.item?.destination ?? ""
        setValidationMessage("")
        updateValidation()
        show(editing: true)
        panel.makeKeyAndOrderFront(nil)
        panel.makeFirstResponder(textField.stringValue.isEmpty ? textField : destinationField)
    }

    private func show(editing: Bool) {
        guard let window = surface?.editorView.window else { return }
        if editing {
            if kind == .image { surface?.linkPopover.close() } else { surface?.imagePopover.close() }
        }
        let animate = isOpen && !isEditing && editing
        isEditing = editing
        panel.takesKeyboardFocus = editing
        summary.isHidden = editing; form.isHidden = !editing
        if !isOpen {
            isOpen = true; originWindow = window
            window.addChildWindow(panel, ordered: .above)
            installDismissal()
        }
        panel.appearance = window.effectiveAppearance
        position(animated: animate)
        if !editing { panel.orderFront(nil) }
    }

    private func anchorRect(for link: EVInlineContentContext.Item?) -> NSRect? {
        guard let surface, let session = surface.session, let snapshot = surface.layoutSnapshot,
              snapshot.info.identity.document_id == surface.viewPresentation.document_id,
              snapshot.info.identity.document_revision == surface.viewPresentation.document_revision else { return nil }
        let editor = surface.editorView
        if let link {
            let clusters = snapshot.clusters.filter { $0.text_start < link.end && link.start < $0.text_end }
            // Pick the first visible row, then its leftmost glyph (including bidi labels).
            for row in snapshot.rows {
                let rects = clusters.filter { $0.row_index == row.row_index }.map { editor.viewRect($0.typographic_bounds) }
                guard let first = rects.first else { continue }
                let rect = rects.dropFirst().reduce(first) { $0.union($1) }
                if rect.intersects(editor.textViewportRect) { return rect }
            }
            return nil
        }
        guard let geometry = try? session.caretGeometry(offset: surface.viewPresentation.cursor_utf8_offset,
                affinity: surface.viewPresentation.cursor_affinity, in: snapshot.info) else { return nil }
        let rect = editor.viewRect(geometry.rect)
        return rect.intersects(editor.textViewportRect) ? rect : nil
    }

    private func position(animated: Bool) {
        guard let surface, let window = originWindow else { return }
        if let current = anchorRect(for: context?.item) { anchor = current }
        let screenRect = window.convertToScreen(surface.editorView.convert(anchor, to: nil))
        let visible = window.screen?.visibleFrame ?? screenRect.insetBy(dx: -600, dy: -400)
        // Hidden validation rows are detached by the stack during layout.
        // Measure only after that pass, so first expansion and later errors
        // use the current native control heights.
        form.needsLayout = true
        form.layoutSubtreeIfNeeded()
        let formSize = form.fittingSize
        let showsReload = kind == .image && surface.backend.sourceFormat == .markdown
        reloadButton.isHidden = !showsReload
        reloadButton.isEnabled = context?.item.map { surface.session?.provider.canReloadImage($0.destination) == true } ?? false
        let actions = showsReload ? [copyButton, editButton, reloadButton, removeButton] : [copyButton, editButton, removeButton]
        let labelWidth = min(263, ceil(destinationButton.attributedTitle.size().width) + 12)
        let compactWidth = 15 + labelWidth + CGFloat(actions.count) * 32
        let size = isEditing ? NSSize(width: formSize.width + 28, height: formSize.height + 26)
            : NSSize(width: compactWidth, height: 34)
        let x = min(max(screenRect.minX, visible.minX + 4), visible.maxX - size.width - 4)
        let below = screenRect.minY - 5 - size.height
        let y = below >= visible.minY + 4 ? below : min(screenRect.maxY + 5, visible.maxY - size.height - 4)
        let frame = NSRect(origin: NSPoint(x: x, y: y), size: size)
        summary.frame = NSRect(origin: .zero, size: NSSize(width: compactWidth, height: 34))
        destinationButton.frame = NSRect(x: 6, y: 4, width: labelWidth, height: 26)
        for (index, button) in actions.enumerated() {
            button.frame = NSRect(x: labelWidth + 10 + CGFloat(index) * 32, y: 4, width: 29, height: 26)
        }
        panel.setFrame(frame, display: true)
        if isEditing {
            form.frame = NSRect(x: 14, y: 12, width: formSize.width, height: formSize.height)
            form.layoutSubtreeIfNeeded()
        }
        if animated && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            form.alphaValue = 0
            NSAnimationContext.runAnimationGroup { animation in
                animation.duration = 0.16
                form.animator().alphaValue = 1
            }
        } else { form.alphaValue = 1 }
    }

    private func installDismissal() {
        eventMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .keyDown]) { [weak self] event in
            guard let self, self.isOpen else { return event }
            if self.imageFilePicker != nil { return event }
            if event.type == .keyDown, event.keyCode == 53 {
                // Let the native combobox dismiss its list before Escape
                // dismisses the authoring form and its retained draft.
                if self.selectingDestination { return event }
                let consumed = self.isEditing
                self.close(restoreFocus: consumed, suppress: true)
                return consumed ? nil : event
            }
            if event.type == .leftMouseDown || event.type == .rightMouseDown {
                if event.window !== self.panel && !self.selectingDestination { self.close(suppress: true) }
            }
            return event
        }
        for (name, object) in [(NSApplication.didResignActiveNotification, nil as AnyObject?),
                               (NSWindow.willCloseNotification, originWindow),
                               (NSWindow.didResignMainNotification, originWindow),
                               (NSWindow.didResignKeyNotification, panel)] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: object, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    if self.imageFilePicker != nil, name != NSWindow.willCloseNotification { return }
                    self.close()
                }
            })
        }
    }

    func close(restoreFocus: Bool = false, suppress: Bool = false) {
        guard isOpen else { return }
        if suppress { suppressed = expected }
        isOpen = false; isEditing = false
        let picker = imageFilePicker
        imageFilePicker = nil
        chooseImageButton.isEnabled = true
        picker?.cancel(nil)
        selectingDestination = false; headingChoices = []
        destinationDraft = nil; pendingDestination = nil
        if let eventMonitor { NSEvent.removeMonitor(eventMonitor) }; eventMonitor = nil
        for observer in observers { NotificationCenter.default.removeObserver(observer) }; observers.removeAll()
        let window = originWindow
        originWindow = nil; context = nil; expected = nil
        window?.removeChildWindow(panel); panel.orderOut(nil)
        if restoreFocus, let surface { window?.makeKey(); window?.makeFirstResponder(surface.editorView) }
    }

    private func currentTarget() throws -> EVLinkMenuTarget? {
        guard let surface, let session = surface.session, let expected, let context,
              expected.isSameSelection(as: try session.listSelection()), let link = context.item else { return nil }
        return EVLinkMenuTarget(documentID: expected.document_id, revision: expected.document_revision, offset: link.start)
    }

    @objc func openDestination() {
        do {
            guard let target = try currentTarget() else { close(); return }
            close(suppress: true)
            if kind == .image { surface?.openImage(at: target) }
            else { surface?.openLink(at: target) }
        }
        catch { surface?.report(error) }
    }
    @objc func copyDestination() {
        do {
            guard let surface, let target = try currentTarget(),
                  let destination = try (kind == .image ? surface.imageDestination(at: target) : surface.backend.linkDestination(at: target)) else { close(); return }
            _ = surface.pasteboard.viemWrite(EVClipboardRepresentations(plainText: destination))
        } catch { surface?.report(error) }
    }
    @objc func edit() {
        do { guard try currentTarget() != nil, context?.item?.editable == true else { close(); return }; beginEditing() }
        catch { surface?.report(error) }
    }
    @objc func reload() {
        do {
            guard kind == .image, let target = try currentTarget() else { close(); return }
            surface?.reloadImage(at: target)
        } catch { surface?.report(error) }
    }
    @objc func chooseImage() {
        guard kind == .image, isEditing, imageFilePicker == nil,
              let surface, let session = surface.session, let window = originWindow,
              let expected, let draft = destinationDraft,
              let current = try? session.listSelection(), expected.isSameSelection(as: current) else { return }
        let document = surface.documentHostEffectHandler?.documentURL(for: surface)
        let panel = NSOpenPanel()
        panel.title = "Choose Image"
        panel.prompt = "Choose"
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowsOtherFileTypes = true
        panel.directoryURL = document?.deletingLastPathComponent()
        if let existing = try? EVLinkOpener.destinationURL(destinationField.stringValue, relativeTo: document), existing.isFileURL {
            panel.directoryURL = existing.deletingLastPathComponent()
        }
        imageFilePicker = panel
        chooseImageButton.isEnabled = false
        let completed: @MainActor (URL?) -> Void = { [weak self, weak panel, weak session, weak window, weak surface] file in
            guard let self, let panel, self.imageFilePicker === panel else { return }
            self.imageFilePicker = nil
            self.chooseImageButton.isEnabled = true
            guard self.isEditing, self.destinationDraft === draft, let session, let window, let surface,
                  self.surface?.session === session, self.originWindow === window,
                  self.surface?.documentHostEffectHandler?.documentURL(for: surface) == document,
                  let current = try? session.listSelection(), expected.isSameSelection(as: current) else {
                self.close()
                return
            }
            if let file, file.isFileURL {
                self.destinationField.stringValue = EVImageFileLocation.destination(for: file, relativeTo: document)
                self.setValidationMessage(""); self.updateValidation()
            }
            self.panel.makeKeyAndOrderFront(nil)
            self.panel.makeFirstResponder(self.destinationField)
        }
        if let imageFilePanelPresenter { imageFilePanelPresenter(panel, window, completed) }
        else { panel.beginSheetModal(for: window) { response in completed(response == .OK ? panel.url : nil) } }
    }
    @objc func remove() { commit(action: 2) }
    @objc func apply() {
        guard applyButton.isEnabled else { return }
        commit(action: context?.item == nil ? 0 : 1)
    }
    private func commit(action: UInt32) {
        guard let surface, let session = surface.session, let context, let expected else { close(); return }
        applying = true
        var succeeded = false
        surface.performInput {
            do {
                let destination = destinationField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
                if action != 2 { try EVLinkOpener.validateForAuthoring(destination) }
                _ = try session.editInlineContent(kind, action: action, context: context, expected: expected,
                    text: kind == .link && textField.stringValue.isEmpty ? destination : textField.stringValue, destination: destination)
                succeeded = true
            } catch {
                setValidationMessage(error.localizedDescription)
                throw error
            }
        }
        applying = false
        if succeeded {
            cachedSelection = nil; cachedContext = nil
            close(restoreFocus: true)
            refresh()
        }
    }
    @objc func cancel() { close(restoreFocus: true, suppress: true) }
    func controlTextDidChange(_ obj: Notification) { setValidationMessage(""); updateValidation() }
    func comboBoxWillPopUp(_ notification: Notification) {
        guard let combo = notification.object as? NSComboBox, let session = surface?.session, let expected else { return }
        selectingDestination = true
        pendingDestination = nil
        let draft = combo.stringValue
        do {
            let list = try session.linkHeadings(expected: expected)
            headingChoices = list.headings
            combo.removeAllItems()
            combo.addItems(withObjectValues: headingChoices.map { $0.text.isEmpty ? "empty" : $0.text })
            if list.truncated { setValidationMessage("Some headings are omitted from this list. You can also enter any #heading destination.") }
        } catch { combo.removeAllItems(); headingChoices = []; setValidationMessage(error.localizedDescription) }
        combo.stringValue = draft
    }
    func comboBoxWillDismiss(_ notification: Notification) {
        selectingDestination = false
        restoreDestinationAfterTracking()
    }
    func comboBoxSelectionDidChange(_ notification: Notification) {
        guard let combo = notification.object as? NSComboBox, headingChoices.indices.contains(combo.indexOfSelectedItem) else { return }
        let heading = headingChoices[combo.indexOfSelectedItem]
        pendingDestination = (combo.indexOfSelectedItem, heading.text.isEmpty ? "empty" : heading.text, heading.destination)
        combo.stringValue = heading.destination
        setValidationMessage(""); updateValidation()
        restoreDestinationAfterTracking()
    }
    private func restoreDestinationAfterTracking() {
        guard pendingDestination != nil, let draft = destinationDraft, let expected,
              scheduledDestinationDraft !== draft else { return }
        scheduledDestinationDraft = draft
        // NSComboBox may copy its displayed row label into the field after
        // selection delegates return. One coalesced restoration after tracking
        // keeps the authored fragment without retargeting a later draft.
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            if self.scheduledDestinationDraft === draft { self.scheduledDestinationDraft = nil }
            guard self.isEditing, self.destinationDraft === draft,
                  let current = self.expected, current.isSameSelection(as: expected),
                  let selection = try? self.surface?.session?.listSelection(), expected.isSameSelection(as: selection),
                  let combo = self.destinationField as? NSComboBox, let choice = self.pendingDestination,
                  combo.indexOfSelectedItem == choice.index else { return }
            guard combo.stringValue == choice.text || combo.stringValue == choice.destination else {
                self.pendingDestination = nil
                return
            }
            combo.stringValue = choice.destination
            self.setValidationMessage(""); self.updateValidation()
            if !self.selectingDestination { self.pendingDestination = nil }
        }
    }
    private func setValidationMessage(_ message: String) {
        let hidden = message.isEmpty
        guard errorLabel.stringValue != message || errorLabel.isHidden != hidden else { return }
        errorLabel.stringValue = message
        errorLabel.isHidden = hidden
        if isEditing { position(animated: true) }
    }
    private func updateValidation() {
        applyButton.isEnabled = !destinationField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}
