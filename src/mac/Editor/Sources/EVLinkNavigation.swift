import AppKit
import CViemCore
import ViemAppShell

extension EVEditorSurfaceController: EVLinkFragmentNavigating {
    /// Resolve a fresh heading against the current document and place the caret
    /// through the core's normal native navigation transaction.
    public func navigateToLinkFragment(_ fragment: String) throws {
        loadViewIfNeeded()
        guard let session else { throw EVDocumentHostError.unsupportedRequest }
        let state = try backend.documentState()
        var offset: UInt64 = 0
        var found: UInt8 = 0
        let bytes = Array(fragment.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            viem_core_find_link_fragment(backend.core, state.document_id, state.document_revision,
                ViemUtf8Slice(data: buffer.baseAddress, length: UInt64(buffer.count)), &offset, &found)
        }
        guard status == UInt32(VIEM_STATUS_OK) else {
            throw EVCoreFrontendError.core(operation: "Find linked heading", status: status)
        }
        guard found != 0 else { throw EVLinkError.missingFragment(fragment) }
        var point = ViemLayoutCaretPointV1()
        point.struct_size = UInt32(MemoryLayout<ViemLayoutCaretPointV1>.size)
        point.document_id = state.document_id
        point.document_revision = state.document_revision
        point.text_offset = offset
        point.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
        _ = try session.placeCursor(point, extendSelection: false)
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
