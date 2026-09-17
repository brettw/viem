import Foundation

/// Resolves the application profile shared by settings, styles, and startup
/// commands. Consumers append their filenames to this directory instead of
/// interpreting the environment or the user's home directory independently.
public struct EVProfileDirectory: Equatable, Sendable {
  public let url: URL
  /// Only the normal user profile may implicitly import legacy preferences.
  /// Explicitly supplied legacy defaults may still be migrated for tests.
  public let usesDefaultDirectory: Bool

  public static func resolve(
    directory: URL? = nil,
    environment: [String: String] = ProcessInfo.processInfo.environment,
    homeDirectory: URL = FileManager.default.homeDirectoryForCurrentUser
  ) -> Self {
    if let directory {
      return Self(url: directory, usesDefaultDirectory: false)
    }
    if let path = environment["VIEM_CONFIG_DIR"] {
      return Self(url: URL(fileURLWithPath: path, isDirectory: true), usesDefaultDirectory: false)
    }
    return Self(url: homeDirectory.appendingPathComponent(".viem", isDirectory: true),
                usesDefaultDirectory: true)
  }
}
