import AppKit
import CViemCore
import ViemAppShell

@MainActor
extension EVEditorSurfaceController: EVDocumentModeMenuProviding {
    /// The mode as this view shows the buffer: its revision, and Preview or
    /// source, since views of one Markdown buffer may differ.
    public func currentDocumentMode() -> EVDocumentModeState? {
        if let session { return session.documentModeState() }
        return try? EVDocumentModeJSON.read { viem_core_copy_document_mode_json(backend.core, $0, $1, $2) }
    }

    public func selectDocumentMode(_ choice: EVDocumentModeChoice, expected: EVDocumentModeState) {
        guard let session else { return }
        guard expected.documentId == documentState.document_id,
              expected.documentRevision == documentState.document_revision else {
            performInput { throw EVCoreFrontendError.core(operation: "Change document mode", status: UInt32(VIEM_STATUS_STALE_REVISION)) }
            return
        }
        guard acceptCompletionForNativeInput(), let accepted = currentDocumentMode() else { return }
        performInput {
            _ = try session.setDocumentMode(choice, expected: accepted)
        }
        view.window?.makeFirstResponder(editorView)
    }
}
