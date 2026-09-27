import CViemCore
import Foundation

extension EVCoreViewSession {
    func htmlExportData() async throws -> Data {
        // Capture the immutable export while UI access to the core is serialized.
        // Rendering never accesses the live core or its measurement provider.
        let retainedDocument = document
        let revision = try retainedDocument.documentState().document_revision
        var export: ViemHtmlExportHandle = 0
        let prepared = viem_core_prepare_html_export(retainedDocument.core, viewID, revision, &export)
        guard prepared == UInt32(VIEM_STATUS_OK) else {
            throw EVCoreFrontendError.core(operation: "Prepare HTML export", status: prepared)
        }
        let exportHandle = export
        defer { withExtendedLifetime((self, retainedDocument)) {} }
        return try await Task.detached(priority: .userInitiated) {
            defer { _ = viem_html_export_release(exportHandle) }
            let rendered = viem_html_export_render(exportHandle)
            guard rendered == UInt32(VIEM_STATUS_OK) else {
                throw EVCoreFrontendError.core(operation: "Render HTML export", status: rendered)
            }
            var required: UInt64 = 0
            let measured = viem_html_export_copy_utf8(exportHandle, nil, 0, &required)
            guard measured == UInt32(VIEM_STATUS_OK) || measured == UInt32(VIEM_STATUS_BUFFER_TOO_SMALL),
                  required <= UInt64(Int.max) else {
                throw EVCoreFrontendError.core(operation: "Prepare HTML export", status: measured)
            }
            var data = Data(count: Int(required))
            let capacity = required
            let copied = data.withUnsafeMutableBytes { buffer in
                viem_html_export_copy_utf8(exportHandle,
                    buffer.bindMemory(to: UInt8.self).baseAddress, capacity, &required)
            }
            guard copied == UInt32(VIEM_STATUS_OK), required <= capacity else {
                throw EVCoreFrontendError.core(operation: "Export HTML", status: copied)
            }
            data.count = Int(required)
            return data
        }.value
    }
}
