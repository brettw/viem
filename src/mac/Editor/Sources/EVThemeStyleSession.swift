import AppKit
import ViemAppShell

@MainActor
protocol EVStyleSettingsSession: AnyObject {
    var configuration: EVConfigurationStore { get }
    var sourceFormat: EVSourceFormat { get }
    var undoManager: UndoManager { get }
    var lastError: String? { get }
    var isEditingGroup: Bool { get }
    func snapshot() throws -> EVStyleSheetSnapshot
    func beginGroup() throws
    func endGroup()
    func edit(key: EVStyleKey, expected: EVStyleSheetIdentity, mutation: EVStyleMutation) throws
}

extension EVStyleSettingsSession {
    var windowTitle: String {
        let family = sourceFormat == .code ? "Code" : sourceFormat == .plainText ? "Plain Text" : "Markdown"
        return "Theme styles — \(family) — \(configuration.currentThemeName ?? "Default")"
    }
}

extension Notification.Name {
    static let viemThemeStyleSessionDidChange = Notification.Name("com.viem.theme-style-session.did-change")
}

/// A settings editing session with its own history, separate from every open
/// document. Style edits stay in this disposable specimen.
@MainActor
final class EVThemeStyleSession: EVStyleSettingsSession {
    let configuration: EVConfigurationStore
    let sourceFormat: EVSourceFormat
    let undoManager = UndoManager()
    private var backend: EVCoreDocumentBackend
    private var viewSession: EVCoreViewSession
    private var groupStart: Data?
    private var isMutating = false
    private var observer: NSObjectProtocol?
    private(set) var lastError: String?
    var isEditingGroup: Bool { groupStart != nil }

    init(configuration: EVConfigurationStore, format: EVSourceFormat) throws {
        self.configuration = configuration
        sourceFormat = format == .markdownSource ? .markdown : format
        backend = try EVCoreDocumentBackend(themeStyleFormat: sourceFormat, configuration: configuration)
        viewSession = try EVCoreViewSession(document: backend, width: 400, height: 200)
        undoManager.groupsByEvent = false
        observer = NotificationCenter.default.addObserver(forName: .viemThemeDidChange, object: configuration, queue: .main) { [weak self] notification in
            MainActor.assumeIsolated {
                guard let self, !self.isMutating,
                      (notification.userInfo?["styleNames"] as? [String])?.contains(self.sourceFormat.defaultStyleName) == true else { return }
                self.groupStart = nil
                self.undoManager.removeAllActions()
                do { try self.rebuild(); self.lastError = nil }
                catch { self.lastError = error.localizedDescription }
                NotificationCenter.default.post(name: .viemThemeStyleSessionDidChange, object: self)
            }
        }
    }

    deinit { if let observer { NotificationCenter.default.removeObserver(observer) } }

    func snapshot() throws -> EVStyleSheetSnapshot { try backend.styleSheetSnapshot() }
    func beginGroup() throws { if groupStart == nil { groupStart = try backend.exportStyleDefaults() } }
    func endGroup() {
        guard let before = groupStart else { return }
        groupStart = nil
        if let after = try? backend.exportStyleDefaults(), before != after { registerUndo(before) }
    }

    func edit(key: EVStyleKey, expected: EVStyleSheetIdentity, mutation: EVStyleMutation) throws {
        try mutate { _ = try viewSession.editStyle(key: key, expected: expected, mutation: mutation) }
    }
    private func mutate(_ action: () throws -> Void) throws {
        try EVCodeStyleSession.validateThemeBeforeWrite(configuration: configuration)
        let before = try backend.exportStyleDefaults()
        isMutating = true
        defer { isMutating = false }
        do {
            try action()
            let after = try backend.exportStyleDefaults()
            guard before != after else { return }
            try configuration.saveStyleDefaults(after, named: sourceFormat.defaultStyleName)
            if groupStart == nil { registerUndo(before) }
            lastError = nil
            NotificationCenter.default.post(name: .viemThemeStyleSessionDidChange, object: self)
        } catch {
            try? rebuild()
            lastError = error.localizedDescription
            throw error
        }
    }

    private func registerUndo(_ data: Data) {
        let ownsGroup = undoManager.groupingLevel == 0
        if ownsGroup { undoManager.beginUndoGrouping() }
        undoManager.registerUndo(withTarget: self) { $0.restore(data) }
        undoManager.setActionName("Theme Style")
        if ownsGroup { undoManager.endUndoGrouping() }
    }

    private func restore(_ data: Data) {
        do {
            try EVCodeStyleSession.validateThemeBeforeWrite(configuration: configuration)
            let before = try backend.exportStyleDefaults()
            isMutating = true
            defer { isMutating = false }
            try configuration.saveStyleDefaults(data, named: sourceFormat.defaultStyleName)
            try rebuild()
            registerUndo(before)
            lastError = nil
        } catch { lastError = error.localizedDescription }
        NotificationCenter.default.post(name: .viemThemeStyleSessionDidChange, object: self)
    }

    private func rebuild() throws {
        let next = try EVCoreDocumentBackend(themeStyleFormat: sourceFormat, configuration: configuration)
        let session = try EVCoreViewSession(document: next, width: 400, height: 200)
        viewSession.detach()
        viewSession = session
        backend = next
    }
}
