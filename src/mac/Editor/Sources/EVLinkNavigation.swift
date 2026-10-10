import AppKit
import CViemCore
import ViemAppShell

extension EVEditorSurfaceController: EVLinkFragmentNavigating {
    /// Resolve against current source and navigate through shared core policy.
    public func navigateToLinkFragment(_ fragment: String) throws {
        loadViewIfNeeded()
        guard let session else { throw EVDocumentHostError.unsupportedRequest }
        guard try session.navigateToLinkFragment(fragment) else { throw EVLinkError.missingFragment(fragment) }
        refreshPresentation()
    }

    /// Both popup and context-menu activation re-read the source target using
    /// the retained exact document/revision identity before any external effect.
    func openLink(at target: EVLinkMenuTarget) {
        do {
            guard let destination = try backend.linkDestination(at: target) else { return }
            let base = documentHostEffectHandler?.documentURL(for: self)
            switch try EVLinkOpener.destination(destination, relativeTo: base) {
            case .fragment(let fragment):
                try navigateToLinkFragment(fragment)
            case .document(let url, let fragment):
                guard let host = documentHostEffectHandler else {
                    throw EVDocumentHostError.unsupportedRequest
                }
                host.openLinkedDocument(url, fragment: fragment, from: self) { [weak self] result in
                    if case .failure(let error) = result { self?.report(error) }
                }
            case .web(let url):
                editorView.openLinkURL(url) { [weak self] error in
                    if let error { self?.report(error) }
                }
            }
        } catch { report(error) }
    }
}
