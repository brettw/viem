import AppKit
import CEvimCore

extension EVCoreViewSession {
    @discardableResult
    func editCommandLine(_ snapshot: EVCommandLineExport, selecting range: Range<Int>? = nil, anchor: Int? = nil, active: Int? = nil, replacement: String? = nil) throws -> EvimCoreOutcomeV1 {
        var identity = snapshot.info.identity
        var outcome = EvimCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<EvimCoreOutcomeV1>.size)
        let start = range?.lowerBound ?? anchor ?? Int(snapshot.selectionAnchorUTF8Offset)
        let end = range?.upperBound ?? active ?? Int(snapshot.info.cursor_utf8_offset)
        let bytes = Array((replacement ?? "").utf8)
        let status = bytes.withUnsafeBufferPointer { bytes in
            evim_core_view_edit_command_line(document.core, viewID, &identity, replacement == nil ? 0 : 1,
                UInt64(start), UInt64(end), bytes.baseAddress, UInt64(bytes.count), &outcome)
        }
        guard status == EVIM_STATUS_OK else { throw EVCoreFrontendError.core(operation: "Edit command line", status: status) }
        finishStyleEdit(outcome)
        return outcome
    }
}
