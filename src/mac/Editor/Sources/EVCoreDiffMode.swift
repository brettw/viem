import AppKit
import CViemCore
import ViemAppShell

// Diff mode's editor side (docs/diff_mode.md): the read-only base buffer and
// the core comparison session between it and the live document.

final class WeakDiffSession {
    weak var value: EVCoreDiffSession?
    init(_ value: EVCoreDiffSession) { self.value = value }
}

private func checkedDiff(_ status: UInt32, operation: String) throws {
    guard status == UInt32(VIEM_STATUS_OK) else {
        throw EVCoreFrontendError.core(operation: operation, status: status)
    }
}

@MainActor
extension EVCoreDocumentBackend: EVDiffBackend {
    public func diffSavedSource() -> Data? {
        guard core != 0 else { return nil }
        var required: UInt64 = 0
        var available: UInt32 = 0
        let status = viem_core_copy_saved_source(core, nil, 0, &required, &available)
        guard available != 0, status == UInt32(VIEM_STATUS_OK) || status == UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) else {
            return nil
        }
        var bytes = Data(count: Int(required))
        let copied = bytes.withUnsafeMutableBytes { buffer in
            viem_core_copy_saved_source(core, buffer.bindMemory(to: UInt8.self).baseAddress,
                                        UInt64(buffer.count), &required, &available)
        }
        return copied == UInt32(VIEM_STATUS_OK) && available != 0 ? bytes : nil
    }

    public func makeDiffBase(source: Data, reusing previous: (any EVDocumentBackend)?) throws -> any EVDocumentBackend {
        // Refilling the previous base replaces its core under the surfaces it
        // already has; its comparison must have ended first.
        let base = (previous as? EVCoreDocumentBackend) ?? EVCoreDocumentBackend(configuration: configuration)
        // The document's filename selects the same language as the document;
        // the base itself is never bound to that file.
        try base.read(source: source, typeName: EVDocument.codeType,
                      origin: openingFilename.isEmpty ? .untitled : .file(path: openingFilename, automaticCode: true))
        try base.setReadOnly(true)
        try checkedDiff(viem_core_set_modifiable(base.core, 0), operation: "Protect the diff base")
        return base
    }

    public func beginDiff(base: any EVDocumentBackend, baseSurface: any EVEditorSurface,
                          surface: any EVEditorSurface, aligned: Bool, collapsed: Bool,
                          editable: Bool) throws -> any EVDiffSessionControlling {
        guard let base = base as? EVCoreDocumentBackend,
              let baseSurface = baseSurface as? EVEditorSurfaceController,
              let surface = surface as? EVEditorSurfaceController,
              surface.backend === self else {
            throw EVCoreFrontendError.core(operation: "Start diff mode", status: UInt32(VIEM_STATUS_INVALID_ARGUMENT))
        }
        let session = try EVCoreDiffSession(document: self, base: base, baseSurface: baseSurface, surface: surface,
                                            aligned: aligned, collapsed: collapsed, editable: editable)
        diffSessions.removeAll { $0.value == nil }
        diffSessions.append(WeakDiffSession(session))
        return session
    }
}

/// One comparison between a base buffer and a live document. It shows the
/// document in Code, keeps the core diff current after every source change
/// and every collapsed run a caret or click opens, and mirrors vertical
/// scrolling while the sides are aligned.
@MainActor
final class EVCoreDiffSession: EVDiffSessionControlling {
    private weak var document: EVCoreDocumentBackend?
    /// The base buffer lives exactly as long as the comparison.
    private var base: EVCoreDocumentBackend?
    private weak var baseSurface: EVEditorSurfaceController?
    private weak var surface: EVEditorSurfaceController?
    private var handle: ViemDiffSessionHandle = 0
    private var aligned: Bool
    private var collapsed: Bool
    private var restoreMode: EVDocumentModeChoice?
    private var restoreModifiable = false
    private var mirroring = false
    private(set) var lastSync: UInt32 = UInt32(VIEM_DIFF_SYNC_UNCHANGED)
    private(set) var lineTotals: (added: UInt64, removed: UInt64) = (0, 0)
    var lineTotalsDidChange: (() -> Void)?
    /// The layout and extent each side's scrollbar marks were measured with.
    private var markKeys: [ObjectIdentifier: (layout: ViemLayoutSnapshotIdentityV1, maximum: CGFloat)] = [:]

