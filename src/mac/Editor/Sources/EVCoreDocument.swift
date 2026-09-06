import AppKit
import CEvimCore
import EvimAppShell
import EvimCoreTextProvider
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
            case UInt32(EVIM_STATUS_VERIFICATION_FAILED):
                "This edit cannot preserve the format's text and structure."
            case UInt32(EVIM_STATUS_AMBIGUOUS_PROJECTION):
                "This selection has no unambiguous editable source range."
            case UInt32(EVIM_STATUS_UNREPRESENTABLE_CHARACTER):
                "This character cannot be represented in the document's encoding."
            case UInt32(EVIM_STATUS_UNSUPPORTED_OPERATION):
                "This operation is not supported for the current format or selection."
            case UInt32(EVIM_STATUS_POLICY_REQUIRED):
                "This change needs a format or encoding policy before it can be applied."
            default:
                "\(operation) failed (eVim core status \(status))."
            }
        case let .command(operation, status):
            status == UInt32(EVIM_COMMAND_STATUS_READ_ONLY)
                ? "E45: readonly option is set (use ! to override)"
                : "\(operation) was rejected (eVim command status \(status))."
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
    var info: EvimLayoutSnapshotInfoV1
    var rows: [EvimVisualRowV1]
    var clusters: [EvimPositionedClusterV1]
    var carets: [EvimPositionedCaretV1]
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

    private(set) var core: EvimCoreHandle = 0
    private var source = Data()
    private var typeName = "public.plain-text"
    private var recoveryInterpretation: EVRecoverySnapshot?
    private(set) var currentDocumentState = EvimDocumentStateV1()
    private(set) var formattedAccessCounters = EVFormattedAccessCounters()
    private var surfaces: [WeakSurface] = []
    private var isRefreshingSurfaces = false
    private var pendingSourceChangeOrigins: [EvimViewId] = []

    let configuration: EVConfigurationStore
    public private(set) var configurationWarning: String?

    public init(configuration: EVConfigurationStore? = nil) {
        let configuration = configuration ?? .shared
        self.configuration = configuration
        do {
            try createCore()
        } catch {
            assertionFailure("Unable to create the initial eVim core: \(error)")
        }
    }

    deinit {
        if core != 0 {
            _ = evim_core_destroy(core)
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
        self.source = source
        self.typeName = typeName
        for surface in surfaces.compactMap(\.value) {
            surface.detachFromCore()
        }
        if core != 0 {
            let status = evim_core_destroy(core)
            guard status == Status.ok else {
                throw EVCoreFrontendError.core(operation: "Close document", status: status)
            }
            core = 0
        }
        try createCore()
        for surface in surfaces.compactMap(\.value) {
            try surface.attachToCore()
        }
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
        try read(source: snapshot.source, typeName: snapshot.format == .markdownSource ? EVDocument.markdownSourceType : (snapshot.format == .htmlSource ? EVDocument.htmlSourceType : EVDocument.typeName(for: snapshot.format)))
        let state = try documentState()
        try checked(evim_core_mark_recovered(core, state.document_id, state.document_revision), operation: "Restore unsaved recovery state")
        _ = try documentState()
        for surface in surfaces.compactMap(\.value) { surface.refreshPresentation() }
    }

    public func setReadOnly(_ readOnly: Bool) throws {
        let state = try documentState()
        try checked(evim_core_set_read_only(core, state.document_id, state.document_revision, readOnly ? 1 : 0), operation: "Set read-only policy")
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
        let status = evim_core_copy_hard_line_source_bytes(core, state.document_id, state.document_revision,
            hardLineRange.lowerBound, hardLineRange.upperBound + 1, nil, 0, &count, &complete)
        if status == EVIM_STATUS_POLICY_REQUIRED { throw EVDocumentHostError.preparedWriteUnavailable }
        guard status == EVIM_STATUS_OK || status == EVIM_STATUS_BUFFER_TOO_SMALL, count <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(operation: "Prepare ranged source write", status: status)
        }
        var data = Data(count: Int(count))
        let copied = data.withUnsafeMutableBytes { buffer in
            evim_core_copy_hard_line_source_bytes(core, state.document_id, state.document_revision,
                hardLineRange.lowerBound, hardLineRange.upperBound + 1, buffer.bindMemory(to: UInt8.self).baseAddress, count, &count, &complete)
        }
        try checked(copied, operation: "Copy ranged source write")
        return EVDocumentSaveSnapshot(data: data, documentID: state.document_id, documentRevision: state.document_revision, isCompleteSource: complete != 0)
    }

    public func acknowledgeNativeSave(_ snapshot: EVDocumentSaveSnapshot) throws {
        guard snapshot.isCompleteSource else { throw EVDocumentHostError.preparedWriteUnavailable }
        var request = EvimMarkSavedV1()
        request.struct_size = UInt32(MemoryLayout<EvimMarkSavedV1>.size)
        request.document_id = snapshot.documentID
        request.document_revision = snapshot.documentRevision
        try checked(evim_core_mark_saved(core, &request), operation: "Acknowledge saved document")
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
        var info = EvimFormattedSnapshotInfoV1()
        info.struct_size = UInt32(MemoryLayout<EvimFormattedSnapshotInfoV1>.size)
        try checked(
            evim_core_formatted_snapshot_info(core, &info),
            operation: "Read formatted snapshot"
        )
        guard info.identity.struct_size >= UInt32(MemoryLayout<EvimFormattedSnapshotIdentityV1>.size),
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
    ) throws -> EvimFormattedPointInfoV1 {
        formattedAccessCounters.pointInfoCalls &+= 1
        var identity = snapshot.info.identity
        var info = EvimFormattedPointInfoV1()
        info.struct_size = UInt32(MemoryLayout<EvimFormattedPointInfoV1>.size)
        try checked(
            evim_core_formatted_point_info(core, &identity, offset, &info),
            operation: "Read formatted point"
        )
        guard info.identity.isSameSnapshot(as: snapshot.info.identity),
              info.utf8_offset == offset
        else {
            throw EVCoreFrontendError.core(
                operation: "Read formatted point",
                status: UInt32(EVIM_STATUS_STALE_REVISION)
            )
        }
        return info
    }

    func resetFormattedAccessCounters() {
        formattedAccessCounters = EVFormattedAccessCounters()
    }

    func revision() throws -> UInt64 {
        var value: UInt64 = 0
        try checked(evim_core_revision(core, &value), operation: "Read revision")
        return value
    }

    @discardableResult
    func documentState() throws -> EvimDocumentStateV1 {
        var state = EvimDocumentStateV1()
        state.struct_size = UInt32(MemoryLayout<EvimDocumentStateV1>.size)
        try checked(evim_core_document_state(core, &state), operation: "Read document state")
        installDocumentState(state)
        return state
    }

    var encodingLabel: String {
        switch currentDocumentState.encoding {
        case UInt32(EVIM_ENCODING_LATIN1): "Latin-1"
        case UInt32(EVIM_ENCODING_UTF16_LE): "UTF-16 LE"
        case UInt32(EVIM_ENCODING_UTF16_BE): "UTF-16 BE"
        default: "UTF-8"
        }
    }

    var formatLabel: String {
        sourceFormat.displayName
    }

    var lineEndingLabel: String {
        switch currentDocumentState.file_format {
        case UInt32(EVIM_FILE_FORMAT_DOS): "CRLF"
        case UInt32(EVIM_FILE_FORMAT_MAC): "CR"
        default: "LF"
        }
    }

    func noteSourceChange(originatingViewID: EvimViewId) {
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
            NotificationCenter.default.post(name: .evimCoreDocumentDidChange, object: self)
        }
    }

    private func createCore() throws {
        var options = EvimDocumentOptions()
        options.struct_size = UInt32(MemoryLayout<EvimDocumentOptions>.size)
        // Opening policy belongs to the portable encoding projection. The
        // frontend identifies the format container but never decodes or scans
        // authoritative source bytes itself.
        options.encoding = recoveryInterpretation?.encoding ?? UInt32(EVIM_ENCODING_DETECT)
        options.format = Self.formatOption(typeName: recoveryInterpretation.map { $0.format == .markdownSource ? EVDocument.markdownSourceType : ($0.format == .htmlSource ? EVDocument.htmlSourceType : EVDocument.typeName(for: $0.format)) } ?? typeName)
        options.file_format = recoveryInterpretation?.fileFormat ?? UInt32(EVIM_FILE_FORMAT_DETECT)
        var handle: EvimCoreHandle = 0
        var revision: UInt64 = 0
        let status = source.withUnsafeBytes { raw in
            evim_core_create(
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
        configurationWarning = configuration.lastError
        do {
            if let defaults = try configuration.styleDefaults(named: sourceFormat.defaultStyleName) {
                let result = defaults.withUnsafeBytes { raw in
                    evim_core_initialize_style_defaults(core, currentDocumentState.document_revision,
                        raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count))
                }
                try checked(result, operation: "Load default style")
            }
        } catch { configurationWarning = error.localizedDescription }
    }

    func saveDefaultStyle() throws -> URL {
        let state = try documentState()
        var required: UInt64 = 0
        let first = evim_core_export_style_defaults(core, state.document_revision, nil, 0, &required)
        guard first == Status.ok || first == Status.bufferTooSmall, required <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(operation: "Read default style", status: first)
        }
        var data = Data(count: Int(required))
        let copied = data.withUnsafeMutableBytes { raw in
            evim_core_export_style_defaults(core, state.document_revision,
                raw.bindMemory(to: UInt8.self).baseAddress, required, &required)
        }
        try checked(copied, operation: "Read default style")
        try configuration.saveStyleDefaults(data, named: sourceFormat.defaultStyleName)
        return configuration.directory.appendingPathComponent("\(sourceFormat.defaultStyleName)_style.json")
    }

    private func copySourceBytes(expectedRevision: UInt64) throws -> Data {
        var required: UInt64 = 0
        let query = evim_core_copy_source_bytes(core, expectedRevision, nil, 0, &required)
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
            return evim_core_copy_source_bytes(core, expectedRevision, output, required, &secondRequired)
        }
        try checked(copied, operation: "Copy document bytes")
        guard secondRequired == required else {
            throw EVCoreFrontendError.core(
                operation: "Copy document bytes",
                status: UInt32(EVIM_STATUS_CORE_FAILURE)
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
                status: UInt32(EVIM_STATUS_INVALID_RANGE)
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

        var request = EvimFormattedUtf8RangeV1()
        request.struct_size = UInt32(MemoryLayout<EvimFormattedUtf8RangeV1>.size)
        request.identity = snapshot.info.identity
        request.utf8_start = utf8Range.lowerBound
        request.utf8_end = utf8Range.upperBound
        var required: UInt64 = 0
        let query = evim_core_copy_formatted_utf8_range(core, &request, nil, 0, &required)
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
            evim_core_copy_formatted_utf8_range(
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
                status: UInt32(EVIM_STATUS_CORE_FAILURE)
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
                    evim_core_map_formatted_utf8_to_utf16(
                        core,
                        &identity,
                        inputBuffer.baseAddress,
                        UInt64(inputBuffer.count),
                        outputBuffer.baseAddress,
                        UInt64(outputBuffer.count),
                        &required
                    )
                case .utf16ToUTF8:
                    evim_core_map_formatted_utf16_to_utf8(
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
                status: UInt32(EVIM_STATUS_CORE_FAILURE)
            )
        }
        return output
    }

    private func installDocumentState(_ state: EvimDocumentStateV1) {
        let oldPersistence = Self.persistenceState(from: currentDocumentState)
        currentDocumentState = state
        let newPersistence = Self.persistenceState(from: state)
        if oldPersistence != newPersistence {
            persistenceStateDidChange?(newPersistence)
        }
    }

    private static func persistenceState(from state: EvimDocumentStateV1) -> EVDocumentPersistenceState {
        EVDocumentPersistenceState(
            isDirty: state.flags & UInt32(EVIM_DOCUMENT_STATE_IS_DIRTY) != 0,
            isReadOnly: state.flags & UInt32(EVIM_DOCUMENT_STATE_READ_ONLY) != 0,
            isRecovered: state.flags & UInt32(EVIM_DOCUMENT_STATE_RECOVERED) != 0,
            documentID: state.document_id,
            documentRevision: state.document_revision
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

    private static func sourceFormat(from state: EvimDocumentStateV1) -> EVSourceFormat {
        switch state.format {
        case UInt32(EVIM_FORMAT_MARKDOWN): .markdown
        case UInt32(EVIM_FORMAT_MARKDOWN_SOURCE): .markdownSource
        case UInt32(EVIM_FORMAT_HTML): .html
        case UInt32(EVIM_FORMAT_HTML_SOURCE): .htmlSource
        case UInt32(EVIM_FORMAT_RTF): .rtf
        default: .plainText
        }
    }

    private static func formatOption(typeName: String) -> UInt32 {
        switch EVDocument.sourceFormat(forTypeName: typeName) {
        case .markdown:
            UInt32(EVIM_FORMAT_MARKDOWN)
        case .markdownSource:
            UInt32(EVIM_FORMAT_MARKDOWN_SOURCE)
        case .html:
            UInt32(EVIM_FORMAT_HTML)
        case .htmlSource:
            UInt32(EVIM_FORMAT_HTML_SOURCE)
        case .rtf:
            UInt32(EVIM_FORMAT_RTF)
        case .plainText, nil:
            UInt32(EVIM_FORMAT_PLAIN_TEXT)
        }
    }
}

@MainActor
final class EVCoreViewSession {
    unowned let document: EVCoreDocumentBackend
    let provider: CoreTextMeasurementProvider
    private nonisolated let coreHandle: EvimCoreHandle
    private(set) var viewID: EvimViewId = 0
    private(set) var lastOutcome = EvimCoreOutcomeV1()
    private var viewportSize = CGSize(width: 1, height: 1)
    private(set) var hasActiveComposition = false
    weak var commandTurnHost: (any EVCommandTurnHost)?
    var hostEffectAccessCounters = EVHostEffectAccessCounters()
    var compositionStateDidChange: ((Bool) -> Void)?

    init(document: EVCoreDocumentBackend, width: CGFloat, height: CGFloat) throws {
        self.document = document
        coreHandle = document.core
        provider = CoreTextMeasurementProvider()
        try attach(width: width, height: height)
    }

    deinit {
        if viewID != 0 {
            _ = evim_core_view_remove(coreHandle, viewID)
        }
        provider.retireResources()
    }

    func detach() {
        if viewID != 0 {
            _ = evim_core_view_remove(document.core, viewID)
            viewID = 0
        }
        publishCompositionState(false)
        provider.retireResources()
    }

    func attach(width: CGFloat, height: CGFloat) throws {
        guard viewID == 0 else { return }
        var options = EvimViewOptionsV1()
        options.struct_size = UInt32(MemoryLayout<EvimViewOptionsV1>.size)
        options.execution_context = UInt32(EVIM_LAYOUT_EXECUTION_FRONTEND_MAIN)
        options.width = Float(max(width, 1))
        options.height = Float(max(height, 1))
        var table = provider.makeProviderTable()
        var newView: EvimViewId = 0
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_add(document.core, &options, &table, &newView, &outcome),
            operation: "Attach editor view"
        )
        viewID = newView
        viewportSize = CGSize(width: max(width, 1), height: max(height, 1))
        lastOutcome = outcome
    }

    func refreshState() throws -> EvimCoreOutcomeV1 {
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(evim_core_view_state(document.core, viewID, &outcome), operation: "Read view state")
        lastOutcome = outcome
        return outcome
    }

    @discardableResult
    func sendText(_ text: String) throws -> EvimCoreOutcomeV1 {
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        var effectBatch: EvimEffectBatchHandle = 0
        let data = Data(text.utf8)
        let status = withCommandTurnContext { context in
            data.withUnsafeBytes { raw in
                evim_core_view_send_text_with_host_context(
                    document.core,
                    viewID,
                    raw.bindMemory(to: UInt8.self).baseAddress,
                    UInt64(raw.count),
                    context,
                    &outcome,
                    &effectBatch
                )
            }
        }
        try finishHostContextTurn(
            status: status,
            outcome: outcome,
            effectBatch: effectBatch,
            operation: "Send text input"
        )
        return outcome
    }

    @discardableResult
    func sendKey(kind: UInt32, codepoint: UInt32 = 0) throws -> EvimCoreOutcomeV1 {
        var input = EvimKeyInputV1()
        input.struct_size = UInt32(MemoryLayout<EvimKeyInputV1>.size)
        input.kind = kind
        input.codepoint = codepoint
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        var effectBatch: EvimEffectBatchHandle = 0
        let status = withCommandTurnContext { context in
            evim_core_view_send_key_with_host_context(
                document.core,
                viewID,
                &input,
                context,
                &outcome,
                &effectBatch
            )
        }
        try finishHostContextTurn(
            status: status,
            outcome: outcome,
            effectBatch: effectBatch,
            operation: "Send key input"
        )
        return outcome
    }

    private func finishHostContextTurn(
        status: UInt32,
        outcome: EvimCoreOutcomeV1,
        effectBatch: EvimEffectBatchHandle,
        operation: String
    ) throws {
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
        if outcome.command_status == UInt32(EVIM_COMMAND_STATUS_READ_ONLY) {
            throw EVCoreFrontendError.command(operation: operation, status: outcome.command_status)
        }
    }

    func lineLocation() throws -> EvimViewLineLocationV1 {
        var value = EvimViewLineLocationV1(); value.struct_size = UInt32(MemoryLayout<EvimViewLineLocationV1>.size)
        try checked(evim_core_view_line_location(document.core, viewID, &value), operation: "Read line position")
        return value
    }

    func lineMode() throws -> EVLineMode {
        var value: UInt32 = 0
        try checked(evim_core_view_line_mode(document.core, viewID, &value), operation: "Read line mode")
        return EVLineMode(rawValue: value) ?? .visual
    }

    func setLineMode(_ mode: EVLineMode) throws {
        var outcome = EvimCoreOutcomeV1(); outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(evim_core_view_set_line_mode(document.core, viewID, mode.rawValue, &outcome), operation: "Change line mode")
        finish(outcome, composition: .cancelIfChanged)
    }

    func setThemePadding(_ padding: EVThemePadding) throws {
        try checked(evim_core_view_set_padding(document.core, viewID, Float(padding.top), Float(padding.left), Float(padding.bottom), Float(padding.right)), operation: "Update document padding")
    }

    func setSmartQuotes(_ enabled: Bool) throws {
        try checked(evim_core_view_set_smart_quotes(document.core, viewID, enabled ? 1 : 0), operation: "Update smart quotes")
    }

    func currentFontEnWidth() throws -> CGFloat {
        var info = EvimTypographyInfoV1()
        info.struct_size = UInt32(MemoryLayout<EvimTypographyInfoV1>.size)
        let status = evim_core_view_typography_export(document.core, viewID,
            try document.revision(), &info, nil, 0, nil, 0)
        if status != UInt32(EVIM_STATUS_BUFFER_TOO_SMALL) { try checked(status, operation: "Resolve caret font") }
        return CGFloat(info.size) / 2
    }

    @discardableResult
    func resize(width: CGFloat, height: CGFloat) throws -> EvimCoreOutcomeV1 {
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_resize(document.core, viewID, Float(max(width, 1)), Float(max(height, 1)), &outcome),
            operation: "Resize editor view"
        )
        viewportSize = CGSize(width: max(width, 1), height: max(height, 1))
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func setScale(_ scale: CGFloat) throws -> EvimCoreOutcomeV1 {
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_set_scale(document.core, viewID, Float(scale), &outcome),
            operation: "Change editor zoom"
        )
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    func semanticStylePresentation(
        _ style: UInt32
    ) throws -> EvimSemanticStylePresentationV1 {
        var value = EvimSemanticStylePresentationV1()
        value.struct_size = UInt32(MemoryLayout<EvimSemanticStylePresentationV1>.size)
        try checked(
            evim_core_view_semantic_style_presentation(
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
        expected selection: EvimLogicalSelectionIdentityV1
    ) throws -> EvimCoreOutcomeV1 {
        var request = EvimSetSemanticStyleV1()
        request.struct_size = UInt32(MemoryLayout<EvimSetSemanticStyleV1>.size)
        request.style = style
        request.enabled = enabled ? 1 : 0
        request.reserved = 0
        request.expected_selection = selection
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_set_semantic_style(
                document.core,
                viewID,
                &request,
                &outcome
            ),
            operation: enabled ? "Apply selection semantic style" : "Clear selection semantic style"
        )
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func useSelectionForFind(
        _ selection: EvimVisualSelectionIdentityV1
    ) throws -> EvimCoreOutcomeV1 {
        var expected = selection
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_use_selection_for_find(
                document.core,
                viewID,
                &expected,
                &outcome
            ),
            operation: "Use selection for find"
        )
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func revealSelection() throws -> EvimCoreOutcomeV1 {
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_reveal_selection(document.core, viewID, &outcome),
            operation: "Jump to selection"
        )
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func setWrap(_ enabled: Bool) throws -> EvimCoreOutcomeV1 {
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_set_wrap(document.core, viewID, enabled ? 1 : 0, &outcome),
            operation: "Change wrapping"
        )
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func setLinebreak(_ enabled: Bool) throws -> EvimCoreOutcomeV1 {
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_set_linebreak(document.core, viewID, enabled ? 1 : 0, &outcome),
            operation: "Change word-boundary wrapping"
        )
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func setFileFormat(
        _ fileFormat: UInt32,
        expected state: EvimDocumentStateV1
    ) throws -> EvimCoreOutcomeV1 {
        var request = EvimSetFileFormatV1()
        request.struct_size = UInt32(MemoryLayout<EvimSetFileFormatV1>.size)
        request.file_format = fileFormat
        request.document_id = state.document_id
        request.document_revision = state.document_revision
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_set_file_format(document.core, viewID, &request, &outcome),
            operation: "Change line endings"
        )
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    func listSelection() throws -> EvimLogicalSelectionIdentityV1 {
        var selection = EvimLogicalSelectionIdentityV1()
        selection.struct_size = UInt32(MemoryLayout<EvimLogicalSelectionIdentityV1>.size)
        try checked(evim_core_view_list_selection(document.core, viewID, &selection), operation: "Read list selection")
        return selection
    }

    @discardableResult
    func setListStyle(_ style: UInt32, expected selection: EvimLogicalSelectionIdentityV1) throws -> EvimCoreOutcomeV1 {
        var request = EvimSetListStyleV1()
        request.struct_size = UInt32(MemoryLayout<EvimSetListStyleV1>.size)
        request.style = style
        request.expected_selection = selection
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(evim_core_view_set_list_style(document.core, viewID, &request, &outcome), operation: "Change paragraph list")
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func createStyle(_ key: EVStyleKey, name: String, identity: EVStyleSheetIdentity) throws -> EvimCoreOutcomeV1 {
        var request = EvimCreateStyleV1()
        request.struct_size = UInt32(MemoryLayout<EvimCreateStyleV1>.size)
        request.namespace = key.namespace.rawValue
        request.identity = identity.abiValue
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        let idBytes = Array(key.id.rawValue.utf8)
        let nameBytes = Array(name.utf8)
        let status = idBytes.withUnsafeBufferPointer { id in
            nameBytes.withUnsafeBufferPointer { name in
                request.style_id.data = id.baseAddress
                request.style_id.length = UInt64(id.count)
                request.display_name.data = name.baseAddress
                request.display_name.length = UInt64(name.count)
                return evim_core_view_create_style(document.core, viewID, &request, &outcome)
            }
        }
        try checked(status, operation: "Create named style")
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func deleteStyle(_ key: EVStyleKey, identity: EVStyleSheetIdentity) throws -> EvimCoreOutcomeV1 {
        var request = EvimDeleteStyleV1()
        request.struct_size = UInt32(MemoryLayout<EvimDeleteStyleV1>.size)
        request.namespace = key.namespace.rawValue
        request.identity = identity.abiValue
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        let bytes = Array(key.id.rawValue.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            request.style_id.data = buffer.baseAddress
            request.style_id.length = UInt64(buffer.count)
            return evim_core_view_delete_style(document.core, viewID, &request, &outcome)
        }
        try checked(status, operation: "Delete named style")
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func assignStyle(_ key: EVStyleKey, identity: EVStyleSheetIdentity,
                     expected selection: EvimLogicalSelectionIdentityV1) throws -> EvimCoreOutcomeV1 {
        var request = EvimAssignStyleV1()
        request.struct_size = UInt32(MemoryLayout<EvimAssignStyleV1>.size)
        request.namespace = key.namespace.rawValue
        request.identity = identity.abiValue
        request.expected_selection = selection
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        let bytes = Array(key.id.rawValue.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            request.style_id.data = buffer.baseAddress
            request.style_id.length = UInt64(buffer.count)
            return evim_core_view_assign_style(document.core, viewID, &request, &outcome)
        }
        try checked(status, operation: "Assign named style")
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func setParagraphStyle(level: UInt32, expected selection: EvimLogicalSelectionIdentityV1) throws -> EvimCoreOutcomeV1 {
        var request = EvimSetParagraphStyleV1()
        request.struct_size = UInt32(MemoryLayout<EvimSetParagraphStyleV1>.size)
        request.level = level
        request.expected_selection = selection
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(evim_core_view_set_paragraph_style(document.core, viewID, &request, &outcome), operation: "Change paragraph style")
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func setFormat(_ format: EVSourceFormat, expected state: EvimDocumentStateV1) throws -> EvimCoreOutcomeV1 {
        var request = EvimSetFormatV1()
        request.struct_size = UInt32(MemoryLayout<EvimSetFormatV1>.size)
        request.format = switch format {
        case .plainText: UInt32(EVIM_FORMAT_PLAIN_TEXT)
        case .markdown: UInt32(EVIM_FORMAT_MARKDOWN)
        case .markdownSource: UInt32(EVIM_FORMAT_MARKDOWN_SOURCE)
        case .html: UInt32(EVIM_FORMAT_HTML)
        case .htmlSource: UInt32(EVIM_FORMAT_HTML_SOURCE)
        case .rtf: UInt32(EVIM_FORMAT_RTF)
        }
        request.document_id = state.document_id
        request.document_revision = state.document_revision
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        var effects: EvimEffectBatchHandle = 0
        let status = evim_core_view_set_format_with_effects(document.core, viewID, &request, &outcome, &effects)
        try finishHostContextTurn(status: status, outcome: outcome, effectBatch: effects, operation: "Change document format")
        return outcome
    }

    @discardableResult
    func setEncoding(_ encoding: UInt32, expected state: EvimDocumentStateV1) throws -> EvimCoreOutcomeV1 {
        var request = EvimSetEncodingV1()
        request.struct_size = UInt32(MemoryLayout<EvimSetEncodingV1>.size)
        request.encoding = encoding
        request.document_id = state.document_id
        request.document_revision = state.document_revision
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        var effects: EvimEffectBatchHandle = 0
        let status = evim_core_view_set_encoding_with_effects(document.core, viewID, &request, &outcome, &effects)
        try finishHostContextTurn(status: status, outcome: outcome, effectBatch: effects, operation: "Change document encoding")
        return outcome
    }

    @discardableResult
    func undo() throws -> EvimCoreOutcomeV1 {
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_undo(document.core, viewID, &outcome),
            operation: "Undo"
        )
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func redo() throws -> EvimCoreOutcomeV1 {
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_redo(document.core, viewID, &outcome),
            operation: "Redo"
        )
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    @discardableResult
    func setViewportOrigin(
        left: CGFloat,
        top: CGFloat? = nil,
        expected state: EvimViewportStateV1? = nil
    ) throws -> EvimCoreOutcomeV1 {
        var request = EvimViewportOriginV1()
        request.struct_size = UInt32(MemoryLayout<EvimViewportOriginV1>.size)
        request.flags = top == nil ? 0 : UInt32(EVIM_VIEWPORT_ORIGIN_HAS_TOP)
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

        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_set_viewport_origin(document.core, viewID, &request, &outcome),
            operation: top == nil ? "Scroll editor horizontally" : "Scroll editor vertically"
        )
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    func viewportState() throws -> EvimViewportStateV1 {
        var state = EvimViewportStateV1()
        state.struct_size = UInt32(MemoryLayout<EvimViewportStateV1>.size)
        try checked(
            evim_core_view_viewport_state(document.core, viewID, &state),
            operation: "Read viewport state"
        )
        return state
    }

    func presentation() throws -> EvimViewPresentationV1 {
        var value = EvimViewPresentationV1()
        value.struct_size = UInt32(MemoryLayout<EvimViewPresentationV1>.size)
        try checked(
            evim_core_view_presentation(document.core, viewID, &value),
            operation: "Read view presentation"
        )
        return value
    }

    func layoutExport() throws -> EVLayoutExport {
        var info = EvimLayoutSnapshotInfoV1()
        info.struct_size = UInt32(MemoryLayout<EvimLayoutSnapshotInfoV1>.size)
        let status = evim_core_view_layout_snapshot_info(document.core, viewID, &info)
        if status == Status.layoutUnavailable { throw EVCoreFrontendError.unavailableLayout }
        try checked(status, operation: "Read layout snapshot")
        guard info.row_count <= UInt64(Int.max),
              info.cluster_count <= UInt64(Int.max),
              info.caret_count <= UInt64(Int.max)
        else {
            throw EVCoreFrontendError.core(operation: "Read layout snapshot", status: Status.lengthOverflow)
        }

        var rows = Array(repeating: EvimVisualRowV1(), count: Int(info.row_count))
        var clusters = Array(repeating: EvimPositionedClusterV1(), count: Int(info.cluster_count))
        var carets = Array(repeating: EvimPositionedCaretV1(), count: Int(info.caret_count))
        var copiedInfo = EvimLayoutSnapshotInfoV1()
        copiedInfo.struct_size = UInt32(MemoryLayout<EvimLayoutSnapshotInfoV1>.size)
        var identity = info.identity
        let copied = rows.withUnsafeMutableBufferPointer { rowBuffer in
            clusters.withUnsafeMutableBufferPointer { clusterBuffer in
                carets.withUnsafeMutableBufferPointer { caretBuffer in
                    evim_core_view_copy_layout_snapshot(
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
        return EVLayoutExport(info: copiedInfo, rows: rows, clusters: clusters, carets: carets)
    }

    /// Font registration can retire the presentation while the view is idle.
    /// Refresh through an explicit core presentation event before requesting
    /// a new identity; never reuse an old hit-test identity or replay an edit.
    func refreshLayoutIfNeeded() throws {
        var info = EvimLayoutSnapshotInfoV1()
        info.struct_size = UInt32(MemoryLayout<EvimLayoutSnapshotInfoV1>.size)
        let status = evim_core_view_layout_snapshot_info(document.core, viewID, &info)
        guard status == Status.layoutUnavailable else {
            try checked(status, operation: "Validate editor layout")
            return
        }
        _ = try resize(width: viewportSize.width, height: viewportSize.height)
    }

    func hitTest(_ point: CGPoint, in snapshot: EvimLayoutSnapshotInfoV1) throws -> EvimLayoutCaretPointV1 {
        var request = EvimLayoutHitTestRequestV1()
        request.struct_size = UInt32(MemoryLayout<EvimLayoutHitTestRequestV1>.size)
        request.identity = snapshot.identity
        request.x = Float(point.x)
        request.y = Float(point.y)
        var result = EvimLayoutCaretPointV1()
        result.struct_size = UInt32(MemoryLayout<EvimLayoutCaretPointV1>.size)
        try checked(
            evim_core_view_layout_hit_test(document.core, viewID, &request, &result),
            operation: "Hit-test editor"
        )
        return result
    }

    @discardableResult
    func placeCursor(_ point: EvimLayoutCaretPointV1, extendSelection: Bool) throws -> EvimCoreOutcomeV1 {
        var request = EvimPlaceCursorV1()
        request.struct_size = UInt32(MemoryLayout<EvimPlaceCursorV1>.size)
        request.flags = extendSelection ? UInt32(EVIM_PLACE_CURSOR_EXTEND_SELECTION) : 0
        request.document_revision = point.document_revision
        request.text_offset = point.text_offset
        request.affinity = point.affinity
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(
            evim_core_view_place_cursor(document.core, viewID, &request, &outcome),
            operation: "Place editor cursor"
        )
        finish(outcome, composition: .cancelIfChanged)
        return outcome
    }

    func caretGeometry(
        offset: UInt64,
        affinity: UInt32,
        in snapshot: EvimLayoutSnapshotInfoV1
    ) throws -> EvimLayoutCaretGeometryV1 {
        var request = EvimLayoutCaretRequestV1()
        request.struct_size = UInt32(MemoryLayout<EvimLayoutCaretRequestV1>.size)
        request.affinity = affinity
        request.identity = snapshot.identity
        request.text_offset = offset
        var geometry = EvimLayoutCaretGeometryV1()
        geometry.struct_size = UInt32(MemoryLayout<EvimLayoutCaretGeometryV1>.size)
        try checked(
            evim_core_view_caret_geometry(document.core, viewID, &request, &geometry),
            operation: "Read caret geometry"
        )
        return geometry
    }

    @discardableResult
    func beginComposition(replacing range: Range<UInt64>) throws -> EvimCoreOutcomeV1 {
        var request = EvimCompositionBeginV1()
        request.struct_size = UInt32(MemoryLayout<EvimCompositionBeginV1>.size)
        request.document_revision = try document.revision()
        request.replacement_start = range.lowerBound
        request.replacement_end = range.upperBound
        return try compositionCall("Begin marked text", transition: .activate) { outcome in
            evim_core_view_composition_begin(document.core, viewID, &request, outcome)
        }
    }

    @discardableResult
    func updateComposition(_ text: String, selected: Range<UInt64>) throws -> EvimCoreOutcomeV1 {
        var request = EvimCompositionUpdateV1()
        request.struct_size = UInt32(MemoryLayout<EvimCompositionUpdateV1>.size)
        request.document_revision = try document.revision()
        request.selected_start = selected.lowerBound
        request.selected_end = selected.upperBound
        let data = Data(text.utf8)
        return try data.withUnsafeBytes { raw in
            request.marked_text.data = raw.bindMemory(to: UInt8.self).baseAddress
            request.marked_text.length = UInt64(raw.count)
            return try compositionCall("Update marked text", transition: .activate) { outcome in
                evim_core_view_composition_update(document.core, viewID, &request, outcome)
            }
        }
    }

    @discardableResult
    func commitComposition(_ text: String) throws -> EvimCoreOutcomeV1 {
        var request = EvimCompositionCommitV1()
        request.struct_size = UInt32(MemoryLayout<EvimCompositionCommitV1>.size)
        request.document_revision = try document.revision()
        let data = Data(text.utf8)
        return try data.withUnsafeBytes { raw in
            request.committed_text.data = raw.bindMemory(to: UInt8.self).baseAddress
            request.committed_text.length = UInt64(raw.count)
            return try compositionCall("Commit marked text", transition: .deactivate) { outcome in
                evim_core_view_composition_commit(document.core, viewID, &request, outcome)
            }
        }
    }

    @discardableResult
    func cancelComposition() throws -> EvimCoreOutcomeV1 {
        var request = EvimCompositionCancelV1()
        request.struct_size = UInt32(MemoryLayout<EvimCompositionCancelV1>.size)
        request.document_revision = try document.revision()
        return try compositionCall("Cancel marked text", transition: .deactivate) { outcome in
            evim_core_view_composition_cancel(document.core, viewID, &request, outcome)
        }
    }

    private func compositionCall(
        _ operation: String,
        transition: CompositionTransition,
        _ body: (UnsafeMutablePointer<EvimCoreOutcomeV1>) -> UInt32
    ) throws -> EvimCoreOutcomeV1 {
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        try checked(body(&outcome), operation: operation)
        finish(outcome, composition: transition)
        return outcome
    }

    func noteExternalDocumentChange() {
        publishCompositionState(false)
    }

    func finishStyleEdit(_ outcome: EvimCoreOutcomeV1) {
        finish(outcome)
    }

    private func finish(
        _ outcome: EvimCoreOutcomeV1,
        composition transition: CompositionTransition = .cancelIfChanged
    ) {
        lastOutcome = outcome
        switch transition {
        case .activate:
            publishCompositionState(true)
        case .deactivate:
            publishCompositionState(false)
        case .cancelIfChanged:
            if outcome.flags & UInt32(EVIM_OUTCOME_HAS_COMPOSITION_CHANGES) != 0 {
                publishCompositionState(false)
            }
        }
        if outcome.flags & UInt32(EVIM_OUTCOME_DOCUMENT_CHANGED) != 0 {
            document.noteSourceChange(originatingViewID: viewID)
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
