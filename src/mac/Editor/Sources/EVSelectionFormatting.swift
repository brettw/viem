import AppKit
import CViemCore

struct EVSelectionFormatting {
  let values: [(EVStyleProperty, EVStyleValue?)]
  let mixed: Set<EVStyleProperty>
  subscript(_ property: EVStyleProperty) -> EVStyleValue? {
    values.first { $0.0 == property }?.1
  }
}

extension EVCoreViewSession {
  func selectedFormatting() throws -> EVSelectionFormatting {
    var selection = try listSelection()
    var info = ViemStyleSheetInfoV1()
    info.struct_size = UInt32(MemoryLayout<ViemStyleSheetInfoV1>.size)
    func check(_ status: UInt32) throws {
      guard status == UInt32(VIEM_STATUS_OK) else {
        throw EVCoreFrontendError.core(operation: "Read selection formatting", status: status)
      }
    }
    let sizing = viem_core_view_copy_formatting(document.core, viewID, &selection,
      &info, nil, 0, nil, 0, nil, 0)
    if sizing != UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) { try check(sizing) }
    var properties = [ViemStylePropertyV1](repeating: .init(), count: Int(info.property_count))
    var items = [ViemStyleValueItemV1](repeating: .init(), count: Int(info.value_item_count))
    var strings = [UInt8](repeating: 0, count: Int(info.string_bytes))
    try properties.withUnsafeMutableBufferPointer { properties in
      try items.withUnsafeMutableBufferPointer { items in
        try strings.withUnsafeMutableBufferPointer { strings in
          try check(viem_core_view_copy_formatting(document.core, viewID, &selection,
            &info, properties.baseAddress, UInt64(properties.count),
            items.baseAddress, UInt64(items.count), strings.baseAddress, UInt64(strings.count)))
        }
      }
    }
    func text(_ ref: ViemStyleStringRefV1) throws -> String {
      guard ref.offset <= strings.count, ref.length <= UInt64(strings.count) - ref.offset,
        let text = String(bytes: strings[Int(ref.offset)..<Int(ref.offset + ref.length)], encoding: .utf8)
      else { throw EVStyleBridgeError.malformedSnapshot("Invalid formatting string") }
      return text
    }
    func value(_ raw: ViemStyleValueV1) throws -> EVStyleValue? {
      guard raw.first_item <= items.count, raw.item_count <= UInt64(items.count) - raw.first_item else {
        throw EVStyleBridgeError.malformedSnapshot("Invalid formatting items")
      }
      let children = items[Int(raw.first_item)..<Int(raw.first_item + raw.item_count)]
      switch raw.kind {
      case UInt32(VIEM_STYLE_VALUE_NONE): return nil
      case UInt32(VIEM_STYLE_VALUE_FLOAT): return .float(raw.number)
      case UInt32(VIEM_STYLE_VALUE_UNSIGNED): return .unsigned(raw.enum_value)
      case UInt32(VIEM_STYLE_VALUE_BOOLEAN): return .boolean(raw.enum_value != 0)
      case UInt32(VIEM_STYLE_VALUE_COLOR): return .color(.init(red: raw.color.red,
        green: raw.color.green, blue: raw.color.blue, alpha: raw.color.alpha))
      case UInt32(VIEM_STYLE_VALUE_STRING): return .string(try text(raw.string))
      case UInt32(VIEM_STYLE_VALUE_STRING_LIST): return .stringList(try children.map { try text($0.string) })
      case UInt32(VIEM_STYLE_VALUE_FONT_SLANT): return .fontSlant(raw.enum_value)
      case UInt32(VIEM_STYLE_VALUE_SCRIPT_POSITION): return .scriptPosition(raw.enum_value)
      case UInt32(VIEM_STYLE_VALUE_WRITING_DIRECTION): return .writingDirection(raw.enum_value)
      case UInt32(VIEM_STYLE_VALUE_OPEN_TYPE_FEATURES): return .openTypeFeatures(try children.map {
        .init(tag: try text($0.string), setting: $0.unsigned_value)
      })
      case UInt32(VIEM_STYLE_VALUE_LINE_SPACING): return .lineSpacing(.init(kind: raw.enum_value, value: raw.number))
      case UInt32(VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT): return .paragraphAlignment(raw.enum_value)
      default: throw EVStyleBridgeError.malformedSnapshot("Unknown formatting value")
      }
    }
    let mixed = Set(properties.filter { $0.flags & UInt32(VIEM_STYLE_PROPERTY_MIXED) != 0 }
      .compactMap { EVStyleProperty(rawValue: $0.property) })
    return EVSelectionFormatting(values: try properties.map { raw in
      guard let property = EVStyleProperty(rawValue: raw.property) else {
        throw EVStyleBridgeError.malformedSnapshot("Unknown formatting property")
      }
      return (property, try value(raw.effective))
    }, mixed: mixed)
  }

  @discardableResult
  func editDirectProperties(_ values: [(EVStyleProperty, EVStyleValue?)],
                            expected selection: ViemLogicalSelectionIdentityV1) throws -> ViemCoreOutcomeV1 {
    let identity = EVStyleSheetIdentity(documentID: selection.document_id,
      documentRevision: selection.document_revision, styleSheetRevision: 0)
    let encoded = values.map { pair in
      EVCoreStyleBridge.EncodedMutation(key: .defaultParagraph, expected: identity,
        mutation: pair.1.map { .setDeclaration(pair.0, $0) } ?? .clearDeclaration(pair.0))
    }
    var requests: [ViemDirectStyleEditV1] = []
    var outcome = ViemCoreOutcomeV1()
    outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
    func withArenas(_ index: Int) -> UInt32 {
      if index == encoded.count {
        return requests.withUnsafeBufferPointer {
          viem_core_view_edit_direct_properties(document.core, viewID, $0.baseAddress, UInt64($0.count), &outcome)
        }
      }
      return encoded[index].withRequest { item in
        var request = ViemDirectStyleEditV1()
        request.struct_size = UInt32(MemoryLayout<ViemDirectStyleEditV1>.size)
        request.operation = item.operation; request.property = item.property
        request.value = item.value; request.expected_selection = selection
        requests.append(request)
        return withArenas(index + 1)
      }
    }
    let status = withArenas(0)
    guard status == UInt32(VIEM_STATUS_OK) else {
      throw EVCoreFrontendError.core(operation: "Change direct formatting", status: status)
    }
    finishStyleEdit(outcome)
    return outcome
  }
}
