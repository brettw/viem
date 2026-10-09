import AppKit
import CViemCore
import ViemAppShell
import Foundation

extension Notification.Name {
    static let viemEditorSelectionDidChange = Notification.Name("EVEditorSelectionDidChange")
}

@MainActor
public final class EVEditorSurfaceController: NSViewController, EVEditorSurface, EVDocumentHostAttachable {
    public var viewController: NSViewController { self }
    public var defaultColumnWidth: CGFloat? { try? session?.defaultColumnWidth() }
    public var windowFocusPoint: NSPoint? { editorView.windowFocusPoint }
    public var defaultLineHeight: CGFloat? { try? session?.defaultLineHeight() }
    public private(set) var statusBarState = EVStatusBarState()
    public var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
    lazy var formattingToolbar = EVFormattingToolbarView(surface: self)
    lazy var linkPopover = EVInlineContentPopoverController(surface: self, kind: .link)
    lazy var imagePopover = EVInlineContentPopoverController(surface: self, kind: .image)
    public weak var documentHostEffectHandler: (any EVDocumentHostEffectHandling)?

    let backend: EVCoreDocumentBackend
    private var sourcedLineRequests: [EVDocumentHostRequest]?
    private(set) var session: EVCoreViewSession?
    var makeCoreViewSession: @MainActor (EVCoreDocumentBackend, CGSize, EVViewMargins) throws -> EVCoreViewSession = { document, size, margins in
        try EVCoreViewSession(document: document, width: size.width, height: size.height, margins: margins)
    }
    private(set) var formattedSnapshot: EVFormattedSnapshot?
    private(set) var layoutTextSlices: [EVFormattedTextSlice] = []
    private(set) var compositionOverlay: EVCompositionOverlayExport?
    private(set) var compositionTextSlices: [EVCompositionTextSlice] = []
    private(set) var layoutSnapshot: EVLayoutExport?
    var layoutPaint: EVLayoutPaintExport?
    private(set) var commandLine: EVCommandLineExport?
    private(set) var substituteConfirmationPrompt: String?
    private(set) var completion: EVCompletionExport?
    private var completionTimer: Timer?
    private var searchTimer: Timer?
    private(set) var searchWorkPending = false
    private(set) var commandOutput: String?
    static let commandOutputDuration: TimeInterval = 30
    var commandOutputClock: () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }
    private(set) var commandOutputDeadline: TimeInterval?
    private var commandOutputTimer: Timer?
    private(set) var visualSelection: EVVisualSelectionExport?
    private(set) var viewPresentation = ViemViewPresentationV1()
    private(set) var viewportState = ViemViewportStateV1()
    private(set) var documentState = ViemDocumentStateV1()
    private(set) var presentationRefreshCount: UInt64 = 0
    private var isRefreshingPresentation = false
    /// Changes only when the immutable text drawing inputs or viewport change.
    /// Selection and caret updates have their own damage tracking in the view.
    private(set) var immutablePresentationGeneration: UInt64 = 0
    private var presentedWhitespaceCopyCount: UInt64 = 0
    private var imageViewportID: UInt64 = 0
    private var imageAdmissionRefreshPending = false
    // The offsets themselves remain scoped to viewPresentation's immutable
    // document revision. This associates that snapshot with its owning view.
    private var selectionPresentationViewID: ViemViewId?
    private var selectionCharacterContextGeneration: UInt64 = 0
    private var themeObserver: NSObjectProtocol?
    private let viewPreferences: EVViewPreferences
    private var viewPreferencesObserver: NSObjectProtocol?
    private var appliedMargins: EVViewMargins?
    private var lastErrorMessage = ""
    private var lastLayoutWarning = ""
    var pasteboard: any EVPasteboardAccess = EVAppKitPasteboardAccess.shared
    var findPasteboard: any EVPasteboardAccess = EVAppKitPasteboardAccess.find
    private var pasteMatchesStyle = false
    private var nativeCopyRepresentations: EVClipboardRepresentations?

    var editorView: EVEditorView { view as! EVEditorView }

    /// Compatibility/testing convenience. This is deliberately an explicit
    /// on-demand full read; presentation refresh and drawing never touch it.
    var formattedText: String {
        fullFormattedText() ?? ""
    }

    func fullFormattedText() -> String? {
        guard let snapshot = formattedSnapshot else { return nil }
        return try? backend.formattedText(
            in: 0 ..< snapshot.info.utf8_length,
            snapshot: snapshot
        )
    }

    /// Explicit on-demand read of the projection currently exposed to native
    /// text clients. Normal painting remains coverage-bounded; accessibility's
    /// full value query is the exceptional API that requests the whole value.
    func fullPresentedText() -> String? {
        guard let overlay = compositionOverlay,
              let length = overlay.utf8Length,
              let upper = UInt64(exactly: length),
              let session
        else { return fullFormattedText() }
        return try? session.compositionTextSlice(in: 0 ..< upper, overlay: overlay)
            .text(in: 0 ..< length)
    }

    var formattedUTF8Length: Int {
        formattedSnapshot?.utf8Length ?? 0
    }

    var formattedUTF16Length: Int {
        formattedSnapshot?.utf16Length ?? 0
    }

    init(backend: EVCoreDocumentBackend, viewPreferences: EVViewPreferences? = nil) {
        self.viewPreferences = viewPreferences ?? .shared
        self.backend = backend
        super.init(nibName: nil, bundle: nil)
        themeObserver = NotificationCenter.default.addObserver(forName: .viemThemeDidChange, object: EVThemeStore.shared, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.applyTheme() }
        }
        viewPreferencesObserver = NotificationCenter.default.addObserver(forName: .viemViewPreferencesDidChange, object: self.viewPreferences, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.applyViewMargins() }
        }
        do {
            try attachToCore()
        } catch {
            lastErrorMessage = error.localizedDescription
        }
    }

    deinit {
        commandOutputTimer?.invalidate()
        completionTimer?.invalidate()
        searchTimer?.invalidate()
        if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) }
        if let viewPreferencesObserver { NotificationCenter.default.removeObserver(viewPreferencesObserver) }
    }

    private func applyTheme() {
        if isViewLoaded { refreshPresentation() }
    }

    private func applyViewMargins() {
        let margins = viewPreferences.margins
        do {
            if appliedMargins != margins { try session?.setViewMargins(margins); appliedMargins = margins }
            if isViewLoaded { refreshPresentation() }
        } catch { report(error) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }

    public override func loadView() {
        EVStartupPerformance.mark("surface.load.begin")
        defer { EVStartupPerformance.mark("surface.load.end") }
        view = EVEditorView(surface: self)
        refreshPresentation()
        if !lastErrorMessage.isEmpty { publishHostMessage(lastErrorMessage) }
    }

    public override func viewDidLayout() {
        EVStartupPerformance.mark("surface.layout.begin")
        defer { EVStartupPerformance.mark("surface.layout.end") }
        super.viewDidLayout()
        guard view.bounds.width > 0, view.bounds.height > 0 else { return }
        do {
            let viewportSize = editorView.layoutViewportSize
            _ = try session?.resize(width: viewportSize.width, height: viewportSize.height)
            refreshPresentation()
        } catch {
            report(error)
        }
    }

    func attachToCore() throws {
        guard session == nil else { return }
        installPreparedSession(try prepareSession(for: backend))
    }

    func prepareReplacementSession(for replacement: EVCoreDocumentBackend) throws -> EVCoreViewSession {
        let prepared = try prepareSession(for: replacement)
        if let session {
            let viewport = try session.viewportState()
            _ = try prepared.setWrap(viewport.flags & UInt32(VIEM_VIEWPORT_STATE_WRAP) != 0)
            _ = try prepared.setScale(CGFloat(viewport.scale))
            if replacement.sourceFormat == backend.sourceFormat {
                try prepared.setLineMode(session.lineMode())
                if replacement.sourceFormat == .markdownSource {
                    try prepared.setParagraphFlow(session.paragraphFlow())
                }
            }
            try prepared.restorePosition(from: session)
        }
        return prepared
    }

    private func prepareSession(for document: EVCoreDocumentBackend) throws -> EVCoreViewSession {
        EVStartupPerformance.mark("surface.session.begin")
        defer { EVStartupPerformance.mark("surface.session.end") }
        let size = isViewLoaded ? view.bounds.size : NSSize(width: 920, height: 655)
        let viewportSize = isViewLoaded ? editorView.layoutViewportSize : EVEditorView.layoutViewportSize(for: size)
        // Initial padding belongs in the first layout request. Applying it
        // afterward discards measured heights and reshapes the opening viewport.
        return try makeCoreViewSession(document, viewportSize, viewPreferences.margins)
    }

    func installPreparedSession(_ attachedSession: EVCoreViewSession) {
        precondition(session == nil)
        attachedSession.adoptDocument(backend)
        attachedSession.compositionStateDidChange = { [weak self] isActive in
            guard !isActive, let self, self.isViewLoaded else { return }
            self.editorView.coreCompositionDidEnd()
        }
        attachedSession.commandTurnHost = self
        session = attachedSession
        appliedMargins = viewPreferences.margins
        lastErrorMessage = backend.configurationWarning ?? ""
        if isViewLoaded { refreshPresentation() }
        if isViewLoaded, let warning = backend.configurationWarning { publishHostMessage(warning) }
    }

    func detachFromCore() {
        linkPopover.close()
        imagePopover.close()
        lastLayoutWarning = ""
        stopSearchPolling()
        completionTimer?.invalidate()
        completionTimer = nil
        completion = nil
        if isViewLoaded { editorView.hideCompletionPopup() }
        session?.detach()
        session = nil
        selectionPresentationViewID = nil
        formattedSnapshot = nil
        layoutTextSlices = []
        compositionOverlay = nil
        compositionTextSlices = []
        layoutSnapshot = nil
        layoutPaint = nil
        commandLine = nil
        substituteConfirmationPrompt = nil
        visualSelection = nil
        viewportState = ViemViewportStateV1()
        documentState = ViemDocumentStateV1()
    }

    func refreshPresentation(advancingSearch: Bool = true) {
        guard let session, !isRefreshingPresentation else { return }
        isRefreshingPresentation = true
        EVStartupPerformance.mark("surface.refresh.begin")
        defer { isRefreshingPresentation = false; EVStartupPerformance.mark("surface.refresh.end") }
        do {
            session.provider.setImageDocumentURL(documentHostEffectHandler?.documentURL(for: self))
            session.provider.imagesDidChange = { [weak self] previous, destinations in
                // Cache publication and its metrics acknowledgement form one
                // main-thread operation; no frame sees a partially installed size.
                MainActor.assumeIsolated {
                    guard let self, let current = self.session else { return }
                    do { try current.imageResourcesChanged(previousMetricsGeneration: previous, destinations: destinations) }
                    catch { self.report(error) }
                    self.refreshPresentation(advancingSearch: false)
                }
            }
            if advancingSearch { _ = session.optionalPresentation("search highlights", fallback: false) { try session.pollSearch() } }
            try session.refreshLayoutIfNeeded()
            if session.optionalPresentation("syntax highlighting", fallback: false, {
                try backend.waitForSyntax(viewID: session.viewID)
            }) {
                // A syntax style may change metrics as well as paint. Capture
                // geometry and colors only after installing the same result.
                try session.refreshLayoutIfNeeded()
            }
            let nextDocumentState = try backend.documentState()
            let nextFormattedSnapshot = try backend.formattedSnapshot()
            let nextPresentation = try session.presentation()
            let nextViewport = try session.viewportState()
            let nextCompositionOverlay = try session.compositionOverlayExport()
            let nextCompletion = session.optionalPresentation("completion popup", fallback: nil as EVCompletionExport?) { try session.completionExport() }
            let nextSearchWorkPending = session.optionalPresentation("search highlights", fallback: false) { try session.searchWorkPending() }
            guard nextFormattedSnapshot.info.identity.document_id == nextDocumentState.document_id,
                  nextFormattedSnapshot.info.identity.document_revision == nextDocumentState.document_revision,
                  nextFormattedSnapshot.info.identity.document_id == nextPresentation.document_id,
                  nextFormattedSnapshot.info.identity.document_revision == nextPresentation.document_revision,
                  nextFormattedSnapshot.info.identity.document_id == nextViewport.document_id,
                  nextFormattedSnapshot.info.identity.document_revision == nextViewport.document_revision
            else {
                throw EVCoreFrontendError.core(
                    operation: "Match formatted snapshot",
                    status: UInt32(VIEM_STATUS_STALE_REVISION)
                )
            }
            if let nextCompositionOverlay {
                guard nextCompositionOverlay.info.identity.document_id == nextDocumentState.document_id,
                      nextCompositionOverlay.info.identity.document_revision
                        == nextDocumentState.document_revision,
                      nextCompositionOverlay.info.identity.view_id == session.viewID,
                      nextCompositionOverlay.info.replacement_end
                        <= nextFormattedSnapshot.info.utf8_length
                else {
                    throw EVCoreFrontendError.core(
                        operation: "Match composition projection",
                        status: UInt32(VIEM_STATUS_STALE_REVISION)
                    )
                }
            }

            let nextLayoutSnapshot: EVLayoutExport?
            do {
                nextLayoutSnapshot = try session.layoutExport()
            } catch EVCoreFrontendError.unavailableLayout {
                nextLayoutSnapshot = nil
            }
            let nextLayoutPaint: EVLayoutPaintExport?
            let nextLayoutTextSlices: [EVFormattedTextSlice]
            let nextCompositionTextSlices: [EVCompositionTextSlice]
            if let nextLayoutSnapshot {
                guard nextLayoutSnapshot.info.identity.document_id
                        == nextFormattedSnapshot.info.identity.document_id,
                      nextLayoutSnapshot.info.identity.document_revision
                        == nextFormattedSnapshot.info.identity.document_revision
                else {
                    throw EVCoreFrontendError.core(
                        operation: "Match formatted layout",
                        status: UInt32(VIEM_STATUS_STALE_REVISION)
                    )
                }
                let paint = try session.layoutPaintExport()
                guard paint.info.identity.isSameLayout(as: nextLayoutSnapshot.info.identity) else {
                    throw EVCoreFrontendError.core(
                        operation: "Match layout paint",
                        status: UInt32(VIEM_STATUS_STALE_REVISION)
                    )
                }
                nextLayoutPaint = paint
                let sameLayout = layoutSnapshot?.info.identity.isSameLayout(as: nextLayoutSnapshot.info.identity) == true
                let sameFormatted = formattedSnapshot?.info.identity.isSameSnapshot(as: nextFormattedSnapshot.info.identity) == true
                let sameComposition = compositionOverlay?.info.identity.isSameOverlay(
                    as: nextCompositionOverlay?.info.identity ?? ViemCompositionOverlayIdentityV1()) == true
                let reuseText = sameLayout && (nextCompositionOverlay != nil
                    ? sameComposition : sameFormatted && compositionOverlay == nil)
                let ranges = reuseText ? [] : layoutUTF8Ranges(
                    for: nextLayoutSnapshot,
                    formattedLength: nextCompositionOverlay?.info.utf8_length
                        ?? nextFormattedSnapshot.info.utf8_length
                )
                if let nextCompositionOverlay {
                    nextLayoutTextSlices = []
                    if reuseText {
                        nextCompositionTextSlices = compositionTextSlices
                    } else {
                        nextCompositionTextSlices = try ranges.map {
                            try session.compositionTextSlice(in: $0, overlay: nextCompositionOverlay)
                        }
                    }
                } else {
                    if reuseText {
                        nextLayoutTextSlices = layoutTextSlices
                    } else {
                        nextLayoutTextSlices = try ranges.map {
                            try backend.formattedSlice(in: $0, snapshot: nextFormattedSnapshot)
                        }
                    }
                    nextCompositionTextSlices = []
                }
            } else {
                nextLayoutPaint = nil
                nextLayoutTextSlices = []
                nextCompositionTextSlices = []
            }

            let nextCommandLine = try session.commandLineExport()
            let nextSubstituteConfirmation = try session.substituteConfirmationPrompt()
            guard nextCommandLine.info.identity.document_id
                    == nextFormattedSnapshot.info.identity.document_id,
                  nextCommandLine.info.identity.document_revision
                    == nextFormattedSnapshot.info.identity.document_revision
            else {
                throw EVCoreFrontendError.core(
                    operation: "Match command line",
                    status: UInt32(VIEM_STATUS_STALE_REVISION)
                )
            }
            let nextVisualSelection: EVVisualSelectionExport?
            do {
                nextVisualSelection = try session.visualSelectionExport()
            } catch EVCoreFrontendError.core(_, let status)
                where status == UInt32(VIEM_STATUS_OUTSIDE_LAYOUT_COVERAGE)
                    || status == UInt32(VIEM_STATUS_LAYOUT_UNAVAILABLE)
            {
                // Exact selection geometry cannot be approximated safely. It
                // will become available when core materializes its coverage.
                nextVisualSelection = nil
            }
            if let nextVisualSelection, let nextLayoutSnapshot {
                guard nextVisualSelection.info.identity.layout.isSameLayout(
                    as: nextLayoutSnapshot.info.identity
                ) else {
                    throw EVCoreFrontendError.core(
                        operation: "Match Visual selection",
                        status: UInt32(VIEM_STATUS_STALE_REVISION)
                    )
                }
            }

            let selectionChanged = selectionPresentationChanged(nextPresentation, viewID: session.viewID,
                characterContextGeneration: session.characterContextGeneration)
            let imageViewportChanged = selectionPresentationViewID != session.viewID
                || documentState.document_id != nextDocumentState.document_id
                || documentState.document_revision != nextDocumentState.document_revision
                || viewportState.left != nextViewport.left || viewportState.top != nextViewport.top
                || layoutSnapshot?.info.viewport_width != nextLayoutSnapshot?.info.viewport_width
                || layoutSnapshot?.info.viewport_height != nextLayoutSnapshot?.info.viewport_height
            if imageViewportChanged { imageViewportID += 1 }
            let immutableInputsChanged = layoutSnapshot?.info.identity.isSameLayout(
                as: nextLayoutSnapshot?.info.identity ?? ViemLayoutSnapshotIdentityV1()) != true
                || viewportState.left != nextViewport.left || viewportState.top != nextViewport.top
                || layoutSnapshot?.info.viewport_width != nextLayoutSnapshot?.info.viewport_width
                || layoutSnapshot?.info.viewport_height != nextLayoutSnapshot?.info.viewport_height
                || presentedWhitespaceCopyCount != session.presentationExportCounters.whitespaceCopies
                || compositionOverlay?.info.identity.generation != nextCompositionOverlay?.info.identity.generation
            if immutableInputsChanged { immutablePresentationGeneration &+= 1 }
            presentedWhitespaceCopyCount = session.presentationExportCounters.whitespaceCopies
            documentState = nextDocumentState
            formattedSnapshot = nextFormattedSnapshot
            compositionOverlay = nextCompositionOverlay
            completion = nextCompletion
            viewPresentation = nextPresentation
            selectionPresentationViewID = session.viewID
            selectionCharacterContextGeneration = session.characterContextGeneration
            viewportState = nextViewport
            layoutSnapshot = nextLayoutSnapshot
            layoutPaint = nextLayoutPaint
            layoutTextSlices = nextLayoutTextSlices
            compositionTextSlices = nextCompositionTextSlices
            commandLine = nextCommandLine
            substituteConfirmationPrompt = nextSubstituteConfirmation
            if nextCommandLine.prompt != nil { clearCommandOutput() }
            visualSelection = nextVisualSelection
            presentationRefreshCount &+= 1
            if immutableInputsChanged, let nextLayoutSnapshot {
                let visible = CGRect(x: CGFloat(nextViewport.left), y: CGFloat(nextViewport.top),
                    width: CGFloat(nextLayoutSnapshot.info.viewport_width), height: CGFloat(nextLayoutSnapshot.info.viewport_height))
                let renderRuns = nextLayoutSnapshot.clusters.compactMap { cluster -> (identifier: UInt64, metricsGeneration: UInt64)? in
                    let rect = cluster.typographic_bounds
                    guard cluster.flags & UInt32(VIEM_POSITIONED_CLUSTER_HAS_RENDER_RUN) != 0,
                          visible.intersects(CGRect(x: CGFloat(rect.x), y: CGFloat(rect.y),
                              width: CGFloat(rect.width), height: CGFloat(rect.height))) else { return nil }
                    return (cluster.render_run.identifier, cluster.render_run.metrics_generation)
                }
                if session.provider.updateVisibleImages(renderRuns, viewportID: imageViewportID), !imageAdmissionRefreshPending {
                    imageAdmissionRefreshPending = true
                    // Keep this frame's resource generation valid until the next
                    // synchronous refresh installs the newly admitted previews.
                    Task { @MainActor [weak self] in
                        guard let self else { return }
                        self.imageAdmissionRefreshPending = false
                        self.session?.provider.invalidateImageResources()
                    }
                }
            }
            synchronizeCompletionPolling()
            synchronizeSearchPolling(pending: nextSearchWorkPending)
            session.tableWidthRefinement.didInstall = { [weak self] in self?.refreshPresentation(advancingSearch: false) }
            session.tableWidthRefinement.update()
            session.backgroundLayout.update()
            updateStatusBar()
            if isViewLoaded {
                editorView.applyPresentation()
                linkPopover.refresh()
                imagePopover.refresh()
            }
            // Warnings are optional presentation data. A warning-copy failure
            // must not suppress an otherwise verified frame or replay input.
            if let layoutSnapshot, let warning = try? session.layoutDiagnostics(identity: layoutSnapshot.info.identity),
               warning != lastLayoutWarning {
                lastLayoutWarning = warning
                if !warning.isEmpty { publishHostMessage(warning) }
            }
            if selectionChanged {
                NotificationCenter.default.post(name: .viemEditorSelectionDidChange, object: self)
            }
        } catch {
            stopSearchPolling()
            // Retain a frame only while core can still attest to its complete
            // source/configuration/metrics identity. Never draw a stale frame.
            if let current = try? session.layoutSnapshotInfo(),
               layoutSnapshot?.info.identity.isSameLayout(as: current.identity) == true {
                // The previous verified frame remains usable.
            } else {
                layoutSnapshot = nil; layoutPaint = nil; layoutTextSlices = []
                compositionOverlay = nil; compositionTextSlices = []; visualSelection = nil
                immutablePresentationGeneration &+= 1
                if isViewLoaded { editorView.applyPresentation() }
            }
            report(error)
        }
    }

    /// Every attached pane can display shared search highlights, including an
    /// inactive pane. Work remains finite and core determines the slice budget.
    private func synchronizeSearchPolling(pending: Bool) {
        searchWorkPending = pending
        guard pending else { stopSearchPolling(); return }
        guard searchTimer == nil else { return }
        let timer = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.pollSearch() }
        }
        RunLoop.main.add(timer, forMode: .common)
        searchTimer = timer
    }

    private func stopSearchPolling() {
        searchTimer?.invalidate()
        searchTimer = nil
        searchWorkPending = false
    }

    func pollSearch() {
        guard let session else { stopSearchPolling(); return }
        do {
            if try session.pollSearch() {
                refreshPresentation(advancingSearch: false)
            } else {
                synchronizeSearchPolling(pending: try session.searchWorkPending())
            }
        } catch {
            stopSearchPolling()
            report(error)
        }
    }

    var isSearchPolling: Bool { searchTimer?.isValid == true }

    /// The frontend schedules opportunities for bounded work; the core decides
    /// which work remains and publishes every candidate/selection transition.
    private func synchronizeCompletionPolling() {
        guard completion?.isSearching == true,
              !isViewLoaded || editorView.isCaretActive else {
            completionTimer?.invalidate()
            completionTimer = nil
            return
        }
        guard completionTimer == nil else { return }
        let timer = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.pollCompletion() }
        }
        RunLoop.main.add(timer, forMode: .common)
        completionTimer = timer
    }

    func pollCompletion() {
        guard let session, completion?.isSearching == true else { return }
        do {
            if try session.pollCompletion() { refreshPresentation() }
        } catch {
            completionTimer?.invalidate()
            completionTimer = nil
            report(error)
        }
    }

    var isCompletionPolling: Bool { completionTimer?.isValid == true }

    func completionFocusDidChange() {
        if isViewLoaded, !editorView.isCaretActive { _ = acceptCompletionForNativeInput() }
        synchronizeCompletionPolling()
    }

    /// Materialize any preview before AppKit constructs a revision-bound native
    /// intention. Core decides acceptance; the native operation then runs as usual.
    @discardableResult
    func acceptCompletionForNativeInput() -> Bool {
        guard completion?.isActive == true, let session else { return true }
        do {
            let previousRefresh = presentationRefreshCount
            if try session.acceptCompletion(), previousRefresh == presentationRefreshCount {
                refreshPresentation()
            }
            return true
        } catch {
            report(error)
            NSSound.beep()
            return false
        }
    }

    /// Detect changes in the caret, selection, or pending character context without
    /// keeping persistent offsets or resolving a second selection/layout query. A new
    /// source revision alone updates the retained snapshot silently: editing a
    /// style may patch source while leaving the logical selection unchanged.
    /// Old offsets are never reused as positions in that new revision.
    private func selectionPresentationChanged(_ next: ViemViewPresentationV1, viewID: ViemViewId,
                                              characterContextGeneration: UInt64) -> Bool {
        guard let previousViewID = selectionPresentationViewID else { return false }
        let previous = viewPresentation
        if previousViewID != viewID || previous.document_id != next.document_id { return true }
        if selectionCharacterContextGeneration != characterContextGeneration { return true }
        if previous.cursor_utf8_offset != next.cursor_utf8_offset || previous.cursor_affinity != next.cursor_affinity { return true }

        let selectionFlags = UInt32(VIEM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR)
            | UInt32(VIEM_VIEW_PRESENTATION_VISUAL_ANCHOR_AFFINITY_EXACT)
            | UInt32(VIEM_VIEW_PRESENTATION_HAS_VISUAL_BLOCK)
        if previous.flags & selectionFlags != next.flags & selectionFlags { return true }
        if next.flags & UInt32(VIEM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR) == 0,
           previous.mode != next.mode {
            func hasCharacterContext(_ mode: UInt32) -> Bool {
                mode == UInt32(VIEM_MODE_NORMAL) || mode == UInt32(VIEM_MODE_INSERT)
                    || mode == UInt32(VIEM_MODE_REPLACE)
            }
            if hasCharacterContext(previous.mode), hasCharacterContext(next.mode) { return true }
        }
        if next.flags & UInt32(VIEM_VIEW_PRESENTATION_HAS_VISUAL_ANCHOR) != 0 {
            if previous.visual_anchor_utf8_offset != next.visual_anchor_utf8_offset { return true }
            if next.flags & UInt32(VIEM_VIEW_PRESENTATION_VISUAL_ANCHOR_AFFINITY_EXACT) != 0,
               previous.visual_anchor_affinity != next.visual_anchor_affinity { return true }
            // Ex/search temporarily changes the command mode while retaining
            // the same Visual selection. It must not act like a new selection.
            if previous.mode != UInt32(VIEM_MODE_COMMAND_LINE), next.mode != UInt32(VIEM_MODE_COMMAND_LINE),
               previous.mode != next.mode { return true }
        }
        if next.flags & UInt32(VIEM_VIEW_PRESENTATION_HAS_VISUAL_BLOCK) != 0 {
            return previous.visual_block_left_x != next.visual_block_left_x
                || previous.visual_block_right_x != next.visual_block_right_x
        }
        return false
    }

    func performInput(_ operation: () throws -> Void) {
        clearCommandOutput()
        do {
            lastErrorMessage = ""
            let refreshCountBeforeInput = presentationRefreshCount
            try operation()
            if presentationRefreshCount == refreshCountBeforeInput {
                refreshPresentation()
            }
        } catch {
            refreshPresentation()
            report(error)
            NSSound.beep()
        }
    }

    func report(_ error: Error) {
        publishHostMessage(error.localizedDescription)
    }

    func sharedDocumentDidChange(originatingViewIDs: Set<ViemViewId>) {
        if originatingViewIDs.count != 1 || !originatingViewIDs.contains(session?.viewID ?? 0) {
            session?.noteExternalDocumentChange()
        }
        refreshPresentation()
    }

    /// Font registration can retire core geometry without a native input turn.
    /// Validate cheaply before hit testing or deriving a scroll target;
    /// copy a new presentation only when geometry was rebuilt or is missing.
    func refreshGeometryBeforeInteraction() -> Bool {
        guard let session else { return false }
        do {
            let rebuilt = try session.refreshLayoutIfNeeded()
            let current = try session.layoutSnapshotInfo()
            let viewport = try session.viewportState()
            if rebuilt || layoutSnapshot?.info.identity.isSameLayout(as: current.identity) != true
                || viewportState.left != viewport.left || viewportState.top != viewport.top
                || layoutSnapshot?.info.viewport_width != current.viewport_width
                || layoutSnapshot?.info.viewport_height != current.viewport_height {
                refreshPresentation(advancingSearch: false)
            }
            // A failed refresh must never leave an unchecked old hit-test target.
            return layoutSnapshot?.info.identity.isSameLayout(as: current.identity) == true
                && viewportState.left == viewport.left && viewportState.top == viewport.top
        } catch {
            report(error)
            return false
        }
    }

    func requestVerticalViewport(top: CGFloat) {
        dismissCommandOutput()
        guard let session else { return }
        guard refreshGeometryBeforeInteraction() else { return }
        do {
            _ = try session.setViewportOrigin(
                left: CGFloat(viewportState.left),
                top: top,
                expected: viewportState
            )
            refreshPresentation()
        } catch {
            report(error)
            NSSound.beep()
        }
    }

    /// Place the command-line caret where the status line was clicked.
    public func selectCommandLine(atUTF8Offset offset: Int, extending: Bool) {
        guard let session, let prompt = commandLine else { return }
        let clamped = min(max(offset, 0), prompt.text.utf8.count)
        performInput {
            _ = try session.editCommandLine(
                prompt,
                anchor: extending ? Int(prompt.selectionAnchorUTF8Offset) : clamped,
                active: clamped
            )
        }
    }

    public func htmlExportData() async throws -> Data {
        guard let session else { throw EVCoreFrontendError.unavailableLayout }
        return try await session.htmlExportData()
    }

    public func perform(statusOption: EVStatusBarOption) {
        guard acceptCompletionForNativeInput() else { return }
        guard let session else { return }
        performInput {
            switch statusOption {
            case let .lineMode(mode): try session.setLineMode(mode)
            }
        }
    }

    func setFormattedView(_ enabled: Bool) {
        guard [.markdown, .markdownSource].contains(backend.sourceFormat),
              acceptCompletionForNativeInput(), let session else { return }
        let expected = documentState
        performInput { _ = try session.setMarkdownSource(!enabled, expected: expected) }
    }

    public func perform(menuCommand: EVMenuCommand, sender: Any?) {
        guard acceptCompletionForNativeInput() else { return }
        if isViewLoaded, editorView.statusBar?.performCommandOutputAction(menuCommand) == true { return }
        dismissCommandOutput()
        guard let session else { return }
        switch menuCommand {
        case .editStyles:
            EVStyleEditorCoordinator.shared.show(document: self, sender: sender)
            return
        case .reloadStyleSheet:
            reloadCodeStyleSheet()
            return
        default: break
        }
        if backend.sourceFormat == .code {
            if (300..<400).contains(menuCommand.rawValue) { return }
        }
        switch menuCommand {
        case .heading0, .heading1, .heading2, .heading3, .heading4, .heading5, .heading6:
            performHeadingShortcut(level: UInt32(menuCommand.rawValue - EVMenuCommand.heading0.rawValue))
        case .undo:
            performInput { _ = try session.undo() }
        case .redo:
            performInput { _ = try session.redo() }
        case .copy:
            copyOrCutSelection(cut: false)
        case .copySource:
            copySelectedSource()
        case .cut:
            copyOrCutSelection(cut: true)
        case .paste, .pasteAndMatchStyle:
            pasteMatchesStyle = menuCommand == .pasteAndMatchStyle
            defer { pasteMatchesStyle = false }
            pastePlainText()
        case .delete:
            performInput {
                if self.isTextSelectionMode {
                    _ = try session.sendKey(kind: UInt32(VIEM_KEY_DELETE))
                } else if self.hasSelection {
                    _ = try self.sendCommandCharacter("d", session: session)
                } else if self.viewPresentation.mode == UInt32(VIEM_MODE_NORMAL) {
                    _ = try self.sendCommandCharacter("x", session: session)
                } else {
                    _ = try session.sendKey(kind: UInt32(VIEM_KEY_DELETE))
                }
            }
        case .selectAll:
            performInput { try session.selectAll() }
        case .selectWord:
            performInput { try self.selectNativeRange(["v", "i", "w"], sender: sender, session: session) }
        case .selectSentence:
            performInput { try self.selectNativeRange(["v", "i", "s"], sender: sender, session: session) }
        case .selectParagraph:
            performInput { try self.selectNativeRange(["v", "i", "p"], sender: sender, session: session) }
        case .selectHardLine:
            performInput { try self.selectNativeRange(["V"], sender: sender, session: session) }
        case .selectVisualRow:
            performInput { try self.selectNativeRange(["g", "0", "v", "g", "$"], sender: sender, session: session) }
        case .find:
            performInput { try self.sendNormalSequence(["/"], session: session) }
        case .findAndReplace:
            performInput { try self.sendNormalSequence([":", "%", "s", "/"], session: session) }
        case .findNext:
            performInput { try self.sendNormalSequence(["n"], session: session) }
        case .findPrevious:
            performInput { try self.sendNormalSequence(["N"], session: session) }
        case .useSelectionForFind:
            useCurrentSelectionForFind(session: session)
        case .jumpToSelection:
            guard hasSelection else { NSSound.beep(); return }
            performInput { _ = try session.revealSelection() }
        case .makeUppercase:
            guard hasSelection else { NSSound.beep(); return }
            performInput { _ = try self.sendCommandCharacter("U", session: session) }
        case .makeLowercase:
            guard hasSelection else { NSSound.beep(); return }
            performInput { _ = try self.sendCommandCharacter("u", session: session) }
        case .toggleCase:
            guard hasSelection || viewPresentation.mode == UInt32(VIEM_MODE_NORMAL) else {
                NSSound.beep()
                return
            }
            performInput { _ = try self.sendCommandCharacter("~", session: session) }
        case .wordWrap:
            performInput { _ = try session.setWrap(!self.wrapEnabled) }
        case .flowParagraphs:
            performInput { try session.setParagraphFlow(!(try session.paragraphFlow())) }
        case .lineEndingUnix:
            setFileFormat(UInt32(VIEM_FILE_FORMAT_UNIX), session: session)
        case .lineEndingWindows:
            setFileFormat(UInt32(VIEM_FILE_FORMAT_DOS), session: session)
        case .lineEndingClassicMac:
            setFileFormat(UInt32(VIEM_FILE_FORMAT_MAC), session: session)
        case .encodingUTF8, .encodingLatin1, .encodingUTF16LE, .encodingUTF16BE:
            let expected = documentState
            performInput { _ = try session.setEncoding(self.encodingValue(menuCommand), expected: expected) }
        case .showInvisibleCharacters:
            guard layoutSnapshot?.whitespace.applicable == true else { return }
            performInput { try session.setVisibleWhitespace(!(self.layoutSnapshot?.whitespace.enabled ?? true)) }
        case .zoomIn:
            setZoom(adjacentTo: zoomScale, increasing: true, session: session)
        case .zoomOut:
            setZoom(adjacentTo: zoomScale, increasing: false, session: session)
        case .actualSize:
            setZoom(1, session: session)
        case .bulletedList, .numberedList, .removeList:
            performInput {
                let selection = try session.listSelection()
                let style: UInt32 = menuCommand == .bulletedList ? UInt32(VIEM_LIST_STYLE_BULLET)
                    : menuCommand == .numberedList ? UInt32(VIEM_LIST_STYLE_NUMBERED) : UInt32(VIEM_LIST_STYLE_NONE)
                _ = try session.setListStyle(style, expected: selection)
            }
        case .increaseIndent, .decreaseIndent:
            guard presentation(for: menuCommand).isEnabled else { return }
            performInput {
                _ = try session.indentList(unindent: menuCommand == .decreaseIndent,
                                          expected: session.listSelection())
            }
        case .bold:
            toggleSemanticStyle(UInt32(VIEM_SEMANTIC_STYLE_STRONG), session: session)
        case .italic:
            toggleSemanticStyle(UInt32(VIEM_SEMANTIC_STYLE_EMPHASIS), session: session)
        case .strikethrough:
            performInput {
                let state = try session.strikethroughState()
                _ = try session.setStrikethrough(state != UInt32(VIEM_SEMANTIC_STYLE_STATE_ON),
                    expected: session.listSelection())
            }
        case .editStyles:
            break // Routed above for both document and global Code styles.
        case .save:
            (view.window?.windowController as? EVDocumentWindowController)?.activeDocument?.save(sender)
        case .saveAs:
            (view.window?.windowController as? EVDocumentWindowController)?.activeDocument?.saveAs(sender)
        case .exportHTML:
            (view.window?.windowController as? EVDocumentWindowController)?.activeDocument?.exportHTML(from: self)
        case .pageSetup:
            NSPageLayout().runModal()
        case .printDocument:
            // Printing is intentionally unavailable until core exposes a
            // paginated projection. The unpaginated viewport is not a print
            // document and must not be submitted as one.
            return
        default:
            NSSound.beep()
        }
    }

    public func presentation(for menuCommand: EVMenuCommand) -> EVMenuItemPresentation {
        if isViewLoaded, let presentation = editorView.statusBar?.commandOutputPresentation(for: menuCommand) {
            return presentation
        }
        if backend.sourceFormat == .code, (300..<400).contains(menuCommand.rawValue) {
            if [.editStyles, .reloadStyleSheet].contains(menuCommand) {
                return .enabled
            }
            return .disabled
        }
        return switch menuCommand {
        case .paragraphStyles, .characterStyles:
            EVMenuItemPresentation(isEnabled: session != nil && backend.sourceFormat != .plainText)
        case .heading0, .heading1, .heading2, .heading3, .heading4, .heading5, .heading6:
            headingShortcutPresentation(level: UInt32(menuCommand.rawValue - EVMenuCommand.heading0.rawValue))
        case .bulletedList, .numberedList, .removeList:
            listStylePresentation(menuCommand)
        case .increaseIndent, .decreaseIndent:
            listIndentPresentation(unindent: menuCommand == .decreaseIndent)
        case .save, .saveAs, .exportHTML, .pageSetup,
             .selectAll, .selectWord, .selectSentence, .selectParagraph,
             .selectHardLine, .selectVisualRow, .find, .findAndReplace,
             .findNext, .findPrevious:
            .enabled
        case .undo:
            EVMenuItemPresentation(
                isEnabled: canUndo,
                title: historyTitle(prefix: "Undo", category: documentState.undo_action_category)
            )
        case .redo:
            EVMenuItemPresentation(
                isEnabled: canRedo,
                title: historyTitle(prefix: "Redo", category: documentState.redo_action_category)
            )
        case .copy, .copySource, .cut:
            // The core owns the logical Visual selection even when none of
            // its geometry is materialized in the current viewport.
            EVMenuItemPresentation(isEnabled: hasSelection)
        case .paste, .pasteAndMatchStyle:
            EVMenuItemPresentation(isEnabled: pasteboard.viemCanReadString())
        case .delete:
            .enabled
        case .makeUppercase, .makeLowercase:
            EVMenuItemPresentation(isEnabled: hasSelection)
        case .toggleCase:
            EVMenuItemPresentation(
                isEnabled: hasSelection || viewPresentation.mode == UInt32(VIEM_MODE_NORMAL)
            )
        case .wordWrap:
            EVMenuItemPresentation(isEnabled: true, state: wrapEnabled ? .on : .off)
        case .flowParagraphs:
            EVMenuItemPresentation(isEnabled: backend.sourceFormat == .markdownSource, state: (try? session?.paragraphFlow()) == true ? .on : .off)
        case .lineEndingUnix:
            fileFormatPresentation(UInt32(VIEM_FILE_FORMAT_UNIX))
        case .lineEndingWindows:
            fileFormatPresentation(UInt32(VIEM_FILE_FORMAT_DOS))
        case .lineEndingClassicMac:
            fileFormatPresentation(UInt32(VIEM_FILE_FORMAT_MAC))
        case .encodingUTF8, .encodingLatin1, .encodingUTF16LE, .encodingUTF16BE:
            EVMenuItemPresentation(
                isEnabled: true,
                state: documentState.encoding == encodingValue(menuCommand) ? .on : .off
            )
        case .showInvisibleCharacters:
            EVMenuItemPresentation(isEnabled: layoutSnapshot?.whitespace.applicable == true,
                                  state: layoutSnapshot?.whitespace.enabled == true ? .on : .off)
        case .useSelectionForFind:
            EVMenuItemPresentation(
                isEnabled: selectedUTF8Ranges().contains(where: { !$0.isEmpty })
            )
        case .jumpToSelection:
            EVMenuItemPresentation(isEnabled: hasSelection)
        case .zoomIn:
            EVMenuItemPresentation(
                isEnabled: ((try? session?.adjacentZoomScale(from: zoomScale, increasing: true)) ?? zoomScale) != zoomScale
            )
        case .zoomOut:
            EVMenuItemPresentation(
                isEnabled: ((try? session?.adjacentZoomScale(from: zoomScale, increasing: false)) ?? zoomScale) != zoomScale
            )
        case .actualSize:
            .enabled
        case .bold:
            semanticStyleMenuPresentation(
                UInt32(VIEM_SEMANTIC_STYLE_STRONG),
                session: session
            )
        case .italic:
            semanticStyleMenuPresentation(
                UInt32(VIEM_SEMANTIC_STYLE_EMPHASIS),
                session: session
            )
        case .strikethrough:
            if [.markdown, .markdownSource].contains(backend.sourceFormat), let session,
               let state = try? session.strikethroughState() {
                EVMenuItemPresentation(isEnabled: documentState.flags & UInt32(VIEM_DOCUMENT_STATE_READ_ONLY) == 0
                    && (try? session.selectedNamedStyles().hasCodeBlock) != true,
                    state: state == UInt32(VIEM_SEMANTIC_STYLE_STATE_ON) ? .on
                        : state == UInt32(VIEM_SEMANTIC_STYLE_STATE_MIXED) ? .mixed : .off)
            } else { .disabled }
        case .reloadStyleSheet:
            .enabled
        case .editStyles:
            EVMenuItemPresentation(isEnabled: backend.sourceFormat != .code)
        case .printDocument:
            .disabled
        default:
            .disabled
        }
    }

    var hasSelection: Bool { EVSelectionModes.hasSelection(viewPresentation.mode) }
    var isTextSelectionMode: Bool { EVSelectionModes.isTextSelection(viewPresentation.mode) }

    var isVisualBlockMode: Bool {
        viewPresentation.mode == UInt32(VIEM_MODE_VISUAL_BLOCK)
            || viewPresentation.mode == UInt32(VIEM_MODE_SELECT_BLOCK)
            || viewPresentation.mode == UInt32(VIEM_MODE_SELECTION_BLOCK)
    }

    var wrapEnabled: Bool {
        viewportState.flags & UInt32(VIEM_VIEWPORT_STATE_WRAP) != 0
    }

    var zoomScale: Float {
        viewportState.scale.isFinite && viewportState.scale > 0
            ? viewportState.scale
            : 1
    }

    var canUndo: Bool {
        documentState.flags & UInt32(VIEM_DOCUMENT_STATE_CAN_UNDO) != 0
    }

    var canRedo: Bool {
        documentState.flags & UInt32(VIEM_DOCUMENT_STATE_CAN_REDO) != 0
    }


    func formattedText(in utf8Range: Range<Int>) -> String? {
        guard let snapshot = formattedSnapshot,
              utf8Range.lowerBound >= 0,
              utf8Range.lowerBound <= utf8Range.upperBound,
              utf8Range.upperBound <= formattedUTF8Length,
              let lower = UInt64(exactly: utf8Range.lowerBound),
              let upper = UInt64(exactly: utf8Range.upperBound)
        else { return nil }
        // AppKit asks for selected text when we notify it of a caret/selection
        // change. Reuse the matching visible slice just as drawing does; larger
        // or offscreen accessibility/input-method requests still read on demand.
        if let slice = layoutTextSlices.first(where: {
            $0.identity.isSameSnapshot(as: snapshot.info.identity)
                && $0.utf8Range.lowerBound <= lower && upper <= $0.utf8Range.upperBound
        }) {
            return slice.text(in: utf8Range)
        }
        return try? backend.formattedText(in: lower ..< upper, snapshot: snapshot)
    }

    func layoutText(in utf8Range: Range<Int>) -> String? {
        if compositionOverlay != nil,
           let slice = compositionTextSlices.first(where: {
               guard let lower = UInt64(exactly: utf8Range.lowerBound),
                     let upper = UInt64(exactly: utf8Range.upperBound)
               else { return false }
               return $0.utf8Range.lowerBound <= lower && upper <= $0.utf8Range.upperBound
           })
        {
            return slice.text(in: utf8Range)
        }
        guard let snapshot = formattedSnapshot,
              let lower = UInt64(exactly: utf8Range.lowerBound),
              let upper = UInt64(exactly: utf8Range.upperBound),
              let slice = layoutTextSlices.first(where: {
                  $0.identity.isSameSnapshot(as: snapshot.info.identity)
                      && $0.utf8Range.lowerBound <= lower
                      && upper <= $0.utf8Range.upperBound
              })
        else { return nil }
        return slice.text(in: utf8Range)
    }

    func presentedText(in utf8Range: Range<Int>) -> String? {
        guard let overlay = compositionOverlay else { return formattedText(in: utf8Range) }
        if let cached = layoutText(in: utf8Range) { return cached }
        guard utf8Range.lowerBound >= 0,
              utf8Range.lowerBound <= utf8Range.upperBound,
              utf8Range.upperBound <= (overlay.utf8Length ?? -1),
              let lower = UInt64(exactly: utf8Range.lowerBound),
              let upper = UInt64(exactly: utf8Range.upperBound),
              let session
        else { return nil }
        return try? session.compositionTextSlice(in: lower ..< upper, overlay: overlay)
            .text(in: utf8Range)
    }

    func formattedByte(atUTF8Offset offset: Int) -> UInt8? {
        guard let snapshot = formattedSnapshot,
              let converted = UInt64(exactly: offset),
              let slice = layoutTextSlices.first(where: {
                  $0.identity.isSameSnapshot(as: snapshot.info.identity)
                      && $0.utf8Range.contains(converted)
              })
        else { return nil }
        return slice.byte(atUTF8Offset: offset)
    }

    func layoutByte(atUTF8Offset offset: Int) -> UInt8? {
        if compositionOverlay != nil,
           let slice = compositionTextSlices.first(where: {
               guard let converted = UInt64(exactly: offset) else { return false }
               return $0.utf8Range.contains(converted)
           })
        {
            return slice.byte(atUTF8Offset: offset)
        }
        return formattedByte(atUTF8Offset: offset)
    }

    func utf16Offsets(forUTF8 offsets: [Int]) -> [Int]? {
        guard let snapshot = formattedSnapshot,
              offsets.allSatisfy({ $0 >= 0 && $0 <= formattedUTF8Length })
        else { return nil }
        let input = offsets.compactMap(UInt64.init(exactly:))
        guard input.count == offsets.count,
              let mapped = try? backend.mapFormattedUTF8ToUTF16(input, snapshot: snapshot)
        else { return nil }
        let result = mapped.compactMap(Int.init(exactly:))
        return result.count == mapped.count ? result : nil
    }

    func utf8Offsets(forUTF16 offsets: [Int]) -> [Int]? {
        guard let snapshot = formattedSnapshot,
              offsets.allSatisfy({ $0 >= 0 && $0 <= formattedUTF16Length })
        else { return nil }
        let input = offsets.compactMap(UInt64.init(exactly:))
        guard input.count == offsets.count,
              let mapped = try? backend.mapFormattedUTF16ToUTF8(input, snapshot: snapshot)
        else { return nil }
        let result = mapped.compactMap(Int.init(exactly:))
        return result.count == mapped.count ? result : nil
    }

    func formattedPointInfo(atUTF8Offset offset: Int) -> ViemFormattedPointInfoV1? {
        guard let snapshot = formattedSnapshot,
              offset >= 0,
              offset <= formattedUTF8Length,
              let converted = UInt64(exactly: offset)
        else { return nil }
        return try? backend.formattedPointInfo(atUTF8Offset: converted, snapshot: snapshot)
    }

    func selectedUTF8Ranges() -> [Range<Int>] {
        if let session, let selection = try? session.tableSelection(), selection.active != 0 {
            return (try? session.tableSelectionRanges(selection)) ?? []
        }
        guard hasSelection,
              let visualSelection,
              visualSelection.info.identity.kind != UInt32(VIEM_VISUAL_SELECTION_KIND_NONE)
        else { return [] }

        let byteCount = formattedUTF8Length
        var ranges: [Range<Int>] = []
        ranges.reserveCapacity(visualSelection.segments.count)
        for segment in visualSelection.segments {
            guard let start = Int(exactly: segment.text_start),
                  let end = Int(exactly: segment.text_end),
                  start >= 0,
                  start <= end,
                  end <= byteCount
            else { return [] }
            ranges.append(start ..< end)
        }
        return ranges
    }

    /// NSTextInputClient and composition replacement accept only one range.
    /// Never turn a multi-row Visual Block into a destructive bounding range.
    func selectedUTF8Range() -> Range<Int>? {
        let ranges = selectedUTF8Ranges()
        return ranges.count == 1 ? ranges[0] : nil
    }

    /// AppKit's text-input protocol can report only one native selection.
    /// For Visual Block, expose its first exact logical segment rather than a
    /// fabricated range spanning unselected text on intervening rows.
    func primarySelectedUTF8Range() -> Range<Int>? {
        selectedUTF8Ranges().first
    }

    func selectionText() -> String? {
        if let session, let selection = try? session.tableSelection(), selection.active != 0 {
            return try? session.tableSelectionText(selection)
        }
        let ranges = selectedUTF8Ranges()
        guard !ranges.isEmpty else { return nil }
        let pieces = ranges.compactMap(formattedText(in:))
        guard pieces.count == ranges.count else { return nil }
        if visualSelection?.info.identity.kind == UInt32(VIEM_VISUAL_SELECTION_KIND_BLOCK) {
            return pieces.joined(separator: "\n")
        }
        return pieces.joined()
    }

    private func copyOrCutSelection(cut: Bool) {
        guard hasSelection, let session else { NSSound.beep(); return }
        performInput {
            // Native Copy and Cut preserve the selected text exactly, including an
            // absent final newline. The core's internal linewise register can
            // still retain its Vim shape for subsequent register commands.
            if [UInt32(VIEM_MODE_VISUAL_LINE), UInt32(VIEM_MODE_SELECT_LINE), UInt32(VIEM_MODE_SELECTION_LINE)].contains(viewPresentation.mode),
               let range = selectedUTF8Range(), let snapshot = formattedSnapshot {
                let json = try backend.clipboardFragmentJSON(in: range, snapshot: snapshot)
                nativeCopyRepresentations = try EVClipboardFragment.decode(json).representations(json: json)
            }
            defer { nativeCopyRepresentations = nil }
            let outcome: ViemCoreOutcomeV1
            if cut {
                _ = try self.sendCommandCharacter("\"", session: session)
                _ = try self.sendCommandCharacter("+", session: session)
                outcome = try self.sendCommandCharacter("d", session: session)
            } else {
                // Copy is a platform intention, so no temporary Visual mode
                // or Vim yank should move the caret or collapse selection.
                outcome = try session.sendKey(kind: UInt32(VIEM_KEY_COPY_SELECTION))
            }
            guard outcome.command_status == UInt32(VIEM_COMMAND_STATUS_COMPLETE) else {
                throw EVCoreFrontendError.command(
                    operation: cut ? "Cut" : "Copy",
                    status: outcome.command_status
                )
            }
        }
    }

    private func copySelectedSource() {
        let ranges = selectedUTF8Ranges()
        guard !ranges.isEmpty, let snapshot = formattedSnapshot else { NSSound.beep(); return }
        performInput {
            let fragments = try ranges.map {
                let fragment = try backend.clipboardFragment(in: $0, snapshot: snapshot)
                guard !fragment.sourceText.isEmpty || fragment.plainText.isEmpty else {
                    throw EVCoreFrontendError.core(operation: "Selection has no exact source fragment", status: UInt32(VIEM_STATUS_AMBIGUOUS_PROJECTION))
                }
                return fragment.sourceText
            }
            let isBlock = visualSelection?.info.identity.kind == UInt32(VIEM_VISUAL_SELECTION_KIND_BLOCK)
            let content = EVClipboardRepresentations(plainText: fragments.joined(separator: isBlock ? "\n" : ""))
            guard pasteboard.viemWrite(content) else { throw EVCoreFrontendError.pasteboardWriteFailed }
        }
    }

    private func useCurrentSelectionForFind(session: EVCoreViewSession) {
        guard let selection = visualSelection,
              selection.info.identity.kind != UInt32(VIEM_VISUAL_SELECTION_KIND_NONE),
              let text = selectionText(),
              !text.isEmpty
        else {
            NSSound.beep()
            return
        }
        performInput {
            _ = try session.useSelectionForFind(selection.info.identity)
            self.findPasteboard.viemClearContents()
            guard self.findPasteboard.viemSetString(text) else {
                throw EVCoreFrontendError.pasteboardWriteFailed
            }
        }
    }

    private func setZoom(
        adjacentTo current: Float,
        increasing: Bool,
        session: EVCoreViewSession
    ) {
        guard let target = try? session.adjacentZoomScale(from: current, increasing: increasing) else { return }
        setZoom(target, session: session)
    }

    private func setZoom(_ scale: Float, session: EVCoreViewSession) {
        guard scale.isFinite, scale > 0, abs(scale - zoomScale) > 0.0001 else { return }
        editorView.performZoomGeometryUpdate {
            self.performInput { _ = try session.setScale(CGFloat(scale)) }
        }
    }

    private func semanticStyleMenuPresentation(
        _ style: UInt32,
        session: EVCoreViewSession?
    ) -> EVMenuItemPresentation {
        guard let session,
              let value = try? session.semanticStylePresentation(style),
              value.struct_size >= UInt32(MemoryLayout<ViemSemanticStylePresentationV1>.size),
              value.style == style,
              value.flags & UInt32(VIEM_SEMANTIC_STYLE_HAS_ACTIVE_RANGE | VIEM_SEMANTIC_STYLE_TYPING_CONTEXT) != 0,
              value.selection.struct_size
                >= UInt32(MemoryLayout<ViemLogicalSelectionIdentityV1>.size)
        else { return .disabled }

        switch value.state {
        case UInt32(VIEM_SEMANTIC_STYLE_STATE_OFF):
            return EVMenuItemPresentation(
                isEnabled: value.flags & UInt32(VIEM_SEMANTIC_STYLE_CAN_SET) != 0,
                state: .off
            )
        case UInt32(VIEM_SEMANTIC_STYLE_STATE_ON):
            return EVMenuItemPresentation(
                isEnabled: value.flags & UInt32(VIEM_SEMANTIC_STYLE_CAN_CLEAR) != 0,
                state: .on
            )
        case UInt32(VIEM_SEMANTIC_STYLE_STATE_MIXED):
            return EVMenuItemPresentation(
                isEnabled: value.flags & UInt32(VIEM_SEMANTIC_STYLE_CAN_SET) != 0,
                state: .mixed
            )
        default:
            return .disabled
        }
    }

    private func toggleSemanticStyle(_ style: UInt32, session: EVCoreViewSession) {
        do {
            // Menu validation is advisory. Re-query at activation so the
            // mutation is bound to the current logical selection, independent
            // of viewport/layout coverage.
            let value = try session.semanticStylePresentation(style)
            guard value.struct_size
                    >= UInt32(MemoryLayout<ViemSemanticStylePresentationV1>.size),
                  value.style == style,
                  value.flags & UInt32(VIEM_SEMANTIC_STYLE_HAS_ACTIVE_RANGE | VIEM_SEMANTIC_STYLE_TYPING_CONTEXT) != 0,
                  value.selection.struct_size
                    >= UInt32(MemoryLayout<ViemLogicalSelectionIdentityV1>.size)
            else {
                NSSound.beep()
                return
            }

            let enabled: Bool
            let requiredCapability: UInt32
            switch value.state {
            case UInt32(VIEM_SEMANTIC_STYLE_STATE_OFF):
                enabled = true
                requiredCapability = UInt32(VIEM_SEMANTIC_STYLE_CAN_SET)
            case UInt32(VIEM_SEMANTIC_STYLE_STATE_ON):
                enabled = false
                requiredCapability = UInt32(VIEM_SEMANTIC_STYLE_CAN_CLEAR)
            case UInt32(VIEM_SEMANTIC_STYLE_STATE_MIXED):
                enabled = true
                requiredCapability = UInt32(VIEM_SEMANTIC_STYLE_CAN_SET)
            default:
                NSSound.beep()
                return
            }
            guard value.flags & requiredCapability != 0 else {
                NSSound.beep()
                return
            }

            lastErrorMessage = ""
            let refreshCountBeforeInput = presentationRefreshCount
            _ = try session.setSemanticStyle(
                style,
                enabled: enabled,
                expected: value.selection
            )
            if presentationRefreshCount == refreshCountBeforeInput {
                refreshPresentation()
            }
        } catch {
            report(error)
            // A stale selection or adapter rejection must invalidate the menu
            // state that led to the attempted action; never retry another
            // inferred range.
            refreshPresentation()
            NSSound.beep()
        }
    }

    private func pastePlainText() {
        guard pasteboard.viemString() != nil,
              let session
        else {
            NSSound.beep()
            return
        }
        performInput {
            switch self.viewPresentation.mode {
            case UInt32(VIEM_MODE_INSERT),
                 UInt32(VIEM_MODE_REPLACE),
                 UInt32(VIEM_MODE_COMMAND_LINE):
                _ = try session.sendKey(
                    kind: UInt32(VIEM_KEY_CONTROL_CHARACTER),
                    codepoint: UInt32(Character("r").asciiValue!)
                )
                _ = try self.sendCommandCharacter("+", session: session)
            default:
                _ = try self.sendCommandCharacter("\"", session: session)
                _ = try self.sendCommandCharacter("+", session: session)
                _ = try self.sendCommandCharacter("p", session: session)
            }
        }
    }

    private func sendNormalSequence(_ characters: [Character], session: EVCoreViewSession) throws {
        if viewPresentation.mode != UInt32(VIEM_MODE_NORMAL) {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        }
        for character in characters {
            _ = try sendCommandCharacter(character, session: session)
        }
    }

    func selectLineFromPointer(_ event: NSEvent, returningTo mode: UInt32) {
        guard let session else { return }
        performInput { try self.selectNativeRange(["V"], sender: event, session: session, returningTo: mode) }
    }

    private func selectNativeRange(_ characters: [Character], sender: Any?, session: EVCoreViewSession, returningTo mode: UInt32? = nil) throws {
        let returnMode = mode ?? viewPresentation.mode
        let before = viewPresentation
        if [UInt32(VIEM_MODE_INSERT), UInt32(VIEM_MODE_REPLACE)].contains(before.mode) {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            if before.document_revision == viewPresentation.document_revision {
                var point = ViemLayoutCaretPointV1()
                point.document_revision = before.document_revision
                point.text_offset = before.cursor_utf8_offset
                point.affinity = before.cursor_affinity
                _ = try session.placeCursor(point, extendSelection: false)
            }
        }
        try sendNormalSequence(characters, session: session)
        let mouse = (sender as? NSEvent).map { [.leftMouseDown, .leftMouseDragged].contains($0.type) } == true
        _ = try session.setSelectionOrigin(mouse ? UInt32(VIEM_SELECTION_ORIGIN_MOUSE) : UInt32(VIEM_SELECTION_ORIGIN_KEY), returningTo: returnMode)
    }

    private func sendCommandCharacter(
        _ character: Character,
        session: EVCoreViewSession
    ) throws -> ViemCoreOutcomeV1 {
        // Native menu actions are commands even when ordinary typed characters
        // replace a Select-mode range. This also keeps synthesized viw working
        // when selectmode includes cmd.
        if EVSelectionModes.isTextSelection(session.lastOutcome.mode) {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 111)
        }
        guard character.unicodeScalars.count == 1,
              let scalar = character.unicodeScalars.first
        else {
            return try session.sendText(String(character))
        }
        return try session.sendKey(
            kind: UInt32(VIEM_KEY_CHARACTER),
            codepoint: scalar.value
        )
    }

    private func setFileFormat(_ target: UInt32, session: EVCoreViewSession) {
        let expected = documentState
        performInput { _ = try session.setFileFormat(target, expected: expected) }
    }

    private func encodingValue(_ command: EVMenuCommand) -> UInt32 {
        switch command {
        case .encodingUTF8: UInt32(VIEM_ENCODING_UTF8)
        case .encodingLatin1: UInt32(VIEM_ENCODING_LATIN1)
        case .encodingUTF16LE: UInt32(VIEM_ENCODING_UTF16_LE)
        case .encodingUTF16BE: UInt32(VIEM_ENCODING_UTF16_BE)
        default: preconditionFailure("Expected an encoding menu command")
        }
    }

    private func fileFormatPresentation(_ target: UInt32) -> EVMenuItemPresentation {
        EVMenuItemPresentation(
            isEnabled: true,
            state: documentState.file_format == target ? .on : .off
        )
    }

    private func historyTitle(prefix: String, category: UInt32) -> String {
        let action = switch category {
        case UInt32(VIEM_HISTORY_ACTION_CATEGORY_TEXT): "Text Change"
        case UInt32(VIEM_HISTORY_ACTION_CATEGORY_STYLE): "Style Change"
        case UInt32(VIEM_HISTORY_ACTION_CATEGORY_FILE_FORMAT): "Line Endings"
        case UInt32(VIEM_HISTORY_ACTION_CATEGORY_HARD_LINE_TRANSFER): "Move Lines"
        case UInt32(VIEM_HISTORY_ACTION_CATEGORY_SOURCE_METADATA): "Source Metadata"
        case UInt32(VIEM_HISTORY_ACTION_CATEGORY_MIXED): "Changes"
        default: ""
        }
        return action.isEmpty ? prefix : "\(prefix) \(action)"
    }

    /// Republish the status line after a focus change, so the command caret
    /// blinks only in the active pane.
    func refreshStatusBarActivity() { updateStatusBar() }

    private func updateStatusBar() {
        let point = try? session?.lineLocation()
        let line = point.map { location in
            location.flags & UInt32(VIEM_LINE_LOCATION_GLOBAL_LINE_EXACT) != 0
                ? String(location.line) : "\(location.hard_line)·\(location.fragment)"
        } ?? "1"
        let column = point?.column ?? 1
        statusBarState = EVStatusBarState(
            mode: modeLabel(viewPresentation.mode),
            message: substituteConfirmationPrompt ?? lastErrorMessage,
            location: "Ln \(line), Col \(column)",
            lineMode: (try? session?.lineMode()) ?? .visual,
            locationIsFragment: point.map { $0.flags & UInt32(VIEM_LINE_LOCATION_GLOBAL_LINE_EXACT) == 0 } ?? false,
            commandLine: editorView.commandLineRenderState().map {
                EVStatusCommandLine(
                    prompt: $0.prompt,
                    text: $0.text,
                    cursorUTF8Offset: $0.cursorUTF8Offset,
                    markedDisplayRange: $0.markedDisplayRange,
                    selectedDisplayRange: $0.selectedDisplayRange
                )
            },
            commandOutput: commandOutput,
            requiresInteraction: substituteConfirmationPrompt != nil,
            isActive: editorView.isCaretActive
        )
        statusBarStateDidChange?(statusBarState)
    }

    private func layoutUTF8Ranges(
        for snapshot: EVLayoutExport,
        formattedLength: UInt64
    ) -> [Range<UInt64>] {
        let maximumSliceLength: UInt64 = 32 * 1024
        var intervals: [Range<UInt64>] = []
        intervals.reserveCapacity(snapshot.clusters.count + snapshot.rows.count)
        for cluster in snapshot.clusters {
            guard cluster.text_start < cluster.text_end,
                  cluster.text_end <= formattedLength
            else { continue }
            intervals.append(cluster.text_start ..< cluster.text_end)
        }
        let rowStarts = Set(snapshot.rows.map(\.text_start))
        for row in snapshot.rows where row.flags & UInt32(VIEM_VISUAL_ROW_WRAPS_TO_NEXT) == 0 {
            // A table cell may end its visual line at an internal hard break
            // before the enclosing table row's hard-line boundary. Accessibility
            // selection needs that real LF to join the materialized line ranges.
            // Two valid text boundaries one byte apart also guarantee a whole
            // scalar, unlike a clipped long-line fragment ending before Unicode.
            if row.text_end < formattedLength && rowStarts.contains(row.text_end + 1) {
                intervals.append(row.text_end ..< row.text_end + 1)
            }
            if row.hard_line_end < formattedLength {
                intervals.append(row.hard_line_end ..< row.hard_line_end + 1)
            }
        }
        intervals.sort {
            $0.lowerBound == $1.lowerBound
                ? $0.upperBound < $1.upperBound
                : $0.lowerBound < $1.lowerBound
        }

        var result: [Range<UInt64>] = []
        for interval in intervals {
            guard let previous = result.last else {
                result.append(interval)
                continue
            }
            if interval.upperBound <= previous.upperBound { continue }
            let mergedUpper = max(previous.upperBound, interval.upperBound)
            if interval.lowerBound <= previous.upperBound,
               mergedUpper - previous.lowerBound <= maximumSliceLength
            {
                result[result.count - 1] = previous.lowerBound ..< mergedUpper
            } else {
                result.append(interval)
            }
        }
        return result
    }

    private func modeLabel(_ mode: UInt32) -> String {
        switch mode {
        case UInt32(VIEM_MODE_INSERT): "INSERT"
        case UInt32(VIEM_MODE_REPLACE): "REPLACE"
        case UInt32(VIEM_MODE_VISUAL_CHARACTER): "VISUAL"
        case UInt32(VIEM_MODE_VISUAL_LINE): "VISUAL LINE"
        case UInt32(VIEM_MODE_VISUAL_BLOCK): "VISUAL BLOCK"
        case UInt32(VIEM_MODE_SELECTION_CHARACTER), UInt32(VIEM_MODE_SELECTION_LINE), UInt32(VIEM_MODE_SELECTION_BLOCK): "SELECTION"
        case UInt32(VIEM_MODE_SELECT_CHARACTER): "SELECT"
        case UInt32(VIEM_MODE_SELECT_LINE): "SELECT LINE"
        case UInt32(VIEM_MODE_SELECT_BLOCK): "SELECT BLOCK"
        case UInt32(VIEM_MODE_COMMAND_LINE): "COMMAND"
        default: "NORMAL"
        }
    }
}

