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
                          surface: any EVEditorSurface, aligned: Bool, editable: Bool) throws -> any EVDiffSessionControlling {
        guard let base = base as? EVCoreDocumentBackend,
              let baseSurface = baseSurface as? EVEditorSurfaceController,
              let surface = surface as? EVEditorSurfaceController,
              surface.backend === self else {
            throw EVCoreFrontendError.core(operation: "Start diff mode", status: UInt32(VIEM_STATUS_INVALID_ARGUMENT))
        }
        let session = try EVCoreDiffSession(document: self, base: base, baseSurface: baseSurface,
                                            surface: surface, aligned: aligned, editable: editable)
        diffSessions.removeAll { $0.value == nil }
        diffSessions.append(WeakDiffSession(session))
        return session
    }
}

/// One comparison between a base buffer and a live document. It shows the
/// document in Code, keeps the core diff current after every source change,
/// and mirrors vertical scrolling while the sides are aligned.
@MainActor
final class EVCoreDiffSession: EVDiffSessionControlling {
    private weak var document: EVCoreDocumentBackend?
    /// The base buffer lives exactly as long as the comparison.
    private var base: EVCoreDocumentBackend?
    private weak var baseSurface: EVEditorSurfaceController?
    private weak var surface: EVEditorSurfaceController?
    private var handle: ViemDiffSessionHandle = 0
    private var aligned: Bool
    private var restoreMode: EVDocumentModeChoice?
    private var restoreModifiable = false
    private var mirroring = false
    private(set) var lastSync: UInt32 = UInt32(VIEM_DIFF_SYNC_UNCHANGED)
    /// The layout and extent each side's scrollbar marks were measured with.
    private var markKeys: [ObjectIdentifier: (layout: ViemLayoutSnapshotIdentityV1, maximum: CGFloat)] = [:]

    init(document: EVCoreDocumentBackend, base: EVCoreDocumentBackend, baseSurface: EVEditorSurfaceController,
         surface: EVEditorSurfaceController, aligned: Bool, editable: Bool) throws {
        self.document = document
        self.base = base
        self.baseSurface = baseSurface
        self.surface = surface
        self.aligned = aligned
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
        try checkedDiff(viem_diff_session_create(base.core, document.core,
                                                 aligned ? UInt32(VIEM_DIFF_SESSION_ALIGNED) : 0, &handle),
                        operation: "Compare with the diff base")
        self.handle = handle
        surface.diffViewportTopDidChange = { [weak self] _ in self?.mirror(fromNew: true) }
        baseSurface.diffViewportTopDidChange = { [weak self] _ in self?.mirror(fromNew: false) }
        surface.diffPresentationDidRefresh = { [weak self] in self?.updateMarks(isNew: true) }
        baseSurface.diffPresentationDidRefresh = { [weak self] in self?.updateMarks(isNew: false) }
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

    /// Update the diff for the document's current revision and redraw.
    func sync(force: Bool = false) {
        guard handle != 0, let document, let base, let surface, let baseSurface,
              let newView = surface.session?.viewID, let baseView = baseSurface.session?.viewID else { return }
        var result = UInt32(VIEM_DIFF_SYNC_UNCHANGED)
        let status = viem_diff_session_sync(handle, base.core, baseView, document.core, newView, &result)
        guard status == UInt32(VIEM_STATUS_OK) else {
            surface.report(EVCoreFrontendError.core(operation: "Update the diff", status: status))
            return
        }
        lastSync = result
        baseSurface.refreshPresentation()
        if force { surface.refreshPresentation() }
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
        // The core placed both sides; mirroring would re-derive the base
        // from the new side's top line instead.
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
        markKeys[key] = (layout, maximum)
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
    }

    func end() {
        guard handle != 0 else { return }
        surface?.diffViewportTopDidChange = nil
        baseSurface?.diffViewportTopDidChange = nil
        surface?.diffPresentationDidRefresh = nil
        baseSurface?.diffPresentationDidRefresh = nil
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

    /// Aligned sides scroll together: the other side shows the line that
    /// corresponds to this side's top line, at the same fraction of it.
    private func mirror(fromNew: Bool) {
        guard aligned, !mirroring, handle != 0, let document, let base, let surface, let baseSurface,
              let newView = surface.session?.viewID, let baseView = baseSurface.session?.viewID else { return }
        let (fromCore, fromView, toCore, toView, other) = fromNew
            ? (document.core, newView, base.core, baseView, baseSurface)
            : (base.core, baseView, document.core, newView, surface)
        var top: Float = 0
        guard viem_diff_session_corresponding_top(handle, fromCore, fromView, fromNew ? 1 : 0,
                                                  toCore, toView, &top) == UInt32(VIEM_STATUS_OK),
              let session = other.session, other.viewportState.top != top else { return }
        mirroring = true
        defer { mirroring = false }
        do {
            _ = try session.setViewportOrigin(left: CGFloat(other.viewportState.left), top: CGFloat(top))
            other.refreshPresentation()
        } catch {
            other.report(error)
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