    init(document: EVCoreDocumentBackend, base: EVCoreDocumentBackend, baseSurface: EVEditorSurfaceController,
         surface: EVEditorSurfaceController, aligned: Bool, collapsed: Bool, editable: Bool) throws {
        self.document = document
        self.base = base
        self.baseSurface = baseSurface
        self.surface = surface
        self.aligned = aligned
        self.collapsed = collapsed
        // The live side is compared and edited as Code. Turning Preview off
        // (or Text into Code) reprojects the same source without converting it.
        if let mode = surface.currentDocumentMode(), mode.format != "code" {
            restoreMode = mode.format == "markdown" ? .markdown : .plainText
            surface.selectDocumentMode(mode.format == "markdown" ? .code("markdown") : .codeFormat, expected: mode)
        }
        if !editable {
            try checkedDiff(viem_core_set_modifiable(document.core, 0), operation: "Protect a deleted file")
            restoreModifiable = true
        }
        var handle: ViemDiffSessionHandle = 0
        let flags = (aligned ? UInt32(VIEM_DIFF_SESSION_ALIGNED) : 0) | (collapsed ? UInt32(VIEM_DIFF_SESSION_COLLAPSED) : 0)
        try checkedDiff(viem_diff_session_create(base.core, document.core, flags, &handle),
                        operation: "Compare with the diff base")
        self.handle = handle
        surface.liveDiffSession = self
        surface.diffPresentationDidRefresh = { [weak self] scrolled, afterLiveEdit in
            self?.didRefresh(isNew: true, scrolled: scrolled, afterLiveEdit: afterLiveEdit)
        }
        baseSurface.diffPresentationDidRefresh = { [weak self] scrolled, afterLiveEdit in
            self?.didRefresh(isNew: false, scrolled: scrolled, afterLiveEdit: afterLiveEdit)
        }
        surface.diffOpenCollapsedContext = { [weak self] line in self?.openCollapsedContext(isNew: true, line: line) ?? false }
        baseSurface.diffOpenCollapsedContext = { [weak self] line in
            self?.openCollapsedContext(isNew: false, line: line) ?? false
        }
        // Both sides are Code and share its remembered wrapping; a change on
        // either side applies to the other.
        let wrap = document.configuration.codeWordWrap
        surface.setWordWrap(wrap)
        baseSurface.setWordWrap(wrap)
        surface.codeWordWrapDidChange = { [weak baseSurface] wrap in baseSurface?.setWordWrap(wrap) }
        baseSurface.codeWordWrapDidChange = { [weak surface] wrap in surface?.setWordWrap(wrap) }
        sync(force: true)
        mirror(fromNew: true)
    }

    deinit {
        let handle = handle
        if handle != 0 { _ = viem_diff_session_destroy(handle) }
    }

    func setAligned(_ aligned: Bool) {
        guard aligned != self.aligned, handle != 0 else { return }
        self.aligned = aligned
        _ = viem_diff_session_set_aligned(handle, aligned ? 1 : 0)
        sync(force: true)
        if aligned { mirror(fromNew: true) }
    }

    func setCollapsed(_ collapsed: Bool) {
        guard collapsed != self.collapsed, handle != 0 else { return }
        self.collapsed = collapsed
        _ = viem_diff_session_set_collapsed(handle, collapsed ? 1 : 0)
        sync(force: true)
        mirror(fromNew: true)
    }

    /// Open the collapsed run whose divider, drawn at `line` of one side, was
    /// clicked, on both sides. False when no run borders that line.
    func openCollapsedContext(isNew: Bool, line: UInt64) -> Bool {
        guard handle != 0 else { return false }
        var opened: UInt32 = 0
        guard viem_diff_session_open_collapsed(handle, isNew ? 1 : 0, line, &opened) == UInt32(VIEM_STATUS_OK),
              opened != 0 else { return false }
        sync(force: true)
        mirror(fromNew: isNew)
        return true
    }

    private func didRefresh(isNew: Bool, scrolled: Bool, afterLiveEdit: Bool) {
        mirror(fromNew: afterLiveEdit || (scrolled ? isNew : !isNew))
        updateMarks(isNew: isNew)
        // A caret that entered collapsed context opened it in this view.
        // Sync so the other side opens it in the same turn.
        guard handle != 0, let side = isNew ? surface : baseSurface,
              let core = isNew ? document?.core : base?.core, let view = side.session?.viewID else { return }
        var opened: UInt32 = 0
        guard viem_diff_view_opened_context(core, view, &opened) == UInt32(VIEM_STATUS_OK), opened != 0 else { return }
        let wasMirroring = mirroring
        mirroring = true
        sync()
        if !isNew { surface?.refreshPresentation() }
        mirroring = wasMirroring
        mirror(fromNew: afterLiveEdit || isNew)
    }

