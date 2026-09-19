import AppKit
import CViemCore
import ViemAppShell
import ViemCoreTextProvider
import Foundation

public enum EVCoreFrontendError: LocalizedError, Equatable {
    case core(operation: String, status: UInt32)
    case command(operation: String, status: UInt32)
    case invalidUTF8
    case unavailableLayout
    case pasteboardWriteFailed
    case invalidHostEffect
    case staleHostEffect
    case unsupportedHostEffect

    public var errorDescription: String? {
        switch self {
        case let .core(operation, status):
            switch status {
            case UInt32(VIEM_STATUS_VERIFICATION_FAILED):
                "This edit cannot preserve the format's text and structure."
            case UInt32(VIEM_STATUS_AMBIGUOUS_PROJECTION):
                "This edit is not supported for the selected content structure."
            case UInt32(VIEM_STATUS_UNREPRESENTABLE_CHARACTER):
                "This character cannot be represented in the document's encoding."
            case UInt32(VIEM_STATUS_UNSUPPORTED_OPERATION):
                "This operation is not supported for the current format or selection."
            case UInt32(VIEM_STATUS_POLICY_REQUIRED):
                "This change needs a format or encoding policy before it can be applied."
            default:
                "\(operation) failed (Viem core status \(status))."
            }
        case let .command(operation, status):
            status == UInt32(VIEM_COMMAND_STATUS_READ_ONLY)
                ? "E45: readonly option is set (use ! to override)"
                : "\(operation) was rejected (Viem command status \(status))."
        case .invalidUTF8:
            "The formatted projection was not valid UTF-8."
        case .unavailableLayout:
            "The current view has no exact layout snapshot."
        case .pasteboardWriteFailed:
            "The system pasteboard did not accept the copied text."
        case .invalidHostEffect:
            "The core returned an invalid host-effect batch."
        case .staleHostEffect:
            "The host effect referred to an older document revision."
        case .unsupportedHostEffect:
            "The requested host operation is not supported."
        }
    }
}

struct EVLayoutExport {
    var info: ViemLayoutSnapshotInfoV1
    var rows: [ViemVisualRowV1]
    var clusters: [ViemPositionedClusterV1]
    var carets: [ViemPositionedCaretV1]
    var decorations: [ViemLayoutDecorationV1] = []
    var decorationLabels: [UInt8] = []
    var whitespace = EVWhitespaceMarkerExport()
}

