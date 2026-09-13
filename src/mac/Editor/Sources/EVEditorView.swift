import AppKit
import CViemCore
import CoreGraphics
import ViemCoreTextProvider
import ViemAppShell
import Foundation

/// The command line's text and ranges. The status line owns its geometry, so
/// nothing here is a rectangle.
struct EVCommandLineRenderState {
    let prompt: String
    let text: String
    let displayText: String
    /// UTF-16 ranges within `displayText`.
    let markedDisplayRange: NSRange?
    let selectedDisplayRange: NSRange?
    /// Caret position as a UTF-8 offset within `text`.
    let cursorUTF8Offset: Int
}

struct EVResolvedTextPaint {
    let foreground: NSColor
    let background: NSColor?
    let underline: Bool
    let strikethrough: Bool
}

enum EVTextDecorationKind: Equatable {
    case underline
    case strikethrough
}

struct EVTextDecoration {
    let kind: EVTextDecorationKind
    let rect: NSRect
    let color: NSColor
}

private struct EVAccessibilityLayoutContext {
    let snapshot: EVLayoutExport
    let viewport: ViemViewportStateV1
    let presentation: ViemViewPresentationV1
}

private struct EVDocumentMarkedTarget {
    let replacementUTF8: Range<Int>
    let mode: UInt32
}

private struct EVCommandLineMarkedTarget {
    let kind: UInt32
    let viewID: UInt64
    let documentID: UInt64
    let documentRevision: UInt64
    let stateIdentity: [UInt8]
    let coreText: String
    let replacementUTF8: Range<Int>
}

private struct EVCommandInputMarkedTarget {
    let mode: UInt32
    let viewID: UInt64
    let documentID: UInt64
    let documentRevision: UInt64
    let cursorUTF8Offset: UInt64
    let visualAnchorUTF8Offset: UInt64
    let replacementUTF8: Range<Int>
}

private enum EVMarkedTextTarget {
    case document(EVDocumentMarkedTarget)
    case commandLine(EVCommandLineMarkedTarget)
    case commandInput(EVCommandInputMarkedTarget)

    var documentReplacementUTF8: Range<Int>? {
        switch self {
        case let .document(target): target.replacementUTF8
        case .commandLine: nil
        case let .commandInput(target): target.replacementUTF8
        }
    }
}

private struct EVCustomCaretState: Equatable {
    let mode: UInt32
    let documentID: UInt64
    let documentRevision: UInt64
    let cursorUTF8Offset: UInt64
    let affinity: UInt32
}

private struct EVTextInputSelectionState: Equatable {
    let mode: UInt32
    let documentID: UInt64
    let documentRevision: UInt64
    let cursorUTF8Offset: UInt64
    let visualAnchorUTF8Offset: UInt64
    let compositionGeneration: UInt64
    let compositionSelectionStart: UInt64
    let compositionSelectionEnd: UInt64
}

@MainActor
class EVEditorView: NSView, @preconcurrency NSTextInputClient {
    // Padding is part of the core canvas, never fixed window chrome.
    static let canvasInsets = NSEdgeInsetsZero
    static let commandLineFont = NSFont(name: "SF Pro", size: 14) ?? NSFont.systemFont(ofSize: 14)

    static func layoutViewportSize(for viewSize: CGSize) -> CGSize {
        CGSize(
            width: max(1, viewSize.width - canvasInsets.left - canvasInsets.right),
            height: max(1, viewSize.height - canvasInsets.top - canvasInsets.bottom)
        )
    }

    /// `NSWindow` and AppKit subsystems may retain a view briefly after its
    /// `NSViewController` has been released (for example while closing the
    /// pristine launch document after Open). The controller therefore cannot
    /// be an `unowned` lifetime precondition. Late AppKit callbacks observe a
    /// detached, inert text surface through this weak reference instead.
    private(set) weak var surface: EVEditorSurfaceController?
    private let insertionIndicator = NSTextInsertionIndicator(frame: .zero)
    let documentScrollbars: EVDocumentScrollbars
    private var markedTextValue = ""
    private var markedSelection = NSRange(location: NSNotFound, length: 0)
    private var markedTextTarget: EVMarkedTextTarget?
    private var suppressInputContextDiscard = false
    private var dragAutoscrollTimer: Timer?
    private var dragAutoscrollLocation: NSPoint?
    private var lastCustomCaretState: EVCustomCaretState?
    private var lastTextInputSelectionState: EVTextInputSelectionState?
    private var textInputGeometryUpdateActive = false
    private(set) var isActiveTextSurface = false
    // Injectable like the blink clock: an AppKit unit-test process does not
    // own the foreground application's activation lifecycle.
    var applicationIsActive: () -> Bool = { NSApp.isActive }
    var isCaretActive: Bool {
        isActiveTextSurface && window?.isKeyWindow == true && applicationIsActive()
    }
    private var caretFocusObservers: [NSObjectProtocol] = []
    var openLinkURL: (URL, @escaping @MainActor (Error?) -> Void) -> Void = EVLinkOpener.open
    private var caretAppearanceObserver: NSObjectProtocol?
    private var editingPreferencesObserver: NSObjectProtocol?
    private weak var configuredEditingSession: EVCoreViewSession?
    private var configuredSmartQuotes: Bool?
    private var configuredTextWidth: UInt32?
    private var configuredIndentation: EVIndentationOptions?
    private var configuredWhitespace: EVWhitespacePresentationOptions?
    var editingPreferences = EVEditingPreferences.shared {
        didSet { configuredSmartQuotes = nil; configuredTextWidth = nil; configuredIndentation = nil; configuredWhitespace = nil; synchronizeEditingPreferences() }
    }
    private lazy var customCaretBlinkController: EVCustomCaretBlinkController = {
        let controller = EVCustomCaretBlinkController()
        controller.onVisibilityChange = { [weak self] _ in
            self?.needsDisplay = true
        }
        return controller
    }()

    var isDragAutoscrollActive: Bool { dragAutoscrollTimer?.isValid == true }
    var isDocumentInsertionIndicatorVisible: Bool { !insertionIndicator.isHidden }
    /// The status line owns the command-line caret.
    var isCommandLineInsertionIndicatorVisible: Bool {
        statusBar?.isCommandCaretShowing ?? false
    }
    var customCaretPresentationForTesting: EVCustomCaretPresentation {
        customCaretBlinkController.presentation
    }
    var isInactiveCommandLineCaretOutlineVisible: Bool {
        !isCaretActive && commandLineRenderState() != nil
    }
    private var compositionActive: Bool { surface != nil && markedTextTarget != nil }