@MainActor
extension EVEditorSurfaceController: EVCommandTurnHost {
    func clipboardSnapshotsForCommandTurn() -> [EVClipboardTurnSnapshot] {
        let text = pasteboard.viemString()
        let fragment = pasteMatchesStyle || backend.sourceFormat == .code ? nil : pasteboard.viemData(forType: EVClipboardRepresentations.fragmentType)
        let generation = text == nil ? 0 : pasteboard.viemGeneration
        let writable = pasteboard.viemIsWritable
        return [
            EVClipboardTurnSnapshot(
                target: UInt32(VIEM_CLIPBOARD_TARGET_CLIPBOARD),
                generation: generation,
                plainText: text,
                isWritable: writable,
                fragmentJSON: fragment
            ),
            EVClipboardTurnSnapshot(
                target: UInt32(VIEM_CLIPBOARD_TARGET_PRIMARY),
                generation: generation,
                plainText: text,
                isWritable: writable,
                fragmentJSON: fragment
            ),
        ]
    }

    /// Map one exported window effect. The core has already validated the
    /// grammar; an unknown command or a missing required count is a broken
    /// contract, not user input.
    private func windowRequest(from effect: EVExHostEffect) throws -> EVWindowRequest {
        let index = effect.windowCount.flatMap { Int(exactly: $0) }
        if effect.windowCount != nil, index == nil {
            throw EVCoreFrontendError.invalidHostEffect
        }
        func required() throws -> Int {
            guard let index, index >= 0 else { throw EVCoreFrontendError.invalidHostEffect }
            return index
        }
        switch effect.windowCommand {
        case UInt32(VIEM_WINDOW_FOCUS_DOWN): return .focusDown(count: try required())
        case UInt32(VIEM_WINDOW_FOCUS_UP): return .focusUp(count: try required())
        case UInt32(VIEM_WINDOW_FOCUS_LEFT): return .focusLeft(count: try required())
        case UInt32(VIEM_WINDOW_FOCUS_RIGHT): return .focusRight(count: try required())
        case UInt32(VIEM_WINDOW_FOCUS_NEXT): return .focusNext(index: index)
        case UInt32(VIEM_WINDOW_FOCUS_PREVIOUS): return .focusPrevious(index: index)
        case UInt32(VIEM_WINDOW_FOCUS_TOP): return .focusTop
        case UInt32(VIEM_WINDOW_FOCUS_BOTTOM): return .focusBottom
        case UInt32(VIEM_WINDOW_FOCUS_LAST_ACCESSED): return .focusLastAccessed
        case UInt32(VIEM_WINDOW_ROTATE_DOWN): return .rotateDown(count: try required())
        case UInt32(VIEM_WINDOW_ROTATE_UP): return .rotateUp(count: try required())
        case UInt32(VIEM_WINDOW_EXCHANGE): return .exchange(index: index)
        case UInt32(VIEM_WINDOW_MOVE_TO_TOP): return .moveToTop
        case UInt32(VIEM_WINDOW_MOVE_TO_BOTTOM): return .moveToBottom
        case UInt32(VIEM_WINDOW_CLOSE_OTHERS): return .closeOthers
        case UInt32(VIEM_WINDOW_GROW): return .grow(rows: try required())
        case UInt32(VIEM_WINDOW_SHRINK): return .shrink(rows: try required())
        case UInt32(VIEM_WINDOW_RESIZE_INDEXED):
            let target = effect.windowTarget.flatMap { Int(exactly: $0) }
            guard effect.windowTarget == nil || target != nil, effect.windowResizeMode <= 2 else { throw EVCoreFrontendError.invalidHostEffect }
            return .resize(index: target, width: effect.flags & UInt32(VIEM_EX_FRONTEND_VERTICAL) != 0,
                size: index, change: effect.windowResizeMode == 2 ? -1 : Int(effect.windowResizeMode))
        case UInt32(VIEM_WINDOW_SET_HEIGHT): return .setHeight(rows: index)
        case UInt32(VIEM_WINDOW_EQUALIZE_HEIGHTS): return .equalizeHeights
        case UInt32(VIEM_WINDOW_MOVE_TO_LEFT): return .moveToLeft
        case UInt32(VIEM_WINDOW_MOVE_TO_RIGHT): return .moveToRight
        case UInt32(VIEM_WINDOW_GROW_WIDTH): return .growWidth(columns: try required())
        case UInt32(VIEM_WINDOW_SHRINK_WIDTH): return .shrinkWidth(columns: try required())
        case UInt32(VIEM_WINDOW_SET_WIDTH): return .setWidth(columns: index)
        case UInt32(VIEM_WINDOW_EQUALIZE_HEIGHT_ONLY): return .equalizeHeightOnly
        case UInt32(VIEM_WINDOW_EQUALIZE_WIDTH_ONLY): return .equalizeWidthOnly
        default: throw EVCoreFrontendError.invalidHostEffect
        }
    }

