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

struct EVWhitespaceExportCache {
    var identity: ViemLayoutSnapshotIdentityV1
    var viewport: ViemLayoutRectV1
    var exported: EVWhitespaceMarkerExport
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
        let state = try viewportState()
        let info = try layoutSnapshotInfo()
        let currentViewport = ViemLayoutRectV1(x: state.left, y: state.top,
                                               width: info.viewport_width, height: info.viewport_height)
        var viewport = expectedViewport ?? currentViewport
        // Let the ABI diagnose stale/invalid explicit requests; cache hits must
        // satisfy the same identity and viewport checks as an actual copy.
        if let cached = cachedWhitespaceExport,
           identity.struct_size >= UInt32(MemoryLayout<ViemLayoutSnapshotIdentityV1>.size),
           info.identity.isSameLayout(as: identity),
           cached.identity.isSameLayout(as: identity),
           viewport.x == currentViewport.x, viewport.y == currentViewport.y,
           viewport.width == currentViewport.width, viewport.height == currentViewport.height,
           cached.viewport.x == viewport.x, cached.viewport.y == viewport.y,
           cached.viewport.width == viewport.width, cached.viewport.height == viewport.height {
            return cached.exported
        }
        var count: UInt64 = 0
        let queried = viem_core_view_copy_whitespace_markers(document.core, viewID, &expected, &viewport, nil, 0, &count)
        if queried != UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) { try checked(queried, operation: "Read whitespace markers") }
        guard let length = Int(exactly: count) else { throw EVCoreFrontendError.unavailableLayout }
        var bytes = [UInt8](repeating: 0, count: length)
        try checked(bytes.withUnsafeMutableBufferPointer {
            viem_core_view_copy_whitespace_markers(document.core, viewID, &expected, &viewport, $0.baseAddress, UInt64($0.count), &count)
        }, operation: "Copy whitespace markers")
        let exported = try JSONDecoder().decode(EVWhitespaceMarkerExport.self, from: Data(bytes))
        presentationExportCounters.whitespaceCopies &+= 1
        cachedWhitespaceExport = EVWhitespaceExportCache(identity: identity, viewport: viewport, exported: exported)
        return exported
    }
}