@MainActor
public final class EVCoreDocumentBackend: EVDocumentBackend {
    public var sourceDidChange: (() -> Void)?
    public var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)?

    public var persistenceState: EVDocumentPersistenceState {
        Self.persistenceState(from: currentDocumentState)
    }

    public var sourceFormat: EVSourceFormat {
        Self.sourceFormat(from: currentDocumentState)
    }

    private(set) var core: ViemCoreHandle = 0
    private var source = Data()
    private var typeName = "public.plain-text"
    private var openingFilename = ""
    private var allowAutomaticCode = false
    private var recoveryInterpretation: EVRecoverySnapshot?
    private(set) var currentDocumentState = ViemDocumentStateV1()
    private(set) var formattedAccessCounters = EVFormattedAccessCounters()
    private var surfaces: [WeakSurface] = []
    private var isRefreshingLayoutSurfaces = false
    private var isRefreshingSurfaces = false
    private var pendingSourceChangeOrigins: [ViemViewId] = []

    let configuration: EVConfigurationStore
    public private(set) var configurationWarning: String?
    private var codeObservers: [NSObjectProtocol] = []
    private var syntaxTimer: Timer?
    private let syntaxDiagnosticSource = UUID().uuidString
    private var lastSyntaxDiagnosticRefresh: TimeInterval = -.infinity

    public init(configuration: EVConfigurationStore? = nil) {
        let configuration = configuration ?? .shared
        self.configuration = configuration
        do {
            try createCore()
        } catch {
            assertionFailure("Unable to create the initial Viem core: \(error)")
        }
        for name in [Notification.Name.viemCodePreferencesDidChange, .viemGlobalCodeStyleDidChange] {
            codeObservers.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] notification in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    if notification.name == .viemCodePreferencesDidChange { self.configureSyntax() }
                    else if self.sourceFormat == .code { self.configurationWarning = self.configuration.lastError }
                    self.pollSyntax()
                    self.refreshSyntaxDiagnostics()
                }
            })
        }
        syntaxTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.pollSyntax() }
        }
        if let syntaxTimer { RunLoop.main.add(syntaxTimer, forMode: .common) }
    }

    /// A replacement has no observers, timers, surfaces, or persistence
    /// callbacks until its fully configured core is transferred to the owner.
    private init(preparing source: Data, typeName: String, filename: String,
                 allowAutomaticCode: Bool, recoveryInterpretation: EVRecoverySnapshot?,
                 configuration: EVConfigurationStore) throws {
        self.configuration = configuration
        self.source = source
        self.typeName = typeName
        self.openingFilename = filename
        self.allowAutomaticCode = allowAutomaticCode
        self.recoveryInterpretation = recoveryInterpretation
        try createCore(publishDiagnostics: false)
    }

    deinit {
        let source = syntaxDiagnosticSource
        Task { @MainActor in EVCodePreferences.shared.reportLoadDiagnostics([], source: source) }
        syntaxTimer?.invalidate()
        for observer in codeObservers { NotificationCenter.default.removeObserver(observer) }
        if core != 0 {
            _ = viem_core_destroy(core)
        }
    }

    public func makeEditorSurface() -> any EVEditorSurface {
        let surface = EVEditorSurfaceController(backend: self)
        surfaces.removeAll { $0.value == nil }
        surfaces.append(WeakSurface(surface))
        return surface
    }

    func alternateStyleEditorSurface(excluding closingSurface: EVEditorSurfaceController) -> EVEditorSurfaceController? {
        surfaces.removeAll { $0.value == nil }
        return surfaces.compactMap(\.value).first { $0 !== closingSurface && $0.session != nil }
    }

    public func read(source: Data, typeName: String) throws {
        try read(source: source, typeName: typeName, filename: "", allowAutomaticCode: false)
    }

    public func updateFilename(_ filename: String) {
        guard openingFilename != filename else { return }
        openingFilename = filename
        guard core != 0 else { return }
        let bytes = Array(filename.utf8)
        let status = bytes.withUnsafeBufferPointer {
            viem_core_redetect_code_language(core, $0.baseAddress, UInt64($0.count))
        }
        if status != Status.ok {
            configurationWarning = "Unable to update syntax filename (\(status))."
        }
        pollSyntax()
    }

    public func read(source: Data, typeName: String, filename: String?, allowAutomaticCode: Bool) throws {
        let replacement = try EVCoreDocumentBackend(
            preparing: source, typeName: typeName, filename: filename ?? "",
            allowAutomaticCode: allowAutomaticCode, recoveryInterpretation: recoveryInterpretation,
            configuration: configuration
        )
        let liveSurfaces = surfaces.compactMap(\.value)
        let replacementSessions = try liveSurfaces.map {
            try $0.prepareReplacementSession(for: replacement)
        }
        // Parsing, configuration, and every replacement view must succeed
        // before the old source, history, or sessions can be discarded. A
        // failed close likewise leaves the original sessions attached.
        if core != 0 {
            let status = viem_core_destroy(core)
            guard status == Status.ok else {
                throw EVCoreFrontendError.core(operation: "Close document", status: status)
            }
        }
        for surface in liveSurfaces {
            surface.detachFromCore()
        }
        core = replacement.core
        replacement.core = 0
        openingFilename = replacement.openingFilename
        self.allowAutomaticCode = replacement.allowAutomaticCode
        self.typeName = replacement.typeName
        configurationWarning = replacement.configurationWarning
        installDocumentState(replacement.currentDocumentState)
        for (surface, session) in zip(liveSurfaces, replacementSessions) {
            surface.installPreparedSession(session)
        }
        refreshSyntaxDiagnostics()
    }

    public func recoverySnapshot() throws -> EVRecoverySnapshot {
        let state = try documentState()
        return EVRecoverySnapshot(source: try copySourceBytes(expectedRevision: state.document_revision),
                                  format: Self.sourceFormat(from: state), encoding: state.encoding, fileFormat: state.file_format,
                                  documentID: state.document_id, documentRevision: state.document_revision)
    }

    public func restoreRecovery(_ snapshot: EVRecoverySnapshot) throws {
        recoveryInterpretation = snapshot
        defer { recoveryInterpretation = nil }
        try read(source: snapshot.source, typeName: Self.openingType(for: snapshot.format))
        let state = try documentState()
        try checked(viem_core_mark_recovered(core, state.document_id, state.document_revision), operation: "Restore unsaved recovery state")
        _ = try documentState()
        for surface in surfaces.compactMap(\.value) { surface.refreshPresentation() }
    }

    public func setReadOnly(_ readOnly: Bool) throws {
        let state = try documentState()
        try checked(viem_core_set_read_only(core, state.document_id, state.document_revision, readOnly ? 1 : 0), operation: "Set read-only policy")
        _ = try documentState()
        for surface in surfaces.compactMap(\.value) { surface.refreshPresentation() }
    }

    public func serializedSource(typeName: String) throws -> Data {
        let state = try documentState()
        try validateSerializationType(typeName, currentFormat: Self.sourceFormat(from: state))
        return try copySourceBytes(expectedRevision: state.document_revision)
    }

    public func nativeSaveSnapshot(typeName: String) throws -> EVDocumentSaveSnapshot {
        let state = try documentState()
        try validateSerializationType(typeName, currentFormat: Self.sourceFormat(from: state))
        return EVDocumentSaveSnapshot(
            data: try copySourceBytes(expectedRevision: state.document_revision),
            documentID: state.document_id,
            documentRevision: state.document_revision
        )
    }

    public func nativeSaveSnapshot(typeName: String, hardLineRange: ClosedRange<UInt64>) throws -> EVDocumentSaveSnapshot {
        let state = try documentState()
        try validateSerializationType(typeName, currentFormat: Self.sourceFormat(from: state))
        guard hardLineRange.upperBound < UInt64.max else { throw EVCoreFrontendError.invalidHostEffect }
        var count: UInt64 = 0
        var complete: UInt32 = 0
        let status = viem_core_copy_hard_line_source_bytes(core, state.document_id, state.document_revision,
            hardLineRange.lowerBound, hardLineRange.upperBound + 1, nil, 0, &count, &complete)
        if status == VIEM_STATUS_POLICY_REQUIRED { throw EVDocumentHostError.preparedWriteUnavailable }
        guard status == VIEM_STATUS_OK || status == VIEM_STATUS_BUFFER_TOO_SMALL, count <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(operation: "Prepare ranged source write", status: status)
        }
        var data = Data(count: Int(count))
        let copied = data.withUnsafeMutableBytes { buffer in
            viem_core_copy_hard_line_source_bytes(core, state.document_id, state.document_revision,
                hardLineRange.lowerBound, hardLineRange.upperBound + 1, buffer.bindMemory(to: UInt8.self).baseAddress, count, &count, &complete)
        }
        try checked(copied, operation: "Copy ranged source write")
        return EVDocumentSaveSnapshot(data: data, documentID: state.document_id, documentRevision: state.document_revision, isCompleteSource: complete != 0)
    }

    public func acknowledgeNativeSave(_ snapshot: EVDocumentSaveSnapshot) throws {
        guard snapshot.isCompleteSource else { throw EVDocumentHostError.preparedWriteUnavailable }
        var request = ViemMarkSavedV1()
        request.struct_size = UInt32(MemoryLayout<ViemMarkSavedV1>.size)
        request.document_id = snapshot.documentID
        request.document_revision = snapshot.documentRevision
        try checked(viem_core_mark_saved(core, &request), operation: "Acknowledge saved document")
        _ = try documentState()
        surfaces.removeAll { $0.value == nil }
        for surface in surfaces.compactMap(\.value) {
            surface.refreshPresentation()
        }
    }

    func formattedText() throws -> String {
        let snapshot = try formattedSnapshot()
        return try formattedText(
            in: 0 ..< snapshot.info.utf8_length,
            snapshot: snapshot
        )
    }

    func formattedSnapshot() throws -> EVFormattedSnapshot {
        formattedAccessCounters.snapshotInfoCalls &+= 1
        var info = ViemFormattedSnapshotInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemFormattedSnapshotInfoV1>.size)
        try checked(
            viem_core_formatted_snapshot_info(core, &info),
            operation: "Read formatted snapshot"
        )
        guard info.identity.struct_size >= UInt32(MemoryLayout<ViemFormattedSnapshotIdentityV1>.size),
              info.utf8_length <= UInt64(Int.max),
              info.utf16_length <= UInt64(Int.max),
              info.hard_line_count <= UInt64(Int.max)
        else {
            throw EVCoreFrontendError.core(
                operation: "Read formatted snapshot",
                status: Status.lengthOverflow
            )
        }
        return EVFormattedSnapshot(info: info)
    }

    func formattedText(
        in utf8Range: Range<UInt64>,
        snapshot: EVFormattedSnapshot
    ) throws -> String {
        let bytes = try formattedBytes(in: utf8Range, snapshot: snapshot)
        guard let text = String(bytes: bytes, encoding: .utf8) else {
            throw EVCoreFrontendError.invalidUTF8
        }
        return text
    }

    func formattedSlice(
        in utf8Range: Range<UInt64>,
        snapshot: EVFormattedSnapshot
    ) throws -> EVFormattedTextSlice {
        EVFormattedTextSlice(
            identity: snapshot.info.identity,
            utf8Range: utf8Range,
            bytes: try formattedBytes(in: utf8Range, snapshot: snapshot)
        )
    }

    func mapFormattedUTF8ToUTF16(
        _ offsets: [UInt64],
        snapshot: EVFormattedSnapshot
    ) throws -> [UInt64] {
        try mapFormattedOffsets(
            offsets,
            snapshot: snapshot,
            operation: "Map formatted UTF-8 to UTF-16",
            direction: .utf8ToUTF16
        )
    }

    func mapFormattedUTF16ToUTF8(
        _ offsets: [UInt64],
        snapshot: EVFormattedSnapshot
    ) throws -> [UInt64] {
        try mapFormattedOffsets(
            offsets,
            snapshot: snapshot,
            operation: "Map formatted UTF-16 to UTF-8",
            direction: .utf16ToUTF8
        )
    }

    func formattedPointInfo(
        atUTF8Offset offset: UInt64,
        snapshot: EVFormattedSnapshot
    ) throws -> ViemFormattedPointInfoV1 {
        formattedAccessCounters.pointInfoCalls &+= 1
        var identity = snapshot.info.identity
        var info = ViemFormattedPointInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemFormattedPointInfoV1>.size)
        try checked(
            viem_core_formatted_point_info(core, &identity, offset, &info),
            operation: "Read formatted point"
        )
        guard info.identity.isSameSnapshot(as: snapshot.info.identity),
              info.utf8_offset == offset
        else {
            throw EVCoreFrontendError.core(
                operation: "Read formatted point",
                status: UInt32(VIEM_STATUS_STALE_REVISION)
            )
        }
        return info
    }

    func resetFormattedAccessCounters() {
        formattedAccessCounters = EVFormattedAccessCounters()
    }

    func revision() throws -> UInt64 {
        var value: UInt64 = 0
        try checked(viem_core_revision(core, &value), operation: "Read revision")
        return value
    }

    @discardableResult
    func documentState() throws -> ViemDocumentStateV1 {
        var state = ViemDocumentStateV1()
        state.struct_size = UInt32(MemoryLayout<ViemDocumentStateV1>.size)
        try checked(viem_core_document_state(core, &state), operation: "Read document state")
        installDocumentState(state)
        return state
    }

    var encodingLabel: String {
        switch currentDocumentState.encoding {
        case UInt32(VIEM_ENCODING_LATIN1): "Latin-1"
        case UInt32(VIEM_ENCODING_UTF16_LE): "UTF-16 LE"
        case UInt32(VIEM_ENCODING_UTF16_BE): "UTF-16 BE"
        default: "UTF-8"
        }
    }

    var formatLabel: String {
        sourceFormat.displayName
    }

    var lineEndingLabel: String {
        switch currentDocumentState.file_format {
        case UInt32(VIEM_FILE_FORMAT_DOS): "CRLF"
        case UInt32(VIEM_FILE_FORMAT_MAC): "CR"
        default: "LF"
        }
    }

    func noteSourceChange(originatingViewID: ViemViewId) {
        pendingSourceChangeOrigins.append(originatingViewID)
        guard !isRefreshingSurfaces else { return }

        isRefreshingSurfaces = true
        defer { isRefreshingSurfaces = false }
        while !pendingSourceChangeOrigins.isEmpty {
            let origins = Set(pendingSourceChangeOrigins)
            let changeCount = pendingSourceChangeOrigins.count
            pendingSourceChangeOrigins.removeAll(keepingCapacity: true)
            for _ in 0..<changeCount {
                sourceDidChange?()
            }
            // NSDocument first observes that a source transaction happened;
            // the exact core dirty bit then wins. This order matters when an
            // undo returns to the saved revision and must clear AppKit's
            // provisional change count again.
            _ = try? documentState()
            surfaces.removeAll { $0.value == nil }
            for surface in surfaces.compactMap(\.value) {
                surface.sharedDocumentDidChange(originatingViewIDs: origins)
            }
            NotificationCenter.default.post(name: .viemCoreDocumentDidChange, object: self)
        }
    }

    /// Buffer presentation options and shared search state can change another
    /// pane without a source edit or a cursor move in the invoking pane.
    func notePresentationChange(originatingViewID: ViemViewId) {
        guard !isRefreshingLayoutSurfaces else { return }
        isRefreshingLayoutSurfaces = true
        defer { isRefreshingLayoutSurfaces = false }
        for surface in surfaces.compactMap(\.value) {
            guard let session = surface.session, session.viewID != originatingViewID else { continue }
            var info = ViemLayoutSnapshotInfoV1()
            info.struct_size = UInt32(MemoryLayout<ViemLayoutSnapshotInfoV1>.size)
            let status = viem_core_view_layout_snapshot_info(core, session.viewID, &info)
            if status == UInt32(VIEM_STATUS_LAYOUT_UNAVAILABLE)
                || (status == UInt32(VIEM_STATUS_OK)
                    && info.identity.layout_revision != surface.layoutSnapshot?.info.identity.layout_revision)
                || (try? session.searchWorkPending()) == true {
                surface.refreshPresentation()
            }
        }
    }

    private func createCore(publishDiagnostics: Bool = true) throws {
        configurationWarning = configuration.lastError
        do { try EVCodeStyleSession.initialize(configuration: configuration) }
        catch { configurationWarning = error.localizedDescription }
        var options = ViemDocumentOptions()
        options.struct_size = UInt32(MemoryLayout<ViemDocumentOptions>.size)
        // Opening policy belongs to the portable encoding projection. The
        // frontend identifies the format container but never decodes or scans
        // authoritative source bytes itself.
        options.encoding = recoveryInterpretation?.encoding ?? UInt32(VIEM_ENCODING_DETECT)
        options.format = Self.formatOption(typeName: recoveryInterpretation.map { Self.openingType(for: $0.format) } ?? typeName)
        options.file_format = recoveryInterpretation?.fileFormat ?? UInt32(VIEM_FILE_FORMAT_DETECT)
        var handle: ViemCoreHandle = 0
        var revision: UInt64 = 0
        let status = source.withUnsafeBytes { raw in
            viem_core_create(
                raw.bindMemory(to: UInt8.self).baseAddress,
                UInt64(raw.count),
                &options,
                &handle,
                &revision
            )
        }
        try checked(status, operation: "Open document")
        core = handle
        source.removeAll(keepingCapacity: false)
        _ = try documentState()
        configureSyntax()
        try checked(viem_core_set_text_width_default(core, configuration.textWidth), operation: "Load text width")
        try configureWhitespace(indentation: configuration.indentation, presentation: configuration.whitespacePresentation)
        let associations = try configuration.codeFilenameAssociationsJSON()
        let configuredAssociations = associations.withUnsafeBytes {
            viem_core_set_code_filename_associations_json(core, $0.bindMemory(to: UInt8.self).baseAddress, UInt64($0.count))
        }
        try checked(configuredAssociations, operation: "Load Code filename associations")
        let filename = Array(openingFilename.utf8)
        let detected = filename.withUnsafeBufferPointer {
            viem_core_initialize_code_detection(core, $0.baseAddress, UInt64($0.count), allowAutomaticCode ? 1 : 0)
        }
        try checked(detected, operation: "Detect code language")
        _ = try documentState()
        do {
            if sourceFormat != .code, let defaults = try configuration.styleDefaults(named: sourceFormat.defaultStyleName) {
                let result = defaults.withUnsafeBytes { raw in
                    viem_core_initialize_style_defaults(core, currentDocumentState.document_revision,
                        raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count))
                }
                try checked(result, operation: "Load default style")
            }
        } catch { configurationWarning = error.localizedDescription }
        let startupDiagnostics = EVCoreStartup.initialize(core: core, file: configuration.startupFile)
        if !startupDiagnostics.isEmpty {
            configurationWarning = ([configurationWarning].compactMap { $0 } + startupDiagnostics).joined(separator: "\n")
        }
        if publishDiagnostics { refreshSyntaxDiagnostics() }
    }

    private static func openingType(for format: EVSourceFormat) -> String {
        switch format {
        case .code: EVDocument.codeType
        case .markdownSource: EVDocument.markdownSourceType
        case .htmlSource: EVDocument.htmlSourceType
        default: EVDocument.typeName(for: format)
        }
    }

    private func configureSyntax() {
        guard core != 0 else { return }
        let bytes = Array((configuration.vimSyntaxDirectory as NSString).expandingTildeInPath.utf8)
        let status = bytes.withUnsafeBufferPointer {
            viem_core_configure_syntax(core, $0.baseAddress, UInt64($0.count))
        }
        if status != Status.ok { configurationWarning = "Unable to configure syntax highlighting (\(status))." }
    }

    func pollSyntax(now: TimeInterval = ProcessInfo.processInfo.systemUptime) {
        guard core != 0, sourceFormat == .code else { return }
        var changed: UInt8 = 0
        guard viem_core_poll_syntax(core, &changed) == Status.ok else { return }
        if changed != 0 {
            // Presentation generations may change; no source transaction or document undo occurs.
            for surface in surfaces.compactMap(\.value) { surface.refreshPresentation() }
        }
        // Providers can report a diagnostic without publishing different styles.
        // Bound text export to four times per second independently of frame polling.
        if now - lastSyntaxDiagnosticRefresh >= 0.25 { refreshSyntaxDiagnostics(now: now) }
    }

    private func refreshSyntaxDiagnostics(now: TimeInterval = ProcessInfo.processInfo.systemUptime) {
        guard core != 0 else { return }
        lastSyntaxDiagnosticRefresh = now
        var diagnostics = configurationWarning.map { [$0] } ?? []
        if sourceFormat == .code {
            var required: UInt64 = 0
            let queried = viem_core_copy_syntax_diagnostics(core, nil, 0, &required)
            if (queried == Status.ok || queried == Status.bufferTooSmall), required <= 256 * 1024, required > 0 {
                var bytes = [UInt8](repeating: 0, count: Int(required))
                let copied = bytes.withUnsafeMutableBufferPointer {
                    viem_core_copy_syntax_diagnostics(core, $0.baseAddress, UInt64($0.count), &required)
                }
                if copied == Status.ok { diagnostics += String(decoding: bytes, as: UTF8.self).split(separator: "\n").map(String.init) }
            }
        }
        EVCodePreferences.shared.reportLoadDiagnostics(diagnostics, source: syntaxDiagnosticSource)
    }

    func saveDefaultStyle() throws -> URL {
        let state = try documentState()
        var required: UInt64 = 0
        let first = viem_core_export_style_defaults(core, state.document_revision, nil, 0, &required)
        guard first == Status.ok || first == Status.bufferTooSmall, required <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(operation: "Read default style", status: first)
        }
        var data = Data(count: Int(required))
        let copied = data.withUnsafeMutableBytes { raw in
            viem_core_export_style_defaults(core, state.document_revision,
                raw.bindMemory(to: UInt8.self).baseAddress, required, &required)
        }
        try checked(copied, operation: "Read default style")
        try configuration.saveStyleDefaults(data, named: sourceFormat.defaultStyleName)
        return configuration.directory.appendingPathComponent("\(sourceFormat.defaultStyleName)_style.json")
    }

    private func copySourceBytes(expectedRevision: UInt64) throws -> Data {
        var required: UInt64 = 0
        let query = viem_core_copy_source_bytes(core, expectedRevision, nil, 0, &required)
        guard query == Status.ok || query == Status.bufferTooSmall else {
            throw EVCoreFrontendError.core(operation: "Measure document bytes", status: query)
        }
        guard required <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(operation: "Measure document bytes", status: Status.lengthOverflow)
        }
        if required == 0 { return Data() }
        var data = Data(count: Int(required))
        var secondRequired: UInt64 = 0
        let copied = data.withUnsafeMutableBytes { raw in
            let output = raw.bindMemory(to: UInt8.self).baseAddress
            return viem_core_copy_source_bytes(core, expectedRevision, output, required, &secondRequired)
        }
        try checked(copied, operation: "Copy document bytes")
        guard secondRequired == required else {
            throw EVCoreFrontendError.core(
                operation: "Copy document bytes",
                status: UInt32(VIEM_STATUS_CORE_FAILURE)
            )
        }
        return data
    }

    private func formattedBytes(
        in utf8Range: Range<UInt64>,
        snapshot: EVFormattedSnapshot
    ) throws -> [UInt8] {
        guard utf8Range.lowerBound <= utf8Range.upperBound,
              utf8Range.upperBound <= snapshot.info.utf8_length
        else {
            throw EVCoreFrontendError.core(
                operation: "Read formatted range",
                status: UInt32(VIEM_STATUS_INVALID_RANGE)
            )
        }

        let requested = utf8Range.upperBound - utf8Range.lowerBound
        formattedAccessCounters.rangeReadCalls &+= 1
        formattedAccessCounters.requestedUTF8Bytes &+= requested
        formattedAccessCounters.maximumRangeReadBytes = max(
            formattedAccessCounters.maximumRangeReadBytes,
            requested
        )
        if snapshot.info.utf8_length > 0,
           utf8Range.lowerBound == 0,
           utf8Range.upperBound == snapshot.info.utf8_length
        {
            formattedAccessCounters.fullRangeReadCalls &+= 1
        }

        var request = ViemFormattedUtf8RangeV1()
        request.struct_size = UInt32(MemoryLayout<ViemFormattedUtf8RangeV1>.size)
        request.identity = snapshot.info.identity
        request.utf8_start = utf8Range.lowerBound
        request.utf8_end = utf8Range.upperBound
        var required: UInt64 = 0
        let query = viem_core_copy_formatted_utf8_range(core, &request, nil, 0, &required)
        guard query == Status.ok || query == Status.bufferTooSmall else {
            throw EVCoreFrontendError.core(operation: "Measure formatted range", status: query)
        }
        guard required == requested, required <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(
                operation: "Measure formatted range",
                status: Status.lengthOverflow
            )
        }
        if required == 0 { return [] }

        var bytes = Array(repeating: UInt8(0), count: Int(required))
        var copiedRequired: UInt64 = 0
        let copied = bytes.withUnsafeMutableBufferPointer { buffer in
            viem_core_copy_formatted_utf8_range(
                core,
                &request,
                buffer.baseAddress,
                UInt64(buffer.count),
                &copiedRequired
            )
        }
        try checked(copied, operation: "Copy formatted range")
        guard copiedRequired == required else {
            throw EVCoreFrontendError.core(
                operation: "Copy formatted range",
                status: UInt32(VIEM_STATUS_CORE_FAILURE)
            )
        }
        return bytes
    }

    private func mapFormattedOffsets(
        _ offsets: [UInt64],
        snapshot: EVFormattedSnapshot,
        operation: String,
        direction: FormattedOffsetMapDirection
    ) throws -> [UInt64] {
        guard !offsets.isEmpty else { return [] }
        switch direction {
        case .utf8ToUTF16:
            formattedAccessCounters.utf8ToUTF16BatchCalls &+= 1
        case .utf16ToUTF8:
            formattedAccessCounters.utf16ToUTF8BatchCalls &+= 1
        }
        var identity = snapshot.info.identity
        var output = Array(repeating: UInt64(0), count: offsets.count)
        var required: UInt64 = 0
        let status = offsets.withUnsafeBufferPointer { inputBuffer in
            output.withUnsafeMutableBufferPointer { outputBuffer in
                switch direction {
                case .utf8ToUTF16:
                    viem_core_map_formatted_utf8_to_utf16(
                        core,
                        &identity,
                        inputBuffer.baseAddress,
                        UInt64(inputBuffer.count),
                        outputBuffer.baseAddress,
                        UInt64(outputBuffer.count),
                        &required
                    )
                case .utf16ToUTF8:
                    viem_core_map_formatted_utf16_to_utf8(
                        core,
                        &identity,
                        inputBuffer.baseAddress,
                        UInt64(inputBuffer.count),
                        outputBuffer.baseAddress,
                        UInt64(outputBuffer.count),
                        &required
                    )
                }
            }
        }
        try checked(status, operation: operation)
        guard required == UInt64(offsets.count) else {
            throw EVCoreFrontendError.core(
                operation: operation,
                status: UInt32(VIEM_STATUS_CORE_FAILURE)
            )
        }
        return output
    }

    private func installDocumentState(_ state: ViemDocumentStateV1) {
        let oldPersistence = Self.persistenceState(from: currentDocumentState)
        currentDocumentState = state
        let newPersistence = Self.persistenceState(from: state)
        if oldPersistence != newPersistence {
            persistenceStateDidChange?(newPersistence)
        }
    }

    private static func persistenceState(from state: ViemDocumentStateV1) -> EVDocumentPersistenceState {
        EVDocumentPersistenceState(
            isDirty: state.flags & UInt32(VIEM_DOCUMENT_STATE_IS_DIRTY) != 0,
            isReadOnly: state.flags & UInt32(VIEM_DOCUMENT_STATE_READ_ONLY) != 0,
            isRecovered: state.flags & UInt32(VIEM_DOCUMENT_STATE_RECOVERED) != 0,
            documentID: state.document_id,
            documentRevision: state.document_revision,
            sourceByteCount: state.source_byte_count
        )
    }

    private func validateSerializationType(
        _ typeName: String,
        currentFormat: EVSourceFormat
    ) throws {
        guard let requestedFormat = EVDocument.sourceFormat(forTypeName: typeName) else {
            throw EVDocumentSerializationError.unsupportedWritableType(typeName)
        }
        guard requestedFormat.hasSameSerialization(as: currentFormat) else {
            throw EVDocumentSerializationError.formatConversionUnavailable(
                current: currentFormat,
                requested: requestedFormat
            )
        }
    }

    private static func sourceFormat(from state: ViemDocumentStateV1) -> EVSourceFormat {
        switch state.format {
        case UInt32(VIEM_FORMAT_MARKDOWN): .markdown
        case UInt32(VIEM_FORMAT_MARKDOWN_SOURCE): .markdownSource
        case UInt32(VIEM_FORMAT_HTML): .html
        case UInt32(VIEM_FORMAT_HTML_SOURCE): .htmlSource
        case UInt32(VIEM_FORMAT_RTF): .rtf
        case UInt32(VIEM_FORMAT_CODE): .code
        default: .plainText
        }
    }

    private static func formatOption(typeName: String) -> UInt32 {
        switch EVDocument.sourceFormat(forTypeName: typeName) {
        case .markdown:
            UInt32(VIEM_FORMAT_MARKDOWN)
        case .markdownSource:
            UInt32(VIEM_FORMAT_MARKDOWN_SOURCE)
        case .html:
            UInt32(VIEM_FORMAT_HTML)
        case .htmlSource:
            UInt32(VIEM_FORMAT_HTML_SOURCE)
        case .rtf:
            UInt32(VIEM_FORMAT_RTF)
        case .code:
            UInt32(VIEM_FORMAT_CODE)
        case .plainText, nil:
            UInt32(VIEM_FORMAT_PLAIN_TEXT)
        }
    }
}

