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

@MainActor
extension EVEditorSurfaceController {
  /// Resolve against the current source/settings authority. A nil kind is the
  /// caret-following policy: prefer a unique non-default character assignment.
  func currentStyleEditorKey(preferredKind: EVStyleKind? = nil) -> EVStyleKey {
    let fallback = EVStyleKey.baseParagraph
    guard let session,
      let selected = try? session.selectedNamedStyles(),
      let snapshot = try? backend.sourceFormat == .code
        ? EVCoreStyleBridge.copyStyleSheet(core: nil) : backend.styleSheetSnapshot(),
      let state = try? backend.documentState(),
      selected.identity.documentID == state.document_id,
      selected.identity.documentRevision == state.document_revision,
      selected.identity.styleSheetRevision == snapshot.identity.styleSheetRevision,
      backend.sourceFormat == .code || selected.identity == snapshot.identity
    else { return fallback }

    func current(_ kind: EVStyleKind, id: EVStyleID?, mixed: Bool) -> EVStyleKey? {
      guard !mixed, let id else { return nil }
      let key = EVStyleKey(namespace: kind == .character ? .character : .block, id: id)
      return snapshot.definition(for: key)?.kind == kind ? key : nil
    }
    let character = current(.character, id: selected.character, mixed: selected.characterMixed)
    let paragraph = current(.paragraph, id: selected.paragraph, mixed: selected.paragraphMixed)
    if preferredKind == .paragraph { return paragraph ?? .baseParagraph }
    if preferredKind == .character { return character ?? paragraph ?? .baseParagraph }
    return character ?? paragraph ?? .baseParagraph
  }
}