    func applyHostEffectBatch(_ batch: EVHostEffectBatch) throws {
        let state = try backend.documentState()
        guard state.document_id == batch.documentID,
              state.document_revision == batch.documentRevision
        else { throw EVCoreFrontendError.staleHostEffect }

        var messages: [String] = []
        var documentRequests: [EVDocumentHostRequest] = []
        var windowRequests: [EVWindowRequest] = []
        for effect in batch.exEffects {
            guard effect.documentID == batch.documentID,
                  effect.documentRevision == batch.documentRevision
            else { throw EVCoreFrontendError.invalidHostEffect }
            if effect.kind == UInt32(VIEM_EX_FRONTEND_WINDOW) {
                windowRequests.append(try windowRequest(from: effect))
            } else if let request = try documentHostRequest(from: effect) {
                documentRequests.append(request)
            } else if let message = try displayMessage(for: effect) {
                messages.append(message)
            }
        }
        if batch.substitutionCount > 0 {
            messages.append(
                batch.substitutionCount == 1
                    ? "1 substitution"
                    : "\(batch.substitutionCount) substitutions"
            )
        }

        // Validate the entire batch before crossing into NSPasteboard so an
        // invalid later record cannot leave a partially applied host turn.
        for write in batch.clipboardWrites {
            guard write.documentID == batch.documentID,
                  write.documentRevision == batch.documentRevision,
                  write.target == UInt32(VIEM_CLIPBOARD_TARGET_CLIPBOARD)
                    || write.target == UInt32(VIEM_CLIPBOARD_TARGET_PRIMARY)
            else { throw EVCoreFrontendError.invalidHostEffect }
        }
        let representations = try batch.clipboardWrites.map { write in
            if let nativeCopyRepresentations { return nativeCopyRepresentations }
            if let json = write.fragmentJSON {
                let fragment = try EVClipboardFragment.decode(json)
                guard (fragment.imagePlainText ?? fragment.plainText) == write.plainText else { throw EVCoreFrontendError.invalidHostEffect }
                return try fragment.representations(json: json)
            }
            return EVClipboardRepresentations(plainText: write.plainText)
        }
        for effect in batch.exEffects {
            try EVSelectionPreferences.apply(effect.options, from: backend)
        }
        for content in representations {
            guard pasteboard.viemWrite(content) else {
                throw EVCoreFrontendError.pasteboardWriteFailed
            }
        }

        if !messages.isEmpty {
            publishHostMessage(messages.joined(separator: "\n"))
        }
        if !windowRequests.isEmpty {
            guard let documentHostEffectHandler else {
                throw EVCoreFrontendError.unsupportedHostEffect
            }
            documentHostEffectHandler.perform(windowRequests: windowRequests, from: self)
        }
        if sourcedLineRequests != nil {
            sourcedLineRequests?.append(contentsOf: documentRequests)
            return
        }
        guard !documentRequests.isEmpty else { return }
        guard let documentHostEffectHandler else {
            throw EVCoreFrontendError.unsupportedHostEffect
        }
        documentHostEffectHandler.perform(documentHostRequests: documentRequests) {
            [weak self] result in
            guard let self else { return }
            switch result {
            case let .success(message):
                if let message, !message.isEmpty { self.publishHostMessage(message) }
                self.refreshPresentation()
            case let .failure(error):
                self.report(error)
                NSSound.beep()
            }
        }
    }

