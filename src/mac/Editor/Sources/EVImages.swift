import AppKit
import CViemCore
import ViemAppShell

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
