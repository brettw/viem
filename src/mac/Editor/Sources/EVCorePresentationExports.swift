import CEvimCore
import Foundation

struct EVFormattedSnapshot {
    var info: EvimFormattedSnapshotInfoV1

    var utf8Length: Int? { Int(exactly: info.utf8_length) }
    var utf16Length: Int? { Int(exactly: info.utf16_length) }
}

/// A scalar-aligned, exact-snapshot window into the formatted projection.
/// Its byte offsets remain document-global so layout clusters can address it
/// without copying or rebasing the surrounding document.
struct EVFormattedTextSlice {
    let identity: EvimFormattedSnapshotIdentityV1
    let utf8Range: Range<UInt64>
    let bytes: [UInt8]

    func text(in requestedRange: Range<Int>) -> String? {
        guard let lower = UInt64(exactly: requestedRange.lowerBound),
              let upper = UInt64(exactly: requestedRange.upperBound),
              utf8Range.lowerBound <= lower,
              lower <= upper,
              upper <= utf8Range.upperBound,
              let localLower = Int(exactly: lower - utf8Range.lowerBound),
              let localUpper = Int(exactly: upper - utf8Range.lowerBound),
              localUpper <= bytes.count
        else { return nil }
        return String(bytes: bytes[localLower ..< localUpper], encoding: .utf8)
    }

    func byte(atUTF8Offset offset: Int) -> UInt8? {
        guard let offset = UInt64(exactly: offset), utf8Range.contains(offset),
              let local = Int(exactly: offset - utf8Range.lowerBound),
              local < bytes.count
        else { return nil }
        return bytes[local]
    }
}

struct EVCompositionOverlayExport {
    var info: EvimCompositionOverlayInfoV1

    var utf8Length: Int? { Int(exactly: info.utf8_length) }
    var markedUTF8Range: Range<Int>? {
        guard let start = Int(exactly: info.marked_start),
              let end = Int(exactly: info.marked_end), start <= end
        else { return nil }
        return start ..< end
    }
    var selectedUTF8Range: Range<Int>? {
        guard let start = Int(exactly: info.selected_start),
              let end = Int(exactly: info.selected_end), start <= end
        else { return nil }
        return start ..< end
    }
}

struct EVCompositionTextSlice {
    let identity: EvimCompositionOverlayIdentityV1
    let utf8Range: Range<UInt64>
    let bytes: [UInt8]

    func text(in requestedRange: Range<Int>) -> String? {
        guard let lower = UInt64(exactly: requestedRange.lowerBound),
              let upper = UInt64(exactly: requestedRange.upperBound),
              utf8Range.lowerBound <= lower,
              lower <= upper,
              upper <= utf8Range.upperBound,
              let localLower = Int(exactly: lower - utf8Range.lowerBound),
              let localUpper = Int(exactly: upper - utf8Range.lowerBound),
              localUpper <= bytes.count
        else { return nil }
        return String(bytes: bytes[localLower ..< localUpper], encoding: .utf8)
    }

    func byte(atUTF8Offset offset: Int) -> UInt8? {
        guard let offset = UInt64(exactly: offset), utf8Range.contains(offset),
              let local = Int(exactly: offset - utf8Range.lowerBound),
              local < bytes.count
        else { return nil }
        return bytes[local]
    }
}

/// Test-visible accounting at the Swift/C boundary. These counters count
/// logical Swift requests, not the range-copy size-query retry mandated by C.
struct EVFormattedAccessCounters: Equatable {
    var snapshotInfoCalls: UInt64 = 0
    var rangeReadCalls: UInt64 = 0
    var requestedUTF8Bytes: UInt64 = 0
    var maximumRangeReadBytes: UInt64 = 0
    var fullRangeReadCalls: UInt64 = 0
    var utf8ToUTF16BatchCalls: UInt64 = 0
    var utf16ToUTF8BatchCalls: UInt64 = 0
    var pointInfoCalls: UInt64 = 0

    /// The legacy whole-projection entry point is intentionally absent from
    /// the implementation. Keeping this explicit makes regression tests say
    /// exactly what they protect.
    let legacyWholeCopyCalls: UInt64 = 0
}

struct EVLayoutPaintExport {
    var info: EvimLayoutPaintInfoV1
    var runs: [EvimPaintStyleRunV1]
}