@MainActor
final class EVCoreViewSession {
    private(set) unowned var document: EVCoreDocumentBackend
    let provider: CoreTextMeasurementProvider
    private nonisolated let coreHandle: ViemCoreHandle
    private(set) var viewID: ViemViewId = 0
    private(set) var lastOutcome = ViemCoreOutcomeV1()
    private var viewportSize = CGSize(width: 1, height: 1)
    private(set) var hasActiveComposition = false
    weak var commandTurnHost: (any EVCommandTurnHost)?
    var hostEffectAccessCounters = EVHostEffectAccessCounters()
    var presentationExportCounters = EVPresentationExportCounters()
    private var cachedLayoutExport: EVLayoutExport?
    var cachedLayoutPaintExport: EVLayoutPaintExport?
    var cachedWhitespaceExport: EVWhitespaceExportCache?

    /// One bounded entry per export; correctness never depends on retention.
    func clearPresentationExportCache() {
        cachedLayoutExport = nil
        cachedLayoutPaintExport = nil
        cachedWhitespaceExport = nil
    }
    var compositionStateDidChange: ((Bool) -> Void)?
    private var mappingTimer: Timer?

    init(document: EVCoreDocumentBackend, width: CGFloat, height: CGFloat) throws {
        self.document = document
        coreHandle = document.core
        provider = CoreTextMeasurementProvider()
        try attach(width: width, height: height)
    }

