import AppKit
import CViemCore
import ViemAppShell
import Foundation

@MainActor
protocol EVPasteboardAccess: AnyObject {
    var viemGeneration: UInt64 { get }
    var viemIsWritable: Bool { get }
    func viemString() -> String?
    func viemCanReadString() -> Bool
    @discardableResult func viemClearContents() -> Int
    @discardableResult func viemSetString(_ string: String) -> Bool
    func viemData(forType type: NSPasteboard.PasteboardType) -> Data?
    @discardableResult func viemWrite(_ content: EVClipboardRepresentations) -> Bool
}

extension EVPasteboardAccess {
    func viemData(forType type: NSPasteboard.PasteboardType) -> Data? { nil }
    @discardableResult func viemWrite(_ content: EVClipboardRepresentations) -> Bool {
        guard viemIsWritable else { return false }
        viemClearContents()
        return viemSetString(content.plainText)
    }
}

struct EVClipboardRepresentations {
    static let fragmentType = NSPasteboard.PasteboardType("com.viem.clipboard.fragment.v1")
    let plainText: String
    var richText: Data? = nil
    var fragment: Data? = nil
}

/// Do not retain AppKit's process-wide pasteboard proxy in every surface.
/// XCTest tears down autorelease pools between cases and older proxy instances
/// can become unusable even though `NSPasteboard.general` itself remains the
/// right native target. A stable Swift owner resolves the current proxy for
/// each operation and also makes sequential surface lifetimes deterministic.
@MainActor
final class EVAppKitPasteboardAccess: EVPasteboardAccess {
    static let shared = EVAppKitPasteboardAccess()
    static let find = EVAppKitPasteboardAccess(NSPasteboard(name: .find))

    private let explicitPasteboard: NSPasteboard?

    init(_ pasteboard: NSPasteboard? = nil) {
        explicitPasteboard = pasteboard
    }

    private var nativePasteboard: NSPasteboard {
        explicitPasteboard ?? NSPasteboard.general
    }

    var viemGeneration: UInt64 { UInt64(max(nativePasteboard.changeCount, 0)) }
    var viemIsWritable: Bool { true }
    func viemString() -> String? { nativePasteboard.string(forType: .string) }
    func viemCanReadString() -> Bool {
        nativePasteboard.canReadObject(forClasses: [NSString.self])
    }
    func viemClearContents() -> Int { nativePasteboard.clearContents() }
    func viemSetString(_ string: String) -> Bool {
        nativePasteboard.setString(string, forType: .string)
    }
    func viemData(forType type: NSPasteboard.PasteboardType) -> Data? {
        nativePasteboard.data(forType: type)
    }
    @discardableResult func viemWrite(_ content: EVClipboardRepresentations) -> Bool {
        let item = NSPasteboardItem()
        guard item.setString(content.plainText, forType: .string) else { return false }
        if let data = content.richText, !item.setData(data, forType: .rtf) { return false }
        if let data = content.fragment, !item.setData(data, forType: EVClipboardRepresentations.fragmentType) { return false }
        let board = nativePasteboard
        board.clearContents()
        return board.writeObjects([item])
    }
}

struct EVClipboardTurnSnapshot: Equatable {
    let target: UInt32
    let generation: UInt64
    let plainText: String?
    let isWritable: Bool
    var fragmentJSON: Data? = nil
}

struct EVHostEffectAccessCounters: Equatable {
    var infoCalls: UInt64 = 0
    var copyCalls: UInt64 = 0
    var releaseCalls: UInt64 = 0
    var lastReleasedHandle: ViemEffectBatchHandle = 0
}

struct EVClipboardWriteEffect: Equatable {
    let target: UInt32
    let registerKind: UInt32
    let documentID: UInt64
    let documentRevision: UInt64
    let plainText: String
    let hardBreaks: [UInt64]
    var fragmentJSON: Data? = nil
}

enum EVExOptionValue: Equatable {
    case boolean(Bool)
    case fileFormat(UInt32)
    case fileFormats([UInt32])
    case number(UInt32)
    case string(String)
}

struct EVExOptionEffect: Equatable {
    let name: UInt32
    let value: EVExOptionValue
}

struct EVExMarkEffect: Equatable {
    let name: UnicodeScalar
    let utf8Offset: UInt64
    let hardLineIndex: UInt64
    let hardLineRange: Range<UInt64>
    let graphemeColumn: UInt64
    let lineText: String
}