struct EVCommandLineExport {
    var info: EvimCommandLineInfoV1
    var text: String
    var selectionAnchorUTF8Offset: UInt64 = 0

    var selectedUTF8Range: Range<Int> {
        let active = Int(info.cursor_utf8_offset)
        let anchor = Int(selectionAnchorUTF8Offset)
        return min(anchor, active)..<max(anchor, active)
    }

    var prompt: Character? {
        switch info.identity.kind {
        case UInt32(EVIM_COMMAND_LINE_KIND_EX): ":"
        case UInt32(EVIM_COMMAND_LINE_KIND_SEARCH_FORWARD): "/"
        case UInt32(EVIM_COMMAND_LINE_KIND_SEARCH_BACKWARD): "?"
        default: nil
        }
    }
}

struct EVVisualSelectionExport {
    var info: EvimVisualSelectionInfoV1
    var segments: [EvimVisualSelectionSegmentV1]
    var rectangles: [EvimVisualSelectionRectangleV1]
}

extension EVCoreViewSession {
    func compositionOverlayExport() throws -> EVCompositionOverlayExport? {
        var info = EvimCompositionOverlayInfoV1()
        info.struct_size = UInt32(MemoryLayout<EvimCompositionOverlayInfoV1>.size)
        try checkedPresentationExport(
            evim_core_view_composition_overlay_info(document.core, viewID, &info),
            operation: "Read composition overlay"
        )
        guard info.flags & UInt32(EVIM_COMPOSITION_OVERLAY_ACTIVE) != 0 else {
            return nil
        }
        guard info.identity.struct_size
                >= UInt32(MemoryLayout<EvimCompositionOverlayIdentityV1>.size),
              info.identity.view_id == viewID,
              info.replacement_start <= info.replacement_end,
              info.marked_start <= info.marked_end,
              info.selected_start <= info.selected_end,
              info.marked_end <= info.utf8_length,
              info.selected_end <= info.utf8_length
        else {
            throw EVCoreFrontendError.core(
                operation: "Validate composition overlay",
                status: UInt32(EVIM_STATUS_CORE_FAILURE)
            )
        }
        return EVCompositionOverlayExport(info: info)
    }