    public func insertFileContents(_ bytes: Data, after: UInt64, expected: EVDocumentPersistenceState) throws {
        guard let session else { throw EVCoreFrontendError.unsupportedHostEffect }
        _ = try session.readFile(bytes, after: after, expected: expected)
        refreshPresentation()
    }

    public func executeSourcedLine(_ text: String, depth: UInt32) throws -> [EVDocumentHostRequest] {
        guard let session, sourcedLineRequests == nil else { throw EVCoreFrontendError.unsupportedHostEffect }
        sourcedLineRequests = []
        defer { sourcedLineRequests = nil }
        let outcome = try session.sourceLine(text, depth: depth)
        refreshPresentation()
        guard outcome.command_status == UInt32(VIEM_COMMAND_STATUS_COMPLETE)
            || outcome.command_status == UInt32(VIEM_COMMAND_STATUS_NONE) else {
            throw EVCoreFrontendError.command(operation: "Execute sourced command", status: outcome.command_status)
        }
        return sourcedLineRequests ?? []
    }

    private func documentHostRequest(
        from effect: EVExHostEffect
    ) throws -> EVDocumentHostRequest? {
        let force = effect.flags & UInt32(VIEM_EX_FRONTEND_FORCE) != 0
        let path = effect.flags & UInt32(VIEM_EX_FRONTEND_HAS_PATH) != 0
            ? effect.text
            : nil
        let kind: EVDocumentHostRequest.Kind
        switch effect.kind {
        case UInt32(VIEM_EX_FRONTEND_READ): kind = .read
        case UInt32(VIEM_EX_FRONTEND_SOURCE): kind = .source
        case UInt32(VIEM_EX_FRONTEND_FILE): kind = .file
        case UInt32(VIEM_EX_FRONTEND_ONLY): kind = .only
        case UInt32(VIEM_EX_FRONTEND_SPLIT): kind = .split
        case UInt32(VIEM_EX_FRONTEND_NEW_PANE): kind = .newPane
        case UInt32(VIEM_EX_FRONTEND_ARGUMENT): kind = .navigateArgument
        case UInt32(VIEM_EX_FRONTEND_EDIT): kind = .edit
        case UInt32(VIEM_EX_FRONTEND_EDIT_NEW_WINDOW): kind = .editNewWindow
        case UInt32(VIEM_EX_FRONTEND_PWD): kind = .printWorkingDirectory
        case UInt32(VIEM_EX_FRONTEND_CHECKTIME): kind = .checkTime
        case UInt32(VIEM_EX_FRONTEND_CD): kind = .changeDirectory
        case UInt32(VIEM_EX_FRONTEND_NEW): kind = .new
        case UInt32(VIEM_EX_FRONTEND_WRITE): kind = .write
        case UInt32(VIEM_EX_FRONTEND_SAVE_AS): kind = .saveAs
        case UInt32(VIEM_EX_FRONTEND_QUIT): kind = .quit
        case UInt32(VIEM_EX_FRONTEND_QUIT_ALL): kind = .quitAll
        case UInt32(VIEM_EX_FRONTEND_CQUIT):
            guard effect.exitStatus != nil else { throw EVCoreFrontendError.invalidHostEffect }
            kind = .cquit
        case UInt32(VIEM_EX_FRONTEND_WRITE_QUIT): kind = .writeQuit
        case UInt32(VIEM_EX_FRONTEND_XIT): kind = .xit
        case UInt32(VIEM_EX_FRONTEND_WRITE_ALL): kind = .writeAll
        case UInt32(VIEM_EX_FRONTEND_MESSAGE),
             UInt32(VIEM_EX_FRONTEND_MARKS),
             UInt32(VIEM_EX_FRONTEND_REGISTERS),
             UInt32(VIEM_EX_FRONTEND_JUMPS),
             UInt32(VIEM_EX_FRONTEND_OPTIONS),
             UInt32(VIEM_EX_FRONTEND_PRINT_LINES):
            return nil
        case UInt32(VIEM_EX_FRONTEND_NORMAL):
            // :normal is executed by the core before publication. Seeing its
            // raw request here would otherwise tempt AppKit to become another
            // command interpreter.
            throw EVCoreFrontendError.unsupportedHostEffect
        default:
            throw EVCoreFrontendError.invalidHostEffect
        }
        let initialHeightRows: Int?
        if let count = effect.windowCount {
            guard kind == .split || kind == .newPane,
                  let rows = Int(exactly: count), rows >= 0
            else { throw EVCoreFrontendError.invalidHostEffect }
            initialHeightRows = rows
        } else {
            initialHeightRows = nil
        }
        return EVDocumentHostRequest(
            kind: kind,
            documentID: effect.documentID,
            documentRevision: effect.documentRevision,
            force: force,
            path: path,
            hardLineRange: effect.hardLineRange,
            initialHeightRows: initialHeightRows,
            verticalSplit: effect.flags & UInt32(VIEM_EX_FRONTEND_VERTICAL) != 0,
            argumentNavigation: effect.argumentNavigation,
            readAfterLine: effect.readAfterLine,
            exitStatus: effect.exitStatus.map { Int32(bitPattern: $0) } ?? 1
        )
    }

