import AppKit
import CViemCore
import ViemAppShell

extension Notification.Name {
    static let viemGlobalCodeStyleDidChange = Notification.Name("com.viem.code-style.did-change")
}

/// An explicit settings target. Its history never touches a document session.
@MainActor
final class EVCodeStyleSession: EVStyleSettingsSession {
    var sourceFormat: EVSourceFormat { .code }
    private static var initializedDirectory: URL?
    private static var initializedThemeFile: URL?
    private static var initializedData: Data?
    private static var initializedOwnData: Data?
    private static var configurationObserver: NSObjectProtocol?
    private static var currentConfiguration: EVConfigurationStore?
    private static var publishingConfiguration: EVConfigurationStore?
    private static var initializationError: String?
    private static var fileMonitor: EVCodeStyleFileMonitor?
    let configuration: EVConfigurationStore
    let undoManager: UndoManager
    private let whitespaceSession: EVVisibleWhitespaceStyleSession
    private var cachedStyleSheet: EVStyleSheetSnapshot?
    private var groupStart: Data?
    private(set) var lastError: String?
    private var changeObserver: NSObjectProtocol?
    var isEditingGroup: Bool { groupStart != nil }

    init(configuration: EVConfigurationStore) throws {
        self.configuration = configuration
        let history = UndoManager()
        undoManager = history
        whitespaceSession = EVVisibleWhitespaceStyleSession(configuration: configuration, undoManager: history)
        undoManager.groupsByEvent = false
        do { try Self.initialize(configuration: configuration) }
        catch { lastError = error.localizedDescription }
        lastError = lastError ?? Self.initializationError ?? configuration.lastError
        whitespaceSession.onChange = { [weak self] in
            guard let self else { return }
            self.lastError = self.whitespaceSession.lastError
            NotificationCenter.default.post(name: .viemStyleSettingsSessionDidChange, object: self)
        }
        changeObserver = NotificationCenter.default.addObserver(forName: .viemGlobalCodeStyleDidChange, object: nil, queue: .main) { [weak self] notification in
            MainActor.assumeIsolated {
                guard let self else { return }
                if notification.userInfo?["external"] as? Bool == true {
                    self.groupStart = nil
                    self.undoManager.removeAllActions()
                    self.lastError = nil
                }
                NotificationCenter.default.post(name: .viemStyleSettingsSessionDidChange, object: self)
            }
        }
    }

    deinit { if let changeObserver { NotificationCenter.default.removeObserver(changeObserver) } }