    func compositionTextSlice(
        in range: Range<UInt64>,
        overlay: EVCompositionOverlayExport
    ) throws -> EVCompositionTextSlice {
        var request = EvimCompositionOverlayUtf8RangeV1()
        request.struct_size = UInt32(MemoryLayout<EvimCompositionOverlayUtf8RangeV1>.size)
        request.identity = overlay.info.identity
        request.start = range.lowerBound
        request.end = range.upperBound
        var required: UInt64 = 0
        let query = evim_core_view_copy_composition_utf8_range(
            document.core,
            viewID,
            &request,
            nil,
            0,
            &required
        )
        if range.isEmpty {
            try checkedPresentationExport(query, operation: "Read composition text")
        } else if query != UInt32(EVIM_STATUS_BUFFER_TOO_SMALL) {
            try checkedPresentationExport(query, operation: "Size composition text")
        }
        guard required <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(
                operation: "Read composition text",
                status: UInt32(EVIM_STATUS_LENGTH_OVERFLOW)
            )
        }
        var bytes = Array(repeating: UInt8(0), count: Int(required))
        let status = bytes.withUnsafeMutableBufferPointer { buffer in
            evim_core_view_copy_composition_utf8_range(
                document.core,
                viewID,
                &request,
                buffer.baseAddress,
                UInt64(buffer.count),
                &required
            )
        }
        try checkedPresentationExport(status, operation: "Copy composition text")
        guard required == UInt64(bytes.count), String(bytes: bytes, encoding: .utf8) != nil else {
            throw EVCoreFrontendError.invalidUTF8
        }
        return EVCompositionTextSlice(
            identity: overlay.info.identity,
            utf8Range: range,
            bytes: bytes
        )
    }

    func layoutDecorationsExport(identity: EvimLayoutSnapshotIdentityV1) throws
        -> ([EvimLayoutDecorationV1], [UInt8]) {
        var expected = identity
        var info = EvimLayoutDecorationsInfoV1()
        let query = evim_core_view_copy_layout_decorations(document.core, viewID, &expected,
                                                          nil, 0, nil, 0, &info)
        if query != UInt32(EVIM_STATUS_BUFFER_TOO_SMALL) {
            try checkedPresentationExport(query, operation: "Read list marker layout")
        }
        guard info.decoration_count <= UInt64(Int.max), info.label_bytes <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(operation: "Read list marker layout", status: UInt32(EVIM_STATUS_LENGTH_OVERFLOW))
        }
        if info.decoration_count == 0 { return ([], []) }
        var decorations = Array(repeating: EvimLayoutDecorationV1(), count: Int(info.decoration_count))
        var labels = Array(repeating: UInt8(0), count: Int(info.label_bytes))
        let copied = decorations.withUnsafeMutableBufferPointer { items in
            labels.withUnsafeMutableBufferPointer { bytes in
                evim_core_view_copy_layout_decorations(document.core, viewID, &expected,
                    items.baseAddress, UInt64(items.count), bytes.baseAddress, UInt64(bytes.count), &info)
            }
        }
        try checkedPresentationExport(copied, operation: "Copy list marker layout")
        guard decorations.allSatisfy({ item in
            item.struct_size >= UInt32(MemoryLayout<EvimLayoutDecorationV1>.size)
                && item.label_byte_start <= UInt64(labels.count)
                && item.label_byte_length <= UInt64(labels.count) - item.label_byte_start
                && item.paint.struct_size >= UInt32(MemoryLayout<EvimTextPaintV1>.size)
        }), String(bytes: labels, encoding: .utf8) != nil else {
            throw EVCoreFrontendError.core(operation: "Copy list marker layout", status: UInt32(EVIM_STATUS_CORE_FAILURE))
        }
        return (decorations, labels)
    }

    func layoutPaintExport() throws -> EVLayoutPaintExport {
        var info = EvimLayoutPaintInfoV1()
        info.struct_size = UInt32(MemoryLayout<EvimLayoutPaintInfoV1>.size)
        try checkedPresentationExport(
            evim_core_view_layout_paint_info(document.core, viewID, &info),
            operation: "Read layout paint"
        )
        guard info.paint_run_count <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(
                operation: "Read layout paint",
                status: UInt32(EVIM_STATUS_LENGTH_OVERFLOW)
            )
        }
        guard info.default_paint.struct_size >= UInt32(MemoryLayout<EvimTextPaintV1>.size)
        else {
            throw EVCoreFrontendError.core(
                operation: "Read layout paint",
                status: UInt32(EVIM_STATUS_CORE_FAILURE)
            )
        }

        var runs = Array(
            repeating: EvimPaintStyleRunV1(),
            count: Int(info.paint_run_count)
        )
        var copiedInfo = EvimLayoutPaintInfoV1()
        copiedInfo.struct_size = UInt32(MemoryLayout<EvimLayoutPaintInfoV1>.size)
        var identity = info.identity
        let status = runs.withUnsafeMutableBufferPointer { buffer in
            evim_core_view_copy_layout_paint(
                document.core,
                viewID,
                &identity,
                buffer.baseAddress,
                UInt64(buffer.count),
                &copiedInfo
            )
        }
        try checkedPresentationExport(status, operation: "Copy layout paint")
        guard copiedInfo.paint_run_count == UInt64(runs.count),
              copiedInfo.default_paint.struct_size >= UInt32(MemoryLayout<EvimTextPaintV1>.size),
              runs.allSatisfy({
                      $0.struct_size >= UInt32(MemoryLayout<EvimPaintStyleRunV1>.size)
                      && $0.paint.struct_size >= UInt32(MemoryLayout<EvimTextPaintV1>.size)
                      && $0.text_start < $0.text_end
              }),
              runs.indices.dropFirst().allSatisfy({
                  runs[$0 - 1].text_end <= runs[$0].text_start
              })
        else {
            throw EVCoreFrontendError.core(
                operation: "Copy layout paint",
                status: UInt32(EVIM_STATUS_CORE_FAILURE)
            )
        }
        return EVLayoutPaintExport(info: copiedInfo, runs: runs)
    }

    func commandLineExport() throws -> EVCommandLineExport {
        var info = EvimCommandLineInfoV1()
        info.struct_size = UInt32(MemoryLayout<EvimCommandLineInfoV1>.size)
        try checkedPresentationExport(
            evim_core_view_command_line_info(document.core, viewID, &info),
            operation: "Read command line"
        )
        guard info.utf8_length <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(
                operation: "Read command line",
                status: UInt32(EVIM_STATUS_LENGTH_OVERFLOW)
            )
        }

        var bytes = Array(repeating: UInt8(0), count: Int(info.utf8_length))
        var copiedInfo = EvimCommandLineInfoV1()
        copiedInfo.struct_size = UInt32(MemoryLayout<EvimCommandLineInfoV1>.size)
        var identity = info.identity
        let status = bytes.withUnsafeMutableBufferPointer { buffer in
            evim_core_view_copy_command_line(
                document.core,
                viewID,
                &identity,
                buffer.baseAddress,
                UInt64(buffer.count),
                &copiedInfo
            )
        }
        try checkedPresentationExport(status, operation: "Copy command line")
        guard copiedInfo.utf8_length == UInt64(bytes.count),
              copiedInfo.cursor_utf8_offset <= copiedInfo.utf8_length,
              let text = String(bytes: bytes, encoding: .utf8)
        else {
            throw EVCoreFrontendError.invalidUTF8
        }
        var selection = EvimCommandLineSelectionV1()
        selection.struct_size = UInt32(MemoryLayout<EvimCommandLineSelectionV1>.size)
        try checkedPresentationExport(evim_core_view_command_line_selection(document.core, viewID, &identity, &selection), operation: "Read command selection")
        return EVCommandLineExport(info: copiedInfo, text: text, selectionAnchorUTF8Offset: selection.anchor_utf8_offset)
    }

    func visualSelectionExport() throws -> EVVisualSelectionExport {
        var info = EvimVisualSelectionInfoV1()
        info.struct_size = UInt32(MemoryLayout<EvimVisualSelectionInfoV1>.size)
        try checkedPresentationExport(
            evim_core_view_visual_selection_info(document.core, viewID, &info),
            operation: "Read Visual selection"
        )
        guard info.segment_count <= UInt64(Int.max),
              info.rectangle_count <= UInt64(Int.max)
        else {
            throw EVCoreFrontendError.core(
                operation: "Read Visual selection",
                status: UInt32(EVIM_STATUS_LENGTH_OVERFLOW)
            )
        }

        var segments = Array(
            repeating: EvimVisualSelectionSegmentV1(),
            count: Int(info.segment_count)
        )
        var rectangles = Array(
            repeating: EvimVisualSelectionRectangleV1(),
            count: Int(info.rectangle_count)
        )
        var copiedInfo = EvimVisualSelectionInfoV1()
        copiedInfo.struct_size = UInt32(MemoryLayout<EvimVisualSelectionInfoV1>.size)
        var identity = info.identity
        let status = segments.withUnsafeMutableBufferPointer { segmentBuffer in
            rectangles.withUnsafeMutableBufferPointer { rectangleBuffer in
                evim_core_view_copy_visual_selection(
                    document.core,
                    viewID,
                    &identity,
                    segmentBuffer.baseAddress,
                    UInt64(segmentBuffer.count),
                    rectangleBuffer.baseAddress,
                    UInt64(rectangleBuffer.count),
                    &copiedInfo
                )
            }
        }
        try checkedPresentationExport(status, operation: "Copy Visual selection")
        guard copiedInfo.segment_count == UInt64(segments.count),
              copiedInfo.rectangle_count == UInt64(rectangles.count)
        else {
            throw EVCoreFrontendError.core(
                operation: "Copy Visual selection",
                status: UInt32(EVIM_STATUS_CORE_FAILURE)
            )
        }
        return EVVisualSelectionExport(
            info: copiedInfo,
            segments: segments,
            rectangles: rectangles
        )
    }
}

extension EvimLayoutSnapshotIdentityV1 {
    func isSameLayout(as other: EvimLayoutSnapshotIdentityV1) -> Bool {
        view_id == other.view_id
            && document_id == other.document_id
            && document_revision == other.document_revision
            && layout_revision == other.layout_revision
            && configuration_generation == other.configuration_generation
            && measurement_environment_id == other.measurement_environment_id
            && metrics_generation == other.metrics_generation
    }
}

extension EvimFormattedSnapshotIdentityV1 {
    func isSameSnapshot(as other: EvimFormattedSnapshotIdentityV1) -> Bool {
        document_id == other.document_id
            && document_revision == other.document_revision
    }
}

private func checkedPresentationExport(_ status: UInt32, operation: String) throws {
    guard status == UInt32(EVIM_STATUS_OK) else {
        throw EVCoreFrontendError.core(operation: operation, status: status)
    }
}
