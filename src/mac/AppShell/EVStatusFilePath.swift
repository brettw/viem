import Foundation

enum EVStatusFilePath {
  /// Keep the displayed path relative only when its common ancestor with CWD is below root.
  static func display(_ file: URL?, relativeTo directory: URL = URL(
    fileURLWithPath: FileManager.default.currentDirectoryPath, isDirectory: true)
  ) -> String {
    guard let file else { return "Untitled" }
    let paths = resolved(file, relativeTo: directory)
    return paths.commonAncestorBelowRoot ? paths.relative : paths.absolute
  }

  /// Clipboard requests explicitly ask for a relative path, even through root.
  static func relative(_ file: URL, relativeTo directory: URL = URL(
    fileURLWithPath: FileManager.default.currentDirectoryPath, isDirectory: true)
  ) -> String {
    resolved(file, relativeTo: directory).relative
  }

  private static func resolved(_ file: URL, relativeTo directory: URL)
    -> (absolute: String, relative: String, commonAncestorBelowRoot: Bool) {
    let file = file.standardizedFileURL
    let target = file.pathComponents
    let base = directory.standardizedFileURL.pathComponents
    let common = zip(base, target).prefix { $0.0 == $0.1 }.count
    let relative = Array(repeating: "..", count: base.count - common) + target.dropFirst(common)
    return (file.path, relative.isEmpty ? "." : relative.joined(separator: "/"), common > 1)
  }
}
