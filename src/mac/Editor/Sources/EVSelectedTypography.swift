import AppKit
import CViemCore

/// A revision-bound, regional query; obtaining this value never requests layout.
struct EVSelectedTypography: Equatable {
  let documentID: UInt64
  let documentRevision: UInt64
  let fontFamily: String
  let size: CGFloat
  let baseWeight: UInt16
  let weight: UInt16
  let bold: Bool
  let slant: UInt32
  let foreground: EVStyleColor?
  let background: EVStyleColor?
  let scriptPosition: UInt32
  let scriptMixed: Bool
  let features: [EVOpenTypeFeature]
  let mixed: Bool
}

extension EVCoreViewSession {
  func selectedTypography() throws -> EVSelectedTypography {
    let revision = try document.revision()
    var info = ViemTypographyInfoV1()
    info.struct_size = UInt32(MemoryLayout<ViemTypographyInfoV1>.size)
    func check(_ status: UInt32) throws {
      guard status == UInt32(VIEM_STATUS_OK) else {
        throw EVCoreFrontendError.core(operation: "Read selection typography", status: status)
      }
    }
    let sizing = viem_core_view_typography_export(
      document.core, viewID, revision,
      &info, nil, 0, nil, 0)
    if sizing != UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) { try check(sizing) }
    var family = [UInt8](repeating: 0, count: Int(info.font_family_bytes))
    var features = [ViemOpenTypeFeatureV1](repeating: .init(), count: Int(info.feature_count))
    let status = family.withUnsafeMutableBufferPointer { family in
      features.withUnsafeMutableBufferPointer { features in
        viem_core_view_typography_export(
          document.core, viewID, revision, &info,
          family.baseAddress, UInt64(family.count), features.baseAddress, UInt64(features.count))
      }
    }
    try check(status)
    return EVSelectedTypography(
      documentID: info.document_id,
      documentRevision: info.document_revision, fontFamily: String(decoding: family, as: UTF8.self),
      size: CGFloat(info.size), baseWeight: UInt16(info.base_weight), weight: UInt16(info.weight),
      bold: info.flags & 1 != 0, slant: info.slant,
      foreground: info.flags & 4 != 0
        ? nil
        : EVStyleColor(
          red: info.foreground.red,
          green: info.foreground.green, blue: info.foreground.blue, alpha: info.foreground.alpha),
      background: info.has_background == 0 ? nil : EVStyleColor(
        red: info.background.red, green: info.background.green,
        blue: info.background.blue, alpha: info.background.alpha),
      scriptPosition: info.script_position,
      scriptMixed: info.flags & 8 != 0,
      features: features.map {
        EVOpenTypeFeature(
          tag: String(decoding: [$0.tag.0, $0.tag.1, $0.tag.2, $0.tag.3], as: UTF8.self),
          setting: $0.value)
      },
      mixed: info.flags & 2 != 0)
  }
}
