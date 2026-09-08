import CViemCore

struct EVSelectedNamedStyles {
  let identity: EVStyleSheetIdentity
  let paragraph: EVStyleID?
  let character: EVStyleID?
  let paragraphMixed: Bool
  let characterMixed: Bool
}

extension EVCoreViewSession {
  func selectedNamedStyles() throws -> EVSelectedNamedStyles {
    let revision = try document.revision()
    var info = ViemSelectedStylesInfoV1()
    func check(_ status: UInt32) throws {
      guard status == UInt32(VIEM_STATUS_OK) else {
        throw EVCoreFrontendError.core(operation: "Read selected styles", status: status)
      }
    }
    let sizing = viem_core_view_selected_styles_export(
      document.core, viewID, revision, &info, nil, 0)
    if sizing != UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) { try check(sizing) }
    var bytes = [UInt8](repeating: 0, count: Int(info.paragraph_id_bytes + info.character_id_bytes))
    let status = bytes.withUnsafeMutableBufferPointer {
      viem_core_view_selected_styles_export(
        document.core, viewID, revision, &info, $0.baseAddress, UInt64($0.count))
    }
    try check(status)
    let split = Int(info.paragraph_id_bytes)
    return EVSelectedNamedStyles(
      identity: EVStyleSheetIdentity(
        documentID: info.document_id, documentRevision: info.document_revision,
        styleSheetRevision: info.style_sheet_revision),
      paragraph: split == 0
        ? nil : EVStyleID(rawValue: String(decoding: bytes[..<split], as: UTF8.self)),
      character: info.character_id_bytes == 0
        ? nil : EVStyleID(rawValue: String(decoding: bytes[split...], as: UTF8.self)),
      paragraphMixed: info.flags & UInt32(VIEM_SELECTED_STYLE_PARAGRAPH_MIXED) != 0,
      characterMixed: info.flags & UInt32(VIEM_SELECTED_STYLE_CHARACTER_MIXED) != 0)
  }
}