    static func initialize(configuration: EVConfigurationStore) throws {
        let directory = configuration.directory.standardizedFileURL
        let file = configuration.selectedThemeURL?.standardizedFileURL
        let data = try configuration.themeStyleDefaults(for: .code)
        let ownData = try configuration.codeStyleSheet() ?? Data()
        if publishingConfiguration === configuration {
            // The core already owns this edit. Saved themes can retain unknown
            // JSON members that the core exporter intentionally does not emit.
            initializedData = data
            initializedOwnData = ownData
            initializationError = nil
            return
        }
        let changedTarget = initializedDirectory != directory || initializedThemeFile != file
            || currentConfiguration !== configuration
        if !changedTarget, initializedData == data {
            if let initializationError { throw NSError(domain: "ViemCodeStyle", code: 1, userInfo: [NSLocalizedDescriptionKey: initializationError]) }
            return
        }
        initializedDirectory = directory
        initializedThemeFile = file
        currentConfiguration = configuration
        if let configurationObserver { NotificationCenter.default.removeObserver(configurationObserver) }
        configurationObserver = NotificationCenter.default.addObserver(forName: .viemThemeDidChange, object: configuration, queue: .main) { [weak configuration] _ in
            MainActor.assumeIsolated {
                guard let configuration else { return }
                do { try initialize(configuration: configuration); fileMonitor?.didWriteFile() }
                catch { initializationError = error.localizedDescription }
            }
        }
        fileMonitor = configuration.selectedThemeURL.map { _ in
            EVCodeStyleFileMonitor(configuration: configuration) { data in
                try configuration.applyThemeFileData(data)
            }
        }
        do {
            // Compare the configuration authority with its previous value.
            // Exported core floats and sparse defaults can have different JSON
            // spellings even though no Code declaration changed.
            let same = initializedOwnData.flatMap { try? JSONSerialization.jsonObject(with: $0) as? NSDictionary }
                == (try? JSONSerialization.jsonObject(with: ownData) as? NSDictionary)
            let dependenciesChanged = initializedData != data
            if !same || dependenciesChanged { try replaceGlobalJSON(data, contextualized: true) }
            initializedData = data
            initializedOwnData = ownData
            initializationError = nil
            if changedTarget || !same || dependenciesChanged {
                NotificationCenter.default.post(name: .viemGlobalCodeStyleDidChange, object: configuration,
                    userInfo: ["external": changedTarget || !same])
            }
        } catch {
            let file = configuration.selectedThemeURL?.path ?? "Default"
            let message = "Code styles in \(file) could not be loaded. \(error.localizedDescription)"
            initializationError = message
            throw NSError(domain: "ViemCodeStyle", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
        }
    }

    func snapshot() throws -> EVStyleSheetSnapshot {
        let core = try coreSnapshot()
        var identity = core.identity
        identity.settingsRevision = whitespaceSession.revision
        return EVStyleSheetSnapshot(identity: identity,
            definitions: core.definitions + [whitespaceSession.style.definition(in: core)])
    }

    private func coreSnapshot() throws -> EVStyleSheetSnapshot {
        let identity = try EVCoreStyleBridge.styleIdentity(core: 0)
        if let cachedStyleSheet, cachedStyleSheet.identity == identity { return cachedStyleSheet }
        for _ in 0..<3 {
            do { let fresh = try EVCoreStyleBridge.copyStyleSheet(core: nil); cachedStyleSheet = fresh; return fresh }
            catch let error as EVStyleBridgeError where error.isStale { continue }
        }
        throw EVStyleBridgeError.core(status: UInt32(VIEM_STATUS_STALE_REVISION))
    }

    func beginGroup() throws {
        guard groupStart == nil else { return }
        groupStart = try Self.exportGlobalJSON()
        whitespaceSession.beginGroup()
    }

    func endGroup() {
        defer { whitespaceSession.endGroup() }
        guard let before = groupStart else { return }
        groupStart = nil
        if let after = try? Self.exportGlobalJSON(), before != after { registerUndo(before) }
    }

    func edit(key: EVStyleKey, expected: EVStyleSheetIdentity, mutation: EVStyleMutation) throws {
        try validate(expected)
        if key == .visibleWhitespace {
            guard whitespaceSession.apply([mutation]) else { throw whitespaceError() }
            lastError = nil
            return
        }
        try mutate { try EVCoreStyleBridge.applyCodeStyle(key: key, expected: expected, mutation: mutation) }
    }

    func create(key: EVStyleKey, name: String, expected: EVStyleSheetIdentity) throws {
        try validate(expected)
        try mutate { try EVCoreStyleBridge.createCodeStyle(key: key, name: name, expected: expected) }
    }

    func delete(key: EVStyleKey, expected: EVStyleSheetIdentity) throws {
        try validate(expected)
        try mutate { try EVCoreStyleBridge.deleteCodeStyle(key: key, expected: expected) }
    }

    func restoreDefaults(for key: EVStyleKey? = nil) throws {
        endGroup()
        if key == .visibleWhitespace {
            guard whitespaceSession.restoreDefaults() else { throw whitespaceError() }
            lastError = nil
            return
        }
        try mutate(replacingInvalidFile: true) { try Self.replaceGlobalJSON(Data()) }
        Self.initializationError = nil
    }

    private func validate(_ expected: EVStyleSheetIdentity) throws {
        guard try expected == snapshot().identity else {
            throw EVStyleBridgeError.core(status: UInt32(VIEM_STATUS_STALE_REVISION))
        }
    }

    private func whitespaceError() -> NSError {
        let message = whitespaceSession.lastError ?? "The Visible whitespace style could not be saved."
        lastError = message
        return NSError(domain: "Viem.VisibleWhitespaceStyle", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
    }

    private func mutate(replacingInvalidFile: Bool = false, _ action: () throws -> Void) throws {
        do { if !replacingInvalidFile { try Self.fileMonitor?.validateBeforeWrite() } }
        catch { lastError = error.localizedDescription; throw error }
        let before = try Self.exportGlobalJSON()
        try action()
        do {
            let after = try Self.exportGlobalJSON()
            guard after != before || replacingInvalidFile else { return }
            let context = try configuration.themeStyleDefaults(for: .code, overriding: after)
            Self.publishingConfiguration = configuration
            defer { Self.publishingConfiguration = nil }
            try configuration.saveCodeStyleSheet(after, replacingInvalidFile: replacingInvalidFile)
            Self.initializedData = try configuration.themeStyleDefaults(for: .code)
            if let savedContext = Self.initializedData, savedContext != context {
                try Self.replaceGlobalJSON(savedContext, contextualized: true)
            }
            Self.initializedOwnData = try configuration.codeStyleSheet() ?? Data()
            Self.fileMonitor?.didWriteFile()
            if groupStart == nil, before != after { registerUndo(before) }
            lastError = nil
            Self.initializationError = nil
            NotificationCenter.default.post(name: .viemGlobalCodeStyleDidChange, object: configuration)
        } catch {
            try? Self.replaceGlobalJSON(before)
            lastError = error.localizedDescription
            throw error
        }
    }

    private func registerUndo(_ before: Data) {
        let ownsGroup = undoManager.groupingLevel == 0
        if ownsGroup { undoManager.beginUndoGrouping() }
        undoManager.registerUndo(withTarget: self) { session in session.restoreFromHistory(before) }
        undoManager.setActionName("Code Style")
        if ownsGroup { undoManager.endUndoGrouping() }
    }

    private func restoreFromHistory(_ data: Data) {
        do { try mutate { try Self.replaceGlobalJSON(data) } }
        catch {
            lastError = error.localizedDescription
            NotificationCenter.default.post(name: .viemGlobalCodeStyleDidChange, object: configuration)
        }
    }

    static func exportGlobalJSON() throws -> Data {
        var required: UInt64 = 0
        let status = viem_code_export_style_json(nil, 0, &required)
        guard status == UInt32(VIEM_STATUS_OK) || status == UInt32(VIEM_STATUS_BUFFER_TOO_SMALL),
              required <= 4 * 1024 * 1024 else {
            throw EVStyleBridgeError.core(status: status)
        }
        var data = Data(count: Int(required))
        let result = data.withUnsafeMutableBytes { raw in
            viem_code_export_style_json(raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count), &required)
        }
        try check(result)
        return data
    }

    /// Explicit user reload re-applies the complete selected theme.
    static func reloadStyleSheet(completion: @escaping @MainActor (String?) -> Void) {
        guard let currentConfiguration else { completion("Themes are not loaded."); return }
        do { try currentConfiguration.reloadCurrentTheme(); completion(nil) }
        catch { completion(error.localizedDescription) }
    }

    static func validateThemeBeforeWrite(configuration: EVConfigurationStore) throws {
        try initialize(configuration: configuration)
        try fileMonitor?.validateBeforeWrite()
    }

    /// Opportunistic re-read used when an in-app write loses its conflict
    /// check. Unlike `reloadStyleSheet` it does nothing when the file is
    /// unchanged, so a local write is never reimported over itself.
    static func checkExternalStyleChanges(completion: @escaping @MainActor () -> Void) {
        guard let fileMonitor else { completion(); return }
        fileMonitor.checkForChanges { _ in completion() }
    }

    private static func replaceGlobalJSON(_ data: Data, contextualized: Bool = false) throws {
        let contextual: Data
        if contextualized { contextual = data }
        else { contextual = try currentConfiguration?.themeStyleDefaults(for: .code, overriding: data) ?? data }
        let result = contextual.withUnsafeBytes { raw in
            viem_code_replace_style_json(raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count))
        }
        try check(result)
    }

    private static func check(_ status: UInt32) throws {
        guard status == UInt32(VIEM_STATUS_OK) else { throw EVStyleBridgeError.core(status: status) }
    }
}