    /// The staged core keeps its identity when ownership transfers from the
    /// temporary preparation backend to the live document backend.
    func adoptDocument(_ document: EVCoreDocumentBackend) {
        precondition(document.core == coreHandle)
        self.document = document
    }

    deinit {
        mappingTimer?.invalidate()
        if viewID != 0 {
            _ = viem_core_view_remove(coreHandle, viewID)
        }
        provider.retireResources()
    }

    func detach() {
        clearPresentationExportCache()
        mappingTimer?.invalidate()
        mappingTimer = nil
        if viewID != 0 {
            _ = viem_core_view_remove(coreHandle, viewID)
            viewID = 0
        }
        publishCompositionState(false)
        provider.retireResources()
    }

    func attach(width: CGFloat, height: CGFloat) throws {
        guard viewID == 0 else { return }
        var options = ViemViewOptionsV1()
        options.struct_size = UInt32(MemoryLayout<ViemViewOptionsV1>.size)
        options.execution_context = UInt32(VIEM_LAYOUT_EXECUTION_FRONTEND_MAIN)
        options.width = Float(max(width, 1))
        options.height = Float(max(height, 1))
        var table = provider.makeProviderTable()
        var newView: ViemViewId = 0
        var outcome = ViemCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        try checked(
            viem_core_view_add(document.core, &options, &table, &newView, &outcome),
            operation: "Attach editor view"
        )
        viewID = newView
        viewportSize = CGSize(width: max(width, 1), height: max(height, 1))
        lastOutcome = outcome
    }

