import AppKit
import CViemCore
import CoreText
import ViemCoreTextProvider

/// Native paint values resolved by the portable stylesheet authority.
public struct EVChromeBorder {
  public var width: CGFloat
  public var color: NSColor?
  public init(width: CGFloat = 0, color: NSColor? = nil) { self.width = width; self.color = color }
}

public struct EVChromeStyle {
  public var font: NSFont
  public var foreground: NSColor
  public var background: NSColor?
  public var margin: NSEdgeInsets
  public var padding: NSEdgeInsets
  public var topBorder: EVChromeBorder
  public var rightBorder: EVChromeBorder
  public var bottomBorder: EVChromeBorder
  public var leftBorder: EVChromeBorder
  public var alignment: NSTextAlignment
  public var letterSpacing: CGFloat
  public init(font: NSFont = .systemFont(ofSize: 11), foreground: NSColor = .labelColor,
              background: NSColor? = nil, margin: NSEdgeInsets = NSEdgeInsets(),
              padding: NSEdgeInsets = NSEdgeInsets(), topBorder: EVChromeBorder = EVChromeBorder(),
              rightBorder: EVChromeBorder = EVChromeBorder(), bottomBorder: EVChromeBorder = EVChromeBorder(),
              leftBorder: EVChromeBorder = EVChromeBorder(), alignment: NSTextAlignment = .right,
              letterSpacing: CGFloat = 0) {
    self.font = font; self.foreground = foreground; self.background = background
    self.margin = margin; self.padding = padding; self.topBorder = topBorder
    self.rightBorder = rightBorder; self.bottomBorder = bottomBorder; self.leftBorder = leftBorder
    self.alignment = alignment; self.letterSpacing = letterSpacing
  }
  public var fontHeight: CGFloat { ceil(font.ascender - font.descender + font.leading) }
}

public struct EVChromeStyles {
  public var statusBar: EVChromeStyle
  public var statusCommand: EVChromeStyle
  public init(statusBar: EVChromeStyle, statusCommand: EVChromeStyle) {
    self.statusBar = statusBar; self.statusCommand = statusCommand
  }
}

