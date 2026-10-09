import CViemCore

extension EVCoreViewSession {
    @discardableResult
    func setCodeBlockLanguage(documentID: UInt64, revision: UInt64, offset: UInt64,
                              language: String) throws -> ViemCoreOutcomeV1 {
        var outcome = ViemCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        let status = Array(language.utf8).withUnsafeBufferPointer { bytes in
            viem_core_view_set_code_block_language(document.core, viewID, documentID, revision,
                offset, ViemUtf8Slice(data: bytes.baseAddress, length: UInt64(bytes.count)), &outcome)
        }
        guard status == UInt32(VIEM_STATUS_OK) else {
            throw EVCoreFrontendError.core(operation: "Set code block language", status: status)
        }
        finishStyleEdit(outcome)
        return outcome
    }
}
