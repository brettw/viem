import AppKit
import CViemCore
import ViemAppShell

enum EVImageFileLocation {
    /// Keep selected filenames as URI path components so punctuation cannot
    /// turn a local filename into a fragment, query or scheme.
    static func destination(for file: URL, relativeTo document: URL?) -> String {
        guard let document, document.isFileURL, document.host == file.host else { return file.absoluteString }
        let directory = Array(Self.components(of: document).dropLast())
        let components = Self.components(of: file)
        var common = 0
        while common < min(directory.count, components.count), directory[common] == components[common] {
            common += 1
        }
        let ascent = directory.count - common
        // Root alone is not a useful common ancestor if we must climb to it.
        guard common > 1 || (common == 1 && ascent == 0) else { return file.absoluteString }
        let unreserved = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~")
        var path = Array(repeating: "..", count: ascent)
        for component in components.dropFirst(common) {
            guard let encoded = component.addingPercentEncoding(withAllowedCharacters: unreserved) else { return file.absoluteString }
            path.append(encoded)
        }
        return path.joined(separator: "/")
    }

    private static func components(of url: URL) -> [String] {
        // Foundation's file-URL standardization decomposes Unicode filenames.
        // Normalize only path separators and dot segments, retaining the exact
        // spelling provided by the native picker.
        var result = ["/"]
        for component in url.path.split(separator: "/") {
            if component == "." { continue }
            if component == ".." {
                if result.count > 1 { result.removeLast() }
            } else { result.append(String(component)) }
        }
        return result
    }
}

extension EVEditorSurfaceController {
    func imageDestination(at target: EVLinkMenuTarget) throws -> String? {
        guard let session else { return nil }
        let state = try backend.documentState()
        guard state.document_id == target.documentID, state.document_revision == target.revision else {
            throw EVCoreFrontendError.core(operation: "Read image", status: UInt32(VIEM_STATUS_STALE_REVISION))
        }
        let context = try session.inlineContentContext(.image)
        guard let item = context.item, item.start == target.offset else { return nil }
        return item.destination
    }

    func reloadImage(at target: EVLinkMenuTarget) {
        do {
            guard backend.sourceFormat == .markdown, let session,
                  let location = try imageDestination(at: target) else { return }
            _ = session.provider.reloadImage(location)
        } catch { report(error) }
    }

    /// Only an explicit location click launches anything. Preview loading uses
    /// the separate local-only raster reader and never calls this method.
    func openImage(at target: EVLinkMenuTarget) {
        do {
            guard let location = try imageDestination(at: target) else { return }
            let base = documentHostEffectHandler?.documentURL(for: self)
            switch try EVLinkOpener.destination(location, relativeTo: base) {
            case .web(let url):
                editorView.openLinkURL(url) { [weak self] error in if let error { self?.report(error) } }
            case .document(let url, _):
                guard let preview = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.Preview") else {
                    throw EVDocumentHostError.unsupportedRequest
                }
                // A source-authored filename must never launch an executable.
                NSWorkspace.shared.open([url], withApplicationAt: preview,
                    configuration: NSWorkspace.OpenConfiguration()) { [weak self] _, error in
                    if let error { Task { @MainActor [weak self] in self?.report(error) } }
                }
            case .fragment:
                throw EVDocumentHostError.unsupportedRequest
            }
        } catch { report(error) }
    }
}
