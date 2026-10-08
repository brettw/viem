import AppKit
import CViemCore

struct EVLinkContext: Decodable {
    struct Link: Decodable, Equatable {
        let start: UInt64
        let end: UInt64
        let text: String
        let destination: String
        let editable: Bool
    }
    let canInsert: Bool
    let link: Link?
    let text: String
}

extension EVCoreViewSession {
    func linkContext() throws -> EVLinkContext {
        var required: UInt64 = 0
        let status = viem_core_view_copy_link_context(document.core, viewID, nil, 0, &required)
        guard status == 0 || status == UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) else {
            throw EVCoreFrontendError.core(operation: "Read link", status: status)
        }
        guard let count = Int(exactly: required) else { throw EVCoreFrontendError.invalidUTF8 }
        var bytes = [UInt8](repeating: 0, count: count)
        let result = bytes.withUnsafeMutableBufferPointer {
            viem_core_view_copy_link_context(document.core, viewID, $0.baseAddress, UInt64($0.count), &required)
        }
        guard result == 0 else { throw EVCoreFrontendError.core(operation: "Read link", status: result) }
        return try JSONDecoder().decode(EVLinkContext.self, from: Data(bytes))
    }

    @discardableResult
    func editLink(action: UInt32, context: EVLinkContext, expected: ViemLogicalSelectionIdentityV1,
                  text: String = "", destination: String = "") throws -> ViemCoreOutcomeV1 {
        var selection = expected
        var outcome = ViemCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        let status = Array(text.utf8).withUnsafeBufferPointer { label in
            Array(destination.utf8).withUnsafeBufferPointer { url in
                viem_core_view_edit_link(document.core, viewID, &selection, action,
                    context.link?.start ?? 0, context.link?.end ?? 0,
                    ViemUtf8Slice(data: label.baseAddress, length: UInt64(label.count)),
                    ViemUtf8Slice(data: url.baseAddress, length: UInt64(url.count)), &outcome)
            }
        }
        guard status == 0 else { throw EVCoreFrontendError.core(operation: "Edit link", status: status) }
        finishStyleEdit(outcome)
        return outcome
    }
}