    /// Update the diff for the document's current revision and redraw.
    func sync(force: Bool = false, afterLiveEdit: Bool = false) {
        guard handle != 0, let document, let base, let surface, let baseSurface,
              let newView = surface.session?.viewID, let baseView = baseSurface.session?.viewID else { return }
        var result = UInt32(VIEM_DIFF_SYNC_UNCHANGED)
        let status = viem_diff_session_sync(handle, base.core, baseView, document.core, newView, &result)
        guard status == UInt32(VIEM_STATUS_OK) else {
            surface.report(EVCoreFrontendError.core(operation: "Update the diff", status: status))
            return
        }
        lastSync = result
        var info = ViemDiffSessionInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemDiffSessionInfoV1>.size)
        if viem_diff_session_info(handle, &info) == UInt32(VIEM_STATUS_OK), lineTotals != (info.added, info.removed) {
            lineTotals = (info.added, info.removed)
            lineTotalsDidChange?()
        }
        // Unaligned insertions may add a scrollbar mark to an otherwise
        // unchanged base. Redraw its editor only when its own frame changed.
        if !aligned { markKeys.removeValue(forKey: ObjectIdentifier(baseSurface)) }
        if force || baseFrameChanged(baseSurface) {
            // Reflowing base filler during a live edit can move its text
            // anchor. Keep the live pane in charge through both refreshes.
            baseSurface.refreshPresentation(afterLiveDiffEdit: afterLiveEdit || result != UInt32(VIEM_DIFF_SYNC_UNCHANGED))
        } else {
            updateMarks(isNew: false)
        }
        if force { surface.refreshPresentation() }
    }

    private func baseFrameChanged(_ surface: EVEditorSurfaceController) -> Bool {
        guard let session = surface.session, let displayed = surface.layoutSnapshot?.info.identity,
              let layout = try? session.layoutSnapshotInfo(), displayed.isSameLayout(as: layout.identity),
              let current = try? session.viewportState() else { return true }
        let previous = surface.viewportState
        return previous.left != current.left || previous.top != current.top
            || previous.maximum_top != current.maximum_top || previous.maximum_left != current.maximum_left
            || previous.flags != current.flags
    }

    /// The installed live view now owns the reloaded core. Sync before its
    /// first presentation, keeping the base view and the same comparison.
    func liveViewDidReload() {
        restoreModifiable = false
        markKeys.removeAll(keepingCapacity: true)
        mirroring = true
        sync()
        mirroring = false
        mirror(fromNew: true)
    }

    /// Move both sides to the next or previous change, wrapping within the
    /// file; beeps when the sides are identical.
    func navigateChange(next: Bool) {
        guard handle != 0, let document, let base, let surface, let baseSurface,
              let newView = surface.session?.viewID, let baseView = baseSurface.session?.viewID else { return }
        var navigation = ViemDiffNavigationV1()
        navigation.struct_size = UInt32(MemoryLayout<ViemDiffNavigationV1>.size)
        let direction = next ? UInt32(VIEM_DIFF_NAVIGATE_NEXT) : UInt32(VIEM_DIFF_NAVIGATE_PREVIOUS)
        let status = viem_diff_session_navigate(handle, base.core, baseView, document.core, newView, direction, &navigation)
        guard status == UInt32(VIEM_STATUS_OK) else {
            surface.report(EVCoreFrontendError.core(operation: "Go to a change", status: status))
            return
        }
        guard navigation.flags & UInt32(VIEM_DIFF_NAVIGATION_FOUND) != 0 else {
            document.configuration.scriptOptions.beep()
            return
        }
        // The core has already positioned both sides.
        mirroring = true
        defer { mirroring = false }
        surface.refreshPresentation()
        baseSurface.refreshPresentation()
    }

    /// Recompute one side's scrollbar marks when its layout or extent moved.
    private func updateMarks(isNew: Bool) {
        guard handle != 0, let side = isNew ? surface : baseSurface, let core = isNew ? document?.core : base?.core,
              let view = side.session?.viewID, let layout = side.layoutSnapshot?.info.identity else { return }
        let scrollbars = side.editorView.documentScrollbars
        let maximum = scrollbars.verticalState.maximum
        let key = ObjectIdentifier(side)
        if let previous = markKeys[key], previous.layout.isSameLayout(as: layout), previous.maximum == maximum { return }
        var count: UInt64 = 0
        var extent: Float = 0
        var status = viem_diff_session_scrollbar_marks(handle, core, view, isNew ? 1 : 0, nil, 0, &count, &extent)
        guard status == UInt32(VIEM_STATUS_OK) || status == UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) else { return }
        var marks = [ViemDiffScrollbarMarkV1](repeating: ViemDiffScrollbarMarkV1(), count: Int(count))
        if count > 0 {
            status = marks.withUnsafeMutableBufferPointer { buffer in
                viem_diff_session_scrollbar_marks(handle, core, view, isNew ? 1 : 0, buffer.baseAddress, count, &count, &extent)
            }
            guard status == UInt32(VIEM_STATUS_OK) else { return }
        }
        var added = ViemRgbaV1()
        var removed = ViemRgbaV1()
        _ = viem_diff_line_colors(&added, &removed)
        func color(_ value: ViemRgbaV1) -> NSColor {
            NSColor(srgbRed: CGFloat(value.red), green: CGFloat(value.green), blue: CGFloat(value.blue), alpha: CGFloat(value.alpha))
        }
        let (addedColor, removedColor) = (color(added), color(removed))
        scrollbars.setChangeMarks(marks.map { mark in
            EVDocumentScrollbars.ChangeMark(top: CGFloat(mark.top), bottom: CGFloat(mark.bottom),
                color: mark.flags & UInt32(VIEM_DIFF_MARK_ADDED) != 0 ? addedColor : removedColor)
        }, extent: CGFloat(extent))
        markKeys[key] = (layout, maximum)
    }

    func end() {
        guard handle != 0 else { return }
        // Pairs in other panes may still compare this document. The format and
        // protection are buffer-wide, so the last comparison restores them.
        if let heir = document?.diffSessions.compactMap(\.value).first(where: { $0 !== self && $0.handle != 0 }) {
            heir.restoreMode = heir.restoreMode ?? restoreMode
            heir.restoreModifiable = heir.restoreModifiable || restoreModifiable
            restoreMode = nil
            restoreModifiable = false
        }
        surface?.liveDiffSession = nil
        surface?.diffPresentationDidRefresh = nil
        baseSurface?.diffPresentationDidRefresh = nil
        surface?.diffOpenCollapsedContext = nil
        baseSurface?.diffOpenCollapsedContext = nil
        surface?.codeWordWrapDidChange = nil
        baseSurface?.codeWordWrapDidChange = nil
        surface?.editorView.documentScrollbars.setChangeMarks([], extent: 0)
        markKeys = [:]
        if let document, let view = surface?.session?.viewID {
            _ = viem_diff_clear_view(document.core, view)
        }
        _ = viem_diff_session_destroy(handle)
        handle = 0
        if restoreModifiable, let document {
            _ = viem_core_set_modifiable(document.core, 1)
        }
        surface?.refreshPresentation()
        // The format is buffer-wide, so restore it through any open view of
        // the document, even when the compared pane has already closed.
        if let restoreMode, let modeSurface = surface ?? document?.liveEditorSurfaces.first,
           let session = modeSurface.session,
           let mode = modeSurface.currentDocumentMode(), mode.format == "code" {
            modeSurface.performInput { _ = try session.setDocumentMode(restoreMode, expected: mode) }
        }
        document?.diffSessions.removeAll { $0.value == nil || $0.value === self }
        base = nil
    }

    /// Reconcile both scrollbars after scrolling or layout refinement.
    private func mirror(fromNew: Bool) {
        guard aligned, !mirroring, handle != 0, let document, let base, let surface, let baseSurface,
              let newView = surface.session?.viewID, let baseView = baseSurface.session?.viewID else { return }
        mirroring = true
        defer { mirroring = false }
        // Refreshing a side can publish newly measured row heights.
        for _ in 0..<3 {
            var changed: UInt32 = 0
            let status = viem_diff_session_sync_scroll(handle, base.core, baseView, document.core, newView, fromNew ? 1 : 0, &changed)
            guard status == UInt32(VIEM_STATUS_OK) else {
                surface.report(EVCoreFrontendError.core(operation: "Align diff scroll positions", status: status))
                return
            }
            if changed == 0 { break }
            if changed & UInt32(VIEM_DIFF_SCROLL_BASE_CHANGED) != 0 { baseSurface.refreshPresentation() }
            if changed & UInt32(VIEM_DIFF_SCROLL_NEW_CHANGED) != 0 { surface.refreshPresentation() }
        }
    }
}

@MainActor
extension EVEditorSurfaceController: EVDiffLineNumberControlling {
    public var showsLineNumbers: Bool { (try? session?.number()) ?? false }

    public func setShowsLineNumbers(_ shows: Bool) {
        guard let session, showsLineNumbers != shows else { return }
        performInput { try session.setNumber(shows) }
    }
}