    private func displayMessage(for effect: EVExHostEffect) throws -> String? {
        switch effect.kind {
        case UInt32(VIEM_EX_FRONTEND_MESSAGE): return effect.text
        case UInt32(VIEM_EX_FRONTEND_OPTIONS):
            return effect.options.map(formatOption).joined(separator: "  ")
        case UInt32(VIEM_EX_FRONTEND_MARKS):
            let rows = effect.marks.map {
                "\($0.name)  \($0.hardLineIndex + 1)  \($0.graphemeColumn)  \($0.lineText)"
            }
            return (["mark  line  col  text"] + rows).joined(separator: "\n")
        case UInt32(VIEM_EX_FRONTEND_REGISTERS):
            let rows = effect.registers.map {
                "\"\($0.name)   \(visibleRegisterText($0.text))"
            }
            return (["--- Registers ---"] + rows).joined(separator: "\n")
        case UInt32(VIEM_EX_FRONTEND_JUMPS):
            let rows = effect.jumps.map {
                let current = $0.isCurrent ? ">" : " "
                return "\(current) \($0.listIndex)  \($0.hardLineIndex + 1)  \($0.graphemeColumn)  \($0.lineText)"
            }
            return ([" jump  line  col  text"] + rows).joined(separator: "\n")
        case UInt32(VIEM_EX_FRONTEND_PRINT_LINES):
            let numbered = effect.flags & UInt32(VIEM_EX_FRONTEND_NUMBER) != 0
            let listed = effect.flags & UInt32(VIEM_EX_FRONTEND_LIST) != 0
            return effect.textLines.map { line in
                var text = listed
                    ? visibleListText(line.text) + "$"
                    : line.text
                if numbered { text = "\(line.hardLineIndex + 1)\t\(text)" }
                return text
            }.joined(separator: "\n")
        default:
            return nil
        }
    }