struct EVExRegisterEffect: Equatable {
    let name: UnicodeScalar
    let registerKind: UInt32
    let text: String
    let hardBreaks: [UInt64]
}

struct EVExJumpEffect: Equatable {
    let isCurrent: Bool
    let listIndex: UInt64
    let utf8Offset: UInt64
    let hardLineIndex: UInt64
    let hardLineRange: Range<UInt64>
    let graphemeColumn: UInt64
    let lineText: String
}

struct EVExTextLineEffect: Equatable {
    let hardLineIndex: UInt64
    let utf8Range: Range<UInt64>
    let text: String
}

struct EVExHostEffect: Equatable {
    let kind: UInt32
    let flags: UInt32
    /// One VIEM_WINDOW_* value when `kind` is a window request.
    let windowCommand: UInt32
    /// Window count/index, or initial row height for split/new-pane requests.
    let windowCount: UInt64?
    let documentID: UInt64
    let documentRevision: UInt64
    let text: String
    let hardLineRange: ClosedRange<UInt64>?
    let options: [EVExOptionEffect]
    let marks: [EVExMarkEffect]
    let registers: [EVExRegisterEffect]
    let jumps: [EVExJumpEffect]
    let textLines: [EVExTextLineEffect]
    var windowTarget: UInt64? = nil
    var windowResizeMode: UInt32 = 0
    var argumentNavigation: EVArgumentNavigation? = nil
    var readAfterLine: UInt64? = nil
}

struct EVHostEffectBatch: Equatable {
    let documentID: UInt64
    let documentRevision: UInt64
    let flags: UInt32
    let navigationUTF8Offset: UInt64?
    let navigationRestoresHistory: Bool
    let substitutionCount: UInt64
    var clipboardWrites: [EVClipboardWriteEffect]
    let exEffects: [EVExHostEffect]
}

@MainActor
protocol EVCommandTurnHost: AnyObject {
    func clipboardSnapshotsForCommandTurn() -> [EVClipboardTurnSnapshot]
    func applyHostEffectBatch(_ batch: EVHostEffectBatch) throws
}

private struct EVRawEffectBatch {
    var info: ViemEffectBatchInfoV1
    var clipboardWrites: [ViemClipboardWriteV1]
    var exRequests: [ViemExFrontendRequestV1]
    var exOptions: [ViemExOptionDisplayV1]
    var exMarks: [ViemExMarkV1]
    var exRegisters: [ViemExRegisterV1]
    var exJumps: [ViemExJumpV1]
    var exTextLines: [ViemExTextLineV1]
    var fileFormats: [UInt32]
    var hardBreaks: [UInt64]
    var strings: [UInt8]
}

extension EVCoreViewSession {
    func withCommandTurnContext<Result>(
        _ body: (UnsafePointer<ViemCommandTurnContextV2>) -> Result
    ) -> Result {
        let snapshots = commandTurnHost?.clipboardSnapshotsForCommandTurn() ?? []
        var arena = Data()
        var ranges: [Range<Int>?] = []
        var fragmentRanges: [Range<Int>?] = []
        ranges.reserveCapacity(snapshots.count)
        for snapshot in snapshots {
            if let text = snapshot.plainText {
                let start = arena.count
                arena.append(contentsOf: text.utf8)
                ranges.append(start ..< arena.count)
            } else {
                ranges.append(nil)
            }
            if let fragment = snapshot.fragmentJSON, snapshot.plainText != nil {
                let start = arena.count
                arena.append(fragment)
                fragmentRanges.append(start ..< arena.count)
            } else { fragmentRanges.append(nil) }
        }

        return arena.withUnsafeBytes { bytes in
            let entries = snapshots.enumerated().map { index, snapshot in
                var entry = ViemClipboardTurnEntryV2()
                entry.struct_size = UInt32(MemoryLayout<ViemClipboardTurnEntryV2>.size)
                entry.target = snapshot.target
                entry.generation = snapshot.generation
                if snapshot.plainText != nil {
                    entry.flags |= UInt32(VIEM_CLIPBOARD_TURN_HAS_READ)
                    if let range = ranges[index] {
                        entry.plain_text.data = bytes.bindMemory(to: UInt8.self).baseAddress?
                            .advanced(by: range.lowerBound)
                        entry.plain_text.length = UInt64(range.count)
                    }
                }
                if snapshot.isWritable {
                    entry.flags |= UInt32(VIEM_CLIPBOARD_TURN_WRITABLE)
                }
                if let range = fragmentRanges[index] {
                    entry.fragment_json.data = bytes.bindMemory(to: UInt8.self).baseAddress?
                        .advanced(by: range.lowerBound)
                    entry.fragment_json.length = UInt64(range.count)
                }
                return entry
            }
            return entries.withUnsafeBufferPointer { entryBuffer in
                var context = ViemCommandTurnContextV2()
                context.struct_size = UInt32(MemoryLayout<ViemCommandTurnContextV2>.size)
                context.clipboards = entryBuffer.baseAddress
                context.clipboard_count = UInt64(entryBuffer.count)
                return withUnsafePointer(to: &context, body)
            }
        }
    }

