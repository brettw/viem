import AppKit
import CViemCore

enum EVInlineContentKind { case link, image }

struct EVInlineContentContext: Decodable {
    struct Item: Decodable, Equatable {
        let start: UInt64
        let end: UInt64
        let text: String
        let destination: String
        let editable: Bool
    }
    let canInsert: Bool
    let item: Item?
    let text: String
    private enum CodingKeys: String, CodingKey { case canInsert, link, image, text }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        canInsert = try values.decode(Bool.self, forKey: .canInsert)
        item = try values.decodeIfPresent(Item.self, forKey: .link)
            ?? values.decodeIfPresent(Item.self, forKey: .image)
        text = try values.decode(String.self, forKey: .text)
    }
}

extension EVCoreViewSession {
    func selectImage(at offset: UInt64, documentID: UInt64, revision: UInt64) throws {
        var outcome = ViemCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        let status = viem_core_view_select_image(document.core, viewID, documentID, revision, offset, &outcome)
        guard status == 0 else { throw EVCoreFrontendError.core(operation: "Select image", status: status) }
        finishStyleEdit(outcome)
    }

    func inlineContentContext(_ kind: EVInlineContentKind) throws -> EVInlineContentContext {
        let copy = kind == .image ? viem_core_view_copy_image_context : viem_core_view_copy_link_context
        var required: UInt64 = 0
        let status = copy(document.core, viewID, nil, 0, &required)
        guard status == 0 || status == UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) else {
            throw EVCoreFrontendError.core(operation: "Read link", status: status)
        }
        guard let count = Int(exactly: required) else { throw EVCoreFrontendError.invalidUTF8 }
        var bytes = [UInt8](repeating: 0, count: count)
        let result = bytes.withUnsafeMutableBufferPointer {
            copy(document.core, viewID, $0.baseAddress, UInt64($0.count), &required)
        }
        guard result == 0 else { throw EVCoreFrontendError.core(operation: "Read link", status: result) }
        return try JSONDecoder().decode(EVInlineContentContext.self, from: Data(bytes))
    }

    @discardableResult
    func editInlineContent(_ kind: EVInlineContentKind, action: UInt32, context: EVInlineContentContext, expected: ViemLogicalSelectionIdentityV1,
                  text: String = "", destination: String = "") throws -> ViemCoreOutcomeV1 {
        let edit = kind == .image ? viem_core_view_edit_image : viem_core_view_edit_link
        var selection = expected
        var outcome = ViemCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        let status = Array(text.utf8).withUnsafeBufferPointer { label in
            Array(destination.utf8).withUnsafeBufferPointer { url in
                edit(document.core, viewID, &selection, action,
                    context.item?.start ?? 0, context.item?.end ?? 0,
                    ViemUtf8Slice(data: label.baseAddress, length: UInt64(label.count)),
                    ViemUtf8Slice(data: url.baseAddress, length: UInt64(url.count)), &outcome)
            }
        }
        guard status == 0 else { throw EVCoreFrontendError.core(operation: "Edit link", status: status) }
        finishStyleEdit(outcome)
        return outcome
    }
}