    private func formatOption(_ option: EVExOptionEffect) -> String {
        let name: String
        switch option.name {
        case UInt32(VIEM_EX_OPTION_WRAP): name = "wrap"
        case UInt32(VIEM_EX_OPTION_FILE_FORMAT): name = "fileformat"
        case UInt32(VIEM_EX_OPTION_FILE_FORMATS): name = "fileformats"
        case 5: name = "ignorecase"
        case 6: name = "smartcase"
        case 7: name = "wrapscan"
        case UInt32(VIEM_EX_OPTION_HLSEARCH): name = "hlsearch"
        case UInt32(VIEM_EX_OPTION_INCSEARCH): name = "incsearch"
        case UInt32(VIEM_EX_OPTION_TEXTWIDTH): name = "textwidth"
        case UInt32(VIEM_EX_OPTION_AUTOINDENT): name = "autoindent"
        case UInt32(VIEM_EX_OPTION_TABSTOP): name = "tabstop"
        case UInt32(VIEM_EX_OPTION_SHIFTWIDTH): name = "shiftwidth"
        case UInt32(VIEM_EX_OPTION_SOFTTABSTOP): name = "softtabstop"
        case UInt32(VIEM_EX_OPTION_EXPANDTAB): name = "expandtab"
        case UInt32(VIEM_EX_OPTION_SMARTTAB): name = "smarttab"
        case UInt32(VIEM_EX_OPTION_CONTINUE_COMMENTS_ON_ENTER): name = "continuecommentsonenter"
        case UInt32(VIEM_EX_OPTION_CONTINUE_COMMENTS_ON_OPEN_LINE): name = "continuecommentsonopenline"
        case UInt32(VIEM_EX_OPTION_LIST): name = "list"
        case UInt32(VIEM_EX_OPTION_LISTCHARS): name = "listchars"
        case UInt32(VIEM_EX_OPTION_AUTOSELECT): name = "autoselect"
        case UInt32(VIEM_EX_OPTION_KEYMODEL): name = "keymodel"
        case UInt32(VIEM_EX_OPTION_SELECTMODE): name = "selectmode"

        default: name = "option\(option.name)"
        }
        switch option.value {
        case let .boolean(value):
            return value ? name : "no\(name)"
        case let .fileFormat(value):
            return "\(name)=\(fileFormatName(value))"
        case let .fileFormats(values):
            return "\(name)=\(values.map(fileFormatName).joined(separator: ","))"
        case let .number(value):
            if option.name == UInt32(VIEM_EX_OPTION_SOFTTABSTOP) { return "\(name)=\(Int32(bitPattern: value))" }
            return "\(name)=\(value)"
        case let .string(value):
            return "\(name)=\(value)"
        }
    }