    func copyAndReleaseHostEffectBatch(
        _ handle: ViemEffectBatchHandle
    ) throws -> EVHostEffectBatch {
        var copied: EVHostEffectBatch?
        var copyError: Error?
        do {
            copied = try copyHostEffectBatch(handle)
        } catch {
            copyError = error
        }

        let releaseStatus = viem_effect_batch_release(handle)
        hostEffectAccessCounters.releaseCalls &+= 1
        hostEffectAccessCounters.lastReleasedHandle = handle
        if let copyError { throw copyError }
        try hostEffectChecked(releaseStatus, operation: "Release host effects")
        guard let copied else {
            throw EVCoreFrontendError.invalidHostEffect
        }
        return copied
    }

    private func copyHostEffectBatch(_ handle: ViemEffectBatchHandle) throws -> EVHostEffectBatch {
        hostEffectAccessCounters.infoCalls &+= 1
        var info = ViemEffectBatchInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemEffectBatchInfoV1>.size)
        try hostEffectChecked(
            viem_effect_batch_info(handle, &info),
            operation: "Read host effects"
        )
        guard info.batch_handle == handle else { throw EVCoreFrontendError.invalidHostEffect }

        var raw = try EVRawEffectBatch(info: info)
        hostEffectAccessCounters.copyCalls &+= 1
        var copiedInfo = ViemEffectBatchInfoV1()
        copiedInfo.struct_size = UInt32(MemoryLayout<ViemEffectBatchInfoV1>.size)
        let status = raw.clipboardWrites.withUnsafeMutableBufferPointer { clipboardBuffer in
            raw.exRequests.withUnsafeMutableBufferPointer { requestBuffer in
                raw.exOptions.withUnsafeMutableBufferPointer { optionBuffer in
                    raw.exMarks.withUnsafeMutableBufferPointer { markBuffer in
                        raw.exRegisters.withUnsafeMutableBufferPointer { registerBuffer in
                            raw.exJumps.withUnsafeMutableBufferPointer { jumpBuffer in
                                raw.exTextLines.withUnsafeMutableBufferPointer { lineBuffer in
                                    raw.fileFormats.withUnsafeMutableBufferPointer { formatBuffer in
                                        raw.hardBreaks.withUnsafeMutableBufferPointer { breakBuffer in
                                            raw.strings.withUnsafeMutableBufferPointer { stringBuffer in
                                                viem_effect_batch_copy(
                                                    handle,
                                                    clipboardBuffer.baseAddress,
                                                    UInt64(clipboardBuffer.count),
                                                    requestBuffer.baseAddress,
                                                    UInt64(requestBuffer.count),
                                                    optionBuffer.baseAddress,
                                                    UInt64(optionBuffer.count),
                                                    markBuffer.baseAddress,
                                                    UInt64(markBuffer.count),
                                                    registerBuffer.baseAddress,
                                                    UInt64(registerBuffer.count),
                                                    jumpBuffer.baseAddress,
                                                    UInt64(jumpBuffer.count),
                                                    lineBuffer.baseAddress,
                                                    UInt64(lineBuffer.count),
                                                    formatBuffer.baseAddress,
                                                    UInt64(formatBuffer.count),
                                                    breakBuffer.baseAddress,
                                                    UInt64(breakBuffer.count),
                                                    stringBuffer.baseAddress,
                                                    UInt64(stringBuffer.count),
                                                    &copiedInfo
                                                )
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        try hostEffectChecked(status, operation: "Copy host effects")
        guard copiedInfo.batch_handle == handle,
              copiedInfo.document_id == info.document_id,
              copiedInfo.document_revision == info.document_revision,
              copiedInfo.clipboard_write_count == info.clipboard_write_count,
              copiedInfo.ex_request_count == info.ex_request_count,
              copiedInfo.ex_option_count == info.ex_option_count,
              copiedInfo.ex_mark_count == info.ex_mark_count,
              copiedInfo.ex_register_count == info.ex_register_count,
              copiedInfo.ex_jump_count == info.ex_jump_count,
              copiedInfo.ex_text_line_count == info.ex_text_line_count,
              copiedInfo.file_format_count == info.file_format_count,
              copiedInfo.hard_break_count == info.hard_break_count,
              copiedInfo.string_bytes == info.string_bytes
        else { throw EVCoreFrontendError.invalidHostEffect }
        var exported = try raw.exported()
        for index in exported.clipboardWrites.indices {
            exported.clipboardWrites[index].fragmentJSON = try readClipboardJSON(operation: "Read clipboard formats") { bytes, capacity, required in
                viem_effect_batch_copy_clipboard_json(handle, UInt64(index), bytes, capacity, required)
            }
        }
        return exported
    }
}

private extension EVRawEffectBatch {
    init(info: ViemEffectBatchInfoV1) throws {
        self.info = info
        clipboardWrites = Array(repeating: ViemClipboardWriteV1(), count: try hostEffectCount(info.clipboard_write_count))
        exRequests = Array(repeating: ViemExFrontendRequestV1(), count: try hostEffectCount(info.ex_request_count))
        exOptions = Array(repeating: ViemExOptionDisplayV1(), count: try hostEffectCount(info.ex_option_count))
        exMarks = Array(repeating: ViemExMarkV1(), count: try hostEffectCount(info.ex_mark_count))
        exRegisters = Array(repeating: ViemExRegisterV1(), count: try hostEffectCount(info.ex_register_count))
        exJumps = Array(repeating: ViemExJumpV1(), count: try hostEffectCount(info.ex_jump_count))
        exTextLines = Array(repeating: ViemExTextLineV1(), count: try hostEffectCount(info.ex_text_line_count))
        fileFormats = Array(repeating: 0, count: try hostEffectCount(info.file_format_count))
        hardBreaks = Array(repeating: 0, count: try hostEffectCount(info.hard_break_count))
        strings = Array(repeating: 0, count: try hostEffectCount(info.string_bytes))
    }

    func exported() throws -> EVHostEffectBatch {
        let writes = try clipboardWrites.map { value -> EVClipboardWriteEffect in
            guard value.struct_size >= UInt32(MemoryLayout<ViemClipboardWriteV1>.size),
                  value.document_id == info.document_id,
                  value.document_revision == info.document_revision,
                  value.target == UInt32(VIEM_CLIPBOARD_TARGET_CLIPBOARD)
                    || value.target == UInt32(VIEM_CLIPBOARD_TARGET_PRIMARY)
            else { throw EVCoreFrontendError.invalidHostEffect }
            let breaks = try slice(hardBreaks, first: value.first_hard_break, count: value.hard_break_count)
            return EVClipboardWriteEffect(
                target: value.target,
                registerKind: value.register_kind,
                documentID: value.document_id,
                documentRevision: value.document_revision,
                plainText: try string(value.plain_text),
                hardBreaks: Array(breaks)
            )
        }

        let requests = try exRequests.map { request -> EVExHostEffect in
            guard request.struct_size >= UInt32(MemoryLayout<ViemExFrontendRequestV1>.size),
                  request.document_id == info.document_id,
                  request.document_revision == info.document_revision
            else { throw EVCoreFrontendError.invalidHostEffect }
            let range: ClosedRange<UInt64>? = request.flags & UInt32(VIEM_EX_FRONTEND_HAS_RANGE) != 0
                ? request.hard_line_start ... request.hard_line_end
                : nil
            guard range == nil || request.hard_line_start <= request.hard_line_end else {
                throw EVCoreFrontendError.invalidHostEffect
            }
            let argumentNavigation: EVArgumentNavigation?
            if request.kind == UInt32(VIEM_EX_FRONTEND_ARGUMENT) {
                guard let target = EVArgumentNavigation.Target(rawValue: request.argument_command),
                      request.argument_count > 0
                else { throw EVCoreFrontendError.invalidHostEffect }
                argumentNavigation = EVArgumentNavigation(
                    target: target, count: request.argument_count,
                    writeFirst: request.flags & UInt32(VIEM_EX_FRONTEND_WRITE_FIRST) != 0,
                    line: request.flags & UInt32(VIEM_EX_FRONTEND_HAS_LINE) != 0
                        ? request.argument_line : nil
                )
            } else {
                argumentNavigation = nil
            }
            return EVExHostEffect(
                kind: request.kind,
                flags: request.flags,
                windowCommand: request.window_command,
                windowCount: request.flags & UInt32(VIEM_EX_FRONTEND_HAS_COUNT) != 0
                    ? request.window_count : nil,
                documentID: request.document_id,
                documentRevision: request.document_revision,
                text: try string(request.text),
                hardLineRange: range,
                options: try options(for: request),
                marks: try marks(for: request),
                registers: try registers(for: request),
                jumps: try jumps(for: request),
                textLines: try textLines(for: request),
                windowTarget: request.kind == UInt32(VIEM_EX_FRONTEND_WINDOW) && request.argument_count > 0 ? request.argument_count : nil,
                windowResizeMode: request.argument_command,
                argumentNavigation: argumentNavigation,
                readAfterLine: request.kind == UInt32(VIEM_EX_FRONTEND_READ) ? request.hard_line_start : nil
            )
        }

        let hasNavigation = info.flags & UInt32(VIEM_EFFECT_BATCH_EX_HAS_NAVIGATION) != 0
        return EVHostEffectBatch(
            documentID: info.document_id,
            documentRevision: info.document_revision,
            flags: info.flags,
            navigationUTF8Offset: hasNavigation ? info.navigation_utf8_offset : nil,
            navigationRestoresHistory: info.flags & UInt32(VIEM_EFFECT_BATCH_EX_NAVIGATION_HISTORY) != 0,
            substitutionCount: info.substitution_count,
            clipboardWrites: writes,
            exEffects: requests
        )
    }

    func options(for request: ViemExFrontendRequestV1) throws -> [EVExOptionEffect] {
        guard request.kind == UInt32(VIEM_EX_FRONTEND_OPTIONS) else {
            guard request.option_count == 0 else { throw EVCoreFrontendError.invalidHostEffect }
            return []
        }
        return try slice(exOptions, first: request.first_option, count: request.option_count).map { option in
            guard option.struct_size >= UInt32(MemoryLayout<ViemExOptionDisplayV1>.size) else {
                throw EVCoreFrontendError.invalidHostEffect
            }
            let value: EVExOptionValue
            switch option.value_kind {
            case UInt32(VIEM_EX_OPTION_VALUE_BOOLEAN):
                guard option.scalar_value <= 1, option.file_format_count == 0 else {
                    throw EVCoreFrontendError.invalidHostEffect
                }
                value = .boolean(option.scalar_value != 0)
            case UInt32(VIEM_EX_OPTION_VALUE_FILE_FORMAT):
                guard option.file_format_count == 0 else { throw EVCoreFrontendError.invalidHostEffect }
                value = .fileFormat(option.scalar_value)
            case UInt32(VIEM_EX_OPTION_VALUE_FILE_FORMATS):
                value = .fileFormats(Array(try slice(
                    fileFormats,
                    first: option.first_file_format,
                    count: option.file_format_count
                )))
            case UInt32(VIEM_EX_OPTION_VALUE_NUMBER):
                guard option.file_format_count == 0 else { throw EVCoreFrontendError.invalidHostEffect }
                value = .number(option.scalar_value)
            case UInt32(VIEM_EX_OPTION_VALUE_STRING):
                guard option.file_format_count == 0 else { throw EVCoreFrontendError.invalidHostEffect }
                value = .string(try string(option.text))
            default:
                throw EVCoreFrontendError.invalidHostEffect
            }
            return EVExOptionEffect(name: option.name, value: value)
        }
    }

    func marks(for request: ViemExFrontendRequestV1) throws -> [EVExMarkEffect] {
        guard request.kind == UInt32(VIEM_EX_FRONTEND_MARKS) else { return [] }
        return try slice(exMarks, first: request.first_payload, count: request.payload_count).map { mark in
            guard mark.struct_size >= UInt32(MemoryLayout<ViemExMarkV1>.size),
                  mark.hard_line_start <= mark.hard_line_end,
                  let name = UnicodeScalar(mark.name)
            else { throw EVCoreFrontendError.invalidHostEffect }
            return EVExMarkEffect(
                name: name,
                utf8Offset: mark.utf8_offset,
                hardLineIndex: mark.hard_line_index,
                hardLineRange: mark.hard_line_start ..< mark.hard_line_end,
                graphemeColumn: mark.grapheme_column,
                lineText: try string(mark.line_text)
            )
        }
    }

    func registers(for request: ViemExFrontendRequestV1) throws -> [EVExRegisterEffect] {
        guard request.kind == UInt32(VIEM_EX_FRONTEND_REGISTERS) else { return [] }
        return try slice(exRegisters, first: request.first_payload, count: request.payload_count).map { value in
            guard value.struct_size >= UInt32(MemoryLayout<ViemExRegisterV1>.size),
                  let name = UnicodeScalar(value.name)
            else { throw EVCoreFrontendError.invalidHostEffect }
            return EVExRegisterEffect(
                name: name,
                registerKind: value.register_kind,
                text: try string(value.text),
                hardBreaks: Array(try slice(
                    hardBreaks,
                    first: value.first_hard_break,
                    count: value.hard_break_count
                ))
            )
        }
    }

    func jumps(for request: ViemExFrontendRequestV1) throws -> [EVExJumpEffect] {
        guard request.kind == UInt32(VIEM_EX_FRONTEND_JUMPS) else { return [] }
        return try slice(exJumps, first: request.first_payload, count: request.payload_count).map { jump in
            guard jump.struct_size >= UInt32(MemoryLayout<ViemExJumpV1>.size),
                  jump.hard_line_start <= jump.hard_line_end
            else { throw EVCoreFrontendError.invalidHostEffect }
            return EVExJumpEffect(
                isCurrent: jump.flags & UInt32(VIEM_EX_JUMP_CURRENT) != 0,
                listIndex: jump.list_index,
                utf8Offset: jump.utf8_offset,
                hardLineIndex: jump.hard_line_index,
                hardLineRange: jump.hard_line_start ..< jump.hard_line_end,
                graphemeColumn: jump.grapheme_column,
                lineText: try string(jump.line_text)
            )
        }
    }

    func textLines(for request: ViemExFrontendRequestV1) throws -> [EVExTextLineEffect] {
        guard request.kind == UInt32(VIEM_EX_FRONTEND_PRINT_LINES) else { return [] }
        return try slice(exTextLines, first: request.first_payload, count: request.payload_count).map { line in
            guard line.struct_size >= UInt32(MemoryLayout<ViemExTextLineV1>.size),
                  line.utf8_start <= line.utf8_end
            else { throw EVCoreFrontendError.invalidHostEffect }
            return EVExTextLineEffect(
                hardLineIndex: line.hard_line_index,
                utf8Range: line.utf8_start ..< line.utf8_end,
                text: try string(line.text)
            )
        }
    }

    func string(_ reference: ViemEffectBytesRefV1) throws -> String {
        let bytes = try slice(strings, first: reference.offset, count: reference.length)
        guard let result = String(bytes: bytes, encoding: .utf8) else {
            throw EVCoreFrontendError.invalidHostEffect
        }
        return result
    }
}

private func slice<Element>(
    _ values: [Element],
    first: UInt64,
    count: UInt64
) throws -> ArraySlice<Element> {
    let lower = try hostEffectCount(first)
    let length = try hostEffectCount(count)
    let (upper, overflow) = lower.addingReportingOverflow(length)
    guard !overflow, upper <= values.count else { throw EVCoreFrontendError.invalidHostEffect }
    return values[lower ..< upper]
}

private func hostEffectCount(_ value: UInt64) throws -> Int {
    guard let result = Int(exactly: value) else { throw EVCoreFrontendError.invalidHostEffect }
    return result
}

private func hostEffectChecked(_ status: UInt32, operation: String) throws {
    guard status == UInt32(VIEM_STATUS_OK) else {
        throw EVCoreFrontendError.core(operation: operation, status: status)
    }
}
