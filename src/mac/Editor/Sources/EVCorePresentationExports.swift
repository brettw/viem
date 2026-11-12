import CViemCore
import Foundation

struct EVFormattedSnapshot {
    var info: ViemFormattedSnapshotInfo

    var utf8Length: Int? { Int(exactly: info.utf8_length) }
    var utf16Length: Int? { Int(exactly: info.utf16_length) }
}

typealias EVFormattedTextSlice = EVSnapshotTextSlice<ViemFormattedSnapshotIdentity>
typealias EVCompositionTextSlice = EVSnapshotTextSlice<ViemCompositionOverlayIdentity>

/// A scalar-aligned window whose identity distinguishes formatted text from a
/// composition overlay. Document-global offsets let layout clusters address
/// its bytes without copying or rebasing the surrounding document.
struct EVSnapshotTextSlice<Identity> {
    let identity: Identity
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
    var info: ViemCompositionOverlayInfo

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

/// Successful immutable payload copies across the Swift/C boundary, separate
/// from core layout work and the independently refreshed caret/selection.
struct EVPresentationExportCounters: Equatable {
    var geometryCopies: UInt64 = 0
    var paintCopies: UInt64 = 0
    var whitespaceCopies: UInt64 = 0
    var compositionTextCopies: UInt64 = 0
}

struct EVLayoutPaintExport {
    var info: ViemLayoutPaintInfo
    var runs: [ViemPaintStyleRun]
}

struct EVCommandLineExport {
    var info: ViemCommandLineInfo
    var text: String
    var selectionAnchorUTF8Offset: UInt64 = 0

    var selectedUTF8Range: Range<Int> {
        let active = Int(info.cursor_utf8_offset)
        let anchor = Int(selectionAnchorUTF8Offset)
        return min(anchor, active)..<max(anchor, active)
    }

    var prompt: Character? {
        switch info.identity.kind {
        case UInt32(VIEM_COMMAND_LINE_KIND_EXPRESSION): "="
        case UInt32(VIEM_COMMAND_LINE_KIND_EX): ":"
        case UInt32(VIEM_COMMAND_LINE_KIND_SEARCH_FORWARD): "/"
        case UInt32(VIEM_COMMAND_LINE_KIND_SEARCH_BACKWARD): "?"
        default: nil
        }
    }
}

struct EVVisualSelectionExport {
    var info: ViemVisualSelectionInfo
    var segments: [ViemVisualSelectionSegment]
    var rectangles: [ViemVisualSelectionRectangle]
}

extension EVCoreViewSession {
    func compositionOverlayExport() throws -> EVCompositionOverlayExport? {
        var info = ViemCompositionOverlayInfo()
        try checkedPresentationExport(
            viem_core_view_composition_overlay_info(document.core, viewID, &info),
            operation: "Read composition overlay"
        )
        guard info.flags & UInt32(VIEM_COMPOSITION_OVERLAY_ACTIVE) != 0 else {
            return nil
        }
        guard info.identity.view_id == viewID,
              info.replacement_start <= info.replacement_end,
              info.marked_start <= info.marked_end,
              info.selected_start <= info.selected_end,
              info.marked_end <= info.utf8_length,
              info.selected_end <= info.utf8_length
        else {
            throw EVCoreFrontendError.core(
                operation: "Validate composition overlay",
                status: UInt32(VIEM_STATUS_CORE_FAILURE)
            )
        }
        return EVCompositionOverlayExport(info: info)
    }