    private func fileFormatName(_ value: UInt32) -> String {
        switch value {
        case UInt32(VIEM_FILE_FORMAT_DOS): "dos"
        case UInt32(VIEM_FILE_FORMAT_MAC): "mac"
        default: "unix"
        }
    }

    /// List logical line content without letting literal controls create new
    /// output rows or affect presentation. Hard-line boundaries are added by
    /// the caller; a literal LF remains visibly distinct as ^J.
    private func visibleListText(_ value: String) -> String {
        value.unicodeScalars.map { scalar in
            switch scalar.value {
            case 0...31: return "^" + String(UnicodeScalar(scalar.value + 64)!)
            case 127: return "^?"
            case 128...159: return String(format: "<%02x>", scalar.value)
            default: return String(scalar)
            }
        }.joined()
    }

    private func visibleRegisterText(_ value: String) -> String {
        value
            .replacingOccurrences(of: "\n", with: "^J")
            .replacingOccurrences(of: "\t", with: "^I")
    }

    private func clearCommandOutput() {
        commandOutput = nil
        commandOutputDeadline = nil
        commandOutputTimer?.invalidate()
        commandOutputTimer = nil
    }

    public func dismissCommandOutput() {
        guard commandOutput != nil else { return }
        clearCommandOutput()
        updateStatusBar()
    }

