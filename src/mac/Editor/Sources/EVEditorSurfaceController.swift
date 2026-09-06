import AppKit
import CEvimCore
import EvimAppShell
import Foundation

@MainActor
public final class EVEditorSurfaceController: NSViewController, EVEditorSurface, EVDocumentHostAttachable {
    public var viewController: NSViewController { self }
    public private(set) var statusBarState = EVStatusBarState()
    public var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
    public weak var documentHostEffectHandler: (any EVDocumentHostEffectHandling)?

    let backend: EVCoreDocumentBackend
    private(set) var session: EVCoreViewSession?
    private(set) var formattedSnapshot: EVFormattedSnapshot?
    private(set) var layoutTextSlices: [EVFormattedTextSlice] = []
    private(set) var compositionOverlay: EVCompositionOverlayExport?
    private(set) var compositionTextSlices: [EVCompositionTextSlice] = []
    private(set) var layoutSnapshot: EVLayoutExport?
    var layoutPaint: EVLayoutPaintExport?
    private(set) var commandLine: EVCommandLineExport?
    private(set) var visualSelection: EVVisualSelectionExport?
    private(set) var viewPresentation = EvimViewPresentationV1()
    private(set) var viewportState = EvimViewportStateV1()
    private(set) var documentState = EvimDocumentStateV1()
    private(set) var presentationRefreshCount: UInt64 = 0
    private var showInvisibles = false
    private var lastErrorMessage = ""
    var pasteboard: any EVPasteboardAccess = EVAppKitPasteboardAccess.shared
    var findPasteboard: any EVPasteboardAccess = EVAppKitPasteboardAccess.find

    /// Deterministic native-menu zoom stops. The exact selected value is
    /// stored by core as per-view state; AppKit never derives scale from the
    /// current font or transforms an already laid-out bitmap.
    private static let zoomStops: [Float] = [
        0.25, 0.33, 0.50, 0.67, 0.75, 0.80, 0.90, 1.00,
        1.10, 1.25, 1.50, 1.75, 2.00, 2.50, 3.00, 4.00,
    ]

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

