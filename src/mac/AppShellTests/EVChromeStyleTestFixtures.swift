import AppKit
import XCTest
@testable import ViemAppShell

@MainActor
func editStatusStyle(_ configuration: EVConfigurationStore? = nil, id: String = "Status bar", family: String = "text",
                     character: [String: Any] = [:], block: [String: Any] = [:]) throws {
  let configuration = configuration ?? .shared
  let data = try XCTUnwrap(configuration.styleDefaults(named: family))
  var sheet = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
  var definitions = try XCTUnwrap(sheet["block_styles"] as? [[String: Any]])
  let index = try XCTUnwrap(definitions.firstIndex { $0["id"] as? String == id })
  var characters = definitions[index]["character"] as? [String: Any] ?? [:]
  characters.merge(character) { _, new in new }
  var blocks = definitions[index]["block"] as? [String: Any] ?? [:]
  blocks.merge(block) { _, new in new }
  definitions[index]["character"] = characters
  definitions[index]["block"] = blocks
  sheet["block_styles"] = definitions
  try configuration.saveStyleDefaults(JSONSerialization.data(withJSONObject: sheet), named: family)
}
