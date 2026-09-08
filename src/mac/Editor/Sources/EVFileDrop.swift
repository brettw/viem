import AppKit
import ViemAppShell

/// Finder supplies NSURL pasteboard objects. Plain text and web URLs must not
/// be interpreted as file names, and a file drop never inserts a path as prose.
@MainActor
enum EVFileDrop {
    static func fileURLs(on pasteboard: NSPasteboard) -> [URL] {
        guard let objects = pasteboard.readObjects(
            forClasses: [NSURL.self],
            options: [.urlReadingFileURLsOnly: true]
        ) else { return [] }
        return objects.compactMap { object in
            guard let url = object as? URL, url.isFileURL else { return nil }
            var isDirectory: ObjCBool = false
            if FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory),
               isDirectory.boolValue { return nil }
            return url
        }
    }

    static func operation(for sourceMask: NSDragOperation) -> NSDragOperation {
        // Opening never moves or deletes the Finder original.
        if sourceMask.contains(.copy) { return .copy }
        if sourceMask.contains(.link) { return .link }
        if sourceMask.contains(.generic) { return .generic }
        return []
    }
}

extension EVEditorSurfaceController {
    var acceptsFileDrops: Bool { session != nil && documentHostEffectHandler != nil }

    @discardableResult
    func openDroppedFiles(_ urls: [URL]) -> Bool {
        guard !urls.isEmpty, urls.allSatisfy(\.isFileURL),
              session != nil, let host = documentHostEffectHandler
        else { return false }
        host.openDroppedFiles(urls, in: self) { [weak self] result in
            guard let self else { return }
            if case let .failure(error) = result {
                self.showDocumentMessage(error.localizedDescription)
                NSSound.beep()
            }
        }
        return true
    }
}