    init(backend: EVCoreDocumentBackend) {
        self.backend = backend
        super.init(nibName: nil, bundle: nil)
        do {
            try attachToCore()
        } catch {
            lastErrorMessage = error.localizedDescription
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }

    public override func loadView() {
        view = EVEditorView(surface: self)
        refreshPresentation()
    }

    public override func viewDidLayout() {
        super.viewDidLayout()
        guard view.bounds.width > 0, view.bounds.height > 0 else { return }
        do {
            let viewportSize = EVEditorView.layoutViewportSize(for: view.bounds.size)
            _ = try session?.resize(width: viewportSize.width, height: viewportSize.height)
            refreshPresentation()
        } catch {
            report(error)
        }
    }

    func attachToCore() throws {
        guard session == nil else { return }
        let size = isViewLoaded ? view.bounds.size : NSSize(width: 920, height: 655)
        let viewportSize = EVEditorView.layoutViewportSize(for: size)
        let attachedSession = try EVCoreViewSession(
            document: backend,
            width: viewportSize.width,
            height: viewportSize.height
        )
        attachedSession.compositionStateDidChange = { [weak self] isActive in
            guard !isActive, let self, self.isViewLoaded else { return }
            self.editorView.coreCompositionDidEnd()
        }
        attachedSession.commandTurnHost = self
        session = attachedSession
        if isViewLoaded { refreshPresentation() }
    }

    func detachFromCore() {
        session?.detach()
        session = nil
        formattedSnapshot = nil
        layoutTextSlices = []
        compositionOverlay = nil
        compositionTextSlices = []
        layoutSnapshot = nil
        layoutPaint = nil
        commandLine = nil
        visualSelection = nil
        viewportState = EvimViewportStateV1()
        documentState = EvimDocumentStateV1()
    }

    func refreshPresentation() {
        guard let session else { return }
        do {
            let nextDocumentState = try backend.documentState()
            let nextFormattedSnapshot = try backend.formattedSnapshot()
            let nextPresentation = try session.presentation()
            let nextViewport = try session.viewportState()
            let nextCompositionOverlay = try session.compositionOverlayExport()
            guard nextFormattedSnapshot.info.identity.document_id == nextDocumentState.document_id,
                  nextFormattedSnapshot.info.identity.document_revision == nextDocumentState.document_revision,
                  nextFormattedSnapshot.info.identity.document_id == nextPresentation.document_id,
                  nextFormattedSnapshot.info.identity.document_revision == nextPresentation.document_revision,
                  nextFormattedSnapshot.info.identity.document_id == nextViewport.document_id,
                  nextFormattedSnapshot.info.identity.document_revision == nextViewport.document_revision
            else {
                throw EVCoreFrontendError.core(
                    operation: "Match formatted snapshot",
                    status: UInt32(EVIM_STATUS_STALE_REVISION)
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
                        status: UInt32(EVIM_STATUS_STALE_REVISION)
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
                        status: UInt32(EVIM_STATUS_STALE_REVISION)
                    )
                }
                let paint = try session.layoutPaintExport()
                guard paint.info.identity.isSameLayout(as: nextLayoutSnapshot.info.identity) else {
                    throw EVCoreFrontendError.core(
                        operation: "Match layout paint",
                        status: UInt32(EVIM_STATUS_STALE_REVISION)
                    )
                }
                nextLayoutPaint = paint
                let ranges = layoutUTF8Ranges(
                    for: nextLayoutSnapshot,
                    formattedLength: nextCompositionOverlay?.info.utf8_length
                        ?? nextFormattedSnapshot.info.utf8_length
                )
                if let nextCompositionOverlay {
                    nextLayoutTextSlices = []
                    nextCompositionTextSlices = try ranges.map {
                        try session.compositionTextSlice(in: $0, overlay: nextCompositionOverlay)
                    }
                } else {
                    nextLayoutTextSlices = try ranges.map {
                        try backend.formattedSlice(in: $0, snapshot: nextFormattedSnapshot)
                    }
                    nextCompositionTextSlices = []
                }
            } else {
                nextLayoutPaint = nil
                nextLayoutTextSlices = []
                nextCompositionTextSlices = []
            }

            let nextCommandLine = try session.commandLineExport()
            guard nextCommandLine.info.identity.document_id
                    == nextFormattedSnapshot.info.identity.document_id,
                  nextCommandLine.info.identity.document_revision
                    == nextFormattedSnapshot.info.identity.document_revision
            else {
                throw EVCoreFrontendError.core(
                    operation: "Match command line",
                    status: UInt32(EVIM_STATUS_STALE_REVISION)
                )
            }
            let nextVisualSelection: EVVisualSelectionExport?
            do {
                nextVisualSelection = try session.visualSelectionExport()
            } catch EVCoreFrontendError.core(_, let status)
                where status == UInt32(EVIM_STATUS_OUTSIDE_LAYOUT_COVERAGE)
                    || status == UInt32(EVIM_STATUS_LAYOUT_UNAVAILABLE)
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
                        status: UInt32(EVIM_STATUS_STALE_REVISION)
                    )
                }
            }

            documentState = nextDocumentState
            formattedSnapshot = nextFormattedSnapshot
            compositionOverlay = nextCompositionOverlay
            viewPresentation = nextPresentation
            viewportState = nextViewport
            layoutSnapshot = nextLayoutSnapshot
            layoutPaint = nextLayoutPaint
            layoutTextSlices = nextLayoutTextSlices
            compositionTextSlices = nextCompositionTextSlices
            commandLine = nextCommandLine
            visualSelection = nextVisualSelection
            presentationRefreshCount &+= 1
            updateStatusBar()
            if isViewLoaded {
                editorView.applyPresentation()
            }
        } catch {
            report(error)
        }
    }

    func performInput(_ operation: () throws -> Void) {
        do {
            lastErrorMessage = ""
            let refreshCountBeforeInput = presentationRefreshCount
            try operation()
            if presentationRefreshCount == refreshCountBeforeInput {
                refreshPresentation()
            }
        } catch {
            report(error)
            NSSound.beep()
        }
    }

    func report(_ error: Error) {
        lastErrorMessage = error.localizedDescription
        updateStatusBar()
    }

    func sharedDocumentDidChange(originatingViewIDs: Set<EvimViewId>) {
        if originatingViewIDs.count != 1 || !originatingViewIDs.contains(session?.viewID ?? 0) {
            session?.noteExternalDocumentChange()
        }
        refreshPresentation()
    }

    func requestVerticalViewport(top: CGFloat) {
        guard let session else { return }
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

    public func perform(menuCommand: EVMenuCommand, sender: Any?) {
        guard let session else { return }
        switch menuCommand {
        case .undo:
            performInput { _ = try session.undo() }
        case .redo:
            performInput { _ = try session.redo() }
        case .copy:
            copyOrCutSelection(cut: false)
        case .cut:
            copyOrCutSelection(cut: true)
        case .paste, .pasteAndMatchStyle:
            pastePlainText()
        case .delete:
            performInput {
                if self.isVisualMode {
                    _ = try self.sendCommandCharacter("d", session: session)
                } else if self.viewPresentation.mode == UInt32(EVIM_MODE_NORMAL) {
                    _ = try self.sendCommandCharacter("x", session: session)
                } else {
                    _ = try session.sendKey(kind: UInt32(EVIM_KEY_DELETE))
                }
            }
        case .selectAll:
            performInput { try self.sendNormalSequence(["g", "g", "V", "G"], session: session) }
        case .selectWord:
            performInput { try self.sendNormalSequence(["v", "i", "w"], session: session) }
        case .selectSentence:
            performInput { try self.sendNormalSequence(["v", "i", "s"], session: session) }
        case .selectParagraph:
            performInput { try self.sendNormalSequence(["v", "i", "p"], session: session) }
        case .selectHardLine:
            performInput { try self.sendNormalSequence(["V"], session: session) }
        case .selectVisualRow:
            performInput { try self.sendNormalSequence(["g", "0", "v", "g", "$"], session: session) }
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
            guard isVisualMode else { NSSound.beep(); return }
            performInput { _ = try session.revealSelection() }
        case .makeUppercase:
            guard isVisualMode else { NSSound.beep(); return }
            performInput { _ = try self.sendCommandCharacter("U", session: session) }
        case .makeLowercase:
            guard isVisualMode else { NSSound.beep(); return }
            performInput { _ = try self.sendCommandCharacter("u", session: session) }
        case .toggleCase:
            guard isVisualMode || viewPresentation.mode == UInt32(EVIM_MODE_NORMAL) else {
                NSSound.beep()
                return
            }
            performInput { _ = try self.sendCommandCharacter("~", session: session) }
        case .wordWrap:
            performInput { _ = try session.setWrap(!self.wrapEnabled) }
        case .wrapAtWordBoundaries:
            performInput { _ = try session.setLinebreak(!self.linebreakEnabled) }
        case .lineEndingUnix:
            setFileFormat(UInt32(EVIM_FILE_FORMAT_UNIX), session: session)
        case .lineEndingWindows:
            setFileFormat(UInt32(EVIM_FILE_FORMAT_DOS), session: session)
        case .lineEndingClassicMac:
            setFileFormat(UInt32(EVIM_FILE_FORMAT_MAC), session: session)
        case .showInvisibleCharacters:
            showInvisibles.toggle()
            editorView.needsDisplay = true
        case .zoomIn:
            setZoom(adjacentTo: zoomScale, increasing: true, session: session)
        case .zoomOut:
            setZoom(adjacentTo: zoomScale, increasing: false, session: session)
        case .actualSize:
            setZoom(1, session: session)
        case .bold:
            toggleSemanticStyle(UInt32(EVIM_SEMANTIC_STYLE_STRONG), session: session)
        case .italic:
            toggleSemanticStyle(UInt32(EVIM_SEMANTIC_STYLE_EMPHASIS), session: session)
        case .editCharacterStyles:
            EVStyleEditorCoordinator.shared.show(
                document: self,
                preferredStyle: .character,
                sender: sender
            )
        case .editParagraphStyles:
            EVStyleEditorCoordinator.shared.show(
                document: self,
                preferredStyle: .paragraph,
                sender: sender
            )
        case .editDocumentStyles:
            EVStyleEditorCoordinator.shared.show(
                document: self,
                preferredStyle: .document,
                sender: sender
            )
        case .save:
            (view.window?.windowController?.document as? NSDocument)?.save(sender)
        case .saveAs:
            (view.window?.windowController?.document as? NSDocument)?.saveAs(sender)
        case .pageSetup:
            NSPageLayout().runModal()
        case .printDocument:
            // Printing is intentionally unavailable until core exposes a
            // paginated projection. The unpaginated viewport is not a print
            // document and must not be submitted as one.
            return
        default:
            // Source-backed rich formatting is disabled for the current plain
            // text adapter; unsupported menu items validate disabled below.
            NSSound.beep()
        }
    }

    public func presentation(for menuCommand: EVMenuCommand) -> EVMenuItemPresentation {
        switch menuCommand {
        case .save, .saveAs, .pageSetup,
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
        case .copy, .cut:
            // The core owns the logical Visual selection even when none of
            // its geometry is materialized in the current viewport.
            EVMenuItemPresentation(isEnabled: isVisualMode)
        case .paste, .pasteAndMatchStyle:
            EVMenuItemPresentation(isEnabled: pasteboard.evimCanReadString())
        case .delete:
            .enabled
        case .makeUppercase, .makeLowercase:
            EVMenuItemPresentation(isEnabled: isVisualMode)
        case .toggleCase:
            EVMenuItemPresentation(
                isEnabled: isVisualMode || viewPresentation.mode == UInt32(EVIM_MODE_NORMAL)
            )
        case .wordWrap:
            EVMenuItemPresentation(isEnabled: true, state: wrapEnabled ? .on : .off)
        case .wrapAtWordBoundaries:
            EVMenuItemPresentation(isEnabled: wrapEnabled, state: linebreakEnabled ? .on : .off)
        case .lineEndingUnix:
            fileFormatPresentation(UInt32(EVIM_FILE_FORMAT_UNIX))
        case .lineEndingWindows:
            fileFormatPresentation(UInt32(EVIM_FILE_FORMAT_DOS))
        case .lineEndingClassicMac:
            fileFormatPresentation(UInt32(EVIM_FILE_FORMAT_MAC))
        case .showInvisibleCharacters:
            EVMenuItemPresentation(isEnabled: true, state: showInvisibles ? .on : .off)
        case .useSelectionForFind:
            EVMenuItemPresentation(
                isEnabled: selectedUTF8Ranges().contains(where: { !$0.isEmpty })
            )
        case .jumpToSelection:
            EVMenuItemPresentation(isEnabled: isVisualMode)
        case .zoomIn:
            EVMenuItemPresentation(
                isEnabled: zoomScale < (Self.zoomStops.last ?? zoomScale)
            )
        case .zoomOut:
            EVMenuItemPresentation(
                isEnabled: zoomScale > (Self.zoomStops.first ?? zoomScale)
            )
        case .actualSize:
            .enabled
        case .bold:
            semanticStyleMenuPresentation(
                UInt32(EVIM_SEMANTIC_STYLE_STRONG),
                session: session
            )
        case .italic:
            semanticStyleMenuPresentation(
                UInt32(EVIM_SEMANTIC_STYLE_EMPHASIS),
                session: session
            )
        case .editCharacterStyles, .editParagraphStyles, .editDocumentStyles:
            .enabled
        case .printDocument:
            .disabled
        default:
            .disabled
        }
    }

    var isVisualMode: Bool {
        switch viewPresentation.mode {
        case UInt32(EVIM_MODE_VISUAL_CHARACTER), UInt32(EVIM_MODE_VISUAL_LINE), UInt32(EVIM_MODE_VISUAL_BLOCK):
            true
        default:
            false
        }
    }

    var isVisualBlockMode: Bool {
        viewPresentation.mode == UInt32(EVIM_MODE_VISUAL_BLOCK)
    }

    var wrapEnabled: Bool {
        viewportState.flags & UInt32(EVIM_VIEWPORT_STATE_WRAP) != 0
    }

    var linebreakEnabled: Bool {
        viewportState.flags & UInt32(EVIM_VIEWPORT_STATE_LINEBREAK) != 0
    }

    var zoomScale: Float {
        viewportState.scale.isFinite && viewportState.scale > 0
            ? viewportState.scale
            : 1
    }

    var canUndo: Bool {
        documentState.flags & UInt32(EVIM_DOCUMENT_STATE_CAN_UNDO) != 0
    }

    var canRedo: Bool {
        documentState.flags & UInt32(EVIM_DOCUMENT_STATE_CAN_REDO) != 0
    }

    var showInvisibleCharactersEnabled: Bool { showInvisibles }

    func formattedText(in utf8Range: Range<Int>) -> String? {
        guard let snapshot = formattedSnapshot,
              utf8Range.lowerBound >= 0,
              utf8Range.lowerBound <= utf8Range.upperBound,
              utf8Range.upperBound <= formattedUTF8Length,
              let lower = UInt64(exactly: utf8Range.lowerBound),
              let upper = UInt64(exactly: utf8Range.upperBound)
        else { return nil }
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

    func formattedPointInfo(atUTF8Offset offset: Int) -> EvimFormattedPointInfoV1? {
        guard let snapshot = formattedSnapshot,
              offset >= 0,
              offset <= formattedUTF8Length,
              let converted = UInt64(exactly: offset)
        else { return nil }
        return try? backend.formattedPointInfo(atUTF8Offset: converted, snapshot: snapshot)
    }

    func selectedUTF8Ranges() -> [Range<Int>] {
        guard isVisualMode,
              let visualSelection,
              visualSelection.info.identity.kind != UInt32(EVIM_VISUAL_SELECTION_KIND_NONE)
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
        let ranges = selectedUTF8Ranges()
        guard !ranges.isEmpty else { return nil }
        let pieces = ranges.compactMap(formattedText(in:))
        guard pieces.count == ranges.count else { return nil }
        if visualSelection?.info.identity.kind == UInt32(EVIM_VISUAL_SELECTION_KIND_BLOCK) {
            return pieces.joined(separator: "\n")
        }
        return pieces.joined()
    }

    private func copyOrCutSelection(cut: Bool) {
        guard isVisualMode, let session else { NSSound.beep(); return }
        performInput {
            _ = try self.sendCommandCharacter("\"", session: session)
            _ = try self.sendCommandCharacter("+", session: session)
            let outcome = try self.sendCommandCharacter(cut ? "d" : "y", session: session)
            guard outcome.command_status == UInt32(EVIM_COMMAND_STATUS_COMPLETE) else {
                throw EVCoreFrontendError.command(
                    operation: cut ? "Cut" : "Copy",
                    status: outcome.command_status
                )
            }
        }
    }

    private func useCurrentSelectionForFind(session: EVCoreViewSession) {
        guard let selection = visualSelection,
              selection.info.identity.kind != UInt32(EVIM_VISUAL_SELECTION_KIND_NONE),
              let text = selectionText(),
              !text.isEmpty
        else {
            NSSound.beep()
            return
        }
        performInput {
            _ = try session.useSelectionForFind(selection.info.identity)
            self.findPasteboard.evimClearContents()
            guard self.findPasteboard.evimSetString(text) else {
                throw EVCoreFrontendError.pasteboardWriteFailed
            }
        }
    }

    private func setZoom(
        adjacentTo current: Float,
        increasing: Bool,
        session: EVCoreViewSession
    ) {
        let epsilon: Float = 0.0001
        let target: Float?
        if increasing {
            target = Self.zoomStops.first(where: { $0 > current + epsilon })
        } else {
            target = Self.zoomStops.last(where: { $0 < current - epsilon })
        }
        guard let target else { return }
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
              value.struct_size >= UInt32(MemoryLayout<EvimSemanticStylePresentationV1>.size),
              value.style == style,
              value.flags & UInt32(EVIM_SEMANTIC_STYLE_HAS_ACTIVE_RANGE) != 0,
              value.selection.struct_size
                >= UInt32(MemoryLayout<EvimLogicalSelectionIdentityV1>.size)
        else { return .disabled }

        switch value.state {
        case UInt32(EVIM_SEMANTIC_STYLE_STATE_OFF):
            return EVMenuItemPresentation(
                isEnabled: value.flags & UInt32(EVIM_SEMANTIC_STYLE_CAN_SET) != 0,
                state: .off
            )
        case UInt32(EVIM_SEMANTIC_STYLE_STATE_ON):
            return EVMenuItemPresentation(
                isEnabled: value.flags & UInt32(EVIM_SEMANTIC_STYLE_CAN_CLEAR) != 0,
                state: .on
            )
        case UInt32(EVIM_SEMANTIC_STYLE_STATE_MIXED):
            return EVMenuItemPresentation(
                isEnabled: value.flags & UInt32(EVIM_SEMANTIC_STYLE_CAN_SET) != 0,
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
                    >= UInt32(MemoryLayout<EvimSemanticStylePresentationV1>.size),
                  value.style == style,
                  value.flags & UInt32(EVIM_SEMANTIC_STYLE_HAS_ACTIVE_RANGE) != 0,
                  value.selection.struct_size
                    >= UInt32(MemoryLayout<EvimLogicalSelectionIdentityV1>.size)
            else {
                NSSound.beep()
                return
            }

            let enabled: Bool
            let requiredCapability: UInt32
            switch value.state {
            case UInt32(EVIM_SEMANTIC_STYLE_STATE_OFF):
                enabled = true
                requiredCapability = UInt32(EVIM_SEMANTIC_STYLE_CAN_SET)
            case UInt32(EVIM_SEMANTIC_STYLE_STATE_ON):
                enabled = false
                requiredCapability = UInt32(EVIM_SEMANTIC_STYLE_CAN_CLEAR)
            case UInt32(EVIM_SEMANTIC_STYLE_STATE_MIXED):
                enabled = true
                requiredCapability = UInt32(EVIM_SEMANTIC_STYLE_CAN_SET)
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
        guard pasteboard.evimString() != nil,
              let session
        else {
            NSSound.beep()
            return
        }
        performInput {
            switch self.viewPresentation.mode {
            case UInt32(EVIM_MODE_INSERT),
                 UInt32(EVIM_MODE_REPLACE),
                 UInt32(EVIM_MODE_COMMAND_LINE):
                _ = try session.sendKey(
                    kind: UInt32(EVIM_KEY_CONTROL_CHARACTER),
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
        if viewPresentation.mode != UInt32(EVIM_MODE_NORMAL) {
            _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE))
        }
        for character in characters {
            _ = try sendCommandCharacter(character, session: session)
        }
    }

    private func sendCommandCharacter(
        _ character: Character,
        session: EVCoreViewSession
    ) throws -> EvimCoreOutcomeV1 {
        guard character.unicodeScalars.count == 1,
              let scalar = character.unicodeScalars.first
        else {
            return try session.sendText(String(character))
        }
        return try session.sendKey(
            kind: UInt32(EVIM_KEY_CHARACTER),
            codepoint: scalar.value
        )
    }

    private func setFileFormat(_ target: UInt32, session: EVCoreViewSession) {
        let expected = documentState
        performInput { _ = try session.setFileFormat(target, expected: expected) }
    }

    private func fileFormatPresentation(_ target: UInt32) -> EVMenuItemPresentation {
        EVMenuItemPresentation(
            isEnabled: true,
            state: documentState.file_format == target ? .on : .off
        )
    }

    private func historyTitle(prefix: String, category: UInt32) -> String {
        let action = switch category {
        case UInt32(EVIM_HISTORY_ACTION_CATEGORY_TEXT): "Text Change"
        case UInt32(EVIM_HISTORY_ACTION_CATEGORY_STYLE): "Style Change"
        case UInt32(EVIM_HISTORY_ACTION_CATEGORY_FILE_FORMAT): "Line Endings"
        case UInt32(EVIM_HISTORY_ACTION_CATEGORY_HARD_LINE_TRANSFER): "Move Lines"
        case UInt32(EVIM_HISTORY_ACTION_CATEGORY_HARD_LINE_SOURCE_RESTORATION): "Restore Lines"
        case UInt32(EVIM_HISTORY_ACTION_CATEGORY_SOURCE_METADATA): "Source Metadata"
        case UInt32(EVIM_HISTORY_ACTION_CATEGORY_MIXED): "Changes"
        default: ""
        }
        return action.isEmpty ? prefix : "\(prefix) \(action)"
    }

    private func updateStatusBar() {
        let point = Int(exactly: viewPresentation.cursor_utf8_offset)
            .flatMap(formattedPointInfo(atUTF8Offset:))
        let line = point.flatMap { Int(exactly: $0.hard_line_index) }.map { $0 + 1 } ?? 1
        let column = point.flatMap { Int(exactly: $0.grapheme_column) }.map { $0 + 1 } ?? 1
        statusBarState = EVStatusBarState(
            mode: modeLabel(viewPresentation.mode),
            message: lastErrorMessage,
            location: "Ln \(line), Col \(column)",
            encoding: backend.encodingLabel,
            lineEnding: backend.lineEndingLabel,
            format: backend.formatLabel
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
        for row in snapshot.rows
            where row.flags & UInt32(EVIM_VISUAL_ROW_WRAPS_TO_NEXT) == 0
                && row.hard_line_end < formattedLength
        {
            intervals.append(row.hard_line_end ..< row.hard_line_end + 1)
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
        case UInt32(EVIM_MODE_INSERT): "INSERT"
        case UInt32(EVIM_MODE_REPLACE): "REPLACE"
        case UInt32(EVIM_MODE_VISUAL_CHARACTER): "VISUAL"
        case UInt32(EVIM_MODE_VISUAL_LINE): "VISUAL LINE"
        case UInt32(EVIM_MODE_VISUAL_BLOCK): "VISUAL BLOCK"
        case UInt32(EVIM_MODE_COMMAND_LINE): "COMMAND"
        default: "NORMAL"
        }
    }
}

@MainActor
extension EVEditorSurfaceController: EVCommandTurnHost {
    func clipboardSnapshotsForCommandTurn() -> [EVClipboardTurnSnapshot] {
        let text = pasteboard.evimString()
        let generation = text == nil ? 0 : pasteboard.evimGeneration
        let writable = pasteboard.evimIsWritable
        return [
            EVClipboardTurnSnapshot(
                target: UInt32(EVIM_CLIPBOARD_TARGET_CLIPBOARD),
                generation: generation,
                plainText: text,
                isWritable: writable
            ),
            EVClipboardTurnSnapshot(
                target: UInt32(EVIM_CLIPBOARD_TARGET_PRIMARY),
                generation: generation,
                plainText: text,
                isWritable: writable
            ),
        ]
    }

    func applyHostEffectBatch(_ batch: EVHostEffectBatch) throws {
        let state = try backend.documentState()
        guard state.document_id == batch.documentID,
              state.document_revision == batch.documentRevision
        else { throw EVCoreFrontendError.staleHostEffect }

        var messages: [String] = []
        var documentRequests: [EVDocumentHostRequest] = []
        for effect in batch.exEffects {
            guard effect.documentID == batch.documentID,
                  effect.documentRevision == batch.documentRevision
            else { throw EVCoreFrontendError.invalidHostEffect }
            if let request = try documentHostRequest(from: effect) {
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
                  write.target == UInt32(EVIM_CLIPBOARD_TARGET_CLIPBOARD)
                    || write.target == UInt32(EVIM_CLIPBOARD_TARGET_PRIMARY)
            else { throw EVCoreFrontendError.invalidHostEffect }
        }
        for write in batch.clipboardWrites {
            pasteboard.evimClearContents()
            guard pasteboard.evimSetString(write.plainText) else {
                throw EVCoreFrontendError.pasteboardWriteFailed
            }
        }

        if !messages.isEmpty {
            publishHostMessage(messages.joined(separator: "\n"))
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

    private func documentHostRequest(
        from effect: EVExHostEffect
    ) throws -> EVDocumentHostRequest? {
        let force = effect.flags & UInt32(EVIM_EX_FRONTEND_FORCE) != 0
        let path = effect.flags & UInt32(EVIM_EX_FRONTEND_HAS_PATH) != 0
            ? effect.text
            : nil
        let kind: EVDocumentHostRequest.Kind
        switch effect.kind {
        case UInt32(EVIM_EX_FRONTEND_EDIT): kind = .edit
        case UInt32(EVIM_EX_FRONTEND_NEW): kind = .new
        case UInt32(EVIM_EX_FRONTEND_WRITE): kind = .write
        case UInt32(EVIM_EX_FRONTEND_SAVE_AS): kind = .saveAs
        case UInt32(EVIM_EX_FRONTEND_QUIT): kind = .quit
        case UInt32(EVIM_EX_FRONTEND_QUIT_ALL): kind = .quitAll
        case UInt32(EVIM_EX_FRONTEND_WRITE_QUIT): kind = .writeQuit
        case UInt32(EVIM_EX_FRONTEND_XIT): kind = .xit
        case UInt32(EVIM_EX_FRONTEND_WRITE_ALL): kind = .writeAll
        case UInt32(EVIM_EX_FRONTEND_MARKS),
             UInt32(EVIM_EX_FRONTEND_REGISTERS),
             UInt32(EVIM_EX_FRONTEND_JUMPS),
             UInt32(EVIM_EX_FRONTEND_OPTIONS),
             UInt32(EVIM_EX_FRONTEND_PRINT_LINES):
            return nil
        case UInt32(EVIM_EX_FRONTEND_NORMAL):
            // :normal is executed by the core before publication. Seeing its
            // raw request here would otherwise tempt AppKit to become another
            // command interpreter.
            throw EVCoreFrontendError.unsupportedHostEffect
        default:
            throw EVCoreFrontendError.invalidHostEffect
        }
        return EVDocumentHostRequest(
            kind: kind,
            documentID: effect.documentID,
            documentRevision: effect.documentRevision,
            force: force,
            path: path,
            hardLineRange: effect.hardLineRange
        )
    }

    private func displayMessage(for effect: EVExHostEffect) throws -> String? {
        switch effect.kind {
        case UInt32(EVIM_EX_FRONTEND_OPTIONS):
            return effect.options.map(formatOption).joined(separator: "  ")
        case UInt32(EVIM_EX_FRONTEND_MARKS):
            let rows = effect.marks.map {
                "\($0.name)  \($0.hardLineIndex + 1)  \($0.graphemeColumn)  \($0.lineText)"
            }
            return (["mark  line  col  text"] + rows).joined(separator: "\n")
        case UInt32(EVIM_EX_FRONTEND_REGISTERS):
            let rows = effect.registers.map {
                "\"\($0.name)   \(visibleRegisterText($0.text))"
            }
            return (["--- Registers ---"] + rows).joined(separator: "\n")
        case UInt32(EVIM_EX_FRONTEND_JUMPS):
            let rows = effect.jumps.map {
                let current = $0.isCurrent ? ">" : " "
                return "\(current) \($0.listIndex)  \($0.hardLineIndex + 1)  \($0.graphemeColumn)  \($0.lineText)"
            }
            return ([" jump  line  col  text"] + rows).joined(separator: "\n")
        case UInt32(EVIM_EX_FRONTEND_PRINT_LINES):
            let numbered = effect.flags & UInt32(EVIM_EX_FRONTEND_NUMBER) != 0
            let listed = effect.flags & UInt32(EVIM_EX_FRONTEND_LIST) != 0
            return effect.textLines.map { line in
                var text = listed
                    ? line.text.replacingOccurrences(of: "\t", with: "^I") + "$"
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
        case UInt32(EVIM_EX_OPTION_WRAP): name = "wrap"
        case UInt32(EVIM_EX_OPTION_LINEBREAK): name = "linebreak"
        case UInt32(EVIM_EX_OPTION_FILE_FORMAT): name = "fileformat"
        case UInt32(EVIM_EX_OPTION_FILE_FORMATS): name = "fileformats"
        default: name = "option\(option.name)"
        }
        switch option.value {
        case let .boolean(value):
            return value ? name : "no\(name)"
        case let .fileFormat(value):
            return "\(name)=\(fileFormatName(value))"
        case let .fileFormats(values):
            return "\(name)=\(values.map(fileFormatName).joined(separator: ","))"
        }
    }

    private func fileFormatName(_ value: UInt32) -> String {
        switch value {
        case UInt32(EVIM_FILE_FORMAT_DOS): "dos"
        case UInt32(EVIM_FILE_FORMAT_MAC): "mac"
        default: "unix"
        }
    }

    private func visibleRegisterText(_ value: String) -> String {
        value
            .replacingOccurrences(of: "\n", with: "^J")
            .replacingOccurrences(of: "\t", with: "^I")
    }

    private func publishHostMessage(_ message: String) {
        lastErrorMessage = message
        updateStatusBar()
    }
}
