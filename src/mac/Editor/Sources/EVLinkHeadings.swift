import CViemCore
import Foundation

struct EVLinkHeadingList: Decodable {
    struct Heading: Decodable {
        let text: String
        let destination: String
        let level: UInt8
        let offset: UInt64
    }
    let headings: [Heading]
    let truncated: Bool
}

extension EVCoreViewSession {
    func linkHeadings(expected: ViemLogicalSelectionIdentityV1) throws -> EVLinkHeadingList {
        guard expected.isSameSelection(as: try listSelection()) else {
            throw EVCoreFrontendError.core(operation: "List linked headings", status: UInt32(VIEM_STATUS_STALE_REVISION))
        }
        var required: UInt64 = 0
        let status = viem_core_copy_link_headings(document.core, expected.document_id, expected.document_revision, nil, 0, &required)
        guard status == 0 || status == UInt32(VIEM_STATUS_BUFFER_TOO_SMALL), let count = Int(exactly: required) else {
            throw EVCoreFrontendError.core(operation: "List linked headings", status: status)
        }
        var bytes = [UInt8](repeating: 0, count: count)
        let result = bytes.withUnsafeMutableBufferPointer {
            viem_core_copy_link_headings(document.core, expected.document_id, expected.document_revision, $0.baseAddress, UInt64($0.count), &required)
        }
        guard result == 0 else { throw EVCoreFrontendError.core(operation: "List linked headings", status: result) }
        return try JSONDecoder().decode(EVLinkHeadingList.self, from: Data(bytes))
    }
}
