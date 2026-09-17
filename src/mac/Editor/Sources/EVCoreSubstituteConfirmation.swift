import CViemCore

extension EVCoreViewSession {
    func substituteConfirmationPrompt() throws -> String? {
        var length: UInt64 = 0
        let status = viem_core_view_copy_substitute_confirmation(document.core, viewID, nil, 0, &length)
        guard status == UInt32(VIEM_STATUS_OK) || status == UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) else {
            throw EVCoreFrontendError.core(operation: "Read substitution confirmation", status: status)
        }
        guard length > 0 else { return nil }
        var bytes = [UInt8](repeating: 0, count: Int(length))
        let copied = bytes.withUnsafeMutableBufferPointer { buffer in
            viem_core_view_copy_substitute_confirmation(document.core, viewID, buffer.baseAddress, UInt64(buffer.count), &length)
        }
        guard copied == UInt32(VIEM_STATUS_OK) else {
            throw EVCoreFrontendError.core(operation: "Read substitution confirmation", status: copied)
        }
        return String(decoding: bytes.prefix(Int(length)), as: UTF8.self)
    }
}