    func refreshState() throws -> ViemCoreOutcomeV1 {
        var outcome = ViemCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        try checked(viem_core_view_state(document.core, viewID, &outcome), operation: "Read view state")
        lastOutcome = outcome
        return outcome
    }

    @discardableResult
    func readFile(_ bytes: Data, after: UInt64, expected: EVDocumentPersistenceState) throws -> ViemCoreOutcomeV1 {
        var outcome = ViemCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        try bytes.withUnsafeBytes { raw in
            try checked(viem_core_view_read_file(document.core, viewID, expected.documentID,
                expected.documentRevision, after, raw.bindMemory(to: UInt8.self).baseAddress,
                UInt64(raw.count), &outcome), operation: "Read file into buffer")
        }
        finish(outcome, composition: .cancelIfChanged)
        guard outcome.command_status == UInt32(VIEM_COMMAND_STATUS_NONE)
            || outcome.command_status == UInt32(VIEM_COMMAND_STATUS_COMPLETE) else {
            throw EVCoreFrontendError.command(operation: "Read file into buffer", status: outcome.command_status)
        }
        return outcome
    }

    func sourceLine(_ text: String, depth: UInt32) throws -> ViemCoreOutcomeV1 {
        let data = Data(text.utf8)
        return try performHostEffectTurn("Execute sourced command") { outcome, effects in
            withCommandTurnContext { context in
                data.withUnsafeBytes { raw in
                    viem_core_view_source_line(document.core, viewID, depth,
                        raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count), context, outcome, effects)
                }
            }
        }
    }

    @discardableResult
    func sendText(_ text: String) throws -> ViemCoreOutcomeV1 {
        let data = Data(text.utf8)
        return try performHostEffectTurn("Send text input") { outcome, effects in
            withCommandTurnContext { context in
                data.withUnsafeBytes { raw in
                    viem_core_view_send_text_with_host_context_v2(
                        document.core,
                        viewID,
                        raw.bindMemory(to: UInt8.self).baseAddress,
                        UInt64(raw.count),
                        context,
                        outcome,
                        effects
                    )
                }
            }
        }
    }

    @discardableResult
    func sendKey(kind: UInt32, codepoint: UInt32 = 0, modifiers: UInt32 = 0) throws -> ViemCoreOutcomeV1 {
        var input = ViemKeyInputV1()
        input.struct_size = UInt32(MemoryLayout<ViemKeyInputV1>.size)
        input.kind = kind
        input.codepoint = codepoint
        input.modifiers = modifiers
        return try performHostEffectTurn("Send key input") { outcome, effects in
            withCommandTurnContext { context in
                viem_core_view_send_key_with_host_context_v2(
                    document.core,
                    viewID,
                    &input,
                    context,
                    outcome,
                    effects
                )
            }
        }
    }

    /// Commands with host effects keep ownership/release and error precedence
    /// together. The caller still decides whether the turn needs host context.
    private func performHostEffectTurn(
        _ operation: String,
        _ body: (UnsafeMutablePointer<ViemCoreOutcomeV1>, UnsafeMutablePointer<ViemEffectBatchHandle>) -> UInt32
    ) throws -> ViemCoreOutcomeV1 {
        var outcome = ViemCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        var effects: ViemEffectBatchHandle = 0
        let status = body(&outcome, &effects)
        try finishHostContextTurn(status: status, outcome: outcome, effectBatch: effects, operation: operation)
        return outcome
    }

    private func finishHostContextTurn(
        status: UInt32,
        outcome: ViemCoreOutcomeV1,
        effectBatch: ViemEffectBatchHandle,
        operation: String
    ) throws {
        defer { scheduleMappingTimeout() }
        var copiedBatch: EVHostEffectBatch?
        var batchError: Error?
        if effectBatch != 0 {
            do {
                copiedBatch = try copyAndReleaseHostEffectBatch(effectBatch)
            } catch {
                batchError = error
            }
        }

        // The C status is the primary result, but an owned batch is copied and
        // released above even on a failing turn.
        try checked(status, operation: operation)
        if let batchError { throw batchError }
        finish(outcome, composition: .cancelIfChanged)
        if let copiedBatch {
            try commandTurnHost?.applyHostEffectBatch(copiedBatch)
        }
        if outcome.command_status == UInt32(VIEM_COMMAND_STATUS_READ_ONLY) {
            throw EVCoreFrontendError.command(operation: operation, status: outcome.command_status)
        }
    }

    func hasPendingMapping() throws -> Bool {
        var pending: UInt8 = 0
        try checked(viem_core_view_has_pending_mapping(document.core, viewID, &pending),
                    operation: "Read pending key mapping")
        return pending != 0
    }

    @discardableResult
    func flushMappingPrefix() throws -> ViemCoreOutcomeV1 {
        try performHostEffectTurn("Resolve key mapping") { outcome, effects in
            withCommandTurnContext { context in
                viem_core_view_flush_mapping_with_host_context_v2(
                    document.core, viewID, context, outcome, effects)
            }
        }
    }

    private func scheduleMappingTimeout() {
        mappingTimer?.invalidate()
        mappingTimer = nil
        guard viewID != 0, (try? hasPendingMapping()) == true else { return }
        let timer = Timer(timeInterval: 1, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                if let surface = self.commandTurnHost as? EVEditorSurfaceController {
                    surface.performInput { _ = try self.flushMappingPrefix() }
                } else {
                    _ = try? self.flushMappingPrefix()
                }
            }
        }
        mappingTimer = timer
        RunLoop.main.add(timer, forMode: .common)
    }

    func lineLocation() throws -> ViemViewLineLocationV1 {
        var value = ViemViewLineLocationV1(); value.struct_size = UInt32(MemoryLayout<ViemViewLineLocationV1>.size)
        try checked(viem_core_view_line_location(document.core, viewID, &value), operation: "Read line position")
        return value
    }

    func paragraphFlow() throws -> Bool {
        var value: UInt32 = 0
        try checked(viem_core_view_paragraph_flow(document.core, viewID, &value), operation: "Read paragraph flow")
        return value != 0
    }

    func setParagraphFlow(_ enabled: Bool) throws {
        _ = try performCoreOperation("Change paragraph flow") { outcome in
            viem_core_view_set_paragraph_flow(document.core, viewID, enabled ? 1 : 0, outcome)
        }
    }

    func lineMode() throws -> EVLineMode {
        var value: UInt32 = 0
        try checked(viem_core_view_line_mode(document.core, viewID, &value), operation: "Read line mode")
        return EVLineMode(rawValue: value) ?? .visual
    }

    func setLineMode(_ mode: EVLineMode) throws {
        _ = try performCoreOperation("Change line mode") { outcome in
            viem_core_view_set_line_mode(document.core, viewID, mode.rawValue, outcome)
        }
    }

    func setViewMargins(_ padding: EVViewMargins) throws {
        try checked(viem_core_view_set_padding(document.core, viewID, Float(padding.top), Float(padding.left), Float(padding.bottom), Float(padding.right)), operation: "Update view margins")
    }

    func setSmartQuotes(_ enabled: Bool) throws {
        try checked(viem_core_view_set_smart_quotes(document.core, viewID, enabled ? 1 : 0), operation: "Update smart quotes")
    }

    /// Buffer-owned application default; explicit `:set textwidth` survives.
    func setTextWidthDefault(_ width: UInt32) throws {
        try checked(viem_core_set_text_width_default(document.core, width), operation: "Update text width")
    }

    func currentFontEnWidth() throws -> CGFloat {
        var info = ViemTypographyInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemTypographyInfoV1>.size)
        let status = viem_core_view_typography_export(document.core, viewID,
            try document.revision(), &info, nil, 0, nil, 0)
        if status != UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) { try checked(status, operation: "Resolve caret font") }
        return CGFloat(info.size) / 2
    }

    @discardableResult
    func resize(width: CGFloat, height: CGFloat) throws -> ViemCoreOutcomeV1 {
        var outcome = ViemCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        try checked(
            viem_core_view_resize(document.core, viewID, Float(max(width, 1)), Float(max(height, 1)), &outcome),
            operation: "Resize editor view"
        )
        viewportSize = CGSize(width: max(width, 1), height: max(height, 1))
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    func adjacentZoomScale(from scale: Float, increasing: Bool) throws -> Float {
        var result: Float = 0
        try checked(viem_core_adjacent_zoom_scale(scale, increasing ? 1 : 0, &result),
                    operation: "Choose adjacent zoom")
        return result
    }

    @discardableResult
    func setScale(_ scale: CGFloat) throws -> ViemCoreOutcomeV1 {
        return try performCoreOperation("Change editor zoom") { outcome in
            viem_core_view_set_scale(document.core, viewID, Float(scale), outcome)
        }
    }

    func semanticStylePresentation(
        _ style: UInt32
    ) throws -> ViemSemanticStylePresentationV1 {
        var value = ViemSemanticStylePresentationV1()
        value.struct_size = UInt32(MemoryLayout<ViemSemanticStylePresentationV1>.size)
        try checked(
            viem_core_view_semantic_style_presentation(
                document.core,
                viewID,
                style,
                &value
            ),
            operation: "Read selection semantic style"
        )
        return value
    }

    @discardableResult
    func setSemanticStyle(
        _ style: UInt32,
        enabled: Bool,
        expected selection: ViemLogicalSelectionIdentityV1
    ) throws -> ViemCoreOutcomeV1 {
        var request = ViemSetSemanticStyleV1()
        request.struct_size = UInt32(MemoryLayout<ViemSetSemanticStyleV1>.size)
        request.style = style
        request.enabled = enabled ? 1 : 0
        request.reserved = 0
        request.expected_selection = selection
        return try performCoreOperation(enabled ? "Apply selection semantic style" : "Clear selection semantic style") { outcome in
            viem_core_view_set_semantic_style(
                document.core,
                viewID,
                &request,
                outcome
            )
        }
    }

    @discardableResult
    func useSelectionForFind(
        _ selection: ViemVisualSelectionIdentityV1
    ) throws -> ViemCoreOutcomeV1 {
        var expected = selection
        return try performCoreOperation("Use selection for find") { outcome in
            viem_core_view_use_selection_for_find(
                document.core,
                viewID,
                &expected,
                outcome
            )
        }
    }

    @discardableResult
    func revealSelection() throws -> ViemCoreOutcomeV1 {
        return try performCoreOperation("Jump to selection") { outcome in
            viem_core_view_reveal_selection(document.core, viewID, outcome)
        }
    }

    @discardableResult
    func setWrap(_ enabled: Bool) throws -> ViemCoreOutcomeV1 {
        return try performCoreOperation("Change wrapping") { outcome in
            viem_core_view_set_wrap(document.core, viewID, enabled ? 1 : 0, outcome)
        }
    }

    @discardableResult
    func setFileFormat(
        _ fileFormat: UInt32,
        expected state: ViemDocumentStateV1
    ) throws -> ViemCoreOutcomeV1 {
        var request = ViemSetFileFormatV1()
        request.struct_size = UInt32(MemoryLayout<ViemSetFileFormatV1>.size)
        request.file_format = fileFormat
        request.document_id = state.document_id
        request.document_revision = state.document_revision
        return try performCoreOperation("Change line endings") { outcome in
            viem_core_view_set_file_format(document.core, viewID, &request, outcome)
        }
    }

    @discardableResult
    func setIncludeStyleDefinitionsInFile(
        _ enabled: Bool,
        expected state: ViemDocumentStateV1
    ) throws -> ViemCoreOutcomeV1 {
        var request = ViemSetIncludeStyleDefinitionsV1()
        request.struct_size = UInt32(MemoryLayout<ViemSetIncludeStyleDefinitionsV1>.size)
        request.enabled = enabled ? 1 : 0
        request.document_id = state.document_id
        request.document_revision = state.document_revision
        return try performCoreOperation("Change inclusion of style definitions") { outcome in
            viem_core_view_set_include_style_definitions(document.core, viewID, &request, outcome)
        }
    }

    func listSelection() throws -> ViemLogicalSelectionIdentityV1 {
        var selection = ViemLogicalSelectionIdentityV1()
        selection.struct_size = UInt32(MemoryLayout<ViemLogicalSelectionIdentityV1>.size)
        try checked(viem_core_view_list_selection(document.core, viewID, &selection), operation: "Read list selection")
        return selection
    }

    func listIndentCapabilities(expected selection: ViemLogicalSelectionIdentityV1) throws -> UInt32 {
        var selection = selection
        var flags: UInt32 = 0
        try checked(viem_core_view_list_indent_capabilities(document.core, viewID, &selection, &flags),
                    operation: "Read list indentation capabilities")
        return flags
    }

    @discardableResult
    func indentList(unindent: Bool, expected selection: ViemLogicalSelectionIdentityV1) throws -> ViemCoreOutcomeV1 {
        var request = ViemListIndentV1()
        request.struct_size = UInt32(MemoryLayout<ViemListIndentV1>.size)
        request.unindent = unindent ? 1 : 0
        request.expected_selection = selection
        return try performCoreOperation(unindent ? "Unindent list item" : "Indent list item") { outcome in
            viem_core_view_indent_list(document.core, viewID, &request, outcome)
        }
    }

    @discardableResult
    func setListStyle(_ style: UInt32, expected selection: ViemLogicalSelectionIdentityV1) throws -> ViemCoreOutcomeV1 {
        var request = ViemSetListStyleV1()
        request.struct_size = UInt32(MemoryLayout<ViemSetListStyleV1>.size)
        request.style = style
        request.expected_selection = selection
        return try performCoreOperation("Change paragraph list") { outcome in
            viem_core_view_set_list_style(document.core, viewID, &request, outcome)
        }
    }

    @discardableResult
    func createStyle(_ key: EVStyleKey, name: String, identity: EVStyleSheetIdentity) throws -> ViemCoreOutcomeV1 {
        var request = ViemCreateStyleV1()
        request.struct_size = UInt32(MemoryLayout<ViemCreateStyleV1>.size)
        request.namespace = key.namespace.rawValue
        request.identity = identity.abiValue
        let idBytes = Array(key.id.rawValue.utf8)
        let nameBytes = Array(name.utf8)
        return try performCoreOperation("Create named style") { outcome in
            idBytes.withUnsafeBufferPointer { id in
                nameBytes.withUnsafeBufferPointer { name in
                    request.style_id.data = id.baseAddress
                    request.style_id.length = UInt64(id.count)
                    request.display_name.data = name.baseAddress
                    request.display_name.length = UInt64(name.count)
                    return viem_core_view_create_style(document.core, viewID, &request, outcome)
                }
            }
        }
    }

    @discardableResult
    func deleteStyle(_ key: EVStyleKey, identity: EVStyleSheetIdentity) throws -> ViemCoreOutcomeV1 {
        var request = ViemDeleteStyleV1()
        request.struct_size = UInt32(MemoryLayout<ViemDeleteStyleV1>.size)
        request.namespace = key.namespace.rawValue
        request.identity = identity.abiValue
        let bytes = Array(key.id.rawValue.utf8)
        return try performCoreOperation("Delete named style") { outcome in
            bytes.withUnsafeBufferPointer { buffer in
                request.style_id.data = buffer.baseAddress
                request.style_id.length = UInt64(buffer.count)
                return viem_core_view_delete_style(document.core, viewID, &request, outcome)
            }
        }
    }

    @discardableResult
    func assignStyle(_ key: EVStyleKey, identity: EVStyleSheetIdentity,
                     expected selection: ViemLogicalSelectionIdentityV1) throws -> ViemCoreOutcomeV1 {
        var request = ViemAssignStyleV1()
        request.struct_size = UInt32(MemoryLayout<ViemAssignStyleV1>.size)
        request.namespace = key.namespace.rawValue
        request.identity = identity.abiValue
        request.expected_selection = selection
        let bytes = Array(key.id.rawValue.utf8)
        return try performCoreOperation("Assign named style") { outcome in
            bytes.withUnsafeBufferPointer { buffer in
                request.style_id.data = buffer.baseAddress
                request.style_id.length = UInt64(buffer.count)
                return viem_core_view_assign_style(document.core, viewID, &request, outcome)
            }
        }
    }

    @discardableResult
    func setParagraphStyle(level: UInt32, expected selection: ViemLogicalSelectionIdentityV1) throws -> ViemCoreOutcomeV1 {
        var request = ViemSetParagraphStyleV1()
        request.struct_size = UInt32(MemoryLayout<ViemSetParagraphStyleV1>.size)
        request.level = level
        request.expected_selection = selection
        return try performCoreOperation("Change paragraph style") { outcome in
            viem_core_view_set_paragraph_style(document.core, viewID, &request, outcome)
        }
    }

    @discardableResult
    func setFormat(_ format: EVSourceFormat, operation: EVFormatOperation = .reinterpret,
                   expected state: ViemDocumentStateV1) throws -> ViemCoreOutcomeV1 {
        var request = ViemSetFormatV1()
        request.struct_size = UInt32(MemoryLayout<ViemSetFormatV1>.size)
        request.format = switch format {
        case .plainText: UInt32(VIEM_FORMAT_PLAIN_TEXT)
        case .markdown: UInt32(VIEM_FORMAT_MARKDOWN)
        case .markdownSource: UInt32(VIEM_FORMAT_MARKDOWN_SOURCE)
        case .html: UInt32(VIEM_FORMAT_HTML)
        case .htmlSource: UInt32(VIEM_FORMAT_HTML_SOURCE)
        case .rtf: UInt32(VIEM_FORMAT_RTF)
        case .code: UInt32(VIEM_FORMAT_CODE)
        }
        request.operation = switch operation {
        case .reinterpret: UInt32(VIEM_FORMAT_OPERATION_REINTERPRET)
        case .convert: UInt32(VIEM_FORMAT_OPERATION_CONVERT)
        }
        request.document_id = state.document_id
        request.document_revision = state.document_revision
        return try performHostEffectTurn("Change document format") { outcome, effects in
            viem_core_view_set_format_with_effects(document.core, viewID, &request, outcome, effects)
        }
    }

    @discardableResult
    func setEncoding(_ encoding: UInt32, expected state: ViemDocumentStateV1) throws -> ViemCoreOutcomeV1 {
        var request = ViemSetEncodingV1()
        request.struct_size = UInt32(MemoryLayout<ViemSetEncodingV1>.size)
        request.encoding = encoding
        request.document_id = state.document_id
        request.document_revision = state.document_revision
        return try performHostEffectTurn("Change document encoding") { outcome, effects in
            viem_core_view_set_encoding_with_effects(document.core, viewID, &request, outcome, effects)
        }
    }

    @discardableResult
    func undo() throws -> ViemCoreOutcomeV1 {
        return try performCoreOperation("Undo") { outcome in
            viem_core_view_undo(document.core, viewID, outcome)
        }
    }

    @discardableResult
    func redo() throws -> ViemCoreOutcomeV1 {
        return try performCoreOperation("Redo") { outcome in
            viem_core_view_redo(document.core, viewID, outcome)
        }
    }

    @discardableResult
    func setViewportOrigin(
        left: CGFloat,
        top: CGFloat? = nil,
        expected state: ViemViewportStateV1? = nil
    ) throws -> ViemCoreOutcomeV1 {
        var request = ViemViewportOriginV1()
        request.struct_size = UInt32(MemoryLayout<ViemViewportOriginV1>.size)
        request.flags = top == nil ? 0 : UInt32(VIEM_VIEWPORT_ORIGIN_HAS_TOP)
        request.left = Float(max(left, 0))
        request.top = Float(max(top ?? 0, 0))
        if let state {
            request.expected_document_id = state.document_id
            request.expected_document_revision = state.document_revision
            request.expected_layout_revision = state.layout_revision
            request.expected_configuration_generation = state.configuration_generation
            request.expected_measurement_environment_id = state.measurement_environment_id
            request.expected_metrics_generation = state.metrics_generation
        }

        return try performCoreOperation(top == nil ? "Scroll editor horizontally" : "Scroll editor vertically") { outcome in
            viem_core_view_set_viewport_origin(document.core, viewID, &request, outcome)
        }
    }

    func viewportState() throws -> ViemViewportStateV1 {
        var state = ViemViewportStateV1()
        state.struct_size = UInt32(MemoryLayout<ViemViewportStateV1>.size)
        try checked(
            viem_core_view_viewport_state(document.core, viewID, &state),
            operation: "Read viewport state"
        )
        return state
    }

    func presentation() throws -> ViemViewPresentationV1 {
        var value = ViemViewPresentationV1()
        value.struct_size = UInt32(MemoryLayout<ViemViewPresentationV1>.size)
        try checked(
            viem_core_view_presentation(document.core, viewID, &value),
            operation: "Read view presentation"
        )
        return value
    }

    func layoutSnapshotInfo() throws -> ViemLayoutSnapshotInfoV1 {
        var info = ViemLayoutSnapshotInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemLayoutSnapshotInfoV1>.size)
        let status = viem_core_view_layout_snapshot_info(document.core, viewID, &info)
        if status == Status.layoutUnavailable { throw EVCoreFrontendError.unavailableLayout }
        try checked(status, operation: "Read layout snapshot")
        return info
    }

    func layoutExport() throws -> EVLayoutExport {
        let info = try layoutSnapshotInfo()
        if var cached = cachedLayoutExport, cached.info.identity.isSameLayout(as: info.identity) {
            // Extent estimates and viewport dimensions are cheap live metadata.
            cached.info = info
            cached.whitespace = try whitespaceMarkersExport(identity: info.identity)
            return cached
        }
        guard info.row_count <= UInt64(Int.max),
              info.cluster_count <= UInt64(Int.max),
              info.caret_count <= UInt64(Int.max)
        else {
            throw EVCoreFrontendError.core(operation: "Read layout snapshot", status: Status.lengthOverflow)
        }

        var rows = Array(repeating: ViemVisualRowV1(), count: Int(info.row_count))
        var clusters = Array(repeating: ViemPositionedClusterV1(), count: Int(info.cluster_count))
        var carets = Array(repeating: ViemPositionedCaretV1(), count: Int(info.caret_count))
        var copiedInfo = ViemLayoutSnapshotInfoV1()
        copiedInfo.struct_size = UInt32(MemoryLayout<ViemLayoutSnapshotInfoV1>.size)
        var identity = info.identity
        let copied = rows.withUnsafeMutableBufferPointer { rowBuffer in
            clusters.withUnsafeMutableBufferPointer { clusterBuffer in
                carets.withUnsafeMutableBufferPointer { caretBuffer in
                    viem_core_view_copy_layout_snapshot(
                        document.core,
                        viewID,
                        &identity,
                        rowBuffer.baseAddress,
                        UInt64(rowBuffer.count),
                        clusterBuffer.baseAddress,
                        UInt64(clusterBuffer.count),
                        caretBuffer.baseAddress,
                        UInt64(caretBuffer.count),
                        &copiedInfo
                    )
                }
            }
        }
        try checked(copied, operation: "Copy layout snapshot")
        let furniture = try layoutDecorationsExport(identity: copiedInfo.identity)
        let exported = EVLayoutExport(info: copiedInfo, rows: rows, clusters: clusters, carets: carets,
                                      decorations: furniture.0, decorationLabels: furniture.1,
                                      whitespace: try whitespaceMarkersExport(identity: copiedInfo.identity))
        presentationExportCounters.geometryCopies &+= 1
        cachedLayoutExport = exported
        return exported
    }

    /// Font registration can retire the presentation while the view is idle.
    /// Refresh through an explicit core presentation event before requesting
    /// a new identity; never reuse an old hit-test identity or replay an edit.
    @discardableResult
    func refreshLayoutIfNeeded() throws -> Bool {
        var info = ViemLayoutSnapshotInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemLayoutSnapshotInfoV1>.size)
        let status = viem_core_view_layout_snapshot_info(document.core, viewID, &info)
        guard status == Status.layoutUnavailable else {
            try checked(status, operation: "Validate editor layout")
            return false
        }
        _ = try resize(width: viewportSize.width, height: viewportSize.height)
        return true
    }

    func hitTest(_ point: CGPoint, in snapshot: ViemLayoutSnapshotInfoV1) throws -> ViemLayoutCaretPointV1 {
        var request = ViemLayoutHitTestRequestV1()
        request.struct_size = UInt32(MemoryLayout<ViemLayoutHitTestRequestV1>.size)
        request.identity = snapshot.identity
        request.x = Float(point.x)
        request.y = Float(point.y)
        var result = ViemLayoutCaretPointV1()
        result.struct_size = UInt32(MemoryLayout<ViemLayoutCaretPointV1>.size)
        try checked(
            viem_core_view_layout_hit_test(document.core, viewID, &request, &result),
            operation: "Hit-test editor"
        )
        return result
    }

    @discardableResult
    func placeCursor(_ point: ViemLayoutCaretPointV1, extendSelection: Bool) throws -> ViemCoreOutcomeV1 {
        var request = ViemPlaceCursorV1()
        request.struct_size = UInt32(MemoryLayout<ViemPlaceCursorV1>.size)
        request.flags = extendSelection ? UInt32(VIEM_PLACE_CURSOR_EXTEND_SELECTION) : 0
        request.document_revision = point.document_revision
        request.text_offset = point.text_offset
        request.affinity = point.affinity
        return try performCoreOperation("Place editor cursor") { outcome in
            viem_core_view_place_cursor(document.core, viewID, &request, outcome)
        }
    }

    @discardableResult
    func goToLine(_ line: UInt64) throws -> ViemCoreOutcomeV1 {
        let state = try document.documentState()
        return try performCoreOperation("Go to logical line") { outcome in
            viem_core_view_go_to_line(document.core, viewID, state.document_id,
                state.document_revision, line, outcome)
        }
    }

    @discardableResult
    func selectAll() throws -> ViemCoreOutcomeV1 {
        let state = try document.documentState()
        return try performCoreOperation("Select all text") { outcome in
            viem_core_view_select_all(document.core, viewID, state.document_id,
                                      state.document_revision, outcome)
        }
    }

    func caretGeometry(
        offset: UInt64,
        affinity: UInt32,
        in snapshot: ViemLayoutSnapshotInfoV1
    ) throws -> ViemLayoutCaretGeometryV1 {
        var request = ViemLayoutCaretRequestV1()
        request.struct_size = UInt32(MemoryLayout<ViemLayoutCaretRequestV1>.size)
        request.affinity = affinity
        request.identity = snapshot.identity
        request.text_offset = offset
        var geometry = ViemLayoutCaretGeometryV1()
        geometry.struct_size = UInt32(MemoryLayout<ViemLayoutCaretGeometryV1>.size)
        try checked(
            viem_core_view_caret_geometry(document.core, viewID, &request, &geometry),
            operation: "Read caret geometry"
        )
        return geometry
    }

    @discardableResult
    func beginComposition(replacing range: Range<UInt64>) throws -> ViemCoreOutcomeV1 {
        var request = ViemCompositionBeginV1()
        request.struct_size = UInt32(MemoryLayout<ViemCompositionBeginV1>.size)
        request.document_revision = try document.revision()
        request.replacement_start = range.lowerBound
        request.replacement_end = range.upperBound
        return try performCoreOperation("Begin marked text", transition: .activate) { outcome in
            viem_core_view_composition_begin(document.core, viewID, &request, outcome)
        }
    }

    @discardableResult
    func updateComposition(_ text: String, selected: Range<UInt64>) throws -> ViemCoreOutcomeV1 {
        var request = ViemCompositionUpdateV1()
        request.struct_size = UInt32(MemoryLayout<ViemCompositionUpdateV1>.size)
        request.document_revision = try document.revision()
        request.selected_start = selected.lowerBound
        request.selected_end = selected.upperBound
        let data = Data(text.utf8)
        return try data.withUnsafeBytes { raw in
            request.marked_text.data = raw.bindMemory(to: UInt8.self).baseAddress
            request.marked_text.length = UInt64(raw.count)
            return try performCoreOperation("Update marked text", transition: .activate) { outcome in
                viem_core_view_composition_update(document.core, viewID, &request, outcome)
            }
        }
    }

    @discardableResult
    func commitComposition(_ text: String) throws -> ViemCoreOutcomeV1 {
        var request = ViemCompositionCommitV1()
        request.struct_size = UInt32(MemoryLayout<ViemCompositionCommitV1>.size)
        request.document_revision = try document.revision()
        let data = Data(text.utf8)
        return try data.withUnsafeBytes { raw in
            request.committed_text.data = raw.bindMemory(to: UInt8.self).baseAddress
            request.committed_text.length = UInt64(raw.count)
            return try performCoreOperation("Commit marked text", transition: .deactivate) { outcome in
                viem_core_view_composition_commit(document.core, viewID, &request, outcome)
            }
        }
    }

    @discardableResult
    func cancelComposition() throws -> ViemCoreOutcomeV1 {
        var request = ViemCompositionCancelV1()
        request.struct_size = UInt32(MemoryLayout<ViemCompositionCancelV1>.size)
        request.document_revision = try document.revision()
        return try performCoreOperation("Cancel marked text", transition: .deactivate) { outcome in
            viem_core_view_composition_cancel(document.core, viewID, &request, outcome)
        }
    }

    /// Publish successful native operations through one composition and source
    /// change path; failed C calls never update the session's last outcome.
    private func performCoreOperation(
        _ operation: String,
        transition: CompositionTransition = .cancelIfChanged,
        _ body: (UnsafeMutablePointer<ViemCoreOutcomeV1>) -> UInt32
    ) throws -> ViemCoreOutcomeV1 {
        var outcome = ViemCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        try checked(body(&outcome), operation: operation)
        finish(outcome, composition: transition)
        return outcome
    }

    func noteExternalDocumentChange() {
        publishCompositionState(false)
    }

    func finishStyleEdit(_ outcome: ViemCoreOutcomeV1) {
        finish(outcome)
    }

    private func finish(
        _ outcome: ViemCoreOutcomeV1,
        composition transition: CompositionTransition = .cancelIfChanged
    ) {
        lastOutcome = outcome
        switch transition {
        case .activate:
            publishCompositionState(true)
        case .deactivate:
            publishCompositionState(false)
        case .cancelIfChanged:
            if outcome.flags & UInt32(VIEM_OUTCOME_HAS_COMPOSITION_CHANGES) != 0 {
                publishCompositionState(false)
            }
        }
        if outcome.flags & UInt32(VIEM_OUTCOME_DOCUMENT_CHANGED) != 0 {
            document.noteSourceChange(originatingViewID: viewID)
        } else {
            document.notePresentationChange(originatingViewID: viewID)
        }
    }

    private func publishCompositionState(_ isActive: Bool) {
        hasActiveComposition = isActive
        compositionStateDidChange?(isActive)
    }
}

private enum CompositionTransition {
    case activate
    case deactivate
    case cancelIfChanged
}

private enum FormattedOffsetMapDirection {
    case utf8ToUTF16
    case utf16ToUTF8
}

private final class WeakSurface {
    weak var value: EVEditorSurfaceController?

    init(_ value: EVEditorSurfaceController) {
        self.value = value
    }
}

private enum Status {
    static let ok: UInt32 = 0
    static let bufferTooSmall: UInt32 = 9
    static let lengthOverflow: UInt32 = 18
    static let layoutUnavailable: UInt32 = 28
}

private func checked(_ status: UInt32, operation: String) throws {
    guard status == Status.ok else {
        throw EVCoreFrontendError.core(operation: operation, status: status)
    }
}