    func compositionTextSlice(
        in range: Range<UInt64>,
        overlay: EVCompositionOverlayExport
    ) throws -> EVCompositionTextSlice {
        var request = ViemCompositionOverlayUtf8Range()
        request.identity = overlay.info.identity
        request.start = range.lowerBound
        request.end = range.upperBound
        var required: UInt64 = 0
        let query = viem_core_view_copy_composition_utf8_range(
            document.core,
            viewID,
            &request,
            nil,
            0,
            &required
        )
        if range.isEmpty {
            try checkedPresentationExport(query, operation: "Read composition text")
        } else if query != UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) {
            try checkedPresentationExport(query, operation: "Size composition text")
        }
        guard required <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(
                operation: "Read composition text",
                status: UInt32(VIEM_STATUS_LENGTH_OVERFLOW)
            )
        }
        var bytes = Array(repeating: UInt8(0), count: Int(required))
        let status = bytes.withUnsafeMutableBufferPointer { buffer in
            viem_core_view_copy_composition_utf8_range(
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
        presentationExportCounters.compositionTextCopies &+= 1
        return EVCompositionTextSlice(
            identity: overlay.info.identity,
            utf8Range: range,
            bytes: bytes
        )
    }

    func layoutDecorationsExport(identity: ViemLayoutSnapshotIdentity) throws
        -> ([ViemLayoutDecoration], [UInt8]) {
        var expected = identity
        var info = ViemLayoutDecorationsInfo()
        let query = viem_core_view_copy_layout_decorations(document.core, viewID, &expected,
                                                          nil, 0, nil, 0, &info)
        if query != UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) {
            try checkedPresentationExport(query, operation: "Read list marker layout")
        }
        guard info.decoration_count <= UInt64(Int.max), info.label_bytes <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(operation: "Read list marker layout", status: UInt32(VIEM_STATUS_LENGTH_OVERFLOW))
        }
        if info.decoration_count == 0 { return ([], []) }
        var decorations = Array(repeating: ViemLayoutDecoration(), count: Int(info.decoration_count))
        var labels = Array(repeating: UInt8(0), count: Int(info.label_bytes))
        let copied = decorations.withUnsafeMutableBufferPointer { items in
            labels.withUnsafeMutableBufferPointer { bytes in
                viem_core_view_copy_layout_decorations(document.core, viewID, &expected,
                    items.baseAddress, UInt64(items.count), bytes.baseAddress, UInt64(bytes.count), &info)
            }
        }
        try checkedPresentationExport(copied, operation: "Copy list marker layout")
        guard decorations.allSatisfy({ item in
            item.label_byte_start <= UInt64(labels.count)
                && item.label_byte_length <= UInt64(labels.count) - item.label_byte_start
        }), String(bytes: labels, encoding: .utf8) != nil else {
            throw EVCoreFrontendError.core(operation: "Copy list marker layout", status: UInt32(VIEM_STATUS_CORE_FAILURE))
        }
        return (decorations, labels)
    }

    func layoutPaintExport() throws -> EVLayoutPaintExport {
        var info = ViemLayoutPaintInfo()
        try checkedPresentationExport(
            viem_core_view_layout_paint_info(document.core, viewID, &info),
            operation: "Read layout paint"
        )
        if let cached = cachedLayoutPaintExport,
           cached.info.identity.isSameLayout(as: info.identity) {
            return cached
        }
        guard info.paint_run_count <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(
                operation: "Read layout paint",
                status: UInt32(VIEM_STATUS_LENGTH_OVERFLOW)
            )
        }

        var runs = Array(
            repeating: ViemPaintStyleRun(),
            count: Int(info.paint_run_count)
        )
        var copiedInfo = ViemLayoutPaintInfo()
        var identity = info.identity
        let status = runs.withUnsafeMutableBufferPointer { buffer in
            viem_core_view_copy_layout_paint(
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
              runs.allSatisfy({
                      $0.text_start < $0.text_end
              }),
              runs.indices.dropFirst().allSatisfy({
                  runs[$0 - 1].text_end <= runs[$0].text_start
              })
        else {
            throw EVCoreFrontendError.core(
                operation: "Copy layout paint",
                status: UInt32(VIEM_STATUS_CORE_FAILURE)
            )
        }
        let exported = EVLayoutPaintExport(info: copiedInfo, runs: runs)
        presentationExportCounters.paintCopies &+= 1
        cachedLayoutPaintExport = exported
        return exported
    }

    func commandLineExport() throws -> EVCommandLineExport {
        var info = ViemCommandLineInfo()
        try checkedPresentationExport(
            viem_core_view_command_line_info(document.core, viewID, &info),
            operation: "Read command line"
        )
        guard info.utf8_length <= UInt64(Int.max) else {
            throw EVCoreFrontendError.core(
                operation: "Read command line",
                status: UInt32(VIEM_STATUS_LENGTH_OVERFLOW)
            )
        }

        var bytes = Array(repeating: UInt8(0), count: Int(info.utf8_length))
        var copiedInfo = ViemCommandLineInfo()
        var identity = info.identity
        let status = bytes.withUnsafeMutableBufferPointer { buffer in
            viem_core_view_copy_command_line(
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
        var selection = ViemCommandLineSelection()
        try checkedPresentationExport(viem_core_view_command_line_selection(document.core, viewID, &identity, &selection), operation: "Read command selection")
        return EVCommandLineExport(info: copiedInfo, text: text, selectionAnchorUTF8Offset: selection.anchor_utf8_offset)
    }

    func visualSelectionExport() throws -> EVVisualSelectionExport {
        var info = ViemVisualSelectionInfo()
        try checkedPresentationExport(
            viem_core_view_visual_selection_info(document.core, viewID, &info),
            operation: "Read Visual selection"
        )
        guard info.segment_count <= UInt64(Int.max),
              info.rectangle_count <= UInt64(Int.max)
        else {
            throw EVCoreFrontendError.core(
                operation: "Read Visual selection",
                status: UInt32(VIEM_STATUS_LENGTH_OVERFLOW)
            )
        }

        var segments = Array(
            repeating: ViemVisualSelectionSegment(),
            count: Int(info.segment_count)
        )
        var rectangles = Array(
            repeating: ViemVisualSelectionRectangle(),
            count: Int(info.rectangle_count)
        )
        var copiedInfo = ViemVisualSelectionInfo()
        var identity = info.identity
        let status = segments.withUnsafeMutableBufferPointer { segmentBuffer in
            rectangles.withUnsafeMutableBufferPointer { rectangleBuffer in
                viem_core_view_copy_visual_selection(
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
                status: UInt32(VIEM_STATUS_CORE_FAILURE)
            )
        }
        return EVVisualSelectionExport(
            info: copiedInfo,
            segments: segments,
            rectangles: rectangles
        )
    }
}

extension ViemLayoutSnapshotIdentity {
    func isSameLayout(as other: ViemLayoutSnapshotIdentity) -> Bool {
        view_id == other.view_id
            && document_id == other.document_id
            && document_revision == other.document_revision
            && layout_revision == other.layout_revision
            && configuration_generation == other.configuration_generation
            && measurement_environment_id == other.measurement_environment_id
            && metrics_generation == other.metrics_generation
    }
}

extension ViemCompositionOverlayIdentity {
    func isSameOverlay(as other: ViemCompositionOverlayIdentity) -> Bool {
        view_id == other.view_id
            && document_id == other.document_id
            && document_revision == other.document_revision
            && generation == other.generation
    }
}

extension ViemFormattedSnapshotIdentity {
    func isSameSnapshot(as other: ViemFormattedSnapshotIdentity) -> Bool {
        document_id == other.document_id
            && document_revision == other.document_revision
    }
}

private func checkedPresentationExport(_ status: UInt32, operation: String) throws {
    guard status == UInt32(VIEM_STATUS_OK) else {
        throw EVCoreFrontendError.core(operation: operation, status: status)
    }
}

/// What the caret occupies, as decided by the portable core.
///
/// The frontend resolves this to geometry; it must never re-derive the choice
/// from the mode, the cursor offset, and the boundary affinity. Affinity is
/// reachable only from `boundary` because it disambiguates which visual row an
/// insertion point belongs to at a soft wrap. A character cell lies on exactly
/// one row, so using affinity to select one draws the wrong character.
enum EVCaretTarget: Equatable {
    /// The caret covers exactly this UTF-8 range: one grapheme of line content.
    case cell(Range<UInt64>)
    /// The caret sits between graphemes at this offset.
    case boundary(offset: UInt64, affinity: UInt32)

    /// The offset the caret is anchored to; for a cell, its first byte.
    var offset: UInt64 {
        switch self {
        case let .cell(range): range.lowerBound
        case let .boundary(offset, _): offset
        }
    }

    /// Row disambiguation, meaningful only for a boundary.
    var affinity: UInt32 {
        switch self {
        case .cell: UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
        case let .boundary(_, affinity): affinity
        }
    }

    /// Read the core's decision out of one presentation export.
    init(_ presentation: ViemViewPresentation) {
        if presentation.caret_shape == UInt32(VIEM_CARET_SHAPE_CELL),
           presentation.caret_utf8_start < presentation.caret_utf8_end
        {
            self = .cell(presentation.caret_utf8_start ..< presentation.caret_utf8_end)
        } else {
            self = .boundary(
                offset: presentation.caret_utf8_start,
                affinity: presentation.cursor_affinity
            )
        }
    }
}
