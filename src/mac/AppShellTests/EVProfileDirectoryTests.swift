import Foundation
import XCTest
@testable import ViemAppShell

@MainActor
final class EVProfileDirectoryTests: XCTestCase {
  func testDefaultProfileAndOverridePrecedence() {
    let home = URL(fileURLWithPath: "/Users/writer", isDirectory: true)
    let normal = EVProfileDirectory.resolve(environment: [:], homeDirectory: home)
    XCTAssertEqual(normal.url.path, "/Users/writer/.viem")
    XCTAssertTrue(normal.usesDefaultDirectory)

    let environment = ["VIEM_CONFIG_DIR": "/tmp/viem alternate profile"]
    let alternate = EVProfileDirectory.resolve(environment: environment, homeDirectory: home)
    XCTAssertEqual(alternate.url.path, "/tmp/viem alternate profile")
    XCTAssertFalse(alternate.usesDefaultDirectory)

    let explicit = EVProfileDirectory.resolve(
      directory: URL(fileURLWithPath: "/tmp/viem-explicit", isDirectory: true),
      environment: environment, homeDirectory: home)
    XCTAssertEqual(explicit.url.path, "/tmp/viem-explicit")
    XCTAssertFalse(explicit.usesDefaultDirectory)
  }

  func testSettingsAndStylesUseTheResolvedProfile() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("viem-profile-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: root) }
    let home = root.appendingPathComponent("home", isDirectory: true)
    let alternate = root.appendingPathComponent("alternate", isDirectory: true)
    let configuration = EVConfigurationStore(
      environment: ["VIEM_CONFIG_DIR": alternate.path], homeDirectory: home)
    XCTAssertEqual(configuration.directory.path, alternate.path)
    try configuration.setSmartQuotes(true)
    try configuration.saveCodeStyleSheet(Data(#"{"version":3}"#.utf8))
    XCTAssertTrue(FileManager.default.fileExists(atPath: alternate.appendingPathComponent("config.json").path))
    XCTAssertTrue(FileManager.default.fileExists(atPath: try XCTUnwrap(configuration.selectedThemeURL).path))
    XCTAssertFalse(FileManager.default.fileExists(atPath: home.appendingPathComponent(".viem").path))
  }

  func testInjectedHomeSeedsItsOwnThemeCatalogue() throws {
    let home = FileManager.default.temporaryDirectory.appendingPathComponent("viem-home-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: home) }
    let configuration = EVConfigurationStore(environment: [:], homeDirectory: home)
    XCTAssertEqual(configuration.directory, home.appendingPathComponent(".viem", isDirectory: true))
    XCTAssertTrue(FileManager.default.fileExists(atPath: home.path))
    XCTAssertEqual(configuration.availableThemeNames, ["Midnight", "Paper"])
    XCTAssertEqual(configuration.currentThemeName, "Midnight")
    XCTAssertFalse(configuration.smartQuotes)
  }
}
