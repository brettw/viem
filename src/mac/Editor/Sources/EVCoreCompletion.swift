import CViemCore
import Foundation

struct EVCompletionExport {
    let info: ViemCompletionInfoV1
    let items: [String]

    var isActive: Bool { info.flags & UInt32(VIEM_COMPLETION_ACTIVE) != 0 }
    var isSearching: Bool { info.flags & UInt32(VIEM_COMPLETION_SEARCHING) != 0 }
    var isTruncated: Bool { info.flags & UInt32(VIEM_COMPLETION_TRUNCATED) != 0 }
    var hasAnchor: Bool { info.flags & UInt32(VIEM_COMPLETION_HAS_ANCHOR) != 0 }
    var rightToLeft: Bool { info.flags & UInt32(VIEM_COMPLETION_RIGHT_TO_LEFT) != 0 }
    var selectedIndex: Int? { info.selected_index >= 0 ? Int(exactly: info.selected_index) : nil }
}

extension EVCoreViewSession {
    func completionExport() throws -> EVCompletionExport {
        var info = ViemCompletionInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemCompletionInfoV1>.size)
        try checkedCompletion(viem_core_view_completion_info(document.core, viewID, &info), operation: "Read word completion")
        guard let count = Int(exactly: info.item_count),
              let length = Int(exactly: info.utf8_length),
              info.selected_index == -1 || (info.selected_index >= 0 && UInt64(info.selected_index) < info.item_count)
        else { throw EVCoreFrontendError.core(operation: "Read word completion", status: UInt32(VIEM_STATUS_LENGTH_OVERFLOW)) }
        if count == 0, length == 0 { return EVCompletionExport(info: info, items: []) }
        var records = Array(repeating: ViemCompletionItemV1(), count: count)
        var bytes = Array(repeating: UInt8(0), count: length)
        var copiedCount: UInt64 = 0
        try records.withUnsafeMutableBufferPointer { buffer in
            try checkedCompletion(viem_core_view_copy_completion_items(
                document.core, viewID, &info, buffer.baseAddress, UInt64(buffer.count), &copiedCount
            ), operation: "Read word completion items")
        }
        guard copiedCount == info.item_count else {
            throw EVCoreFrontendError.core(operation: "Read word completion items", status: UInt32(VIEM_STATUS_STALE_REVISION))
        }
        try bytes.withUnsafeMutableBufferPointer { buffer in
            try checkedCompletion(viem_core_view_copy_completion_utf8(
                document.core, viewID, &info, buffer.baseAddress, UInt64(buffer.count), &copiedCount
            ), operation: "Read word completion text")
        }
        guard copiedCount == info.utf8_length else {
            throw EVCoreFrontendError.core(operation: "Read word completion text", status: UInt32(VIEM_STATUS_STALE_REVISION))
        }
        let items = try records.map { record -> String in
            guard record.text_offset <= info.utf8_length,
                  record.text_length <= info.utf8_length - record.text_offset,
                  let start = Int(exactly: record.text_offset),
                  let size = Int(exactly: record.text_length),
                  let text = String(bytes: bytes[start ..< start + size], encoding: .utf8)
            else { throw EVCoreFrontendError.invalidUTF8 }
            return text
        }
        return EVCompletionExport(info: info, items: items)
    }

    /// A bounded core work slice. Scheduling is native; searching, ordering,
    /// stale-result rejection, and selected-index updates remain portable.
    @discardableResult
    func pollCompletion() throws -> Bool {
        var changed: UInt8 = 0
        try checkedCompletion(viem_core_view_poll_completion(document.core, viewID, &changed), operation: "Continue word completion")
        return changed != 0
    }

    @discardableResult
    func acceptCompletion() throws -> Bool {
        let previousRevision = try document.revision()
        var changed: UInt8 = 0
        try checkedCompletion(viem_core_view_accept_completion(document.core, viewID, &changed), operation: "Accept word completion")
        if changed != 0 {
            _ = try refreshState()
            if try document.revision() != previousRevision {
                document.noteSourceChange(originatingViewID: viewID)
            } else {
                document.notePresentationChange(originatingViewID: viewID)
            }
        }
        return changed != 0
    }
}

private func checkedCompletion(_ status: UInt32, operation: String) throws {
    guard status == UInt32(VIEM_STATUS_OK) else {
        throw EVCoreFrontendError.core(operation: operation, status: status)
    }
}
