import AppKit
import CViemCore

@MainActor
private final class EVLinkPanel: NSPanel {
    var takesKeyboardFocus = false
    override var canBecomeKey: Bool { takesKeyboardFocus }
    override var canBecomeMain: Bool { false }
}

/// Transient controls retain an exact core selection; the document owns all edits.
@MainActor
final class EVInlineContentPopoverController: NSObject, NSTextFieldDelegate {
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
    let removeButton = NSButton()
    let textField = NSTextField()
    let destinationField = NSTextField()
    let applyButton = NSButton(title: "Apply", target: nil, action: nil)
    private let cancelButton = NSButton(title: "Cancel", target: nil, action: nil)
    private let errorLabel = NSTextField(wrappingLabelWithString: "")
    private var context: EVInlineContentContext?
    private var expected: ViemLogicalSelectionIdentityV1?
    private var suppressed: ViemLogicalSelectionIdentityV1?
    private var cachedSelection: ViemLogicalSelectionIdentityV1?
    private var cachedContext: EVInlineContentContext?
    private weak var originWindow: NSWindow?
    private var eventMonitor: Any?
    private var observers: [NSObjectProtocol] = []
    private(set) var isOpen = false
    private(set) var isEditing = false
    private var applying = false
    private var anchor = NSRect.zero
    var popupFrame: NSRect { panel.frame }

    init(surface: EVEditorSurfaceController, kind: EVInlineContentKind) {
        self.surface = surface
        self.kind = kind
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
        configure(removeButton, title: kind == .image ? "Delete image" : "Remove link", symbol: kind == .image ? "trash" : "link.badge.minus", action: #selector(remove))
        // Draw the requested struck chain rather than relying on OS symbol availability.
        if kind == .link { removeButton.image = Self.unlinkImage() }
        for button in [copyButton, editButton, removeButton] { summary.addSubview(button) }
        // Native two-column form: regular labels, intrinsic control heights,
        // and a footer immediately below the fields. Validation has no empty slot.
        form.orientation = .vertical
        form.alignment = .trailing
        form.spacing = 12
        form.detachesHiddenViews = true
        form.widthAnchor.constraint(equalToConstant: 360).isActive = true
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
        destinationField.nextKeyView = applyButton
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
        if let cachedSelection, cachedSelection.isSameSelection(as: selection), let cachedContext { return cachedContext }
        let result = try session.inlineContentContext(kind)
        cachedSelection = selection; cachedContext = result
        return result
    }

    var canOpenEditor: Bool {
        guard surface?.commandLine?.prompt == nil, let session = surface?.session, !session.hasActiveComposition,
              let selection = try? session.listSelection(),
              let context = try? readContext(session: session, selection: selection) else { return false }
        return context.canInsert || context.item?.editable == true
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
        destinationButton.title = link.destination
        destinationButton.toolTip = link.destination
        editButton.isEnabled = link.editable
        removeButton.isEnabled = link.editable
        show(editing: false)
    }

    func openEditor() {
        guard let surface, surface.commandLine?.prompt == nil, surface.acceptCompletionForNativeInput(), let session = surface.session,
              !session.hasActiveComposition else { return }
        do {
            let selection = try session.listSelection()
            let next = try readContext(session: session, selection: selection)
            guard next.item?.editable == true || next.canInsert,
                  let rect = anchorRect(for: next.item) else { return }
            context = next; expected = selection; suppressed = nil; anchor = rect
            beginEditing()
        } catch { surface.report(error) }
    }

    private func beginEditing() {
        guard let context else { return }
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
        let formSize = form.fittingSize
        let size = isEditing ? NSSize(width: formSize.width + 28, height: formSize.height + 26)
            : NSSize(width: 374, height: 34)
        let x = min(max(screenRect.minX, visible.minX + 4), visible.maxX - size.width - 4)
        let below = screenRect.minY - 5 - size.height
        let y = below >= visible.minY + 4 ? below : min(screenRect.maxY + 5, visible.maxY - size.height - 4)
        let frame = NSRect(origin: NSPoint(x: x, y: y), size: size)
        summary.frame = NSRect(origin: .zero, size: NSSize(width: 374, height: 34))
        if isEditing { form.frame = NSRect(x: 14, y: 12, width: formSize.width, height: formSize.height) }
        destinationButton.frame = NSRect(x: 6, y: 4, width: 263, height: 26)
        for (index, button) in [copyButton, editButton, removeButton].enumerated() {
            button.frame = NSRect(x: 273 + CGFloat(index) * 32, y: 4, width: 29, height: 26)
        }
        if animated && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            NSAnimationContext.runAnimationGroup { animation in
                animation.duration = 0.16
                panel.animator().setFrame(frame, display: true)
            }
        } else { panel.setFrame(frame, display: true) }
    }

    private func installDismissal() {
        eventMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .keyDown]) { [weak self] event in
            guard let self, self.isOpen else { return event }
            if event.type == .keyDown, event.keyCode == 53 {
                let consumed = self.isEditing
                self.close(restoreFocus: consumed, suppress: true)
                return consumed ? nil : event
            }
            if event.type == .leftMouseDown || event.type == .rightMouseDown {
                if event.window !== self.panel { self.close(suppress: true) }
            }
            return event
        }
        for (name, object) in [(NSApplication.didResignActiveNotification, nil as AnyObject?),
                               (NSWindow.willCloseNotification, originWindow),
                               (NSWindow.didResignMainNotification, originWindow),
                               (NSWindow.didResignKeyNotification, panel)] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: object, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.close() }
            })
        }
    }

    func close(restoreFocus: Bool = false, suppress: Bool = false) {
        guard isOpen else { return }
        if suppress { suppressed = expected }
        isOpen = false; isEditing = false
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
