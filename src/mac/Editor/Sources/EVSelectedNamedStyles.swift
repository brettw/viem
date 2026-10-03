import AppKit
import CViemCore

struct EVSelectedNamedStyles {
  let identity: EVStyleSheetIdentity
  let paragraph: EVStyleID?
  let character: EVStyleID?
  let paragraphMixed: Bool
  let characterMixed: Bool
  let hasBullets: Bool
  let hasNumbering: Bool
  let hasNonList: Bool
  let hasQuotes: Bool
  let hasNonQuote: Bool
  let hasTable: Bool
  let hasCodeBlock: Bool

  var quoteState: NSControl.StateValue {
    hasQuotes ? (hasNonQuote ? .mixed : .on) : .off
  }

  var bulletState: NSControl.StateValue {
    hasBullets ? (hasNumbering || hasNonList ? .mixed : .on) : .off
  }
  var numberedState: NSControl.StateValue {
    hasNumbering ? (hasBullets || hasNonList ? .mixed : .on) : .off
  }
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
      characterMixed: info.flags & UInt32(VIEM_SELECTED_STYLE_CHARACTER_MIXED) != 0,
      hasBullets: info.flags & UInt32(VIEM_SELECTED_STYLE_HAS_BULLETS) != 0,
      hasNumbering: info.flags & UInt32(VIEM_SELECTED_STYLE_HAS_NUMBERING) != 0,
      hasNonList: info.flags & UInt32(VIEM_SELECTED_STYLE_HAS_NON_LIST) != 0,
      hasQuotes: info.flags & UInt32(VIEM_SELECTED_STYLE_HAS_QUOTES) != 0,
      hasNonQuote: info.flags & UInt32(VIEM_SELECTED_STYLE_HAS_NON_QUOTE) != 0,
      hasTable: info.flags & UInt32(VIEM_SELECTED_STYLE_HAS_TABLE) != 0,
      hasCodeBlock: info.flags & UInt32(VIEM_SELECTED_STYLE_HAS_CODE_BLOCK) != 0)
  }
}

@MainActor
extension EVEditorSurfaceController {
  /// Opening and caret following share one policy: prefer a unique non-default
  /// character assignment, then the paragraph style, against current authority.
  func currentStyleEditorKey() -> EVStyleKey {
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
      guard let definition = snapshot.definition(for: key) else { return nil }
      return definition.kind == kind || (kind == .paragraph && definition.kind.isContainer) ? key : nil
    }
    let character = current(.character, id: selected.character, mixed: selected.characterMixed)
    let paragraph = current(.paragraph, id: selected.paragraph, mixed: selected.paragraphMixed)
    return character ?? paragraph ?? .baseParagraph
  }
}
