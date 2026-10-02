import AppKit
import CoreText
import Foundation

public struct EVFontAxis: Equatable, Sendable {
  public let tag: String
  public let name: String
  public let minimum: Double
  public let defaultValue: Double
  public let maximum: Double
  public let hidden: Bool
}
public struct EVFontInstance: Equatable, Sendable {
  public let name: String
  public let values: [String: Double]
}
public struct EVFontStyleLink: Sendable {
  public let from: Double
  public let to: Double
}
public struct EVFontVariationInfo: Sendable {
  public let axes: [EVFontAxis]
  public let instances: [EVFontInstance]
  public let links: [String: [EVFontStyleLink]]
  public var defaults: [String: Double] { Dictionary(uniqueKeysWithValues: axes.map { ($0.tag, $0.defaultValue) }) }
  public static let empty = EVFontVariationInfo(axes: [], instances: [], links: [:])
}

public enum EVFontVariations {
  private enum CacheKey: Hashable {
    case request(String)
    case font(name: String, url: URL?)
  }
  private static let lock = NSLock()
  nonisolated(unsafe) private static var generation: UInt64 = 1
  nonisolated(unsafe) private static var cache: [CacheKey: EVFontVariationInfo] = [:]
  static func invalidate() {
    lock.lock()
    generation += 1
    cache.removeAll()
    lock.unlock()
  }
  private static func store(_ value: EVFontVariationInfo, for key: CacheKey, generation expected: UInt64) {
    lock.lock()
    if generation == expected { cache[key] = value }
    lock.unlock()
  }
  public static func info(for name: String) -> EVFontVariationInfo {
    let key = CacheKey.request(name)
    lock.lock(); let existing = cache[key]; let expected = generation; lock.unlock()
    if let existing { return existing }
    guard let font = EVFontCatalog.availableFont(named: name, size: 14) else { return .empty }
    let value = info(font: font)
    store(value, for: key, generation: expected)
    return value
  }
  public static func info(font: CTFont) -> EVFontVariationInfo {
    let key = CacheKey.font(name: CTFontCopyPostScriptName(font) as String,
      url: CTFontCopyAttribute(font, kCTFontURLAttribute) as? URL)
    lock.lock(); let existing = cache[key]; let expected = generation; lock.unlock()
    if let existing { return existing }
    guard let data = CTFontCopyTable(font, CTFontTableTag(0x66766172), []) as Data? else {
      store(.empty, for: key, generation: expected)
      return .empty
    }
    let fvar = [UInt8](data)
    let names = (CTFontCopyTable(font, CTFontTableTag(0x6e616d65), []) as Data?).map { [UInt8]($0) } ?? []
    let stat = (CTFontCopyTable(font, CTFontTableTag(0x53544154), []) as Data?).map { [UInt8]($0) } ?? []
    func u16(_ b: [UInt8], _ p: Int) -> Int { p >= 0 && p + 2 <= b.count ? Int(b[p]) << 8 | Int(b[p + 1]) : 0 }
    func u32(_ b: [UInt8], _ p: Int) -> UInt32 { p >= 0 && p + 4 <= b.count ? UInt32(b[p]) << 24 | UInt32(b[p + 1]) << 16 | UInt32(b[p + 2]) << 8 | UInt32(b[p + 3]) : 0 }
    func fixed(_ b: [UInt8], _ p: Int) -> Double { Double(Int32(bitPattern: u32(b, p))) / 65536 }
    func tag(_ b: [UInt8], _ p: Int) -> String { p >= 0 && p + 4 <= b.count ? String(bytes: b[p..<p+4], encoding: .ascii) ?? "" : "" }
    func name(_ id: Int, _ fallback: String) -> String {
      guard names.count >= 6 else { return fallback }
      var result: String?
      for i in 0..<u16(names, 2) {
        let p = 6 + i * 12
        guard p + 12 <= names.count else { break }
        let platform = u16(names, p), size = u16(names, p + 8), offset = u16(names, 4) + u16(names, p + 10)
        guard u16(names, p + 6) == id, platform == 0 || platform == 3, offset + size <= names.count,
              let value = String(bytes: names[offset..<offset+size], encoding: .utf16BigEndian) else { continue }
        if u16(names, p + 4) == 0x409 { return value }
        if result == nil { result = value }
      }
      return result ?? fallback
    }
    guard fvar.count >= 16 else { return .empty }
    let offset = u16(fvar, 4), count = u16(fvar, 8), size = u16(fvar, 10), instances = u16(fvar, 12), instanceSize = u16(fvar, 14)
    guard count <= 64, size >= 20, offset + count * size <= fvar.count,
          instances <= 4096, instances == 0 || instanceSize >= 4 + count * 4 else { return .empty }
    var axes: [EVFontAxis] = []
    for i in 0..<count {
      let p = offset + i * size, t = tag(fvar, p)
      let minimum = fixed(fvar, p + 4), defaultValue = fixed(fvar, p + 8), maximum = fixed(fvar, p + 12)
      guard t.utf8.count == 4, minimum <= defaultValue, defaultValue <= maximum, !axes.contains(where: { $0.tag == t }) else { return .empty }
      axes.append(EVFontAxis(tag: t, name: name(u16(fvar, p + 18), t), minimum: minimum, defaultValue: defaultValue, maximum: maximum, hidden: u16(fvar, p + 16) & 1 != 0))
    }
    var presets = [EVFontInstance(name: "Default", values: Dictionary(uniqueKeysWithValues: axes.map { ($0.tag, $0.defaultValue) }))]
    for i in 0..<instances {
      let p = offset + count * size + i * instanceSize
      guard p + instanceSize <= fvar.count else { break }
      let values = Dictionary(uniqueKeysWithValues: axes.enumerated().map { ($0.element.tag, fixed(fvar, p + 4 + $0.offset * 4)) })
      guard axes.allSatisfy({ values[$0.tag]! >= $0.minimum && values[$0.tag]! <= $0.maximum }) else { continue }
      presets.append(EVFontInstance(name: name(u16(fvar, p), "Instance \(i + 1)"), values: values))
    }
    var links: [String: [EVFontStyleLink]] = [:]
    if stat.count >= 18 {
      let axisSize = u16(stat, 4), axisCount = u16(stat, 6), axisOffset = Int(u32(stat, 8)), valueCount = u16(stat, 12), valueOffset = Int(u32(stat, 14))
      if axisSize >= 8 && axisCount <= 64 && axisOffset + axisCount * axisSize <= stat.count && valueCount <= 4096 && valueOffset + valueCount * 2 <= stat.count {
        for i in 0..<valueCount {
          let p = valueOffset + u16(stat, valueOffset + i * 2), axis = u16(stat, p + 2)
          if p + 16 <= stat.count && u16(stat, p) == 3 && axis < axisCount { links[tag(stat, axisOffset + axis * axisSize), default: []].append(EVFontStyleLink(from: fixed(stat, p + 8), to: fixed(stat, p + 12))) }
        }
      }
    }
    let result = EVFontVariationInfo(axes: axes, instances: presets, links: links)
    store(result, for: key, generation: expected)
    return result
  }
  public static func decode(_ json: String) -> [String: Double] {
    guard let data = json.data(using: .utf8), let values = try? JSONDecoder().decode([String: Double].self, from: data) else { return [:] }
    return values
  }
  public static func encode(_ values: [String: Double]) -> String {
    let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
    return (try? encoder.encode(values)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
  }
  public static func effective(_ info: EVFontVariationInfo, saved: [String: Double], weight: Double, bold: Bool, slant: UInt32) -> [String: Double] {
    var values = info.defaults
    for axis in info.axes {
      var value = saved[axis.tag] ?? axis.defaultValue
      if axis.tag == "wght" {
        value = saved["wght"] ?? (bold ? weight - 300 : weight)
        if bold { value = info.links["wght"]?.first(where: { abs(value - $0.from) < 0.001 && $0.to > value })?.to ?? value + 300 }
      }
      if slant != 0 && axis.tag == "ital" { value = 1 }
      if slant != 0 && axis.tag == "slnt" && !info.axes.contains(where: { $0.tag == "ital" }) { value = min(value, info.links["slnt"]?.first(where: { abs(value - $0.from) < 0.001 && $0.to < value })?.to ?? -12) }
      values[axis.tag] = min(axis.maximum, max(axis.minimum, value))
    }
    return values
  }
  public static func identifier(_ tag: String) -> UInt32 { tag.utf8.reduce(UInt32(0)) { ($0 << 8) | UInt32($1) } }
}
