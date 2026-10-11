import Foundation

enum EVStatusFilePath {
  /// Drop whole leading directories before shortening the filename itself.
  static func fitting(_ path: String, width: CGFloat, measure: (String) -> CGFloat) -> String {
    guard width > 0 else { return "" }
    if measure(path) <= width { return path }
    var filename = path
    let scalars = path.unicodeScalars
    for separator in scalars.indices where scalars[separator] == "/" {
      let suffix = String(scalars[scalars.index(after: separator)...])
      guard !suffix.isEmpty else { continue }
      filename = suffix
      if measure(suffix) <= width { return suffix }
    }
    let ellipsis = "..."
    guard measure(ellipsis) <= width else { return "" }
    let boundaries = Array(filename.indices) + [filename.endIndex]
    var low = 0, high = boundaries.count - 1
    while low < high {
      let middle = low + (high - low + 1) / 2
      if measure(String(filename[..<boundaries[middle]]) + ellipsis) <= width { low = middle }
      else { high = middle - 1 }
    }
    return String(filename[..<boundaries[low]]) + ellipsis
  }

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
    // Foundation can shorten an existing /private/var directory to /var while
    // leaving a not-yet-created filename below it unchanged. Normalize the
    // parent separately so renamed/unsaved bindings still share CWD's prefix.
    let target = file.deletingLastPathComponent().standardizedFileURL
      .appendingPathComponent(file.lastPathComponent).pathComponents
    let base = directory.standardizedFileURL.pathComponents
    let common = zip(base, target).prefix { $0.0 == $0.1 }.count
    let relative = Array(repeating: "..", count: base.count - common) + target.dropFirst(common)
    return (file.path, relative.isEmpty ? "." : relative.joined(separator: "/"), common > 1)
  }
}