@MainActor
enum EVChromeStyleResolver {
  /// A small immutable specimen resolves sparse inheritance in the core. It
  /// has no view, shaped document, source edits, or shared buffer history.
  static func resolve(configuration: EVConfigurationStore, theme: EVTheme) -> EVChromeStyles {
    let fallback = EVChromeStyle(font: .systemFont(ofSize: 11), foreground: theme.foreground.color,
      background: theme.background.color, padding: NSEdgeInsets(top: 5, left: 10.5, bottom: 5, right: 10.5))
    let emergency = EVChromeStyles(statusBar: fallback, statusCommand: fallback)
    var options = ViemDocumentOptions()
    options.struct_size = UInt32(MemoryLayout<ViemDocumentOptions>.size)
    options.encoding = UInt32(VIEM_ENCODING_UTF8)
    options.format = UInt32(VIEM_FORMAT_PLAIN_TEXT)
    options.file_format = UInt32(VIEM_FILE_FORMAT_UNIX)
    var core: ViemCoreHandle = 0
    var revision: UInt64 = 0
    guard viem_core_create(nil, 0, &options, &core, &revision) == UInt32(VIEM_STATUS_OK) else { return emergency }
    defer { _ = viem_core_destroy(core) }
    if let data = try? configuration.styleDefaults(named: "text") {
      _ = data.withUnsafeBytes { raw in
        viem_core_initialize_style_defaults(core, revision, raw.bindMemory(to: UInt8.self).baseAddress,
          UInt64(raw.count), nil, nil)
      }
    }
    var info = ViemStyleSheetInfoV1()
    info.struct_size = UInt32(MemoryLayout<ViemStyleSheetInfoV1>.size)
    guard viem_core_style_sheet_info(core, &info) == UInt32(VIEM_STATUS_OK),
          info.definition_count <= 4096, info.property_count <= 200_000,
          info.value_item_count <= 200_000, info.dependency_count <= 200_000,
          info.string_bytes <= 4 * 1024 * 1024 else { return emergency }
    var definitions = Array(repeating: ViemStyleDefinitionV1(), count: Int(info.definition_count))
    var properties = Array(repeating: ViemStylePropertyV1(), count: Int(info.property_count))
    var items = Array(repeating: ViemStyleValueItemV1(), count: Int(info.value_item_count))
    var dependencies = Array(repeating: ViemStyleDependencyV1(), count: Int(info.dependency_count))
    var strings = Array(repeating: UInt8(0), count: Int(info.string_bytes))
    var copied = ViemStyleSheetInfoV1()
    copied.struct_size = UInt32(MemoryLayout<ViemStyleSheetInfoV1>.size)
    var expected = info.identity
    let status = definitions.withUnsafeMutableBufferPointer { d in
      properties.withUnsafeMutableBufferPointer { p in
        items.withUnsafeMutableBufferPointer { i in
          dependencies.withUnsafeMutableBufferPointer { deps in
            strings.withUnsafeMutableBufferPointer { s in
              viem_core_copy_style_sheet(core, &expected, d.baseAddress, UInt64(d.count),
                p.baseAddress, UInt64(p.count), i.baseAddress, UInt64(i.count),
                deps.baseAddress, UInt64(deps.count), s.baseAddress, UInt64(s.count), &copied)
            }
          }
        }
      }
    }
    guard status == UInt32(VIEM_STATUS_OK) else { return emergency }
    func string(_ ref: ViemStyleStringRefV1) -> String {
      guard ref.offset <= UInt64(strings.count), ref.length <= UInt64(strings.count) - ref.offset else { return "" }
      return String(decoding: strings[Int(ref.offset)..<Int(ref.offset + ref.length)], as: UTF8.self)
    }
    func resolve(_ id: String) -> EVChromeStyle {
      guard let definition = definitions.first(where: { $0.namespace_id == 1 && string($0.stable_id) == id }),
        definition.first_property <= UInt64(properties.count),
        definition.property_count <= UInt64(properties.count) - definition.first_property else { return fallback }
      let entries = properties[Int(definition.first_property)..<Int(definition.first_property + definition.property_count)]
      let declarations = Dictionary(uniqueKeysWithValues: entries.map { ($0.property, $0) })
      let values = declarations.mapValues(\.effective)
      func number(_ key: UInt32) -> CGFloat { values[key].map { CGFloat($0.number) } ?? 0 }
      func color(_ key: UInt32) -> NSColor? {
        guard let entry = declarations[key], let v = values[key],
          v.kind == UInt32(VIEM_STYLE_VALUE_COLOR) else { return nil }
        if [18, 35, 37, 39, 41].contains(key),
          entry.flags & UInt32(VIEM_STYLE_PROPERTY_DECLARED | VIEM_STYLE_PROPERTY_CONTRIBUTOR_HAS_STYLE) == 0,
          entry.contributor_kind == UInt32(VIEM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY) { return nil }
        return NSColor(srgbRed: CGFloat(v.color.red), green: CGFloat(v.color.green), blue: CGFloat(v.color.blue), alpha: CGFloat(v.color.alpha))
      }
      let size = max(0.1, number(15))
      let fontValue = values[14]
      var families: [String] = []
      if let v = fontValue, v.first_item <= UInt64(items.count), v.item_count <= UInt64(items.count) - v.first_item {
        families = items[Int(v.first_item)..<Int(v.first_item + v.item_count)].map { string($0.string) }
      }
      let face = values[44].map { string($0.string) } ?? ""
      let axes = values[43].map { EVFontVariations.decode(string($0.string)) } ?? [:]
      let features: [(String, UInt32)]
      if let value = values[24], value.first_item <= UInt64(items.count),
          value.item_count <= UInt64(items.count) - value.first_item {
        features = items[Int(value.first_item)..<Int(value.first_item + value.item_count)].map {
          (string($0.string), $0.unsigned_value)
        }
      } else { features = [] }
      let bold = values[27]?.enum_value == 1
      let weight = values[16]?.enum_value ?? 400
      let selectedFont = resolveFont(families: families, size: size,
        cssWeight: CGFloat(bold ? min(1000, weight + 300) : weight), slant: values[17]?.enum_value ?? 0,
        features: features, relativeBold: bold,
        axes: axes, faceName: face) as NSFont
      func border(_ width: UInt32, _ tint: UInt32) -> EVChromeBorder { EVChromeBorder(width: number(width), color: color(tint)) }
      return EVChromeStyle(font: selectedFont, foreground: color(18) ?? theme.foreground.color,
        background: color(42), margin: NSEdgeInsets(top: number(6), left: number(29), bottom: number(7), right: number(28)),
        padding: NSEdgeInsets(top: number(30), left: number(33), bottom: number(32), right: number(31)),
        topBorder: border(34, 35), rightBorder: border(36, 37), bottomBorder: border(38, 39), leftBorder: border(40, 41),
        alignment: .left, letterSpacing: number(25))
    }
    return EVChromeStyles(statusBar: resolve("Status bar"), statusCommand: resolve("Status bar command"))
  }
}
