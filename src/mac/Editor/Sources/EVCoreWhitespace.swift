import AppKit
import CViemCore
import ViemAppShell

private func checked(_ status: UInt32, operation: String) throws {
    guard status == UInt32(VIEM_STATUS_OK) else { throw EVCoreFrontendError.core(operation: operation, status: status) }
}

struct EVWhitespaceMarker: Codable, Equatable {
    var text: String
    var rowIndex: Int
    var x: CGFloat
    var y: CGFloat
    var width: CGFloat
    var height: CGFloat
}

struct EVWhitespaceMarkerExport: Codable, Equatable {
    var style = EVVisibleWhitespaceStyle.defaultStyle
    var markers: [EVWhitespaceMarker] = []
    var enabled = true
    var applicable = true
}

extension EVCoreDocumentBackend {
    func configureWhitespace(indentation: EVIndentationOptions, presentation: EVWhitespacePresentationOptions) throws {
        let encoder = JSONEncoder()
        let indentData = try encoder.encode(indentation)
        try checked(indentData.withUnsafeBytes {
            viem_core_set_indentation_defaults(core, $0.bindMemory(to: UInt8.self).baseAddress, UInt64($0.count))
        }, operation: "Update indentation")
        let presentationData = try encoder.encode(presentation)
        try checked(presentationData.withUnsafeBytes {
            viem_core_set_whitespace_presentation_defaults(core, $0.bindMemory(to: UInt8.self).baseAddress, UInt64($0.count))
        }, operation: "Update whitespace presentation")
    }
}

extension EVCoreViewSession {
    func setVisibleWhitespace(_ enabled: Bool) throws {
        try checked(viem_core_view_set_visible_whitespace(document.core, viewID, enabled ? 1 : 0), operation: "Show visible whitespace")
    }
    func setWhitespaceDefaults(indentation: EVIndentationOptions, presentation: EVWhitespacePresentationOptions) throws {
        try document.configureWhitespace(indentation: indentation, presentation: presentation)
    }

    func whitespaceMarkersExport(identity: ViemLayoutSnapshotIdentityV1, expectedViewport: ViemLayoutRectV1? = nil) throws -> EVWhitespaceMarkerExport {
        var expected = identity
        var viewport: ViemLayoutRectV1
        if let expectedViewport {
            viewport = expectedViewport
        } else {
            let state = try viewportState()
            var info = ViemLayoutSnapshotInfoV1()
            info.struct_size = UInt32(MemoryLayout<ViemLayoutSnapshotInfoV1>.size)
            try checked(viem_core_view_layout_snapshot_info(document.core, viewID, &info), operation: "Read whitespace viewport")
            viewport = ViemLayoutRectV1(x: state.left, y: state.top, width: info.viewport_width, height: info.viewport_height)
        }
        var count: UInt64 = 0
        let queried = viem_core_view_copy_whitespace_markers(document.core, viewID, &expected, &viewport, nil, 0, &count)
        if queried != UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) { try checked(queried, operation: "Read whitespace markers") }
        guard let length = Int(exactly: count) else { throw EVCoreFrontendError.unavailableLayout }
        var bytes = [UInt8](repeating: 0, count: length)
        try checked(bytes.withUnsafeMutableBufferPointer {
            viem_core_view_copy_whitespace_markers(document.core, viewID, &expected, &viewport, $0.baseAddress, UInt64($0.count), &count)
        }, operation: "Copy whitespace markers")
        return try JSONDecoder().decode(EVWhitespaceMarkerExport.self, from: Data(bytes))
    }
}
