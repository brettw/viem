import AppKit
import UniformTypeIdentifiers

extension EVDocument {
    /// Export is an independent copy operation: it never enters NSDocument's
    /// save lifecycle or acknowledges a saved source revision.
    public func exportHTML(from surface: any EVEditorSurface) {
        let panel = makeHTMLExportPanel()
        let completed: (URL?) -> Void = { [weak self] destination in
            guard let self, let destination else { return }
            Task {
                do {
                    let data = try await surface.htmlExportData()
                    try await self.writeHTMLExport(data, to: destination)
                } catch {
                    self.presentError(error)
                }
            }
        }
        if let htmlExportPanelHandler {
            htmlExportPanelHandler(panel, completed)
        } else if let window = surface.viewController.view.window {
            panel.beginSheetModal(for: window) { completed($0 == .OK ? panel.url : nil) }
        } else {
            panel.begin { completed($0 == .OK ? panel.url : nil) }
        }
    }

    func makeHTMLExportPanel() -> NSSavePanel {
        let panel = NSSavePanel()
        panel.title = "Export"
        panel.prompt = "Export"
        panel.allowedContentTypes = [.html]
        panel.allowsOtherFileTypes = false
        panel.canCreateDirectories = true
        panel.isExtensionHidden = false
        let filename = fileURL?.deletingPathExtension().lastPathComponent ?? "Untitled"
        let suffix = ["html", "htm", "xhtml"].contains(fileURL?.pathExtension.lowercased() ?? "") ? "-export" : ""
        panel.nameFieldStringValue = "\(filename)\(suffix).html"
        panel.directoryURL = fileURL?.deletingLastPathComponent()
        return panel
    }

    func writeHTMLExport(_ data: Data, to destination: URL) async throws {
        let sourceURLs = NSDocumentController.shared.documents.compactMap(\.fileURL) + (fileURL.map { [$0] } ?? [])
        if sourceURLs.contains(where: { EVDocumentIdentity.sameFile($0, destination) }) {
            throw EVDocumentSerializationError.exportOverwritesSource
        }
        try await Task.detached(priority: .userInitiated) {
            try data.write(to: destination, options: .atomic)
        }.value
    }
}
