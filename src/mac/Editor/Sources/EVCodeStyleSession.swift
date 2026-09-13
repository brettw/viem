import AppKit
import CViemCore
import ViemAppShell

extension Notification.Name {
    static let viemGlobalCodeStyleDidChange = Notification.Name("com.viem.code-style.did-change")
}

/// An explicit settings target. Its history never touches a document session.
@MainActor
final class EVCodeStyleSession {
    private static var initializedDirectory: URL?
    private static var initializationError: String?
    private static var fileMonitor: EVCodeStyleFileMonitor?
    let configuration: EVConfigurationStore
    let undoManager = UndoManager()
    private var groupStart: Data?
    private(set) var lastError: String?
    private var changeObserver: NSObjectProtocol?
    var isEditingGroup: Bool { groupStart != nil }

    init(configuration: EVConfigurationStore) throws {
        self.configuration = configuration
        undoManager.groupsByEvent = false
        do { try Self.initialize(configuration: configuration) }
        catch { lastError = error.localizedDescription }
        lastError = lastError ?? Self.initializationError
        changeObserver = NotificationCenter.default.addObserver(forName: .viemGlobalCodeStyleDidChange, object: nil, queue: .main) { [weak self] notification in
            MainActor.assumeIsolated {
                guard notification.userInfo?["external"] as? Bool == true, let self else { return }
                self.groupStart = nil
                self.undoManager.removeAllActions()
                self.lastError = nil
            }
        }
    }

    deinit { if let changeObserver { NotificationCenter.default.removeObserver(changeObserver) } }

    static func initialize(configuration: EVConfigurationStore) throws {
        let directory = configuration.directory.standardizedFileURL
        if initializedDirectory == directory {
            if let initializationError { throw NSError(domain: "ViemCodeStyle", code: 1, userInfo: [NSLocalizedDescriptionKey: initializationError]) }
            return
        }
        initializedDirectory = directory
        fileMonitor = EVCodeStyleFileMonitor(configuration: configuration) { data in
            guard initializedDirectory == directory else { return }
            let before = try exportGlobalJSON()
            // An external empty file is malformed, not an implicit Reset action.
            guard !data.isEmpty else { throw NSError(domain: "ViemCodeStyle", code: 2, userInfo: [NSLocalizedDescriptionKey: "The stylesheet is empty."]) }
            try replaceGlobalJSON(data)
            initializationError = nil
            let changed = try exportGlobalJSON() != before
            NotificationCenter.default.post(name: .viemGlobalCodeStyleDidChange, object: configuration, userInfo: ["external": changed])
        }
        do {
            let data = try configuration.codeStyleSheet() ?? Data()
            try replaceGlobalJSON(data)
            initializationError = nil
        } catch {
            try? replaceGlobalJSON(Data())
            let reason: String
            if let bridgeError = error as? EVStyleBridgeError,
               case .core(let status) = bridgeError,
               status == UInt32(VIEM_STATUS_INVALID_ARGUMENT) {
                reason = "The file contains invalid style definitions."
            } else {
                reason = error.localizedDescription
            }
            let file = directory.appendingPathComponent("code_style.json")
            let message = "The Code stylesheet at \(file.path) could not be loaded. \(reason) Default Code styles are active."
            initializationError = message
            throw NSError(domain: "ViemCodeStyle", code: 1, userInfo: [
                NSLocalizedDescriptionKey: message,
                NSUnderlyingErrorKey: error,
            ])
        }
    }

    func snapshot() throws -> EVStyleSheetSnapshot {
        for _ in 0..<3 {
            do { return try EVCoreStyleBridge.copyStyleSheet(core: nil) }
            catch let error as EVStyleBridgeError where error.isStale { continue }
        }
        throw EVStyleBridgeError.core(status: UInt32(VIEM_STATUS_STALE_REVISION))
    }

    func beginGroup() throws {
        guard groupStart == nil else { return }
        groupStart = try Self.exportGlobalJSON()
    }

    func endGroup() {
        guard let before = groupStart else { return }
        groupStart = nil
        if let after = try? Self.exportGlobalJSON(), before != after { registerUndo(before) }
    }

    func edit(key: EVStyleKey, expected: EVStyleSheetIdentity, mutation: EVStyleMutation) throws {
        try mutate { try EVCoreStyleBridge.applyCodeStyle(key: key, expected: expected, mutation: mutation) }
    }

    func create(key: EVStyleKey, name: String, expected: EVStyleSheetIdentity) throws {
        try mutate { try EVCoreStyleBridge.createCodeStyle(key: key, name: name, expected: expected) }
    }

    func delete(key: EVStyleKey, expected: EVStyleSheetIdentity) throws {
        try mutate { try EVCoreStyleBridge.deleteCodeStyle(key: key, expected: expected) }
    }

    func restoreDefaults() throws {
        endGroup()
        try mutate(replacingInvalidFile: true) { try Self.replaceGlobalJSON(Data()) }
        Self.initializationError = nil
    }

    private func mutate(replacingInvalidFile: Bool = false, _ action: () throws -> Void) throws {
        do { if !replacingInvalidFile { try Self.fileMonitor?.validateBeforeWrite() } }
        catch { lastError = error.localizedDescription; throw error }
        let before = try Self.exportGlobalJSON()
        try action()
        do {
            let after = try Self.exportGlobalJSON()
            guard after != before || replacingInvalidFile else { return }
            try configuration.saveCodeStyleSheet(after, replacingInvalidFile: replacingInvalidFile)
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

    static func checkExternalStyleChanges(completion: @escaping @MainActor () -> Void) {
        guard let fileMonitor else { completion(); return }
        fileMonitor.checkForChanges(completion: completion)
    }

    private static func replaceGlobalJSON(_ data: Data) throws {
        let result = data.withUnsafeBytes { raw in
            viem_code_replace_style_json(raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count))
        }
        try check(result)
    }

    private static func check(_ status: UInt32) throws {
        guard status == UInt32(VIEM_STATUS_OK) else { throw EVStyleBridgeError.core(status: status) }
    }
}