    init(surface: EVEditorSurfaceController,
         scrollbarStyleProvider: @escaping @MainActor () -> NSScroller.Style = { NSScroller.preferredScrollerStyle }) {
        self.surface = surface
        documentScrollbars = EVDocumentScrollbars(preferredStyleProvider: scrollbarStyleProvider)
        super.init(frame: .zero)
        wantsLayer = true
        layer?.masksToBounds = true
        registerForDraggedTypes([.fileURL])
        setAccessibilityElement(true)
        setAccessibilityRole(.textArea)
        setAccessibilityLabel("Viem editor")
        setAccessibilityHelp("A modal, keyboard-first document editor")

        insertionIndicator.displayMode = .hidden
        insertionIndicator.isHidden = true
        insertionIndicator.automaticModeOptions = [.showEffectsView, .showWhileTracking]
        addSubview(insertionIndicator, positioned: .above, relativeTo: nil)
        documentScrollbars.autoresizingMask = [.width, .height]
        documentScrollbars.onScroll = { [weak self] axis, fraction in
            self?.scrollDocument(axis: axis, fraction: fraction)
        }
        documentScrollbars.onGeometryChange = { [weak self] in
            guard let self else { return }
            self.needsLayout = true
            self.surface?.viewDidLayout()
        }
        addSubview(documentScrollbars, positioned: .above, relativeTo: nil)
        caretAppearanceObserver = NotificationCenter.default.addObserver(
            forName: .viemCaretAppearanceDidChange,
            object: EVCaretAppearanceResolver.shared,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.applyPresentation() }
        }
        editingPreferencesObserver = NotificationCenter.default.addObserver(
            forName: .viemEditingPreferencesDidChange, object: nil, queue: .main
        ) { [weak self] notification in
            MainActor.assumeIsolated {
                guard let self, notification.object as AnyObject? === self.editingPreferences else { return }
                self.synchronizeEditingPreferences()
                self.surface?.refreshPresentation()
                self.needsDisplay = true
            }
        }
        synchronizeEditingPreferences()
        for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification,
                     NSApplication.didBecomeActiveNotification, NSApplication.didResignActiveNotification] {
            caretFocusObservers.append(NotificationCenter.default.addObserver(
                forName: name, object: nil, queue: .main
            ) { [weak self] notification in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    if let changedWindow = notification.object as? NSWindow, changedWindow !== self.window { return }
                    self.refreshCaretActivity()
                }
            })
        }
    }

    deinit {
        for observer in caretFocusObservers { NotificationCenter.default.removeObserver(observer) }
        if let caretAppearanceObserver {
            NotificationCenter.default.removeObserver(caretAppearanceObserver)
        }
        if let editingPreferencesObserver {
            NotificationCenter.default.removeObserver(editingPreferencesObserver)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }

    override var isFlipped: Bool { true }

    var layoutViewportSize: CGSize {
        let size = Self.layoutViewportSize(for: bounds.size)
        let chrome = documentScrollbars.contentInsets
        return CGSize(width: max(1, size.width - chrome.left - chrome.right),
                      height: max(1, size.height - chrome.top - chrome.bottom))
    }

    override func layout() {
        super.layout()
        layoutDocumentScrollbars()
    }
    override var acceptsFirstResponder: Bool { true }
    override var isOpaque: Bool {
        guard let surface,
              let snapshot = surface.layoutSnapshot,
              let paint = exactLayoutPaint(for: snapshot)
        else { return true }
        return nativeCanvas(paint.info).alphaComponent >= 1
    }

    override func becomeFirstResponder() -> Bool {
        let accepted = super.becomeFirstResponder()
        if accepted {
            isActiveTextSurface = true
            applyPresentation()
            surface?.refreshStatusBarActivity()
        }
        return accepted
    }

    override func resignFirstResponder() -> Bool {
        let accepted = super.resignFirstResponder()
        if accepted {
            isActiveTextSurface = false
            stopDragAutoscroll()
            applyPresentation()
            surface?.refreshStatusBarActivity()
        }
        return accepted
    }

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        if newWindow == nil {
            isActiveTextSurface = false
            stopDragAutoscroll()
            customCaretBlinkController.stop()
            endTextInputGeometryUpdate()
        }
        super.viewWillMove(toWindow: newWindow)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        refreshCaretActivity()
    }

    private func refreshCaretActivity() {
        updateCustomCaretPresentation()
        updateInsertionIndicator()
        needsDisplay = true
        surface?.refreshStatusBarActivity()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        EVCaretAppearanceResolver.shared.noteEffectiveAppearanceChange()
    }

    override func draw(_ dirtyRect: NSRect) {
        let snapshot = surface?.layoutSnapshot
        let paint = snapshot.flatMap(exactLayoutPaint(for:))
        let background = paint.map { nativeCanvas($0.info) }
            ?? resolvedColor(.textBackgroundColor)
        background.setFill()
        dirtyRect.fill()
        if let snapshot,
           let context = NSGraphicsContext.current?.cgContext
        {
            context.saveGState()
            context.clip(to: textViewportRect)
            let clusters = drawingClusters(in: dirtyRect, snapshot: snapshot)
            if let paint { drawPaintBackgrounds(clusters, paint: paint) }
            drawSelection(snapshot, in: context)
            drawText(snapshot, clusters: clusters, paint: paint, in: context)
            drawParagraphDecorations(snapshot, dirtyRect: dirtyRect, in: context)
            if let paint { drawTextDecorations(snapshot, clusters: clusters, paint: paint) }
            drawWhitespaceMarkers(snapshot, in: context)
            drawMarkedText(snapshot, clusters: clusters, in: context)
            drawCustomCaret(snapshot, in: context)
            context.restoreGState()
        }
    }

    /// The pane's status line, which renders and hit-tests the command line.
    var statusBar: EVStatusBarView? {
        superview?.subviews.lazy.compactMap { $0 as? EVStatusBarView }.first
    }

    func applyPresentation() {
        synchronizeEditingPreferences()
        reconcileMarkedTextWithCore()
        updateDocumentScrollbars()
        updateCustomCaretPresentation()
        needsDisplay = true
        updateInsertionIndicator()
        notifyTextInputStateChanged()
    }

    /// Refresh a newly created/replaced core view as well as live preferences.
    /// Quote classification and source-aware insertion remain entirely in core.
    private func synchronizeEditingPreferences() {
        guard let session = surface?.session else { return }
        let enabled = editingPreferences.smartQuotes && surface?.backend.sourceFormat != .code
        let textWidth = editingPreferences.textWidth
        guard configuredEditingSession !== session || configuredSmartQuotes != enabled
            || configuredTextWidth != textWidth || configuredIndentation != editingPreferences.indentation
            || configuredWhitespace != editingPreferences.whitespacePresentation else { return }
        do {
            try session.setSmartQuotes(enabled)
            try session.setTextWidthDefault(textWidth)
            try session.setWhitespaceDefaults(indentation: editingPreferences.indentation, presentation: editingPreferences.whitespacePresentation)
            configuredEditingSession = session
            configuredSmartQuotes = enabled
            configuredTextWidth = textWidth
            configuredIndentation = editingPreferences.indentation
            configuredWhitespace = editingPreferences.whitespacePresentation
        } catch {
            // A closing or replaced session may be unavailable. The next
            // presentation retries against the live session instead.
        }
    }

    private func updateCustomCaretPresentation() {
        guard let surface else {
            lastCustomCaretState = nil
            customCaretBlinkController.stop()
            return
        }
        let presentation = surface.viewPresentation
        guard isCustomCaretMode(presentation.mode)
                || (presentation.mode == UInt32(VIEM_MODE_INSERT) && !isCaretActive) else {
            lastCustomCaretState = nil
            customCaretBlinkController.stop()
            return
        }

        let state = EVCustomCaretState(
            mode: presentation.mode,
            documentID: presentation.document_id,
            documentRevision: presentation.document_revision,
            cursorUTF8Offset: presentation.cursor_utf8_offset,
            affinity: presentation.cursor_affinity
        )
        let active = isCaretActive
        if !customCaretBlinkController.isStarted {
            customCaretBlinkController.start(active: active)
        } else {
            customCaretBlinkController.setActive(active)
            if state != lastCustomCaretState {
                customCaretBlinkController.restartAfterActivity()
            }
        }
        lastCustomCaretState = state
    }

    private func isCustomCaretMode(_ mode: UInt32) -> Bool {
        mode == UInt32(VIEM_MODE_NORMAL)
            || mode == UInt32(VIEM_MODE_VISUAL_CHARACTER)
            || mode == UInt32(VIEM_MODE_VISUAL_LINE)
            || mode == UInt32(VIEM_MODE_VISUAL_BLOCK)
            || mode == UInt32(VIEM_MODE_REPLACE)
    }

    private func notifyTextInputStateChanged() {
        guard let surface else {
            lastTextInputSelectionState = nil
            return
        }
        let presentation = surface.viewPresentation
        let state = EVTextInputSelectionState(
            mode: presentation.mode,
            documentID: presentation.document_id,
            documentRevision: presentation.document_revision,
            cursorUTF8Offset: presentation.cursor_utf8_offset,
            visualAnchorUTF8Offset: presentation.visual_anchor_utf8_offset,
            compositionGeneration: surface.compositionOverlay?.info.identity.generation ?? 0,
            compositionSelectionStart: surface.compositionOverlay?.info.selected_start ?? 0,
            compositionSelectionEnd: surface.compositionOverlay?.info.selected_end ?? 0
        )
        inputContext?.invalidateCharacterCoordinates()
        if state != lastTextInputSelectionState {
            inputContext?.textInputClientDidUpdateSelection()
            lastTextInputSelectionState = state
        }
    }

    private func beginTextInputGeometryUpdate() {
        guard !textInputGeometryUpdateActive else { return }
        textInputGeometryUpdateActive = true
        inputContext?.textInputClientWillStartScrollingOrZooming()
    }

    private func endTextInputGeometryUpdate() {
        guard textInputGeometryUpdateActive else { return }
        inputContext?.invalidateCharacterCoordinates()
        inputContext?.textInputClientDidEndScrollingOrZooming()
        textInputGeometryUpdateActive = false
    }

    /// Bracket a synchronous core zoom/reflow with the native text-input
    /// geometry lifecycle so the system insertion indicator, dictation, and
    /// input accessories never retain pre-zoom coordinates.
    func performZoomGeometryUpdate(_ update: () -> Void) {
        beginTextInputGeometryUpdate()
        defer { endTextInputGeometryUpdate() }
        update()
    }

    // MARK: - Accessibility

    override func accessibilityValue() -> Any? {
        surface?.fullPresentedText()
    }

    override func setAccessibilityValue(_ value: Any?) {
        guard let value, let value = plainText(from: value) else { return }
        _ = replaceAccessibilityText(value, in: 0 ..< presentedUTF8Length)
    }

    override func accessibilityNumberOfCharacters() -> Int {
        presentedUTF16Length
    }

    override func accessibilitySelectedText() -> String? {
        guard let surface else { return nil }
        guard let context = accessibilityLayoutContext() else { return nil }
        if isVisualMode(context.presentation.mode) {
            guard let selection = accessibilitySelection(in: context),
                  let ranges = accessibilityUTF8Ranges(in: selection)
            else { return nil }
            return accessibilityText(in: ranges, selectionKind: selection.info.identity.kind)
        }
        if let selected = surface.compositionOverlay?.selectedUTF8Range {
            return surface.presentedText(in: selected)
        }
        return ""
    }

    override func setAccessibilitySelectedText(_ selectedText: String?) {
        guard let selectedText,
              let target = accessibilityReplacementTarget()
        else { return }
        _ = replaceAccessibilityText(selectedText, in: target)
    }

    override func accessibilitySelectedTextRange() -> NSRange {
        accessibilitySelectedTextRanges()?.first?.rangeValue ?? notFoundRange
    }

    override func setAccessibilitySelectedTextRange(_ selectedTextRange: NSRange) {
        _ = routeAccessibilitySelection(selectedTextRange)
    }

    override func accessibilitySelectedTextRanges() -> [NSValue]? {
        guard let surface else { return nil }
        guard let context = accessibilityLayoutContext() else { return nil }
        if isVisualMode(context.presentation.mode) {
            guard let selection = accessibilitySelection(in: context),
                  let ranges = accessibilityUTF8Ranges(in: selection)
            else { return nil }
            let converted = ranges.compactMap(utf16Range(forUTF8:))
            guard converted.count == ranges.count else { return nil }
            return converted.map(NSValue.init(range:))
        }
        if let selected = surface.compositionOverlay?.selectedUTF8Range,
           let range = presentedUTF16Range(forUTF8: selected)
        {
            return [NSValue(range: range)]
        }
        guard let cursor = Int(exactly: context.presentation.cursor_utf8_offset),
              let range = utf16Range(forUTF8: cursor ..< cursor)
        else { return nil }
        return [NSValue(range: range)]
    }

    override func setAccessibilitySelectedTextRanges(_ selectedTextRanges: [NSValue]?) {
        guard let selectedTextRanges, selectedTextRanges.count == 1 else { return }
        _ = routeAccessibilitySelection(selectedTextRanges[0].rangeValue)
    }

    override func accessibilityInsertionPointLineNumber() -> Int {
        guard let surface else { return NSNotFound }
        guard let context = accessibilityLayoutContext(),
              let offset = Int(exactly: surface.compositionOverlay?.info.selected_end
                ?? context.presentation.cursor_utf8_offset)
        else { return NSNotFound }
        if surface.compositionOverlay == nil,
           let point = surface.formattedPointInfo(atUTF8Offset: offset),
           point.identity.document_id == context.presentation.document_id,
           point.identity.document_revision == context.presentation.document_revision,
           let line = Int(exactly: point.hard_line_index)
        {
            return line
        }
        guard let row = context.snapshot.rows.first(where: {
            $0.hard_line_start <= UInt64(offset) && UInt64(offset) <= $0.hard_line_end
        }) else { return NSNotFound }
        return Int(exactly: row.hard_line_index) ?? NSNotFound
    }

    override func accessibilitySharedCharacterRange() -> NSRange {
        NSRange(location: 0, length: presentedUTF16Length)
    }

    override func setAccessibilitySharedCharacterRange(_: NSRange) {
        // The formatted projection is shared by the core, not by AppKit text storage.
    }

    override func accessibilityVisibleCharacterRange() -> NSRange {
        guard let context = accessibilityLayoutContext() else { return notFoundRange }
        let viewportStart = CGFloat(context.viewport.top)
        let viewportEnd = viewportStart + CGFloat(context.snapshot.info.viewport_height)
        let visibleRows = context.snapshot.rows.filter { row in
            let rowStart = CGFloat(row.y)
            let rowEnd = rowStart + max(CGFloat(row.line_advance), 1)
            return rowStart < viewportEnd && viewportStart < rowEnd
        }
        guard let first = visibleRows.first, let last = visibleRows.last,
              let start = Int(exactly: first.text_start),
              var end = Int(exactly: last.text_end)
        else { return notFoundRange }
        if last.flags & UInt32(VIEM_VISUAL_ROW_WRAPS_TO_NEXT) == 0,
           isLineBreak(atUTF8Offset: end)
        {
            end += 1
        }
        return presentedUTF16Range(forUTF8: start ..< end) ?? notFoundRange
    }

    override func setAccessibilityVisibleCharacterRange(_: NSRange) {
        // AppKit cannot own or silently clamp the core viewport. A future
        // scroll-to-range intention can be added when it has a dedicated ABI.
    }

    override func accessibilityAttributedString(for range: NSRange) -> NSAttributedString? {
        guard let utf8Range = presentedUTF8Range(forUTF16: range),
              let substring = accessibilityString(for: range)
        else { return nil }

        let font = Self.commandLineFont
        let fontDescription: [NSAccessibility.FontAttributeKey: Any] = [
            .fontName: font.fontName,
            // AppKit intentionally reports its private `.AppleSystemUIFont`
            // implementation name for the system face. Expose the public
            // document request to accessibility clients instead.
            .fontFamily: CoreTextMeasurementProvider.defaultFontFamily,
            .visibleName: font.displayName ?? font.fontName,
            .fontSize: NSNumber(value: Double(font.pointSize)),
        ]
        let context = accessibilityLayoutContext()
        let paint = context.flatMap { exactAccessibilityPaint(in: $0) }
        let foreground = paint.map { nativeForeground($0.info.default_paint) }
            ?? resolvedColor(.textColor)
        let background = paint.map { nativeCanvas($0.info) }
            ?? resolvedColor(.textBackgroundColor)
        let attributed = NSMutableAttributedString(
            string: substring,
            attributes: [
                .accessibilityFont: fontDescription,
                .accessibilityForegroundColor: foreground.cgColor,
                .accessibilityBackgroundColor: background.cgColor,
            ]
        )

        guard let paint else { return attributed }
        for run in paint.runs {
            guard let runStart = Int(exactly: run.text_start),
                  let runEnd = Int(exactly: run.text_end)
            else { continue }
            let intersectionStart = max(utf8Range.lowerBound, runStart)
            let intersectionEnd = min(utf8Range.upperBound, runEnd)
            guard intersectionStart < intersectionEnd,
                  let global = presentedUTF16Range(
                    forUTF8: intersectionStart ..< intersectionEnd
                  ),
                  let localLocation = subtractingWithoutOverflow(global.location, range.location),
                  let local = makeRange(location: localLocation, length: global.length)
            else { continue }

            let foreground = nativeForeground(run.paint)
            var attributes: [NSAttributedString.Key: Any] = [
                .accessibilityForegroundColor: foreground.cgColor,
            ]
            if run.paint.flags & UInt32(VIEM_TEXT_PAINT_HAS_BACKGROUND) != 0 {
                attributes[.accessibilityBackgroundColor] = nativeColor(run.paint.background).cgColor
            }
            if run.paint.flags & UInt32(VIEM_TEXT_PAINT_UNDERLINE) != 0 {
                attributes[.accessibilityUnderline] = NSUnderlineStyle.single.rawValue
                attributes[.accessibilityUnderlineColor] = foreground.cgColor
            }
            if run.paint.flags & UInt32(VIEM_TEXT_PAINT_STRIKETHROUGH) != 0 {
                attributes[.accessibilityStrikethrough] = true
                attributes[.accessibilityStrikethroughColor] = foreground.cgColor
            }
            attributed.addAttributes(attributes, range: local)
        }
        return attributed
    }

    override func accessibilityString(for range: NSRange) -> String? {
        guard let utf8Range = presentedUTF8Range(forUTF16: range) else { return nil }
        return surface?.presentedText(in: utf8Range)
    }

    override func accessibilityRange(forLine line: Int) -> NSRange {
        guard line >= 0,
              let context = accessibilityLayoutContext(),
              let lineIndex = UInt64(exactly: line)
        else { return notFoundRange }
        let rows = context.snapshot.rows.filter { $0.hard_line_index == lineIndex }
        guard let first = rows.first, let last = rows.last,
              let start = Int(exactly: first.hard_line_start),
              var end = Int(exactly: last.hard_line_end)
        else { return notFoundRange }
        if end < presentedUTF8Length, isLineBreak(atUTF8Offset: end) {
            end += 1
        }
        return presentedUTF16Range(forUTF8: start ..< end) ?? notFoundRange
    }

    override func accessibilityLine(for index: Int) -> Int {
        guard let surface else { return NSNotFound }
        guard index >= 0, index <= presentedUTF16Length else { return NSNotFound }
        var offset = textInputUTF8Offset(forUTF16Location: index)
        if offset == nil {
            let composed = accessibilityRange(for: index)
            if composed.location != NSNotFound {
                offset = textInputUTF8Offset(forUTF16Location: composed.location)
            }
        }
        guard let offset else { return NSNotFound }
        if surface.compositionOverlay == nil,
           let point = surface.formattedPointInfo(atUTF8Offset: offset)
        {
            return Int(exactly: point.hard_line_index) ?? NSNotFound
        }
        guard let row = surface.layoutSnapshot?.rows.first(where: {
            $0.hard_line_start <= UInt64(offset) && UInt64(offset) <= $0.hard_line_end
        }) else { return NSNotFound }
        return Int(exactly: row.hard_line_index) ?? NSNotFound
    }

    override func accessibilityRange(for index: Int) -> NSRange {
        guard let surface else { return notFoundRange }
        guard index >= 0, index <= presentedUTF16Length else { return notFoundRange }
        if index == presentedUTF16Length { return NSRange(location: index, length: 0) }
        guard let snapshot = surface.layoutSnapshot else { return notFoundRange }

        var utf8Ranges = snapshot.clusters.compactMap { cluster -> Range<Int>? in
            guard let start = Int(exactly: cluster.text_start),
                  let end = Int(exactly: cluster.text_end),
                  start < end
            else { return nil }
            return start ..< end
        }
        for row in snapshot.rows where row.flags & UInt32(VIEM_VISUAL_ROW_WRAPS_TO_NEXT) == 0 {
            guard let end = Int(exactly: row.hard_line_end),
                  end < presentedUTF8Length,
                  isLineBreak(atUTF8Offset: end)
            else { continue }
            utf8Ranges.append(end ..< end + 1)
        }
        let endpoints = utf8Ranges.flatMap { [$0.lowerBound, $0.upperBound] }
        let mapped = endpoints.compactMap(textInputUTF16Offset(forLayoutUTF8Offset:))
        guard mapped.count == endpoints.count else { return notFoundRange }
        for rangeIndex in utf8Ranges.indices {
            let start = mapped[rangeIndex * 2]
            let end = mapped[rangeIndex * 2 + 1]
            if start <= index, index < end {
                let utf8Range = utf8Ranges[rangeIndex]
                guard let text = surface.layoutText(in: utf8Range)
                    ?? surface.presentedText(in: utf8Range)
                else { return notFoundRange }
                let local = index - start
                let composed = (text as NSString).rangeOfComposedCharacterSequence(at: local)
                return NSRange(location: start + composed.location, length: composed.length)
            }
        }
        return notFoundRange
    }

    override func accessibilityRange(for point: NSPoint) -> NSRange {
        guard let surface else { return notFoundRange }
        guard let context = accessibilityLayoutContext(), let window else {
            return notFoundRange
        }
        let windowPoint = window.convertPoint(fromScreen: point)
        let local = convert(windowPoint, from: nil)
        guard textViewportRect.contains(local), let session = surface.session else {
            return notFoundRange
        }
        do {
            let hit = try session.hitTest(
                layoutPoint(fromViewPoint: local, viewport: context.viewport),
                in: context.snapshot.info
            )
            guard let range = composedUTF8Range(for: hit) else { return notFoundRange }
            return presentedUTF16Range(forUTF8: range) ?? notFoundRange
        } catch {
            return notFoundRange
        }
    }

    override func accessibilityFrame(for range: NSRange) -> NSRect {
        guard let utf8Range = presentedUTF8Range(forUTF16: range),
              let context = accessibilityLayoutContext(),
              let window,
              let localFrame = accessibilityViewFrame(for: utf8Range, in: context)
        else { return .zero }
        return window.convertToScreen(convert(localFrame, to: nil))
    }

    private func accessibilityLayoutContext() -> EVAccessibilityLayoutContext? {
        guard let surface else { return nil }
        guard let session = surface.session,
              let snapshot = surface.layoutSnapshot,
              let presentation = try? session.presentation(),
              let viewport = try? session.viewportState()
        else { return nil }

        let identity = snapshot.info.identity
        guard identity.view_id == session.viewID,
              surface.formattedSnapshot?.info.identity.document_id == identity.document_id,
              surface.formattedSnapshot?.info.identity.document_revision == identity.document_revision,
              identity.document_id == presentation.document_id,
              identity.document_revision == presentation.document_revision,
              identity.document_id == surface.viewPresentation.document_id,
              identity.document_revision == surface.viewPresentation.document_revision,
              identity.document_id == viewport.document_id,
              identity.document_revision == viewport.document_revision,
              identity.layout_revision == viewport.layout_revision,
              identity.configuration_generation == viewport.configuration_generation,
              identity.measurement_environment_id == viewport.measurement_environment_id,
              identity.metrics_generation == viewport.metrics_generation
        else { return nil }

        return EVAccessibilityLayoutContext(
            snapshot: snapshot,
            viewport: viewport,
            presentation: presentation
        )
    }

    private func exactAccessibilityPaint(
        in context: EVAccessibilityLayoutContext
    ) -> EVLayoutPaintExport? {
        guard let surface else { return nil }
        guard let paint = try? surface.session?.layoutPaintExport(),
              paint.info.identity.isSameLayout(as: context.snapshot.info.identity)
        else { return nil }
        return paint
    }

    private func accessibilitySelection(
        in context: EVAccessibilityLayoutContext
    ) -> EVVisualSelectionExport? {
        guard let surface else { return nil }
        guard isVisualMode(context.presentation.mode),
              let selection = try? surface.session?.visualSelectionExport(),
              selection.info.identity.kind != UInt32(VIEM_VISUAL_SELECTION_KIND_NONE),
              selection.info.identity.layout.isSameLayout(as: context.snapshot.info.identity)
        else { return nil }
        return selection
    }

    private func accessibilityUTF8Ranges(
        in selection: EVVisualSelectionExport
    ) -> [Range<Int>]? {
        guard let surface else { return nil }
        let byteCount = surface.formattedUTF8Length
        var previousEnd = 0
        var result: [Range<Int>] = []
        result.reserveCapacity(selection.segments.count)
        for (index, segment) in selection.segments.enumerated() {
            guard let start = Int(exactly: segment.text_start),
                  let end = Int(exactly: segment.text_end),
                  start >= 0,
                  start <= end,
                  end <= byteCount,
                  index == 0 || previousEnd <= start
            else { return nil }
            result.append(start ..< end)
            previousEnd = end
        }
        return result
    }

    private func accessibilityText(
        in ranges: [Range<Int>],
        selectionKind: UInt32
    ) -> String? {
        guard let surface else { return nil }
        let pieces = ranges.compactMap(surface.formattedText(in:))
        guard pieces.count == ranges.count else { return nil }
        if selectionKind == UInt32(VIEM_VISUAL_SELECTION_KIND_BLOCK) {
            return pieces.joined(separator: "\n")
        }
        return pieces.joined()
    }

    private func isVisualMode(_ mode: UInt32) -> Bool {
        mode == UInt32(VIEM_MODE_VISUAL_CHARACTER)
            || mode == UInt32(VIEM_MODE_VISUAL_LINE)
            || mode == UInt32(VIEM_MODE_VISUAL_BLOCK)
    }

    private func accessibilityReplacementTarget() -> Range<Int>? {
        guard let surface else { return nil }
        guard surface.compositionOverlay == nil else { return nil }
        guard let context = accessibilityLayoutContext() else { return nil }
        let target: Range<Int>
        if isVisualMode(context.presentation.mode) {
            guard let selection = accessibilitySelection(in: context),
                  let ranges = accessibilityUTF8Ranges(in: selection),
                  ranges.count == 1
            else { return nil }
            target = ranges[0]
        } else {
            guard let cursor = Int(exactly: context.presentation.cursor_utf8_offset) else {
                return nil
            }
            target = cursor ..< cursor
        }
        return isMaterialized(target, in: context) ? target : nil
    }

    @discardableResult
    private func routeAccessibilitySelection(_ utf16Range: NSRange) -> Bool {
        guard let surface else { return false }
        guard surface.compositionOverlay == nil,
              let context = accessibilityLayoutContext(),
              let range = utf8Range(forUTF16: utf16Range),
              isMaterialized(range, in: context),
              let session = surface.session
        else { return false }

        do {
            let start = try session.caretGeometry(
                offset: UInt64(range.lowerBound),
                affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM),
                in: context.snapshot.info
            ).point
            let end: ViemLayoutCaretPointV1?
            if range.isEmpty {
                end = nil
            } else {
                guard let activeOffset = finalGraphemeStart(in: range) else { return false }
                end = try session.caretGeometry(
                    offset: UInt64(activeOffset),
                    affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM),
                    in: context.snapshot.info
                ).point
            }
            var succeeded = false
            surface.performInput {
                _ = try session.placeCursor(start, extendSelection: false)
                if let end {
                    _ = try session.placeCursor(end, extendSelection: true)
                }
                succeeded = true
            }
            return succeeded
        } catch {
            return false
        }
    }

    private func finalGraphemeStart(in range: Range<Int>) -> Int? {
        guard let surface else { return nil }
        guard !range.isEmpty,
              let text = surface.presentedText(in: range),
              !text.isEmpty
        else { return nil }
        let final = text.index(before: text.endIndex)
        guard let finalUTF8 = final.samePosition(in: text.utf8) else { return nil }
        return range.lowerBound + text.utf8.distance(from: text.utf8.startIndex, to: finalUTF8)
    }

    @discardableResult
    private func replaceAccessibilityText(_ text: String, in range: Range<Int>) -> Bool {
        guard let surface else { return false }
        guard surface.compositionOverlay == nil,
              let context = accessibilityLayoutContext(),
              range.lowerBound >= 0,
              range.upperBound <= presentedUTF8Length,
              isMaterialized(range, in: context),
              let session = surface.session
        else { return false }

        do {
            _ = try session.caretGeometry(
                offset: UInt64(range.lowerBound),
                affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM),
                in: context.snapshot.info
            )
            if !range.isEmpty {
                _ = try session.caretGeometry(
                    offset: UInt64(range.upperBound),
                    affinity: UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM),
                    in: context.snapshot.info
                )
            }
        } catch {
            return false
        }

        var succeeded = false
        surface.performInput {
            do {
                if session.hasActiveComposition {
                    _ = try session.cancelComposition()
                    self.clearMarkedText()
                }
                _ = try session.beginComposition(
                    replacing: UInt64(range.lowerBound) ..< UInt64(range.upperBound)
                )
                let insertedEnd = UInt64(text.utf8.count)
                _ = try session.updateComposition(
                    text,
                    selected: insertedEnd ..< insertedEnd
                )
                _ = try session.commitComposition(text)
                succeeded = true
            } catch {
                if session.hasActiveComposition {
                    _ = try? session.cancelComposition()
                }
                throw error
            }
        }
        return succeeded
    }

    private func accessibilityViewFrame(
        for range: Range<Int>,
        in context: EVAccessibilityLayoutContext
    ) -> NSRect? {
        guard let surface else { return nil }
        if range.isEmpty {
            guard let session = surface.session else { return nil }
            let geometry = (try? session.caretGeometry(
                offset: UInt64(range.lowerBound),
                affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM),
                in: context.snapshot.info
            )) ?? (try? session.caretGeometry(
                offset: UInt64(range.lowerBound),
                affinity: UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM),
                in: context.snapshot.info
            ))
            return geometry.map { accessibilityViewRect($0.rect, viewport: context.viewport) }
        }
        guard isMaterialized(range, in: context) else { return nil }

        var rectangles: [NSRect] = []
        if let selection = accessibilitySelection(in: context),
           let ranges = accessibilityUTF8Ranges(in: selection)
        {
            let matchingSegments = Set(
                ranges.enumerated().compactMap { index, candidate in
                    candidate == range ? UInt64(index) : nil
                }
            )
            rectangles.append(contentsOf: selection.rectangles.compactMap { rectangle in
                guard matchingSegments.contains(rectangle.segment_index) else { return nil }
                return accessibilityViewRect(rectangle.rect, viewport: context.viewport)
            })
        }

        if rectangles.isEmpty {
            rectangles.append(contentsOf: context.snapshot.clusters.compactMap { cluster in
                guard let start = Int(exactly: cluster.text_start),
                      let end = Int(exactly: cluster.text_end),
                      range.lowerBound < end,
                      start < range.upperBound
                else { return nil }
                return accessibilityViewRect(cluster.typographic_bounds, viewport: context.viewport)
            })
            if let session = surface.session {
                for row in context.snapshot.rows
                    where row.flags & UInt32(VIEM_VISUAL_ROW_WRAPS_TO_NEXT) == 0
                {
                    guard let breakOffset = Int(exactly: row.hard_line_end),
                          range.contains(breakOffset),
                          isLineBreak(atUTF8Offset: breakOffset),
                          let geometry = try? session.caretGeometry(
                              offset: UInt64(breakOffset),
                              affinity: UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM),
                              in: context.snapshot.info
                          )
                    else { continue }
                    rectangles.append(
                        accessibilityViewRect(geometry.rect, viewport: context.viewport)
                    )
                }
            }
        }

        return rectangles.reduce(nil as NSRect?) { partial, rectangle in
            partial.map { $0.union(rectangle) } ?? rectangle
        }
    }

    private func composedUTF8Range(for point: ViemLayoutCaretPointV1) -> Range<Int>? {
        guard let surface else { return nil }
        guard let offset = Int(exactly: point.text_offset),
              let snapshot = surface.layoutSnapshot
        else { return nil }
        let cluster: ViemPositionedClusterV1?
        if point.affinity == UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM) {
            cluster = snapshot.clusters.last {
                $0.text_start < point.text_offset && point.text_offset <= $0.text_end
            }
        } else {
            cluster = snapshot.clusters.first {
                $0.text_start <= point.text_offset && point.text_offset < $0.text_end
            }
        }
        guard let cluster,
              let start = Int(exactly: cluster.text_start),
              let end = Int(exactly: cluster.text_end),
              let text = surface.layoutText(in: start ..< end),
              let local = stringIndex(utf8Offset: offset - start, in: text)
        else { return offset ..< offset }
        let characterStart: String.Index
        if point.affinity == UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM) {
            guard local > text.startIndex else { return offset ..< offset }
            characterStart = text.index(before: local)
        } else {
            guard local < text.endIndex else { return offset ..< offset }
            characterStart = local
        }
        let characterEnd = text.index(after: characterStart)
        guard let startUTF8 = characterStart.samePosition(in: text.utf8),
              let endUTF8 = characterEnd.samePosition(in: text.utf8)
        else { return nil }
        return start + text.utf8.distance(from: text.utf8.startIndex, to: startUTF8)
            ..< start + text.utf8.distance(from: text.utf8.startIndex, to: endUTF8)
    }

    private func isMaterialized(
        _ range: Range<Int>,
        in context: EVAccessibilityLayoutContext
    ) -> Bool {
        guard let surface else { return false }
        if range.isEmpty {
            return context.snapshot.carets.contains { $0.text_offset == UInt64(range.lowerBound) }
                || (try? surface.session?.caretGeometry(
                    offset: UInt64(range.lowerBound),
                    affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM),
                    in: context.snapshot.info
                )) != nil
        }

        var intervals: [Range<Int>] = []
        for row in context.snapshot.rows {
            guard let start = Int(exactly: row.text_start),
                  let end = Int(exactly: row.text_end)
            else { return false }
            if start < end { intervals.append(start ..< end) }
            if row.flags & UInt32(VIEM_VISUAL_ROW_WRAPS_TO_NEXT) == 0,
               isLineBreak(atUTF8Offset: end)
            {
                intervals.append(end ..< end + 1)
            }
        }
        intervals.sort { $0.lowerBound < $1.lowerBound }
        var coveredThrough = range.lowerBound
        for interval in intervals {
            if interval.upperBound <= coveredThrough { continue }
            if interval.lowerBound > coveredThrough { return false }
            coveredThrough = interval.upperBound
            if coveredThrough >= range.upperBound { return true }
        }
        return false
    }

    private func accessibilityViewRect(
        _ rect: ViemLayoutRectV1,
        viewport: ViemViewportStateV1
    ) -> NSRect {
        NSRect(
            x: CGFloat(rect.x) + Self.canvasInsets.left - CGFloat(viewport.left),
            y: CGFloat(rect.y) + Self.canvasInsets.top - CGFloat(viewport.top),
            width: CGFloat(rect.width),
            height: CGFloat(rect.height)
        )
    }

    private func layoutPoint(
        fromViewPoint point: CGPoint,
        viewport: ViemViewportStateV1
    ) -> CGPoint {
        CGPoint(
            x: point.x - Self.canvasInsets.left + CGFloat(viewport.left),
            y: point.y - Self.canvasInsets.top + CGFloat(viewport.top)
        )
    }

    private var textViewportRect: NSRect {
        NSRect(
            x: Self.canvasInsets.left,
            y: Self.canvasInsets.top,
            width: layoutViewportSize.width,
            height: layoutViewportSize.height
        )
    }

    private func layoutDocumentScrollbars() {
        documentScrollbars.frame = NSRect(x: bounds.minX, y: bounds.minY,
            width: bounds.width, height: bounds.height)
        documentScrollbars.needsLayout = true
    }

    private func updateDocumentScrollbars() {
        layoutDocumentScrollbars()
        guard let surface, let snapshot = surface.layoutSnapshot else { return }
        let viewport = surface.viewportState
        let height = max(1, CGFloat(snapshot.info.viewport_height))
        let width = max(1, CGFloat(snapshot.info.viewport_width))
        let step = discreteWheelScrollDistance(in: snapshot)
        let horizontalMaximum = max(0, CGFloat(viewport.maximum_left))
        if let paint = exactLayoutPaint(for: snapshot),
           let canvas = nativeCanvas(paint.info).usingColorSpace(.sRGB) {
            let luminance = 0.2126 * canvas.redComponent + 0.7152 * canvas.greenComponent + 0.0722 * canvas.blueComponent
            let style: NSScroller.KnobStyle = luminance < 0.5 ? .light : .dark
            // Legacy scrollers ignore knobStyle. Match their native appearance
            // to the document canvas, which can differ from the system theme.
            let appearance: NSAppearance.Name = luminance < 0.5 ? .darkAqua : .aqua
            if documentScrollbars.appearance?.name != appearance {
                documentScrollbars.appearance = NSAppearance(named: appearance)
            }
            documentScrollbars.verticalScroller.knobStyle = style
            documentScrollbars.horizontalScroller.knobStyle = style
        }
        documentScrollbars.update(
            vertical: .init(position: CGFloat(viewport.top),
                maximum: max(0, CGFloat(snapshot.info.total_height) - height),
                viewportLength: height, lineStep: step),
            horizontal: .init(position: CGFloat(viewport.left), maximum: horizontalMaximum,
                viewportLength: width, lineStep: step),
            horizontalAvailable: horizontalMaximum > 0.5)
    }

    private func scrollDocument(axis: EVDocumentScrollbars.Axis, fraction: Double) {
        guard let surface, surface.refreshGeometryBeforeScrolling(),
              let snapshot = surface.layoutSnapshot, let session = surface.session else { return }
        let value = min(max(fraction, 0), 1)
        beginTextInputGeometryUpdate()
        defer {
            inputContext?.invalidateCharacterCoordinates()
            inputContext?.textInputClientDidScroll()
            endTextInputGeometryUpdate()
        }
        switch axis {
        case .vertical:
            let maximum = max(0, CGFloat(snapshot.info.total_height - snapshot.info.viewport_height))
            // The total height may still be estimated. A thumb at its lower
            // endpoint requests the actual document end, allowing core to
            // refine the final rows and clamp against their exact geometry.
            let top = value == 1 ? CGFloat(Float.greatestFiniteMagnitude) : CGFloat(value) * maximum
            surface.requestVerticalViewport(top: top)
        case .horizontal:
            surface.performInput {
                _ = try session.setViewportOrigin(left: CGFloat(value) * CGFloat(surface.viewportState.maximum_left),
                    expected: surface.viewportState)
            }
        }
    }

    private func isLineBreak(atUTF8Offset offset: Int) -> Bool {
        surface?.layoutByte(atUTF8Offset: offset) == 0x0A
    }

    // MARK: - Keyboard input

    @objc(undo:) func undoDocument(_ sender: Any?) { surface?.perform(menuCommand: .undo, sender: sender) }
    @objc(redo:) func redoDocument(_ sender: Any?) { surface?.perform(menuCommand: .redo, sender: sender) }
    @objc(cut:) func cutDocumentSelection(_ sender: Any?) { if !performCommandLineMenu(.cut) { surface?.perform(menuCommand: .cut, sender: sender) } }
    @objc(copy:) func copyDocumentSelection(_ sender: Any?) { if !performCommandLineMenu(.copy) { surface?.perform(menuCommand: .copy, sender: sender) } }
    @objc(copySource:) func copyDocumentSource(_ sender: Any?) { if !performCommandLineMenu(.copySource) { surface?.perform(menuCommand: .copySource, sender: sender) } }
    @objc(paste:) func pasteIntoDocument(_ sender: Any?) { if !performCommandLineMenu(.paste) { surface?.perform(menuCommand: .paste, sender: sender) } }
    @objc(pasteAsPlainText:) func pastePlainTextIntoDocument(_ sender: Any?) { if !performCommandLineMenu(.pasteAndMatchStyle) { surface?.perform(menuCommand: .pasteAndMatchStyle, sender: sender) } }
    @objc(delete:) func deleteDocumentSelection(_ sender: Any?) { if !performCommandLineMenu(.delete) { surface?.perform(menuCommand: .delete, sender: sender) } }
    override func selectAll(_ sender: Any?) { if !performCommandLineMenu(.selectAll) { surface?.perform(menuCommand: .selectAll, sender: sender) } }

    override func keyDown(with event: NSEvent) {
        guard let surface else { return }
        surface.dismissCommandOutput()
        guard let session = surface.session else { return }
        customCaretBlinkController.restartAfterActivity()
        let shortcutModifiers = event.modifierFlags.intersection([.command, .control, .option, .shift])
        if event.keyCode == 100, shortcutModifiers.isEmpty {
            surface.perform(menuCommand: .editStyles, sender: event)
            return
        }
        if shortcutModifiers == [.command], let key = event.charactersIgnoringModifiers,
           key == "-" || key == "=" {
            surface.perform(menuCommand: key == "=" ? .zoomIn : .zoomOut, sender: event)
            return
        }
        if event.modifierFlags.intersection(.deviceIndependentFlagsMask).contains(.command) {
            super.keyDown(with: event)
            return
        }
        if event.keyCode == 109, event.modifierFlags.contains(.shift) { showEditorContextMenu(event); return }
        if !compositionActive, moveCommandLineSelection(with: event) { return }
        let textModifiers = event.modifierFlags.intersection([.command, .control, .option, .shift])
        if textModifiers == [.option], event.charactersIgnoringModifiers?.lowercased() == "i",
           !compositionActive,
           [UInt32(VIEM_MODE_INSERT), UInt32(VIEM_MODE_REPLACE)].contains(surface.viewPresentation.mode) {
            surface.perform(menuCommand: .italic, sender: event)
            return
        }

        if event.keyCode == 53 || isControlEscape(event) {
            cancelOperation(event)
            return
        }

        if compositionActive, specialKeyKind(for: event) != nil {
            if inputContext?.handleEvent(event) != true {
                interpretKeyEvents([event])
            }
            reconcileMarkedTextWithCore()
            return
        }

        if event.modifierFlags.intersection(.deviceIndependentFlagsMask).contains(.control),
           let scalar = event.charactersIgnoringModifiers?.unicodeScalars.first,
           scalar.value <= 0x7F
        {
            surface.performInput {
                _ = try session.sendKey(
                    kind: UInt32(VIEM_KEY_CONTROL_CHARACTER),
                    codepoint: scalar.value
                )
            }
            return
        }

        if (event.keyCode == 115 || event.keyCode == 119),
           !event.modifierFlags.contains(.control) {
            // AppKit resolves Home/End through the user's key bindings. Native
            // line and document selectors remain separate core intentions.
            interpretKeyEvents([event])
            return
        }

        if let kind = specialKeyKind(for: event) {
            surface.performInput { _ = try session.sendKey(kind: kind) }
            return
        }
        interpretKeyEvents([event])
    }

    override func doCommand(by selector: Selector) {
        guard let surface else { return }
        if selector == #selector(cancelOperation(_:)) {
            cancelOperation(nil)
            return
        }
        guard let session = surface.session, let kind = keyKind(for: selector) else {
            NSSound.beep()
            return
        }
        surface.performInput { _ = try session.sendKey(kind: kind) }
        reconcileMarkedTextWithCore()
    }

    override func cancelOperation(_ sender: Any?) {
        guard let surface, let session = surface.session else { return }
        if compositionActive {
            cancelActiveMarkedText(using: session, discardInputContext: true)
            return
        }
        // Press-and-hold can own an accent candidate for a letter that was
        // already inserted, without giving the client a marked range. End
        // that native input session even when there is no core overlay.
        if window?.firstResponder === self {
            inputContext?.discardMarkedText()
        }
        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
    }

    private func isControlEscape(_ event: NSEvent) -> Bool {
        event.modifierFlags.contains(.control)
            && (event.charactersIgnoringModifiers == "["
                || event.charactersIgnoringModifiers == "\u{1b}")
    }

    func insertText(_ string: Any, replacementRange: NSRange) {
        guard let surface else { return }
        guard let session = surface.session, let value = plainText(from: string) else { return }
        reconcileMarkedTextWithCore()
        let replacesCurrentMarkedText = compositionActive
            && (replacementRange.location == NSNotFound
                || markedRangeValue().map { $0 == replacementRange } == true)
        if compositionActive, replacesCurrentMarkedText {
            commitActiveMarkedText(value, using: session)
            return
        }
        if compositionActive {
            cancelActiveMarkedText(using: session, discardInputContext: false)
        }

        let mode = surface.viewPresentation.mode
        switch mode {
        case UInt32(VIEM_MODE_INSERT), UInt32(VIEM_MODE_REPLACE):
            let explicitReplacement: Range<Int>?
            if replacementRange.location == NSNotFound {
                explicitReplacement = nil
            } else {
                guard let converted = utf8Range(
                    forUTF16: replacementRange,
                    requireGraphemeBoundaries: true
                ) else { return }
                explicitReplacement = converted
            }
            guard !value.isEmpty || explicitReplacement?.isEmpty == false else { return }
            if let explicitReplacement {
                commitDocumentReplacement(value, replacing: explicitReplacement, mode: mode, using: session)
            } else {
                surface.performInput { _ = try session.sendText(value) }
            }

        case UInt32(VIEM_MODE_COMMAND_LINE):
            guard let commandLine = surface.commandLine else { return }
            let range: Range<Int>
            if replacementRange.location == NSNotFound { range = commandLine.selectedUTF8Range }
            else {
                guard let converted = utf8Range(forUTF16: replacementRange, in: commandLine.text, requireGraphemeBoundaries: true) else { return }
                range = converted
            }
            surface.performInput { _ = try session.editCommandLine(commandLine, selecting: range, replacement: value) }

        case UInt32(VIEM_MODE_NORMAL),
             UInt32(VIEM_MODE_VISUAL_CHARACTER),
             UInt32(VIEM_MODE_VISUAL_LINE),
             UInt32(VIEM_MODE_VISUAL_BLOCK):
            if replacementRange.location != NSNotFound,
               utf8Range(
                   forUTF16: replacementRange,
                   requireGraphemeBoundaries: true
               ) == nil
            {
                return
            }
            guard !value.isEmpty else { return }
            // AppKit delivers printable keys through insertText even in a
            // command mode. Normalize them to keys so core's layout preflight
            // runs for j/k, Visual Block, and other visual commands. Preserve
            // multi-scalar graphemes as text operands for r/f and IME input.
            surface.performInput {
                for character in value {
                    let scalars = character.unicodeScalars
                    if scalars.count == 1, let scalar = scalars.first,
                       session.lastOutcome.mode != UInt32(VIEM_MODE_INSERT),
                       session.lastOutcome.mode != UInt32(VIEM_MODE_REPLACE),
                       session.lastOutcome.mode != UInt32(VIEM_MODE_COMMAND_LINE)
                    {
                        _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: scalar.value)
                    } else {
                        _ = try session.sendText(String(character))
                    }
                }
            }
        default:
            return
        }
    }

    private func specialKeyKind(for event: NSEvent) -> UInt32? {
        switch event.keyCode {
        case 53: UInt32(VIEM_KEY_ESCAPE)
        case 36, 76: UInt32(event.modifierFlags.contains(.shift) ? VIEM_KEY_SHIFT_ENTER : VIEM_KEY_ENTER)
        case 48:
            UInt32(event.modifierFlags.contains(.shift)
                ? VIEM_KEY_BACK_TAB : VIEM_KEY_TAB)
        case 51: UInt32(VIEM_KEY_BACKSPACE)
        case 117: UInt32(VIEM_KEY_DELETE)
        case 123: UInt32(VIEM_KEY_LEFT)
        case 124: UInt32(VIEM_KEY_RIGHT)
        case 125: UInt32(VIEM_KEY_DOWN)
        case 126: UInt32(VIEM_KEY_UP)
        case 115: UInt32(event.modifierFlags.contains(.control) ? VIEM_KEY_DOCUMENT_START : VIEM_KEY_HOME)
        case 119: UInt32(event.modifierFlags.contains(.control) ? VIEM_KEY_DOCUMENT_END : VIEM_KEY_END)
        case 116: UInt32(VIEM_KEY_PAGE_UP)
        case 121: UInt32(VIEM_KEY_PAGE_DOWN)
        default: nil
        }
    }

    private func keyKind(for selector: Selector) -> UInt32? {
        switch selector {
        case #selector(moveLeft(_:)), #selector(moveBackward(_:)): UInt32(VIEM_KEY_LEFT)
        case #selector(moveRight(_:)), #selector(moveForward(_:)): UInt32(VIEM_KEY_RIGHT)
        case #selector(moveUp(_:)): UInt32(VIEM_KEY_UP)
        case #selector(moveDown(_:)): UInt32(VIEM_KEY_DOWN)
        case #selector(moveToBeginningOfLine(_:)), #selector(moveToBeginningOfParagraph(_:)): UInt32(VIEM_KEY_HOME)
        case #selector(moveToEndOfLine(_:)), #selector(moveToEndOfParagraph(_:)): UInt32(VIEM_KEY_END)
        case #selector(moveToBeginningOfDocument(_:)), #selector(scrollToBeginningOfDocument(_:)): UInt32(VIEM_KEY_DOCUMENT_START)
        case #selector(moveToEndOfDocument(_:)), #selector(scrollToEndOfDocument(_:)): UInt32(VIEM_KEY_DOCUMENT_END)
        case #selector(pageUp(_:)), #selector(scrollPageUp(_:)): UInt32(VIEM_KEY_PAGE_UP)
        case #selector(pageDown(_:)), #selector(scrollPageDown(_:)): UInt32(VIEM_KEY_PAGE_DOWN)
        case #selector(deleteBackward(_:)): UInt32(VIEM_KEY_BACKSPACE)
        case #selector(deleteForward(_:)): UInt32(VIEM_KEY_DELETE)
        case #selector(insertNewline(_:)): UInt32(VIEM_KEY_ENTER)
        case #selector(insertLineBreak(_:)): UInt32(VIEM_KEY_SHIFT_ENTER)
        case #selector(insertTab(_:)): UInt32(VIEM_KEY_TAB)
        case #selector(insertBacktab(_:)): UInt32(VIEM_KEY_BACK_TAB)
        case #selector(cancelOperation(_:)): UInt32(VIEM_KEY_ESCAPE)
        default: nil
        }
    }

    func commandLineMenuEnabled(_ command: EVMenuCommand) -> Bool? {
        guard let surface, let prompt = surface.commandLine, prompt.prompt != nil else { return nil }
        switch command {
        case .copy, .copySource, .cut, .delete: return !prompt.selectedUTF8Range.isEmpty
        case .paste, .pasteAndMatchStyle: return surface.pasteboard.viemCanReadString()
        case .selectAll: return !prompt.text.isEmpty
        case .undo, .redo: return false
        default: return nil
        }
    }

    @discardableResult private func performCommandLineMenu(_ command: EVMenuCommand) -> Bool {
        guard let surface, let session = surface.session, let prompt = surface.commandLine, prompt.prompt != nil else { return false }
        let range = prompt.selectedUTF8Range
        switch command {
        case .copy, .copySource, .cut:
            guard !range.isEmpty, let lower = stringIndex(utf8Offset: range.lowerBound, in: prompt.text), let upper = stringIndex(utf8Offset: range.upperBound, in: prompt.text) else { return true }
            guard surface.pasteboard.viemIsWritable else { return true }
            _ = surface.pasteboard.viemClearContents()
            guard surface.pasteboard.viemSetString(String(prompt.text[lower..<upper])) else { return true }
            if command == .cut { surface.performInput { _ = try session.editCommandLine(prompt, selecting: range, replacement: "") } }
        case .paste, .pasteAndMatchStyle:
            if let value = surface.pasteboard.viemString() { surface.performInput { _ = try session.editCommandLine(prompt, selecting: range, replacement: value) } }
        case .delete:
            if !range.isEmpty { surface.performInput { _ = try session.editCommandLine(prompt, selecting: range, replacement: "") } }
        case .selectAll:
            surface.performInput { _ = try session.editCommandLine(prompt, anchor: 0, active: prompt.text.utf8.count) }
        default: return false
        }
        return true
    }

    private func moveCommandLineSelection(with event: NSEvent) -> Bool {
        guard let surface, let session = surface.session, let prompt = surface.commandLine, prompt.prompt != nil,
              event.modifierFlags.intersection([.command, .control, .option]).isEmpty,
              [123, 124, 115, 119].contains(Int(event.keyCode)),
              let index = stringIndex(utf8Offset: Int(prompt.info.cursor_utf8_offset), in: prompt.text)
        else { return false }
        let extend = event.modifierFlags.contains(.shift)
        let active: Int
        switch event.keyCode {
        case 115: active = 0
        case 119: active = prompt.text.utf8.count
        case 123:
            if !extend, !prompt.selectedUTF8Range.isEmpty { active = prompt.selectedUTF8Range.lowerBound }
            else { active = prompt.text[..<(index == prompt.text.startIndex ? index : prompt.text.index(before: index))].utf8.count }
        default:
            if !extend, !prompt.selectedUTF8Range.isEmpty { active = prompt.selectedUTF8Range.upperBound }
            else { active = prompt.text[..<(index == prompt.text.endIndex ? index : prompt.text.index(after: index))].utf8.count }
        }
        surface.performInput { _ = try session.editCommandLine(prompt, anchor: extend ? Int(prompt.selectionAnchorUTF8Offset) : active, active: active) }
        return true
    }


    override func menu(for event: NSEvent) -> NSMenu? {
        surface?.dismissCommandOutput()
        let menu = NSMenu(title: "Edit")
        if let target = linkMenuTarget(for: event) {
            let item = NSMenuItem(title: "Open link", action: #selector(openLink(_:)), keyEquivalent: "")
            item.image = NSImage(systemSymbolName: "globe", accessibilityDescription: nil)
            item.target = self
            item.representedObject = target
            menu.addItem(item)
            menu.addItem(.separator())
        }
        for (title, action) in [("Cut", #selector(cutDocumentSelection(_:))), ("Copy", #selector(copyDocumentSelection(_:))), ("Paste", #selector(pasteIntoDocument(_:))), ("Paste and Match Style", #selector(pastePlainTextIntoDocument(_:))), ("Select All", #selector(selectAll(_:)))] {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
            item.target = self
            menu.addItem(item)
        }
        return menu
    }

    private func linkMenuTarget(for event: NSEvent) -> EVLinkMenuTarget? {
        guard let surface, let session = surface.session, let snapshot = surface.layoutSnapshot else { return nil }
        let target: EVLinkMenuTarget
        if event.type == .keyDown {
            let presentation = surface.viewPresentation
            target = EVLinkMenuTarget(documentID: presentation.document_id,
                                      revision: presentation.document_revision,
                                      offset: presentation.cursor_utf8_offset)
        } else {
            let local = convert(event.locationInWindow, from: nil)
            let point = layoutPoint(fromViewPoint: local)
            // Hit testing alone snaps distant whitespace to the nearest caret.
            // Only actual link glyphs should acquire the special context menu.
            guard snapshot.clusters.contains(where: { clusterRect($0).contains(local) }),
                  let caret = try? session.hitTest(point, in: snapshot.info) else { return nil }
            var offset = caret.text_offset
            if let cluster = snapshot.clusters.first(where: { clusterRect($0).contains(local) }),
               offset >= cluster.text_end { offset = cluster.text_start }
            target = EVLinkMenuTarget(documentID: caret.document_id,
                                      revision: caret.document_revision, offset: offset)
        }
        guard (try? surface.backend.linkDestination(at: target)) != nil else { return nil }
        return target
    }

    @objc func openLink(_ sender: NSMenuItem) {
        guard let surface, let target = sender.representedObject as? EVLinkMenuTarget else { return }
        do {
            guard let destination = try surface.backend.linkDestination(at: target) else { return }
            let base = surface.documentHostEffectHandler?.documentURL(for: surface)
            let url = try EVLinkOpener.destinationURL(destination, relativeTo: base)
            openLinkURL(url) { [weak surface] error in
                if let error { surface?.report(error) }
            }
        } catch { surface.report(error) }
    }

    private func showEditorContextMenu(_ event: NSEvent) {
        guard let menu = menu(for: event) else { return }
        if event.type == .keyDown { menu.popUp(positioning: nil, at: NSPoint(x: bounds.midX, y: bounds.midY), in: self) }
        else { NSMenu.popUpContextMenu(menu, with: event, for: self) }
    }

    // MARK: - Pointer and scrolling

    override func mouseDown(with event: NSEvent) {
        surface?.dismissCommandOutput()
        if event.modifierFlags.contains(.control) { showEditorContextMenu(event); return }
        stopDragAutoscroll()
        window?.makeFirstResponder(self)
        customCaretBlinkController.restartAfterActivity()
        placeCursor(for: event, extending: event.modifierFlags.contains(.shift))
        if event.clickCount >= 3 {
            surface?.perform(menuCommand: .selectHardLine, sender: event)
        } else if event.clickCount == 2 {
            surface?.perform(menuCommand: .selectWord, sender: event)
        }
    }

    override func mouseDragged(with event: NSEvent) {
        _ = autoscroll(with: event)
        customCaretBlinkController.restartAfterActivity()
        placeCursor(for: event, extending: true)
        updateDragAutoscroll(for: convert(event.locationInWindow, from: nil))
    }

    override func mouseUp(with event: NSEvent) {
        stopDragAutoscroll()
        super.mouseUp(with: event)
    }

    private func placeCursor(for event: NSEvent, extending: Bool) {
        let local = convert(event.locationInWindow, from: nil)
        placeCursor(at: local, extending: extending)
    }

    private func placeCursor(at local: NSPoint, extending: Bool) {
        guard let surface else { return }
        guard let session = surface.session else { return }
        if compositionActive {
            cancelActiveMarkedText(using: session, discardInputContext: true)
        }
        surface.refreshPresentation()
        guard let snapshot = surface.layoutSnapshot else { return }
        let layoutPoint = layoutPoint(fromViewPoint: local)
        surface.performInput {
            let point = try session.hitTest(layoutPoint, in: snapshot.info)
            _ = try session.placeCursor(point, extendSelection: extending)
        }
    }

    override func scrollWheel(with event: NSEvent) {
        guard let surface else { return }
        guard surface.refreshGeometryBeforeScrolling() else { return }
        guard let snapshot = surface.layoutSnapshot else { return }
        documentScrollbars.noteScrollActivity()
        let phases = event.phase.union(event.momentumPhase)
        let discrete = phases.isEmpty
        let deltaScale = event.hasPreciseScrollingDeltas
            ? 1
            : discreteWheelScrollDistance(in: snapshot)
        // AppKit already applies the user's natural-scrolling preference.
        // Its deltas describe content movement; viewport origins move oppositely.
        let deltaX = -event.scrollingDeltaX * deltaScale
        let deltaY = -event.scrollingDeltaY * deltaScale
        if !textInputGeometryUpdateActive {
            beginTextInputGeometryUpdate()
        }
        defer {
            inputContext?.invalidateCharacterCoordinates()
            inputContext?.textInputClientDidScroll()
            if discrete || phases.contains(.ended) || phases.contains(.cancelled) {
                endTextInputGeometryUpdate()
            }
        }
        if deltaX != 0, let session = surface.session
        {
            let maximum = max(0, CGFloat(surface.viewportState.maximum_left))
            let proposed = min(
                max(0, CGFloat(surface.viewportState.left) + deltaX),
                maximum
            )
            surface.performInput {
                _ = try session.setViewportOrigin(
                    left: proposed,
                    expected: surface.viewportState
                )
            }
        }
        if deltaY != 0 {
            surface.requestVerticalViewport(
                top: max(0, CGFloat(surface.viewportState.top) + deltaY)
            )
        }
    }

    /// AppKit reports trackpad deltas in points, but traditional wheel deltas
    /// in abstract line units. Use the exact row intersecting the viewport so
    /// one non-precise unit remains readable with mixed fonts and zoom.
    private func discreteWheelScrollDistance(in snapshot: EVLayoutExport) -> CGFloat {
        guard let surface else { return 1 }
        let viewportTop = CGFloat(surface.viewportState.top)
        if let row = snapshot.rows.first(where: { row in
            let advance = max(CGFloat(row.line_advance), 1)
            return CGFloat(row.y) + advance > viewportTop
        }) {
            let advance = CGFloat(row.line_advance)
            if advance.isFinite, advance > 0 {
                return advance
            }
        }

        let font = Self.commandLineFont
        return max(font.ascender - font.descender + font.leading, 1)
    }

    override func viewWillStartLiveResize() {
        beginTextInputGeometryUpdate()
        super.viewWillStartLiveResize()
    }

    override func viewDidEndLiveResize() {
        inputContext?.invalidateCharacterCoordinates()
        endTextInputGeometryUpdate()
        super.viewDidEndLiveResize()
    }

    override func setFrameSize(_ newSize: NSSize) {
        let changed = frame.size != newSize
        super.setFrameSize(newSize)
        if changed {
            inputContext?.invalidateCharacterCoordinates()
        }
    }

    // MARK: - NSTextInputClient

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        guard let surface else { return }
        guard let session = surface.session,
              let value = plainText(from: string),
              let selection = utf8Range(inMarkedText: value, utf16Range: selectedRange)
        else { return }

        reconcileMarkedTextWithCore()

        let keepsCurrentTarget = compositionActive
            && (replacementRange.location == NSNotFound
                || markedRangeValue().map { $0 == replacementRange } == true)
        let mode = surface.viewPresentation.mode
        switch mode {
        case UInt32(VIEM_MODE_INSERT), UInt32(VIEM_MODE_REPLACE):
            let target: Range<Int>
            if keepsCurrentTarget,
               case let .document(documentTarget)? = markedTextTarget
            {
                target = documentTarget.replacementUTF8
            } else if replacementRange.location != NSNotFound {
                guard let replacement = utf8Range(
                    forUTF16: replacementRange,
                    requireGraphemeBoundaries: true
                ) else { return }
                target = replacement
            } else {
                guard let replacement = surface.selectedUTF8Range() ?? cursorUTF8Range() else { return }
                target = replacement
            }
            setDocumentMarkedText(
                value,
                selectedRange: selectedRange,
                selectedUTF8: selection,
                target: target,
                mode: mode,
                keepsCurrentTarget: keepsCurrentTarget,
                using: session
            )

        case UInt32(VIEM_MODE_COMMAND_LINE):
            guard let commandLine = surface.commandLine else { return }
            let target: EVCommandLineMarkedTarget
            if keepsCurrentTarget,
               case let .commandLine(existing)? = markedTextTarget
            {
                target = existing
            } else {
                let replacement: Range<Int>
                if replacementRange.location == NSNotFound {
                    guard let cursor = Int(exactly: commandLine.info.cursor_utf8_offset),
                          cursor <= commandLine.text.utf8.count
                    else { return }
                    replacement = commandLine.selectedUTF8Range
                } else {
                    guard let converted = utf8Range(
                        forUTF16: replacementRange,
                        in: commandLine.text,
                        requireGraphemeBoundaries: true
                    ) else { return }
                    replacement = converted
                }
                target = commandLineMarkedTarget(commandLine, replacement: replacement)
            }
            replaceWithLocalMarkedText(
                value,
                selectedRange: selectedRange,
                target: .commandLine(target),
                keepsCurrentTarget: keepsCurrentTarget,
                using: session
            )

        case UInt32(VIEM_MODE_NORMAL),
             UInt32(VIEM_MODE_VISUAL_CHARACTER),
             UInt32(VIEM_MODE_VISUAL_LINE),
             UInt32(VIEM_MODE_VISUAL_BLOCK):
            let target: EVCommandInputMarkedTarget
            if keepsCurrentTarget,
               case let .commandInput(existing)? = markedTextTarget
            {
                target = existing
            } else {
                let replacement: Range<Int>
                if replacementRange.location == NSNotFound {
                    guard let fallback = surface.selectedUTF8Range() ?? cursorUTF8Range() else { return }
                    replacement = fallback
                } else {
                    guard let converted = utf8Range(
                        forUTF16: replacementRange,
                        requireGraphemeBoundaries: true
                    ) else { return }
                    replacement = converted
                }
                guard let current = commandInputMarkedTarget(replacement: replacement) else { return }
                target = current
            }
            replaceWithLocalMarkedText(
                value,
                selectedRange: selectedRange,
                target: .commandInput(target),
                keepsCurrentTarget: keepsCurrentTarget,
                using: session
            )
        default:
            return
        }
    }

    override func draggingEntered(_ sender: any NSDraggingInfo) -> NSDragOperation {
        fileDropOperation(on: sender.draggingPasteboard, sourceMask: sender.draggingSourceOperationMask)
    }

    override func draggingUpdated(_ sender: any NSDraggingInfo) -> NSDragOperation {
        draggingEntered(sender)
    }

    override func prepareForDragOperation(_ sender: any NSDraggingInfo) -> Bool {
        !draggingEntered(sender).isEmpty
    }

    override func performDragOperation(_ sender: any NSDraggingInfo) -> Bool {
        guard !draggingEntered(sender).isEmpty else { return false }
        return performFileDrop(on: sender.draggingPasteboard)
    }

    func fileDropOperation(on pasteboard: NSPasteboard, sourceMask: NSDragOperation) -> NSDragOperation {
        guard surface?.acceptsFileDrops == true, !EVFileDrop.fileURLs(on: pasteboard).isEmpty else { return [] }
        return EVFileDrop.operation(for: sourceMask)
    }

    @discardableResult
    func performFileDrop(on pasteboard: NSPasteboard) -> Bool {
        let urls = EVFileDrop.fileURLs(on: pasteboard)
        guard !urls.isEmpty, let surface, surface.acceptsFileDrops else { return false }
        // Commit native marked input before the host checks portable dirty
        // state. A failed composition commit must never discard its overlay.
        unmarkText()
        guard !hasMarkedText(), surface.session?.hasActiveComposition != true else { return false }
        return surface.openDroppedFiles(urls)
    }

    func unmarkText() {
        guard let surface else {
            clearMarkedText()
            return
        }
        reconcileMarkedTextWithCore()
        guard compositionActive, let session = surface.session else { return }
        commitActiveMarkedText(markedTextValue, using: session)
    }

    func selectedRange() -> NSRange {
        guard let surface else { return notFoundRange }
        if compositionActive,
           let marked = markedRangeValue(),
           let location = addingWithoutOverflow(marked.location, markedSelection.location),
           let selection = makeRange(location: location, length: markedSelection.length)
        {
            return selection
        }
        if surface.viewPresentation.mode == UInt32(VIEM_MODE_COMMAND_LINE),
           let commandLine = surface.commandLine,
           let range = utf16Range(forUTF8: commandLine.selectedUTF8Range, in: commandLine.text)
        {
            return range
        }
        if let range = surface.primarySelectedUTF8Range() {
            return utf16Range(forUTF8: range) ?? notFoundRange
        }
        guard let offset = Int(exactly: surface.viewPresentation.cursor_utf8_offset) else {
            return notFoundRange
        }
        return utf16Range(forUTF8: offset ..< offset) ?? notFoundRange
    }

    func markedRange() -> NSRange {
        markedRangeValue() ?? notFoundRange
    }

    func hasMarkedText() -> Bool { compositionActive }

    func attributedSubstring(
        forProposedRange range: NSRange,
        actualRange: NSRangePointer?
    ) -> NSAttributedString? {
        guard let result = textInputSubstring(forUTF16: range) else { return nil }
        let (text, actual) = result
        actualRange?.pointee = actual
        return NSAttributedString(
            string: text,
            attributes: [.font: Self.commandLineFont]
        )
    }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] {
        [.font, .foregroundColor, .backgroundColor, .underlineStyle]
    }

    /// Public NSTextInputClient geometry used by dictation, cursor accessories,
    /// and Writing Tools. Both values are in screen coordinates as required by
    /// AppKit; the core remains the selection and layout authority.
    @objc var unionRectInVisibleSelectedRange: NSRect {
        guard let surface else { return .zero }
        guard let window else { return .zero }
        let local: NSRect
        if let snapshot = surface.layoutSnapshot {
            let compositionSelection = surface.compositionOverlay?.selectedUTF8Range
            let rectangles: [NSRect]
            if let compositionSelection, !compositionSelection.isEmpty {
                rectangles = snapshot.clusters.compactMap { cluster in
                    guard let start = Int(exactly: cluster.text_start),
                          let end = Int(exactly: cluster.text_end),
                          compositionSelection.lowerBound < end,
                          start < compositionSelection.upperBound
                    else { return nil }
                    return clusterRect(cluster)
                }
            } else {
                rectangles = selectionRectsForDrawing(in: snapshot)
            }
            if let first = rectangles.first {
                local = rectangles.dropFirst().reduce(first) { $0.union($1) }
            } else if let caret = caretRect(
                offset: presentationCaretUTF8Offset,
                affinity: presentationCaretAffinity,
                snapshot: snapshot
            ) {
                local = caret
            } else {
                local = .zero
            }
        } else {
            local = .zero
        }
        return window.convertToScreen(convert(local, to: nil))
    }

    @objc var documentVisibleRect: NSRect {
        guard let window else { return .zero }
        let local = textViewportRect.intersection(visibleRect)
        return window.convertToScreen(convert(local, to: nil))
    }

    func firstRect(
        forCharacterRange range: NSRange,
        actualRange: NSRangePointer?
    ) -> NSRect {
        guard let surface else { return .zero }
        if surface.viewPresentation.mode == UInt32(VIEM_MODE_COMMAND_LINE),
           let state = commandLineRenderState(),
           let utf8 = utf8Range(forUTF16: range, in: state.text),
           let actual = utf16Range(forUTF8: utf8.lowerBound ..< utf8.lowerBound, in: state.text)
        {
            actualRange?.pointee = actual
            // The status line draws the command caret, so its geometry is the
            // one an input method must anchor to.
            guard let statusBar, let caret = statusBar.commandCaretRect() else { return .zero }
            return window?.convertToScreen(statusBar.convert(caret, to: nil)) ?? .zero
        }
        guard let session = surface.session,
              let snapshot = surface.layoutSnapshot,
              let utf8Offset = textInputUTF8Offset(forUTF16Location: range.location)
        else { return .zero }
        do {
            let geometry = try? session.caretGeometry(
                offset: UInt64(utf8Offset),
                affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM),
                in: snapshot.info
            )
            let resolved = try geometry ?? session.caretGeometry(
                offset: UInt64(utf8Offset),
                affinity: UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM),
                in: snapshot.info
            )
            guard let actual = makeRange(location: range.location, length: 0) else { return .zero }
            actualRange?.pointee = actual
            let local = viewRect(resolved.rect)
            let windowRect = convert(local, to: nil)
            return window?.convertToScreen(windowRect) ?? .zero
        } catch {
            return .zero
        }
    }

    func characterIndex(for point: NSPoint) -> Int {
        guard let surface else { return NSNotFound }
        guard let session = surface.session, let snapshot = surface.layoutSnapshot else { return 0 }
        let windowPoint = window?.convertPoint(fromScreen: point) ?? point
        let local = convert(windowPoint, from: nil)
        do {
            let hit = try session.hitTest(
                layoutPoint(fromViewPoint: local),
                in: snapshot.info
            )
            guard let offset = Int(exactly: hit.text_offset),
                  let location = textInputUTF16Offset(forLayoutUTF8Offset: offset)
            else { return NSNotFound }
            return location
        } catch {
            return NSNotFound
        }
    }

    func coreCompositionDidEnd() {
        reconcileMarkedTextWithCore()
    }

    private func clearMarkedText() {
        markedTextTarget = nil
        markedTextValue = ""
        markedSelection = NSRange(location: NSNotFound, length: 0)
    }

    private var notFoundRange: NSRange {
        NSRange(location: NSNotFound, length: 0)
    }

    private func plainText(from value: Any) -> String? {
        if let attributed = value as? NSAttributedString { return attributed.string }
        return value as? String
    }

    private func cursorUTF8Range() -> Range<Int>? {
        guard let surface else { return nil }
        guard let offset = Int(exactly: surface.viewPresentation.cursor_utf8_offset),
              offset >= 0,
              offset <= surface.formattedUTF8Length
        else { return nil }
        return offset ..< offset
    }

    private func markedRangeValue() -> NSRange? {
        guard let markedTextTarget else { return nil }
        let base: NSRange?
        switch markedTextTarget {
        case let .document(target):
            base = utf16Range(forUTF8: target.replacementUTF8)
        case let .commandLine(target):
            base = utf16Range(forUTF8: target.replacementUTF8, in: target.coreText)
        case let .commandInput(target):
            base = utf16Range(forUTF8: target.replacementUTF8)
        }
        guard let base else { return nil }
        return makeRange(location: base.location, length: markedTextValue.utf16.count)
    }

    private func setDocumentMarkedText(
        _ value: String,
        selectedRange: NSRange,
        selectedUTF8: Range<Int>,
        target: Range<Int>,
        mode: UInt32,
        keepsCurrentTarget: Bool,
        using session: EVCoreViewSession
    ) {
        guard let surface else { return }
        surface.performInput {
            if !keepsCurrentTarget {
                if case .document? = self.markedTextTarget, session.hasActiveComposition {
                    try self.withoutInputContextDiscard {
                        _ = try session.cancelComposition()
                    }
                }
                self.clearMarkedText()
                _ = try session.beginComposition(
                    replacing: UInt64(target.lowerBound) ..< UInt64(target.upperBound)
                )
                self.markedTextTarget = .document(
                    EVDocumentMarkedTarget(replacementUTF8: target, mode: mode)
                )
            }
            self.markedTextValue = value
            self.markedSelection = selectedRange
            _ = try session.updateComposition(
                value,
                selected: UInt64(selectedUTF8.lowerBound) ..< UInt64(selectedUTF8.upperBound)
            )
        }
    }

    private func replaceWithLocalMarkedText(
        _ value: String,
        selectedRange: NSRange,
        target: EVMarkedTextTarget,
        keepsCurrentTarget: Bool,
        using session: EVCoreViewSession
    ) {
        if !keepsCurrentTarget, compositionActive {
            cancelActiveMarkedText(using: session, discardInputContext: false)
            guard !compositionActive else { return }
        }
        if !keepsCurrentTarget { markedTextTarget = target }
        markedTextValue = value
        markedSelection = selectedRange
        applyPresentation()
    }

    private func commitActiveMarkedText(_ value: String, using session: EVCoreViewSession) {
        guard let surface else {
            clearMarkedText()
            return
        }
        guard let target = markedTextTarget else { return }
        markedTextValue = value
        markedSelection = NSRange(location: value.utf16.count, length: 0)
        surface.performInput {
            switch target {
            case let .document(documentTarget):
                let presentation = try session.presentation()
                guard session.hasActiveComposition,
                      presentation.mode == documentTarget.mode,
                      documentTarget.mode == UInt32(VIEM_MODE_INSERT)
                        || documentTarget.mode == UInt32(VIEM_MODE_REPLACE)
                else { return }
                try self.withoutInputContextDiscard {
                    _ = try session.commitComposition(value)
                }
            case let .commandLine(commandLineTarget):
                guard try self.commitCommandLineMarkedText(
                    value,
                    target: commandLineTarget,
                    using: session
                ) else { return }
            case let .commandInput(commandInputTarget):
                let presentation = try session.presentation()
                guard self.commandInputTarget(
                    commandInputTarget,
                    matches: presentation,
                    viewID: session.viewID
                ) else { return }
                if !value.isEmpty { _ = try session.sendText(value) }
            }
            self.clearMarkedText()
        }
    }

    private func commitDocumentReplacement(
        _ value: String,
        replacing replacement: Range<Int>,
        mode: UInt32,
        using session: EVCoreViewSession
    ) {
        guard let surface else { return }
        surface.performInput {
            _ = try session.beginComposition(
                replacing: UInt64(replacement.lowerBound) ..< UInt64(replacement.upperBound)
            )
            self.markedTextTarget = .document(
                EVDocumentMarkedTarget(replacementUTF8: replacement, mode: mode)
            )
            self.markedTextValue = value
            self.markedSelection = NSRange(location: value.utf16.count, length: 0)
            try self.withoutInputContextDiscard {
                _ = try session.commitComposition(value)
            }
            self.clearMarkedText()
        }
    }

    private func cancelActiveMarkedText(
        using session: EVCoreViewSession,
        discardInputContext: Bool
    ) {
        guard let surface else {
            clearMarkedText()
            return
        }
        guard let target = markedTextTarget else { return }
        surface.performInput {
            try self.withoutInputContextDiscard {
                if case .document = target, session.hasActiveComposition {
                    _ = try session.cancelComposition()
                }
                self.clearMarkedText()
            }
        }
        if discardInputContext, window?.firstResponder === self {
            inputContext?.discardMarkedText()
        }
    }

    private func commitCommandLineMarkedText(
        _ value: String,
        target: EVCommandLineMarkedTarget,
        using session: EVCoreViewSession
    ) throws -> Bool {
        let current = try session.commandLineExport()
        guard commandLineTarget(target, matches: current),
              let start = stringIndex(utf8Offset: target.replacementUTF8.lowerBound, in: target.coreText),
              let end = stringIndex(utf8Offset: target.replacementUTF8.upperBound, in: target.coreText)
        else { return false }

        _ = start; _ = end
        _ = try session.editCommandLine(current, selecting: target.replacementUTF8, replacement: value)
        return true
    }

    private func commandLineMarkedTarget(
        _ commandLine: EVCommandLineExport,
        replacement: Range<Int>
    ) -> EVCommandLineMarkedTarget {
        let identity = commandLine.info.identity
        return EVCommandLineMarkedTarget(
            kind: identity.kind,
            viewID: identity.view_id,
            documentID: identity.document_id,
            documentRevision: identity.document_revision,
            stateIdentity: commandLineStateIdentityBytes(identity),
            coreText: commandLine.text,
            replacementUTF8: replacement
        )
    }

    private func commandLineTarget(
        _ target: EVCommandLineMarkedTarget,
        matches commandLine: EVCommandLineExport
    ) -> Bool {
        let identity = commandLine.info.identity
        return target.kind == identity.kind
            && target.viewID == identity.view_id
            && target.documentID == identity.document_id
            && target.documentRevision == identity.document_revision
            && target.stateIdentity == commandLineStateIdentityBytes(identity)
            && target.coreText == commandLine.text
    }

    private func commandLineStateIdentityBytes(_ identity: ViemCommandLineIdentityV1) -> [UInt8] {
        withUnsafeBytes(of: identity.state_identity) { Array($0) }
    }

    private func commandInputMarkedTarget(
        replacement: Range<Int>
    ) -> EVCommandInputMarkedTarget? {
        guard let surface else { return nil }
        guard let session = surface.session else { return nil }
        let presentation = surface.viewPresentation
        return EVCommandInputMarkedTarget(
            mode: presentation.mode,
            viewID: session.viewID,
            documentID: presentation.document_id,
            documentRevision: presentation.document_revision,
            cursorUTF8Offset: presentation.cursor_utf8_offset,
            visualAnchorUTF8Offset: presentation.visual_anchor_utf8_offset,
            replacementUTF8: replacement
        )
    }

    private func commandInputTargetStillMatches(_ target: EVCommandInputMarkedTarget) -> Bool {
        guard let surface else { return false }
        guard let session = surface.session else { return false }
        return commandInputTarget(target, matches: surface.viewPresentation, viewID: session.viewID)
    }

    private func commandInputTarget(
        _ target: EVCommandInputMarkedTarget,
        matches presentation: ViemViewPresentationV1,
        viewID: UInt64
    ) -> Bool {
        return target.viewID == viewID
            && target.mode == presentation.mode
            && target.documentID == presentation.document_id
            && target.documentRevision == presentation.document_revision
            && target.cursorUTF8Offset == presentation.cursor_utf8_offset
            && target.visualAnchorUTF8Offset == presentation.visual_anchor_utf8_offset
    }

    private func textInputSubstring(forUTF16 range: NSRange) -> (String, NSRange)? {
        guard let surface else { return nil }
        if surface.viewPresentation.mode == UInt32(VIEM_MODE_COMMAND_LINE)
            || {
                if case .commandLine? = markedTextTarget { return true }
                return false
            }()
        {
            let text: String
            if case let .commandLine(target)? = markedTextTarget {
                text = replacingUTF8(
                    target.replacementUTF8,
                    in: target.coreText,
                    with: markedTextValue
                ) ?? target.coreText
            } else {
                text = surface.commandLine?.text ?? ""
            }
            guard let utf8Range = utf8Range(forUTF16: range, in: text),
                  let actual = utf16Range(forUTF8: utf8Range, in: text)
            else { return nil }
            let bytes = Array(text.utf8)
            return (String(decoding: bytes[utf8Range], as: UTF8.self), actual)
        }

        guard let replacement = markedTextTarget?.documentReplacementUTF8 else {
            guard let utf8Range = utf8Range(forUTF16: range),
                  let text = surface.formattedText(in: utf8Range)
            else { return nil }
            return (text, range)
        }
        guard let replacement16 = utf16Range(forUTF8: replacement),
              let replacementEnd = addingWithoutOverflow(
                  replacement16.location,
                  replacement16.length
              ),
              let markedEnd = addingWithoutOverflow(
                  replacement16.location,
                  markedTextValue.utf16.count
              ),
              let projectedLengthWithoutReplacement = subtractingWithoutOverflow(
                  surface.formattedUTF16Length,
                  replacement16.length
              ),
              let projectedLength = addingWithoutOverflow(
                  projectedLengthWithoutReplacement,
                  markedTextValue.utf16.count
              ),
              let requestedEnd = validatedUpperBound(for: range, limit: projectedLength)
        else { return nil }

        if range.length == 0 {
            guard textInputUTF8Offset(forUTF16Location: range.location) != nil else { return nil }
            return ("", range)
        }

        let requested = range.location ..< requestedEnd
        var result = ""

        let prefixEnd = min(requested.upperBound, replacement16.location)
        if requested.lowerBound < prefixEnd {
            let prefix = requested.lowerBound ..< prefixEnd
            guard let text = documentBaseSubstring(forUTF16: prefix) else { return nil }
            result += text
        }

        let markedStart = max(requested.lowerBound, replacement16.location)
        let markedFinish = min(requested.upperBound, markedEnd)
        if markedStart < markedFinish {
            let local = NSRange(
                location: markedStart - replacement16.location,
                length: markedFinish - markedStart
            )
            guard let utf8 = utf8Range(forUTF16: local, in: markedTextValue) else { return nil }
            let bytes = Array(markedTextValue.utf8)
            result += String(decoding: bytes[utf8], as: UTF8.self)
        }

        let suffixStart = max(requested.lowerBound, markedEnd)
        if suffixStart < requested.upperBound {
            let localStart = suffixStart - markedEnd
            let localEnd = requested.upperBound - markedEnd
            guard let baseStart = addingWithoutOverflow(replacementEnd, localStart),
                  let baseEnd = addingWithoutOverflow(replacementEnd, localEnd),
                  let text = documentBaseSubstring(forUTF16: baseStart ..< baseEnd)
            else { return nil }
            result += text
        }
        return (result, range)
    }

    private func documentBaseSubstring(forUTF16 range: Range<Int>) -> String? {
        guard let surface else { return nil }
        guard let utf8Range = utf8Range(
            forUTF16: NSRange(location: range.lowerBound, length: range.count)
        ) else { return nil }
        return surface.formattedText(in: utf8Range)
    }

    private func textInputUTF8Offset(forUTF16Location location: Int) -> Int? {
        guard let surface else { return nil }
        guard location >= 0 else { return nil }
        guard case let .document(target)? = markedTextTarget,
              surface.compositionOverlay != nil else {
            return surface.utf8Offsets(forUTF16: [location])?.first
        }
        let replacement = target.replacementUTF8
        guard
              let replacement16 = utf16Range(forUTF8: replacement),
              let replacementEnd = addingWithoutOverflow(
                  replacement16.location,
                  replacement16.length
              ),
              let markedEnd = addingWithoutOverflow(
                  replacement16.location,
                  markedTextValue.utf16.count
              )
        else { return nil }

        if location < replacement16.location {
            return surface.utf8Offsets(forUTF16: [location])?.first
        }
        if location <= markedEnd {
            let local = location - replacement16.location
            guard let marked = utf8Range(
                forUTF16: NSRange(location: local, length: 0),
                in: markedTextValue
            ) else { return nil }
            return addingWithoutOverflow(replacement.lowerBound, marked.lowerBound)
        }
        guard let suffixDelta = subtractingWithoutOverflow(location, markedEnd),
              let baseLocation = addingWithoutOverflow(replacementEnd, suffixDelta),
              let baseOffset = surface.utf8Offsets(forUTF16: [baseLocation])?.first,
              let baseDelta = subtractingWithoutOverflow(baseOffset, replacement.upperBound),
              let markedUTF8End = addingWithoutOverflow(
                  replacement.lowerBound,
                  markedTextValue.utf8.count
              )
        else { return nil }
        return addingWithoutOverflow(markedUTF8End, baseDelta)
    }

    private func textInputUTF16Offset(forLayoutUTF8Offset offset: Int) -> Int? {
        guard let surface else { return nil }
        guard offset >= 0 else { return nil }
        guard case let .document(target)? = markedTextTarget,
              let overlay = surface.compositionOverlay,
              let overlayLength = overlay.utf8Length,
              offset <= overlayLength
        else {
            return utf16Range(forUTF8: offset ..< offset)?.location
        }
        let replacement = target.replacementUTF8
        guard let replacement16 = utf16Range(forUTF8: replacement),
              let replacementEnd = addingWithoutOverflow(
                  replacement16.location,
                  replacement16.length
              ),
              let markedEnd = addingWithoutOverflow(
                  replacement.lowerBound,
                  markedTextValue.utf8.count
              )
        else { return nil }

        if offset < replacement.lowerBound {
            return utf16Range(forUTF8: offset ..< offset)?.location
        }
        if offset <= markedEnd {
            let local = offset - replacement.lowerBound
            guard let marked = utf16Range(forUTF8: local ..< local, in: markedTextValue) else {
                return nil
            }
            return addingWithoutOverflow(replacement16.location, marked.location)
        }
        guard let suffixDelta = subtractingWithoutOverflow(offset, markedEnd),
              let baseOffset = addingWithoutOverflow(replacement.upperBound, suffixDelta),
              let baseLocation = utf16Range(forUTF8: baseOffset ..< baseOffset)?.location,
              let baseDelta = subtractingWithoutOverflow(baseLocation, replacementEnd),
              let visibleMarkedEnd = addingWithoutOverflow(
                  replacement16.location,
                  markedTextValue.utf16.count
              )
        else { return nil }
        return addingWithoutOverflow(visibleMarkedEnd, baseDelta)
    }

    private func replacingUTF8(_ range: Range<Int>, in text: String, with replacement: String) -> String? {
        guard let start = stringIndex(utf8Offset: range.lowerBound, in: text),
              let end = stringIndex(utf8Offset: range.upperBound, in: text)
        else { return nil }
        var result = text
        result.replaceSubrange(start ..< end, with: replacement)
        return result
    }

    private func makeRange(location: Int, length: Int) -> NSRange? {
        guard location >= 0, length >= 0,
              addingWithoutOverflow(location, length) != nil
        else { return nil }
        return NSRange(location: location, length: length)
    }

    private func addingWithoutOverflow(_ left: Int, _ right: Int) -> Int? {
        let result = left.addingReportingOverflow(right)
        return result.overflow ? nil : result.partialValue
    }

    private func subtractingWithoutOverflow(_ left: Int, _ right: Int) -> Int? {
        let result = left.subtractingReportingOverflow(right)
        return result.overflow ? nil : result.partialValue
    }

    private func reconcileMarkedTextWithCore() {
        guard let surface else {
            clearMarkedText()
            return
        }
        guard let target = markedTextTarget else { return }
        let remainsValid: Bool
        switch target {
        case let .document(documentTarget):
            remainsValid = surface.session?.hasActiveComposition == true
                && surface.viewPresentation.mode == documentTarget.mode
                && (documentTarget.mode == UInt32(VIEM_MODE_INSERT)
                    || documentTarget.mode == UInt32(VIEM_MODE_REPLACE))
        case let .commandLine(commandLineTarget):
            remainsValid = surface.session?.hasActiveComposition != true
                && surface.viewPresentation.mode == UInt32(VIEM_MODE_COMMAND_LINE)
                && surface.commandLine.map {
                    self.commandLineTarget(commandLineTarget, matches: $0)
                } == true
        case let .commandInput(commandInputTarget):
            remainsValid = surface.session?.hasActiveComposition != true
                && commandInputTargetStillMatches(commandInputTarget)
        }
        guard !remainsValid else { return }

        let mustCancelCoreComposition: Bool
        if case .document = target {
            mustCancelCoreComposition = surface.session?.hasActiveComposition == true
        } else {
            mustCancelCoreComposition = false
        }
        clearMarkedText()
        if mustCancelCoreComposition, let session = surface.session {
            try? withoutInputContextDiscard { _ = try session.cancelComposition() }
        }
        if !suppressInputContextDiscard, window?.firstResponder === self {
            inputContext?.discardMarkedText()
        }
    }

    private func withoutInputContextDiscard<T>(_ operation: () throws -> T) rethrows -> T {
        let previous = suppressInputContextDiscard
        suppressInputContextDiscard = true
        defer { suppressInputContextDiscard = previous }
        return try operation()
    }

    private func updateDragAutoscroll(for location: NSPoint) {
        guard !textViewportRect.contains(location) else {
            stopDragAutoscroll()
            return
        }
        dragAutoscrollLocation = location
        guard dragAutoscrollTimer == nil else { return }
        let timer = Timer.scheduledTimer(
            timeInterval: 0.075,
            target: self,
            selector: #selector(repeatDragSelectionAutoscroll(_:)),
            userInfo: nil,
            repeats: true
        )
        timer.tolerance = 0.015
        dragAutoscrollTimer = timer
    }

    @objc private func repeatDragSelectionAutoscroll(_: Timer) {
        guard performDragAutoscrollStep() else {
            stopDragAutoscroll()
            return
        }
    }

    /// Advance the core-owned viewport before extending a drag selection at
    /// the nearest visible edge. AppKit's `autoscroll(with:)` cannot scroll
    /// this custom surface because there is deliberately no NSClipView-owned
    /// document offset; the exact origin and hit-test coverage live in core.
    @discardableResult
    func performDragAutoscrollStep() -> Bool {
        guard let surface else { return false }
        guard surface.refreshGeometryBeforeScrolling() else { return false }
        guard let location = dragAutoscrollLocation,
              let session = surface.session
        else { return false }

        let viewport = textViewportRect
        guard !viewport.isEmpty else { return false }
        let current = surface.viewportState
        var left = CGFloat(current.left)
        var top = CGFloat(current.top)

        if location.y < viewport.minY {
            top = max(0, top - dragAutoscrollDistance(viewport.minY - location.y))
        } else if location.y > viewport.maxY {
            top += dragAutoscrollDistance(location.y - viewport.maxY)
        }

        let maximumLeft = max(0, CGFloat(current.maximum_left))
        if location.x < viewport.minX {
            left = max(0, left - dragAutoscrollDistance(viewport.minX - location.x))
        } else if location.x > viewport.maxX {
            left = min(
                maximumLeft,
                left + dragAutoscrollDistance(location.x - viewport.maxX)
            )
        }

        surface.performInput {
            _ = try session.setViewportOrigin(
                left: left,
                top: top,
                expected: current
            )
        }

        // The viewport request synchronously publishes exact local coverage.
        // Hit-test inside that coverage instead of reusing the off-canvas
        // pointer, whose nearest row would otherwise stop advancing.
        let edgePoint = NSPoint(
            x: min(max(location.x, viewport.minX), max(viewport.minX, viewport.maxX - 0.5)),
            y: min(max(location.y, viewport.minY), max(viewport.minY, viewport.maxY - 0.5))
        )
        placeCursor(at: edgePoint, extending: true)
        return true
    }

    private func dragAutoscrollDistance(_ overshoot: CGFloat) -> CGFloat {
        min(48, max(4, ceil(overshoot * 0.35)))
    }

    private func stopDragAutoscroll() {
        dragAutoscrollTimer?.invalidate()
        dragAutoscrollTimer = nil
        dragAutoscrollLocation = nil
    }

    // MARK: - Drawing

    /// Long-line layout slices deliberately include offscreen shaping context.
    /// Only ink, backgrounds, and decorations intersecting this paint damage
    /// reach AppKit/Core Text; geometry and editing coverage remain intact.
    func drawingClusters(in dirtyRect: NSRect, snapshot: EVLayoutExport) -> [ViemPositionedClusterV1] {
        let origin = viewPoint(fromLayoutPoint: .zero)
        let fringe = 1 / max(window?.backingScaleFactor ?? 1, 1)
        let damage = dirtyRect.intersection(bounds)
        guard !damage.isEmpty else { return [] }
        let query = damage.offsetBy(dx: -origin.x, dy: -origin.y)
            .insetBy(dx: -fringe, dy: -fringe)
        let minX = Float(query.minX), maxX = Float(query.maxX)
        let minY = Float(query.minY), maxY = Float(query.maxY)
        return snapshot.clusters.filter { cluster in
            let ink = cluster.ink_bounds
            let cell = cluster.typographic_bounds
            let left = min(ink.x, cell.x)
            let top = min(ink.y, cell.y)
            let right = max(ink.x + ink.width, cell.x + cell.width)
            let bottom = max(ink.y + ink.height, cell.y + cell.height)
            return left < maxX && minX < right && top < maxY && minY < bottom
        }
    }

    private func drawText(
        _ snapshot: EVLayoutExport,
        clusters: [ViemPositionedClusterV1],
        paint: EVLayoutPaintExport?,
        in context: CGContext
    ) {
        guard let surface else { return }
        var colors: [UInt64: (NSColor, CGColor)] = [:]
        for cluster in clusters {
            guard let row = row(for: cluster.row_index, in: snapshot.rows) else { continue }
            let run = paint.flatMap { paintRun(containing: cluster.text_start, in: $0) }
            let key = run?.text_start ?? UInt64.max
            let color: (NSColor, CGColor)
            if let cached = colors[key] { color = cached }
            else {
                let native = paint.map { nativeForeground(run?.paint ?? $0.info.default_paint) }
                    ?? resolvedColor(.textColor)
                color = (native, native.cgColor)
                colors[key] = color
            }
            let baseline = viewPoint(
                fromLayoutPoint: CGPoint(x: CGFloat(cluster.x), y: CGFloat(row.baseline))
            )
            let drewNative = cluster.flags & UInt32(VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN) != 0
                && surface.session?.provider.renderRegistry.draw(
                   identifier: cluster.render_run.identifier,
                   metricsGeneration: cluster.render_run.metrics_generation,
                   atBaseline: baseline,
                   color: color.1,
                   in: context
               ) == true
            if drewNative { continue }
            drawFallback(cluster, row: row, color: color.0)
        }

    }

    /// Paragraph furniture is drawn from its own exact-layout export. It never
    /// participates in text slicing, caret, selection, search or accessibility text.
    private func drawParagraphDecorations(_ snapshot: EVLayoutExport, dirtyRect: NSRect, in context: CGContext) {
        guard let session = surface?.session else { return }
        for item in listMarkersForDrawing(in: snapshot, dirtyRect: dirtyRect) {
            guard let row = row(for: item.row_index, in: snapshot.rows) else { continue }
            let foreground = nativeForeground(item.paint)
            if item.flags & UInt32(VIEM_LAYOUT_DECORATION_BLOCK_QUOTE_BORDER) != 0 {
                foreground.setFill()
                viewRect(item.ink_bounds).fill()
                continue
            }
            if item.paint.flags & UInt32(VIEM_TEXT_PAINT_HAS_BACKGROUND) != 0 {
                nativeColor(item.paint.background).setFill()
                viewRect(item.typographic_bounds).fill()
            }
            let baseline = viewPoint(fromLayoutPoint: CGPoint(x: CGFloat(item.x), y: CGFloat(row.baseline)))
            let native = item.flags & UInt32(VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN) != 0
                && session.provider.renderRegistry.draw(identifier: item.render_run.identifier,
                    metricsGeneration: item.render_run.metrics_generation, atBaseline: baseline,
                    color: foreground.cgColor, in: context)
            if !native {
                let start = Int(item.label_byte_start), end = start + Int(item.label_byte_length)
                if let value = String(bytes: snapshot.decorationLabels[start..<end], encoding: .utf8) {
                    value.draw(at: viewPoint(fromLayoutPoint: CGPoint(x: CGFloat(item.x), y: CGFloat(row.baseline - row.ascent))),
                               withAttributes: [.font: NSFont.systemFont(ofSize: CGFloat(item.font_size)), .foregroundColor: foreground])
                }
            }
            foreground.setFill()
            if item.paint.flags & UInt32(VIEM_TEXT_PAINT_UNDERLINE) != 0 {
                viewRect(x: CGFloat(item.x), y: floor(CGFloat(row.baseline) + max(1, CGFloat(row.descent) * 0.35)),
                         width: CGFloat(item.advance), height: 1).fill()
            }
            if item.paint.flags & UInt32(VIEM_TEXT_PAINT_STRIKETHROUGH) != 0 {
                viewRect(x: CGFloat(item.x), y: floor(CGFloat(row.baseline) - CGFloat(row.ascent) * 0.32),
                         width: CGFloat(item.advance), height: 1).fill()
            }
        }
    }

    func listMarkersForDrawing(in snapshot: EVLayoutExport, dirtyRect: NSRect) -> [ViemLayoutDecorationV1] {
        snapshot.decorations.filter { item in
            row(for: item.row_index, in: snapshot.rows) != nil
                && viewRect(item.ink_bounds).union(viewRect(item.typographic_bounds)).intersects(dirtyRect)
        }
    }

    private func drawPaintBackgrounds(_ clusters: [ViemPositionedClusterV1], paint: EVLayoutPaintExport) {
        for cluster in clusters {
            guard let background = resolvedTextPaint(for: cluster, paint: paint).background else {
                continue
            }
            background.setFill()
            viewRect(cluster.typographic_bounds).fill()
        }
    }

    private func drawTextDecorations(_ snapshot: EVLayoutExport, clusters: [ViemPositionedClusterV1], paint: EVLayoutPaintExport) {
        for decoration in textDecorationsForDrawing(in: snapshot, paint: paint, clusters: clusters) {
            decoration.color.setFill()
            decoration.rect.fill()
        }
    }

    func textDecorationsForDrawing(
        in snapshot: EVLayoutExport,
        paint: EVLayoutPaintExport? = nil,
        clusters: [ViemPositionedClusterV1]? = nil
    ) -> [EVTextDecoration] {
        guard let paint = paint ?? exactLayoutPaint(for: snapshot) else { return [] }
        var result: [EVTextDecoration] = []
        for cluster in clusters ?? snapshot.clusters {
            guard let row = row(for: cluster.row_index, in: snapshot.rows) else { continue }
            let resolved = resolvedTextPaint(for: cluster, paint: paint)
            let thickness: CGFloat = 1
            let left = CGFloat(cluster.typographic_bounds.x)
            let width = CGFloat(cluster.typographic_bounds.width)
            if resolved.underline {
                let y = floor(CGFloat(row.baseline) + max(1, CGFloat(row.descent) * 0.35))
                result.append(
                    EVTextDecoration(
                        kind: .underline,
                        rect: viewRect(x: left, y: y, width: width, height: thickness),
                        color: resolved.foreground
                    )
                )
            }
            if resolved.strikethrough {
                let y = floor(CGFloat(row.baseline) - CGFloat(row.ascent) * 0.32)
                result.append(
                    EVTextDecoration(
                        kind: .strikethrough,
                        rect: viewRect(x: left, y: y, width: width, height: thickness),
                        color: resolved.foreground
                    )
                )
            }
        }
        return result
    }

    func resolvedTextPaint(
        for cluster: ViemPositionedClusterV1,
        paint: EVLayoutPaintExport
    ) -> EVResolvedTextPaint {
        let value = paintRun(containing: cluster.text_start, in: paint)?.paint
            ?? paint.info.default_paint
        let flags = value.flags
        return EVResolvedTextPaint(
            foreground: nativeForeground(value),
            background: flags & UInt32(VIEM_TEXT_PAINT_HAS_BACKGROUND) != 0
                ? nativeColor(value.background)
                : nil,
            underline: flags & UInt32(VIEM_TEXT_PAINT_UNDERLINE) != 0,
            strikethrough: flags & UInt32(VIEM_TEXT_PAINT_STRIKETHROUGH) != 0
        )
    }

    private func paintRun(
        containing offset: UInt64,
        in paint: EVLayoutPaintExport
    ) -> ViemPaintStyleRunV1? {
        var lower = 0
        var upper = paint.runs.count
        while lower < upper {
            let middle = lower + (upper - lower) / 2
            if paint.runs[middle].text_start <= offset {
                lower = middle + 1
            } else {
                upper = middle
            }
        }
        guard lower > 0 else { return nil }
        let candidate = paint.runs[lower - 1]
        return offset < candidate.text_end ? candidate : nil
    }

    private func exactLayoutPaint(for snapshot: EVLayoutExport) -> EVLayoutPaintExport? {
        guard let surface else { return nil }
        guard let paint = surface.layoutPaint,
              paint.info.identity.isSameLayout(as: snapshot.info.identity)
        else { return nil }
        return paint
    }

    private func nativeForeground(_ paint: ViemTextPaintV1) -> NSColor {
        paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) != 0
            ? EVThemeStore.shared.theme.foreground.color : nativeColor(paint.foreground)
    }

    private func nativeCanvas(_ paint: ViemLayoutPaintInfoV1) -> NSColor {
        paint.flags & UInt32(VIEM_LAYOUT_PAINT_DEFAULT_CANVAS) != 0
            ? EVThemeStore.shared.theme.background.color : nativeColor(paint.canvas_background)
    }

    private func nativeColor(_ color: ViemRgbaV1) -> NSColor {
        NSColor(
            srgbRed: CGFloat(color.red),
            green: CGFloat(color.green),
            blue: CGFloat(color.blue),
            alpha: CGFloat(color.alpha)
        )
    }

    private func drawSelection(_ snapshot: EVLayoutExport, in _: CGContext) {
        let color = EVThemeStore.shared.theme.selection.color
        color.setFill()
        for rect in selectionRectsForDrawing(in: snapshot) where rect.intersects(bounds) {
            rect.fill()
        }
    }

    func selectionRectsForDrawing(in snapshot: EVLayoutExport) -> [NSRect] {
        guard let surface else { return [] }
        guard let selection = surface.visualSelection,
              sameLayoutIdentity(selection.info.identity.layout, snapshot.info.identity)
        else { return [] }
        return selection.rectangles.map { viewRect($0.rect) }
    }

    private func sameLayoutIdentity(
        _ left: ViemLayoutSnapshotIdentityV1,
        _ right: ViemLayoutSnapshotIdentityV1
    ) -> Bool {
        left.view_id == right.view_id
            && left.document_id == right.document_id
            && left.document_revision == right.document_revision
            && left.layout_revision == right.layout_revision
            && left.configuration_generation == right.configuration_generation
            && left.measurement_environment_id == right.measurement_environment_id
            && left.metrics_generation == right.metrics_generation
    }

    private func drawMarkedText(_ snapshot: EVLayoutExport, clusters: [ViemPositionedClusterV1], in _: CGContext) {
        guard let surface else { return }
        guard case .document? = markedTextTarget,
              let overlay = surface.compositionOverlay,
              let marked = overlay.markedUTF8Range,
              !marked.isEmpty
        else { return }

        if let selected = overlay.selectedUTF8Range, !selected.isEmpty {
            resolvedColor(.selectedTextBackgroundColor)
                .withAlphaComponent(0.34)
                .setFill()
            for cluster in clusters
                where selected.lowerBound < Int(cluster.text_end)
                    && Int(cluster.text_start) < selected.upperBound
            {
                clusterRect(cluster).fill()
            }
        }

        resolvedColor(.controlAccentColor).setFill()
        let thickness = max(1.0 / (window?.backingScaleFactor ?? 1), 1)
        for cluster in clusters
            where marked.lowerBound < Int(cluster.text_end)
                && Int(cluster.text_start) < marked.upperBound
        {
            let rect = clusterRect(cluster)
            NSRect(
                x: rect.minX,
                y: rect.maxY - thickness,
                width: max(rect.width, thickness),
                height: thickness
            ).fill()
        }
    }

    static func inactiveCaretRect(_ rect: NSRect, mode: UInt32) -> NSRect {
        if mode == UInt32(VIEM_MODE_INSERT) {
            return NSRect(x: rect.minX, y: rect.minY, width: 2, height: rect.height)
        }
        if mode == UInt32(VIEM_MODE_REPLACE) {
            return NSRect(x: rect.minX, y: rect.maxY - 2, width: rect.width, height: 2)
        }
        return rect
    }

    private func drawCustomCaret(_ snapshot: EVLayoutExport, in context: CGContext) {
        guard let surface else { return }
        let mode = surface.viewPresentation.mode
        let active = isCaretActive
        if mode == UInt32(VIEM_MODE_COMMAND_LINE) { return }
        if active && mode == UInt32(VIEM_MODE_INSERT) { return }

        let customPresentation: EVCustomCaretPresentation
        if active {
            customPresentation = customCaretBlinkController.presentation
        } else {
            customPresentation = .inactiveOutline
        }
        guard customPresentation != .hidden else { return }

        let color = EVCaretAppearanceResolver.shared.color(for: self)
        let geometry = caretItemGeometry(snapshot)
        guard var rect = geometry.rect else { return }
        if geometry.cluster == nil || rect.width < 1 {
            rect.size.width = minimumCaretWidth(near: geometry.cluster, in: snapshot)
        }

        if customPresentation == .inactiveOutline {
            rect = Self.inactiveCaretRect(rect, mode: mode)
            color.withAlphaComponent(0.75).setStroke()
            let outline = rect.insetBy(dx: 0.5, dy: 0.5)
            NSBezierPath(rect: outline).stroke()
            return
        }

        if mode == UInt32(VIEM_MODE_REPLACE) {
            color.setFill()
            NSRect(x: rect.minX, y: rect.maxY - 2, width: rect.width, height: 2).fill()
            return
        }

        guard mode == UInt32(VIEM_MODE_NORMAL)
                || mode == UInt32(VIEM_MODE_VISUAL_CHARACTER)
                || mode == UInt32(VIEM_MODE_VISUAL_LINE)
                || mode == UInt32(VIEM_MODE_VISUAL_BLOCK)
        else { return }

        if let cluster = geometry.cluster,
           cluster.flags & UInt32(VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN) != 0,
           surface.session?.provider.renderRegistry.isColorGlyph(
               identifier: cluster.render_run.identifier,
               metricsGeneration: cluster.render_run.metrics_generation
           ) == true
        {
            color.withAlphaComponent(0.28).setFill()
            rect.fill()
            color.setStroke()
            NSBezierPath(rect: rect.insetBy(dx: 0.5, dy: 0.5)).stroke()
            return
        }

        color.setFill()
        rect.fill()
        // Ink can cross a caret cell, especially with italics, kerning, and
        // tightly spaced rows. The fill covers that ink too, so redraw every
        // intersecting fragment, not just the character associated with the
        // caret. Keep the logical cell unchanged and allow one device pixel
        // beyond outline bounds when deciding which antialiased ink to draw.
        let glyphColor = EVCaretAppearanceResolver.glyphColor(contrastingWith: color)
        for cluster in caretRedrawClusters(in: rect, snapshot: snapshot) {
            guard let row = row(for: cluster.row_index, in: snapshot.rows) else { continue }
            let baseline = viewPoint(
                fromLayoutPoint: CGPoint(x: CGFloat(cluster.x), y: CGFloat(row.baseline))
            )
            _ = surface.session?.provider.renderRegistry.draw(
                identifier: cluster.render_run.identifier,
                metricsGeneration: cluster.render_run.metrics_generation,
                atBaseline: baseline,
                color: glyphColor,
                in: context,
                clip: rect
            )
        }
    }

    func caretRedrawClusters(in rect: NSRect, snapshot: EVLayoutExport) -> [ViemPositionedClusterV1] {
        let fringe = 1 / max(window?.backingScaleFactor ?? 1, 1)
        return snapshot.clusters.filter {
            $0.flags & UInt32(VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN) != 0
                && viewRect($0.ink_bounds).insetBy(dx: -fringe, dy: -fringe).intersects(rect)
        }
    }

    /// Geometry for whatever the core says the caret occupies.
    ///
    /// A cell resolves to the shaping cluster containing its first byte, so an
    /// indivisible ligature is covered whole. Everything else is an insertion
    /// boundary, where affinity picks the row. This deliberately does not look
    /// at the mode or re-derive the choice: `EVCaretTarget` already carries it.
    func caretItemGeometry(_ snapshot: EVLayoutExport) -> (
        rect: NSRect?, cluster: ViemPositionedClusterV1?
    ) {
        let caret = presentationCaretTarget
        if case let .cell(range) = caret {
            let start = range.lowerBound
            if let cluster = snapshot.clusters.first(where: { cluster in
                cluster.text_start <= start && start < cluster.text_end
            }) {
                return (clusterRect(cluster), cluster)
            }
        }
        return (
            caretRect(offset: caret.offset, affinity: caret.affinity, snapshot: snapshot), nil
        )
    }

    /// The caret the core published, except while an input method owns the
    /// caret: a composition overlay is not part of the core presentation, and
    /// its selected end is an insertion boundary inside the marked text.
    var presentationCaretTarget: EVCaretTarget {
        guard let surface else {
            return .boundary(offset: 0, affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM))
        }
        if let overlay = surface.compositionOverlay {
            return .boundary(
                offset: overlay.info.selected_end,
                affinity: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
            )
        }
        return EVCaretTarget(surface.viewPresentation)
    }

    private var presentationCaretUTF8Offset: UInt64 { presentationCaretTarget.offset }

    private var presentationCaretAffinity: UInt32 { presentationCaretTarget.affinity }

    private func caretRect(
        offset: UInt64,
        affinity: UInt32? = nil,
        snapshot: EVLayoutExport
    ) -> NSRect? {
        guard let surface else { return nil }
        guard let session = surface.session else { return nil }
        let requested = affinity ?? surface.viewPresentation.cursor_affinity
        let alternate = requested == UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM)
            ? UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
            : UInt32(VIEM_BOUNDARY_AFFINITY_UPSTREAM)
        do {
            let geometry = try? session.caretGeometry(
                offset: offset,
                affinity: requested,
                in: snapshot.info
            )
            let resolved = try geometry ?? session.caretGeometry(
                offset: offset,
                affinity: alternate,
                in: snapshot.info
            )
            return viewRect(resolved.rect)
        } catch {
            return nil
        }
    }

    private func updateInsertionIndicator() {
        guard let surface else {
            insertionIndicator.displayMode = .hidden
            insertionIndicator.isHidden = true
            return
        }
        guard let snapshot = surface.layoutSnapshot else {
            insertionIndicator.displayMode = .hidden
            insertionIndicator.isHidden = true
            return
        }
        let mode = surface.viewPresentation.mode
        let active = isCaretActive
        guard mode == UInt32(VIEM_MODE_INSERT), active,
              var rect = caretRect(
                  offset: presentationCaretUTF8Offset,
                  affinity: presentationCaretAffinity,
                  snapshot: snapshot
              )
        else {
            insertionIndicator.displayMode = .hidden
            insertionIndicator.isHidden = true
            return
        }
        rect.size.width = 2
        insertionIndicator.frame = rect.integral
        insertionIndicator.color = EVCaretAppearanceResolver.shared.color(for: self)
        insertionIndicator.isHidden = false
        insertionIndicator.displayMode = .automatic
    }

    func commandLineRenderState() -> EVCommandLineRenderState? {
        guard let surface else { return nil }
        guard let commandLine = surface.commandLine,
              let prompt = commandLine.prompt,
              commandLine.info.identity.kind != UInt32(VIEM_COMMAND_LINE_KIND_NONE),
              let coreCursor = Int(exactly: commandLine.info.cursor_utf8_offset),
              coreCursor >= 0,
              coreCursor <= commandLine.text.utf8.count
        else { return nil }

        let promptText = String(prompt)
        var text = commandLine.text
        var cursor = coreCursor
        var markedDisplayRange: NSRange?
        var selectedDisplayRange = utf16Range(forUTF8: commandLine.selectedUTF8Range, in: commandLine.text).map {
            NSRange(location: $0.location + promptText.utf16.count, length: $0.length)
        }
        if case let .commandLine(target)? = markedTextTarget,
           commandLineTarget(target, matches: commandLine),
           let projected = replacingUTF8(
               target.replacementUTF8,
               in: target.coreText,
               with: markedTextValue
           ),
           let base = utf16Range(
               forUTF8: target.replacementUTF8.lowerBound ..< target.replacementUTF8.lowerBound,
               in: target.coreText
           ),
           let markedLocation = addingWithoutOverflow(promptText.utf16.count, base.location),
           let marked = makeRange(location: markedLocation, length: markedTextValue.utf16.count),
           let selectedLocation = addingWithoutOverflow(markedLocation, markedSelection.location),
           let selected = makeRange(location: selectedLocation, length: markedSelection.length),
           let selectedEnd = addingWithoutOverflow(markedSelection.location, markedSelection.length),
           let selectionUTF8 = utf8Range(
               inMarkedText: markedTextValue,
               utf16Range: NSRange(location: 0, length: selectedEnd)
           )
        {
            text = projected
            cursor = target.replacementUTF8.lowerBound + selectionUTF8.upperBound
            markedDisplayRange = marked
            selectedDisplayRange = selected
        }
        guard cursor >= 0, cursor <= text.utf8.count,
              String(bytes: Array(text.utf8.prefix(cursor)), encoding: .utf8) != nil
        else { return nil }
        return EVCommandLineRenderState(
            prompt: promptText,
            text: text,
            displayText: promptText + text,
            markedDisplayRange: markedDisplayRange,
            selectedDisplayRange: selectedDisplayRange,
            cursorUTF8Offset: cursor
        )
    }



    private func clusterRect(_ cluster: ViemPositionedClusterV1) -> NSRect {
        let bounds = cluster.typographic_bounds
        return viewRect(
            x: CGFloat(bounds.x),
            y: CGFloat(bounds.y),
            width: max(CGFloat(bounds.width), CGFloat(cluster.advance)),
            height: CGFloat(bounds.height)
        )
    }

    var viewportOrigin: CGPoint {
        guard let surface else { return .zero }
        return CGPoint(x: CGFloat(surface.viewportState.left), y: CGFloat(surface.viewportState.top))
    }

    func layoutPoint(fromViewPoint point: CGPoint) -> CGPoint {
        CGPoint(
            x: point.x - Self.canvasInsets.left + viewportOrigin.x,
            y: point.y - Self.canvasInsets.top + viewportOrigin.y
        )
    }

    func viewPoint(fromLayoutPoint point: CGPoint) -> CGPoint {
        CGPoint(
            x: point.x + Self.canvasInsets.left - viewportOrigin.x,
            y: point.y + Self.canvasInsets.top - viewportOrigin.y
        )
    }

    func viewRect(_ rect: ViemLayoutRectV1) -> NSRect {
        viewRect(
            x: CGFloat(rect.x),
            y: CGFloat(rect.y),
            width: CGFloat(rect.width),
            height: CGFloat(rect.height)
        )
    }

    private func viewRect(x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat) -> NSRect {
        let origin = viewPoint(
            fromLayoutPoint: CGPoint(x: x, y: y)
        )
        return NSRect(
            origin: origin,
            size: NSSize(width: width, height: height)
        )
    }

    private func row(for index: UInt64, in rows: [ViemVisualRowV1]) -> ViemVisualRowV1? {
        guard index < UInt64(rows.count) else { return nil }
        return rows[Int(index)]
    }

    private func drawFallback(_ cluster: ViemPositionedClusterV1, row: ViemVisualRowV1, color: NSColor) {
        guard let surface else { return }
        guard let start = Int(exactly: cluster.text_start),
              let end = Int(exactly: cluster.text_end),
              start < end,
              let value = surface.layoutText(in: start ..< end)
        else { return }
        value.draw(
            at: viewPoint(
                fromLayoutPoint: NSPoint(
                    x: CGFloat(cluster.x),
                    y: CGFloat(row.baseline - row.ascent)
                )
            ),
            withAttributes: [
                .font: Self.commandLineFont,
                .foregroundColor: color,
            ]
        )
    }


    private func minimumCaretWidth(
        near associatedCluster: ViemPositionedClusterV1?,
        in snapshot: EVLayoutExport
    ) -> CGFloat {
        guard let surface else {
            return ceil(Self.commandLineFont.pointSize / 2)
        }
        let cursor = presentationCaretUTF8Offset
        let affinity = presentationCaretAffinity
        let rowIndex = snapshot.carets.first {
            $0.text_offset == cursor
                && $0.affinity == affinity
        }?.row_index
        let nearbyCluster = associatedCluster ?? snapshot.clusters
            .filter { rowIndex == nil || $0.row_index == rowIndex }
            .min { left, right in
                let leftDistance = min(
                    offsetDistance(left.text_start, cursor),
                    offsetDistance(left.text_end, cursor)
                )
                let rightDistance = min(
                    offsetDistance(right.text_start, cursor),
                    offsetDistance(right.text_end, cursor)
                )
                return leftDistance < rightDistance
            }
        if let cluster = nearbyCluster,
           cluster.flags & UInt32(VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN) != 0,
           let width = surface.session?.provider.renderRegistry.enAdvance(
               identifier: cluster.render_run.identifier,
               metricsGeneration: cluster.render_run.metrics_generation
           )
        {
            return ceil(max(width, 1))
        }
        if let width = try? surface.session?.currentFontEnWidth() {
            return ceil(max(width * CGFloat(surface.zoomScale), 1))
        }
        return ceil(Self.commandLineFont.pointSize / 2)
    }

    private func offsetDistance(_ left: UInt64, _ right: UInt64) -> UInt64 {
        left >= right ? left - right : right - left
    }

    private func resolvedColor(_ dynamicColor: NSColor) -> NSColor {
        var result = dynamicColor
        effectiveAppearance.performAsCurrentDrawingAppearance {
            result = dynamicColor.usingColorSpace(.deviceRGB) ?? dynamicColor
        }
        return result
    }

    // MARK: - UTF-8 / UTF-16 conversion

    private var presentedUTF8Length: Int {
        guard let surface else { return 0 }
        return surface.compositionOverlay?.utf8Length ?? surface.formattedUTF8Length
    }

    private var presentedUTF16Length: Int {
        guard let surface else { return 0 }
        guard case let .document(target)? = markedTextTarget,
              surface.compositionOverlay != nil,
              let replacement = utf16Range(forUTF8: target.replacementUTF8),
              let withoutReplacement = subtractingWithoutOverflow(
                  surface.formattedUTF16Length,
                  replacement.length
              ),
              let result = addingWithoutOverflow(withoutReplacement, markedTextValue.utf16.count)
        else { return surface.formattedUTF16Length }
        return result
    }

    private func presentedUTF8Range(forUTF16 range: NSRange) -> Range<Int>? {
        guard let end = validatedUpperBound(for: range, limit: presentedUTF16Length),
              let start8 = textInputUTF8Offset(forUTF16Location: range.location),
              let end8 = textInputUTF8Offset(forUTF16Location: end),
              start8 <= end8
        else { return nil }
        return start8 ..< end8
    }

    private func presentedUTF16Range(forUTF8 range: Range<Int>) -> NSRange? {
        guard range.lowerBound >= 0,
              range.lowerBound <= range.upperBound,
              range.upperBound <= presentedUTF8Length,
              let start16 = textInputUTF16Offset(forLayoutUTF8Offset: range.lowerBound),
              let end16 = textInputUTF16Offset(forLayoutUTF8Offset: range.upperBound),
              start16 <= end16
        else { return nil }
        return makeRange(location: start16, length: end16 - start16)
    }

    private func utf16Range(forUTF8 range: Range<Int>) -> NSRange? {
        guard let surface else { return nil }
        guard range.lowerBound >= 0,
              range.lowerBound <= range.upperBound,
              range.upperBound <= surface.formattedUTF8Length,
              let mapped = surface.utf16Offsets(forUTF8: [range.lowerBound, range.upperBound]),
              mapped.count == 2
        else { return nil }
        return makeRange(location: mapped[0], length: mapped[1] - mapped[0])
    }

    private func utf16Range(forUTF8 range: Range<Int>, in text: String) -> NSRange? {
        guard range.lowerBound >= 0,
              range.upperBound >= range.lowerBound,
              range.upperBound <= text.utf8.count
        else { return nil }
        guard let start = stringIndex(utf8Offset: range.lowerBound, in: text),
              let end = stringIndex(utf8Offset: range.upperBound, in: text),
              let start16 = start.samePosition(in: text.utf16),
              let end16 = end.samePosition(in: text.utf16)
        else { return nil }
        let location = text.utf16.distance(from: text.utf16.startIndex, to: start16)
        let finish = text.utf16.distance(from: text.utf16.startIndex, to: end16)
        return makeRange(location: location, length: finish - location)
    }

    private func utf8Range(
        forUTF16 range: NSRange,
        requireGraphemeBoundaries: Bool = false
    ) -> Range<Int>? {
        guard let surface else { return nil }
        guard let upperBound = validatedUpperBound(
            for: range,
            limit: surface.formattedUTF16Length
        ),
        let mapped = surface.utf8Offsets(forUTF16: [range.location, upperBound]),
        mapped.count == 2,
        mapped[0] <= mapped[1]
        else { return nil }
        if requireGraphemeBoundaries,
           (surface.formattedPointInfo(atUTF8Offset: mapped[0]) == nil
               || surface.formattedPointInfo(atUTF8Offset: mapped[1]) == nil)
        {
            return nil
        }
        return mapped[0] ..< mapped[1]
    }

    private func utf8Range(
        forUTF16 range: NSRange,
        in text: String,
        requireGraphemeBoundaries: Bool = false
    ) -> Range<Int>? {
        guard let upperBound = validatedUpperBound(
            for: range,
            limit: text.utf16.count
        )
        else { return nil }
        let start16 = text.utf16.index(text.utf16.startIndex, offsetBy: range.location)
        let end16 = text.utf16.index(text.utf16.startIndex, offsetBy: upperBound)
        guard let start = String.Index(start16, within: text),
              let end = String.Index(end16, within: text),
              let start8 = start.samePosition(in: text.utf8),
              let end8 = end.samePosition(in: text.utf8)
        else { return nil }
        if requireGraphemeBoundaries,
           (!isGraphemeBoundary(start, in: text) || !isGraphemeBoundary(end, in: text))
        {
            return nil
        }
        return text.utf8.distance(from: text.utf8.startIndex, to: start8)
            ..< text.utf8.distance(from: text.utf8.startIndex, to: end8)
    }

    private func utf8Range(inMarkedText text: String, utf16Range: NSRange) -> Range<Int>? {
        guard let upperBound = validatedUpperBound(
            for: utf16Range,
            limit: text.utf16.count
        ) else { return nil }
        let start16 = text.utf16.index(text.utf16.startIndex, offsetBy: utf16Range.location)
        let end16 = text.utf16.index(text.utf16.startIndex, offsetBy: upperBound)
        guard let start = String.Index(start16, within: text)?.samePosition(in: text.utf8),
              let end = String.Index(end16, within: text)?.samePosition(in: text.utf8)
        else { return nil }
        return text.utf8.distance(from: text.utf8.startIndex, to: start)
            ..< text.utf8.distance(from: text.utf8.startIndex, to: end)
    }

    private func validatedUpperBound(for range: NSRange, limit: Int) -> Int? {
        guard range.location != NSNotFound,
              range.location >= 0,
              range.length >= 0,
              range.location <= limit,
              let upperBound = addingWithoutOverflow(range.location, range.length),
              upperBound <= limit
        else { return nil }
        return upperBound
    }

    private func stringIndex(utf8Offset: Int, in text: String) -> String.Index? {
        guard utf8Offset >= 0, utf8Offset <= text.utf8.count else { return nil }
        let index = text.utf8.index(text.utf8.startIndex, offsetBy: utf8Offset)
        return String.Index(index, within: text)
    }

    private func isGraphemeBoundary(_ index: String.Index, in text: String) -> Bool {
        index == text.endIndex || text.indices.contains(index)
    }
}
