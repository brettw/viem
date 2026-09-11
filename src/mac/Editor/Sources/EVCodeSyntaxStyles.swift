import CViemCore
import Foundation

@MainActor
extension EVCoreDocumentBackend {
    /// Reads only names in accepted syntax output. Opening a menu never waits
    /// for a provider or scans source to discover more styles.
    func syntaxStyleNames() throws -> [String] {
        var required: UInt64 = 0
        let measured = viem_core_copy_syntax_style_names(core, nil, 0, &required)
        guard measured == UInt32(VIEM_STATUS_OK) || measured == UInt32(VIEM_STATUS_BUFFER_TOO_SMALL),
              required <= 4 * 1024 * 1024 else {
            throw EVCoreFrontendError.core(operation: "Read syntax styles", status: measured)
        }
        var data = Data(count: Int(required))
        let copied = data.withUnsafeMutableBytes {
            viem_core_copy_syntax_style_names(core, $0.bindMemory(to: UInt8.self).baseAddress,
                                             UInt64($0.count), &required)
        }
        guard copied == UInt32(VIEM_STATUS_OK), required <= UInt64(data.count) else {
            throw EVCoreFrontendError.core(operation: "Read syntax styles", status: copied)
        }
        data.count = Int(required)
        return try JSONDecoder().decode([String].self, from: data)
    }
}
