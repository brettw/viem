import AppKit
import CViemCore
import ViemAppShell

enum EVInlineContentKind { case link, image }

struct EVInlineContentContext: Decodable {
    struct Item: Decodable, Equatable {
        let start: UInt64
        let end: UInt64
        let text: String
        let destination: String
        let editable: Bool
        let width: UInt32?
        let height: UInt32?
    }
    struct Reference: Decodable {
        struct Choice: Decodable { let name: String; let destination: String; let offset: UInt64 }
        struct Use: Decodable { let text: String; let offset: UInt64; let image: Bool }
        let start: UInt64; let end: UInt64; let name: String; let text: String
        let destination: String; let title: String; let definition: Bool
        let definitionOffset: UInt64?; let inactive: Bool; let links: Int; let images: Int
        let complete: Bool; let editable: Bool; let choices: [Choice]; let uses: [Use]
        var usage: String { "Used by \(links) link\(links == 1 ? "" : "s") and \(images) image\(images == 1 ? "" : "s")" }
    }
    let reference: Reference?
    let canInsert: Bool
    let item: Item?
    let text: String
    let linked: Bool
    let canExitLink: Bool
    let canRemoveSelection: Bool
    private enum CodingKeys: String, CodingKey { case canInsert, link, image, text, linked, canExitLink, canRemoveSelection, reference }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        reference = try values.decodeIfPresent(Reference.self, forKey: .reference)
        canInsert = try values.decode(Bool.self, forKey: .canInsert)
        item = try values.decodeIfPresent(Item.self, forKey: .link)
            ?? values.decodeIfPresent(Item.self, forKey: .image)
        text = try values.decode(String.self, forKey: .text)
        linked = try values.decodeIfPresent(Bool.self, forKey: .linked) ?? false
        canExitLink = try values.decodeIfPresent(Bool.self, forKey: .canExitLink) ?? false
        canRemoveSelection = try values.decodeIfPresent(Bool.self, forKey: .canRemoveSelection) ?? false
    }

    var forInsertion: Self {
        Self(canInsert: canInsert, item: nil, text: text,
             linked: false, canExitLink: false, canRemoveSelection: false)
    }

    private init(canInsert: Bool, item: Item?, text: String, linked: Bool,
                 canExitLink: Bool, canRemoveSelection: Bool) {
        self.reference = nil
        self.canInsert = canInsert; self.item = item; self.text = text
        self.linked = linked; self.canExitLink = canExitLink
        self.canRemoveSelection = canRemoveSelection
    }
}

extension EVCoreViewSession {
    func imageResourcesChanged(previousMetricsGeneration: UInt64, destinations: [String]) throws {
        let encoded = destinations.map { Array($0.utf8) }
        let bytes = encoded.flatMap { $0 }
        let status = bytes.withUnsafeBufferPointer { buffer in
            var offset = 0
            let slices = encoded.map { value -> ViemUtf8Slice in
                defer { offset += value.count }
                return ViemUtf8Slice(data: buffer.baseAddress?.advanced(by: offset), length: UInt64(value.count))
            }
            return slices.withUnsafeBufferPointer {
                viem_core_image_resources_changed(document.core, viewID, previousMetricsGeneration, $0.baseAddress, UInt64($0.count))
            }
        }
        guard status == 0 else { throw EVCoreFrontendError.core(operation: "Refresh image layout", status: status) }
    }

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
                  text: String = "", destination: String = "", imageSize: (width: UInt32?, height: UInt32?)? = nil) throws -> ViemCoreOutcomeV1 {
        var selection = expected
        var outcome = ViemCoreOutcomeV1()
        outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        let status = Array(text.utf8).withUnsafeBufferPointer { label in
            Array(destination.utf8).withUnsafeBufferPointer { url in
                let label = ViemUtf8Slice(data: label.baseAddress, length: UInt64(label.count))
                let url = ViemUtf8Slice(data: url.baseAddress, length: UInt64(url.count))
                if kind == .image {
                    let size = imageSize ?? (context.item?.width, context.item?.height)
                    return viem_core_view_edit_image(document.core, viewID, &selection, action,
                        context.item?.start ?? 0, context.item?.end ?? 0, label, url,
                        size.width ?? 0, size.height ?? 0, &outcome)
                }
                return viem_core_view_edit_link(document.core, viewID, &selection, action,
                    context.item?.start ?? 0, context.item?.end ?? 0,
                    label, url, &outcome)
            }
        }
        guard status == 0 else { throw EVCoreFrontendError.core(operation: kind == .image ? "Edit image" : "Edit link", status: status) }
        if kind == .link, action == 3 {
            notePendingCharacterContextChange(outcome, expected: expected)
        }
        finishStyleEdit(outcome)
        return outcome
    }
}

extension EVCoreViewSession {
    func referenceAction(_ action: String, context: EVInlineContentContext, expected: ViemLogicalSelectionIdentityV1,
                         fields: [String: Any] = [:]) throws {
        guard context.reference != nil || action == "insert" else { throw EVDocumentHostError.unsupportedRequest }
        var request = fields
        request["action"] = action; request["offset"] = context.reference?.start ?? expected.text_start
        if action == "insert" { request["end"] = expected.text_end }
        let bytes = Array(try JSONSerialization.data(withJSONObject: request))
        var selection = expected
        var outcome = ViemCoreOutcomeV1(); outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
        let status = bytes.withUnsafeBufferPointer { buffer in
            viem_core_view_reference_action(document.core, viewID, &selection,
                ViemUtf8Slice(data: buffer.baseAddress, length: UInt64(buffer.count)), &outcome)
        }
        guard status == 0 else { throw EVCoreFrontendError.core(operation: "Change reference", status: status) }
        finishStyleEdit(outcome)
    }
}