    /// The deadline is checked separately from scheduling so a stale timer
    /// cannot dismiss a replacement message, and tests need no real-time wait.
    func expireCommandOutput(at now: TimeInterval) {
        guard let deadline = commandOutputDeadline, now >= deadline else { return }
        dismissCommandOutput()
    }

    public func handleStatusMessageKey(_ event: NSEvent) {
        dismissCommandOutput()
        editorView.window?.makeFirstResponder(editorView)
        if event.characters == ":",
           event.modifierFlags.intersection([.command, .control, .option]).isEmpty,
           [UInt32(VIEM_MODE_INSERT), UInt32(VIEM_MODE_REPLACE)].contains(viewPresentation.mode) {
            performInput { _ = try session?.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        }
        editorView.keyDown(with: event)
    }

    public func showDocumentMessage(_ message: String) { publishHostMessage(message) }

    /// Reload the selected theme aggregate, including all format style families.
    private func reloadCodeStyleSheet() {
        do {
            try backend.configuration.reloadCurrentTheme()
            publishHostMessage("Reloaded theme \(backend.configuration.currentThemeName ?? "Default")")
        } catch { publishHostMessage(error.localizedDescription) }
    }

    func publishHostMessage(_ message: String) {
        clearCommandOutput()
        commandOutput = message
        commandOutputDeadline = commandOutputClock() + Self.commandOutputDuration
        let timer = Timer(timeInterval: Self.commandOutputDuration, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.expireCommandOutput(at: self.commandOutputClock())
            }
        }
        commandOutputTimer = timer
        RunLoop.main.add(timer, forMode: .common)
        lastErrorMessage = ""
        updateStatusBar()
    }
}
