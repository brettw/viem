import AppKit
import UniformTypeIdentifiers

public enum EVSourceFormat: String, CaseIterable, Equatable, Sendable, Codable {
    case plainText
    case markdown
    case markdownSource
    case html
    case htmlSource
    case rtf

    public var displayName: String {
        switch self {
        case .plainText: "Plain Text"
        case .markdown: "Markdown WYSIWYG"
        case .markdownSource: "Markdown Source"
        case .html: "HTML WYSIWYG"
        case .htmlSource: "HTML Source"
        case .rtf: "RTF"
        }
    }

    public func hasSameSerialization(as other: Self) -> Bool {
        self == other || ([Self.markdown, .markdownSource].contains(self)
            && [Self.markdown, .markdownSource].contains(other))
            || ([Self.html, .htmlSource].contains(self) && [Self.html, .htmlSource].contains(other))
    }
}

public enum EVDocumentSerializationError: LocalizedError, Equatable {
    case unsupportedWritableType(String)
    case formatConversionUnavailable(current: EVSourceFormat, requested: EVSourceFormat)

    public var errorDescription: String? {
        switch self {
        case let .unsupportedWritableType(typeName):
            "Viem cannot serialize the requested document type ‘\(typeName)’ safely."
        case let .formatConversionUnavailable(current, requested):
            "Saving \(current.displayName) as \(requested.displayName) requires a document format conversion, which is not available yet."
        }
    }
}

public enum EVRecoveryOpenDecision: Equatable {
    case readOnly
    case editAnyway
    case recover(Int)
    case cancel
}

@MainActor
public final class EVDocument: NSDocument {
    public static let plainTextType = UTType.plainText.identifier
    public static let markdownType = UTType(filenameExtension: "md")?.identifier
        ?? "net.daringfireball.markdown"
    public static let markdownSourceType = "com.viem.markdown-source"
    public static let htmlSourceType = "com.viem.html-source"
    public static let htmlType = UTType.html.identifier
    public static let rtfType = UTType.rtf.identifier

    nonisolated(unsafe) public let editorBackend: any EVDocumentBackend
    private struct ActiveSave {
        let snapshot: EVDocumentSaveSnapshot
        let sourceFormat: EVSourceFormat
    }

    private var activeSave: ActiveSave?
    var fileBaseline: EVFileFingerprint?
    var fileBaselineURL: URL?
    var fileBaselineGeneration: UInt64 = 0
    public internal(set) var externalFileChange: EVExternalFileChange?
    var externalSaveDecisionHandler: ((EVExternalFileChange) -> Bool)?

    public private(set) var isReadOnly = false
    public private(set) var wasRecovered = false
    public private(set) var recoveryFailure: String?
    private var recoveryStore: EVRecoveryStore?
    private var recoveryTarget: URL?
    private var recoveryRequestedTarget: URL?
    private var retiredRecoveryStores: [EVRecoveryStore] = []
    private var recoveryWriteGeneration: UInt64 = 0
    var recoveryStoreFactory: (URL) throws -> EVRecoveryStore = { try EVRecoveryStore.claim(for: $0) }
    private var recoveryTimer: DispatchWorkItem?
    private var recoveryGeneration: UInt64 = 0
    private var recoveryLifecycleObserver: NSObjectProtocol?
    var recoveryIdleDelay: TimeInterval = 4
    var recoveryDecisionHandler: (([EVRecoveryCandidate]) -> EVRecoveryOpenDecision)?
    var readOnlySaveDecisionHandler: (() -> Bool)?
    var closeReviewDecisionHandler: ((@escaping (Bool) -> Void) -> Void)?

    public override var windowForSheet: NSWindow? {
        super.windowForSheet ?? EVDocumentWindowController.windowShowing(document: self)
    }

    public override func canClose(withDelegate delegate: Any, shouldClose: Selector?, contextInfo: UnsafeMutableRawPointer?) {
        guard let closeReviewDecisionHandler else {
            super.canClose(withDelegate: delegate, shouldClose: shouldClose, contextInfo: contextInfo)
            return
        }
        closeReviewDecisionHandler { [self] approved in
            replyToCloseReview(delegate: delegate, selector: shouldClose, approved: approved, contextInfo: contextInfo)
        }
    }

    private func replyToCloseReview(delegate: Any?, selector: Selector?, approved: Bool, contextInfo: UnsafeMutableRawPointer?) {
        guard let recipient = delegate as? NSObject, let selector,
              recipient.responds(to: selector) else { return }
        typealias Reply = @convention(c) (AnyObject, Selector, NSDocument, Bool, UnsafeMutableRawPointer?) -> Void
        let reply = unsafeBitCast(recipient.method(for: selector), to: Reply.self)
        reply(recipient, selector, self, approved, contextInfo)
    }

    public override func shouldCloseWindowController(
        _ windowController: NSWindowController, delegate: Any?, shouldClose: Selector?,
        contextInfo: UnsafeMutableRawPointer?
    ) {
        guard let controller = windowController as? EVDocumentWindowController else {
            super.shouldCloseWindowController(windowController, delegate: delegate,
                shouldClose: shouldClose, contextInfo: contextInfo)
            return
        }
        // NSWindow calls this before windowShouldClose. Use the same pane-aware
        // review here, rather than reviewing the primary document a second time
        // after AppKit has already accepted its Save or Delete decision.
        controller.reviewDocumentsForClose { [self] approved in
            replyToCloseReview(delegate: delegate, selector: shouldClose, approved: approved, contextInfo: contextInfo)
        }
    }

    public override func close() {
        recoveryTimer?.cancel()
        recoveryGeneration &+= 1
        let stores = retiredRecoveryStores + (recoveryStore.map { [$0] } ?? [])
        for store in stores { store.closeAndRemove() }
        // Ordinary termination must not exit before its owned locks are removed.
        for store in stores { store.waitForPendingOperations() }
        recoveryStore = nil
        retiredRecoveryStores.removeAll()
        recoveryRequestedTarget = nil
        super.close()
    }

    public override nonisolated func read(from url: URL, ofType typeName: String) throws {
        let target = EVDocumentIdentity.canonicalURL(url)
        let original = Result { try Data(contentsOf: target) }
        try onMainActor {
            let ownURLs = Set(self.retiredRecoveryStores.map(\.url) + (self.recoveryStore.map { [$0.url] } ?? []))
            let candidates = EVRecoveryStore.candidates(for: target).filter { !ownURLs.contains($0.url) }
            let decision = candidates.isEmpty ? EVRecoveryOpenDecision.editAnyway
                : self.recoveryDecisionHandler?(candidates) ?? self.askRecoveryDecision(candidates, target: target)
            if case .cancel = decision { throw CocoaError(.userCancelled) }
            let recovered: Bool
            switch decision {
            case let .recover(index):
                guard candidates.indices.contains(index), let snapshot = candidates[index].snapshot else { throw EVRecoveryError.noAvailableSlot }
                try self.editorBackend.restoreRecovery(snapshot)
                recovered = true
            case .readOnly, .editAnyway:
                try self.editorBackend.read(source: original.get(), typeName: typeName)
                recovered = false
            case .cancel: return
            }
            try self.setReadOnly(decision == .readOnly)
            self.wasRecovered = recovered
            if let original = try? original.get() { self.recordFileBaseline(original, at: target) }
            else { self.recordMissingFileBaseline(at: target) }
            self.recoveryTimer?.cancel()
            self.recoveryGeneration &+= 1
            self.recoveryRequestedTarget = target
            self.beginRecovery(for: target)
            self.synchronizeEditedState(self.editorBackend.persistenceState)
            if self.wasRecovered { self.updateChangeCount(.changeDone) }
        }
    }

    public func setReadOnly(_ value: Bool) throws {
        try editorBackend.setReadOnly(value)
        isReadOnly = value
    }

    /// Called when a previously untitled buffer acquires an original target,
    /// including an Ex new-file path before its first explicit write.
    public func configureRecovery(for url: URL) {
        let target = EVDocumentIdentity.canonicalURL(url)
        recoveryRequestedTarget = target
        if fileBaseline == nil && !FileManager.default.fileExists(atPath: target.path) { recordMissingFileBaseline(at: target) }
        guard recoveryTarget != target || recoveryStore == nil else { return }
        recoveryTimer?.cancel()
        beginRecovery(for: target)
    }

    private func beginRecovery(for target: URL) {
        do {
            let snapshot = try editorBackend.recoverySnapshot()
            let candidate = try recoveryStoreFactory(target)
            if let previous = recoveryStore {
                previous.invalidatePendingWrites()
                retiredRecoveryStores.append(previous)
            }
            recoveryStore = candidate
            recoveryTarget = target
            writeRecovery(snapshot, to: candidate)
        } catch { recoveryFailure = error.localizedDescription }
    }

    private func scheduleRecovery() {
        guard recoveryStore != nil || recoveryRequestedTarget != nil else { return }
        recoveryTimer?.cancel()
        recoveryStore?.invalidatePendingWrites()
        recoveryGeneration &+= 1
        let generation = recoveryGeneration
        let work = DispatchWorkItem { [weak self] in
            guard let self, self.recoveryGeneration == generation else { return }
            self.captureRecoveryNow()
        }
        recoveryTimer = work
        DispatchQueue.main.asyncAfter(deadline: .now() + recoveryIdleDelay, execute: work)
    }

    /// Copies one immutable snapshot on the document actor; JSON encoding and
    /// file replacement run on the store's utility queue.
    public func flushRecoverySnapshot() {
        recoveryTimer?.cancel()
        captureRecoveryNow()
    }

    private func captureRecoveryNow() {
        if let target = recoveryRequestedTarget, target != recoveryTarget || recoveryStore == nil {
            beginRecovery(for: target)
            return
        }
        guard let store = recoveryStore else { return }
        do { writeRecovery(try editorBackend.recoverySnapshot(), to: store) }
        catch { recoveryFailure = error.localizedDescription }
    }

    private func writeRecovery(_ snapshot: EVRecoverySnapshot, to store: EVRecoveryStore) {
        recoveryWriteGeneration &+= 1
        let generation = recoveryWriteGeneration
        store.write(snapshot, didCommit: { [weak self] in
            DispatchQueue.main.async {
                guard let self, self.recoveryStore === store, self.recoveryWriteGeneration == generation,
                      self.recoveryRequestedTarget == self.recoveryTarget else { return }
                let current = self.editorBackend.persistenceState
                guard current.documentID == snapshot.documentID && current.documentRevision == snapshot.documentRevision else {
                    self.scheduleRecovery()
                    return
                }
                self.recoveryFailure = nil
                // Rebinding keeps the old valid snapshot until a current one
                // has actually committed under the new target's ownership.
                for old in self.retiredRecoveryStores {
                    old.closeAndRemove { [weak self] in
                        DispatchQueue.main.async { self?.retiredRecoveryStores.removeAll { $0 === old } }
                    }
                }
            }
        }, completion: { [weak self] error in
            guard let error else { return }
            DispatchQueue.main.async {
                guard let self, self.recoveryStore === store, self.recoveryWriteGeneration == generation,
                      self.recoveryRequestedTarget == self.recoveryTarget else { return }
                self.recoveryFailure = error.localizedDescription
            }
        })
    }

    private func askRecoveryDecision(_ candidates: [EVRecoveryCandidate], target: URL) -> EVRecoveryOpenDecision {
        let alert = NSAlert()
        alert.messageText = "An editing session already exists for “\(target.lastPathComponent)”."
        alert.informativeText = "A swap or recovery file is present. Another editor may still be using this file. Opening read-only allows editing, and requires confirmation before saving."
        alert.addButton(withTitle: "Open Read-Only")
        alert.addButton(withTitle: "Edit Anyway")
        let recoverable = candidates.firstIndex { $0.snapshot != nil }
        if recoverable != nil { alert.addButton(withTitle: "Recover") }
        alert.addButton(withTitle: "Cancel")
        let response = alert.runModal().rawValue - NSApplication.ModalResponse.alertFirstButtonReturn.rawValue
        if response == 0 { return .readOnly }
        if response == 1 { return .editAnyway }
        if response == 2, let recoverable { return .recover(recoverable) }
        return .cancel
    }

    private func confirmReadOnlySave() -> Bool {
        if let handler = readOnlySaveDecisionHandler { return handler() }
        let alert = NSAlert()
        alert.messageText = "Save this read-only document?"
        alert.informativeText = "This document was opened read-only because another editing session may exist. Saving can replace changes made by that session."
        alert.addButton(withTitle: "Save Anyway")
        alert.addButton(withTitle: "Cancel")
        return alert.runModal() == .alertFirstButtonReturn
    }

    public override convenience init() {
        self.init(editorBackend: EVFrontendRegistry.makeDocumentBackend())
    }

    /// Dependency-injected construction keeps NSDocument's native read/write
    /// boundary directly testable without changing the production registry.
    init(editorBackend: any EVDocumentBackend) {
        self.editorBackend = editorBackend
        super.init()
        configureBackendCallbacks()
        recoveryLifecycleObserver = NotificationCenter.default.addObserver(forName: NSApplication.willResignActiveNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.flushRecoverySnapshot() }
        }
        hasUndoManager = false
    }

    deinit {
        if let recoveryLifecycleObserver { NotificationCenter.default.removeObserver(recoveryLifecycleObserver) }
    }

    public override class var autosavesInPlace: Bool {
        false
    }

    public override class var readableTypes: [String] {
        [plainTextType, markdownType, htmlType, rtfType]
    }

    public override class var writableTypes: [String] {
        readableTypes
    }

    public override class func isNativeType(_ type: String) -> Bool {
        readableTypes.contains(type)
    }

    public override nonisolated func writableTypes(for saveOperation: NSDocument.SaveOperationType) -> [String] {
        // The format selector owns adapter changes. A native Save panel must
        // offer the current serialization type, not imply a lossy conversion.
        (try? onMainActor { [Self.typeName(for: self.editorBackend.sourceFormat)] }) ?? []
    }

    public static func typeName(for format: EVSourceFormat) -> String {
        switch format {
        case .plainText: plainTextType
        case .markdown, .markdownSource: markdownType
        case .html, .htmlSource: htmlType
        case .rtf: rtfType
        }
    }

    /// Interactive Viem layout is deliberately continuous and unpaginated.
    /// Keep AppKit's standard Print item present, but disabled until a
    /// separate paginated print projection exists; printing the editor view
    /// would otherwise emit only the currently materialized viewport.
    public override func validateUserInterfaceItem(
        _ item: any NSValidatedUserInterfaceItem
    ) -> Bool {
        if item.action == #selector(NSDocument.printDocument(_:)) {
            return false
        }
        return super.validateUserInterfaceItem(item)
    }

    public override func makeWindowControllers() {
        addEditorWindowController()
    }

    public func addEditorWindowController() {
        let controller = EVDocumentWindowController(
            document: self,
            editorSurface: editorBackend.makeEditorSurface()
        )
        addWindowController(controller)
    }

    public func showAdditionalWindow() {
        addEditorWindowController()
        windowControllers.last?.showWindow(nil)
    }

    public override nonisolated func read(from data: Data, ofType typeName: String) throws {
        try onMainActor {
            try self.editorBackend.read(source: data, typeName: typeName)
            self.fileBaseline = nil
            self.fileBaselineURL = nil
            self.fileBaselineGeneration &+= 1
            self.externalFileChange = nil
        }
    }

    var recoveryURLForTesting: URL? { recoveryStore?.url }
    func drainRecoveryForTesting() { recoveryStore?.drainForTesting(); retiredRecoveryStores.forEach { $0.drainForTesting() } }

    public override nonisolated func data(ofType typeName: String) throws -> Data {
        try onMainActor {
            guard !self.isReadOnly || self.activeSave != nil else { throw EVRecoveryError.readOnly }
            if let activeSave = self.activeSave {
                _ = try Self.validateSerializationType(
                    typeName,
                    currentFormat: activeSave.sourceFormat
                )
                return activeSave.snapshot.data
            }
            _ = try Self.validateSerializationType(
                typeName,
                currentFormat: self.editorBackend.sourceFormat
            )
            return try self.editorBackend.serializedSource(typeName: typeName)
        }
    }

    /// Keep AppKit in charge of save panels, coordinated replacement, file
    /// identity, versions, and change-count tokens while binding the bytes and
    /// the core acknowledgement to one exact source revision.
    public override func save(
        to url: URL,
        ofType typeName: String,
        for saveOperation: NSDocument.SaveOperationType,
        completionHandler: @escaping (Error?) -> Void
    ) {
        if saveOperation == .autosaveInPlaceOperation || saveOperation == .autosaveElsewhereOperation {
            flushRecoverySnapshot()
            completionHandler(nil)
            return
        }
        guard !(isReadOnly || editorBackend.persistenceState.isReadOnly) || confirmReadOnlySave() else {
            completionHandler(CocoaError(.userCancelled))
            return
        }
        let snapshot: EVDocumentSaveSnapshot
        let sourceFormat: EVSourceFormat
        do {
            do { try validateExternalWrite(to: url, force: false) }
            catch EVExternalFileError.changed(let change) {
                guard confirmExternalOverwrite(change) else { throw CocoaError(.userCancelled) }
            }
            sourceFormat = try Self.validateSerializationType(
                typeName,
                currentFormat: editorBackend.sourceFormat
            )
            snapshot = try editorBackend.nativeSaveSnapshot(typeName: typeName)
        } catch {
            completionHandler(error)
            return
        }

        save(
            snapshot: snapshot,
            sourceFormat: sourceFormat,
            to: url,
            ofType: typeName,
            for: saveOperation,
            completionHandler: completionHandler
        )
    }

    /// Perform a host-requested save only when the raw Ex request still names
    /// the exact source revision that will be serialized. This closes the race
    /// between a command turn and an asynchronous AppKit save operation.
    func saveHostRevision(
        documentID: UInt64,
        documentRevision: UInt64,
        force: Bool = false,
        to url: URL,
        ofType typeName: String,
        for saveOperation: NSDocument.SaveOperationType,
        completionHandler: @escaping (Error?) -> Void
    ) {
        guard !(isReadOnly || editorBackend.persistenceState.isReadOnly) || force else { completionHandler(EVRecoveryError.readOnly); return }
        let snapshot: EVDocumentSaveSnapshot
        let sourceFormat: EVSourceFormat
        do {
            sourceFormat = try Self.validateSerializationType(
                typeName,
                currentFormat: editorBackend.sourceFormat
            )
            snapshot = try editorBackend.nativeSaveSnapshot(typeName: typeName)
            guard snapshot.documentID == documentID,
                  snapshot.documentRevision == documentRevision
            else {
                completionHandler(EVDocumentHostError.staleRequest)
                return
            }
            try validateExternalWrite(to: url, force: force)
        } catch {
            completionHandler(error)
            return
        }

        save(
            snapshot: snapshot,
            sourceFormat: sourceFormat,
            to: url,
            ofType: typeName,
            for: saveOperation,
            completionHandler: completionHandler
        )
    }

    private func save(
        snapshot: EVDocumentSaveSnapshot,
        sourceFormat: EVSourceFormat,
        to url: URL,
        ofType typeName: String,
        for saveOperation: NSDocument.SaveOperationType,
        completionHandler: @escaping (Error?) -> Void
    ) {

        activeSave = ActiveSave(snapshot: snapshot, sourceFormat: sourceFormat)
        super.save(
            to: url,
            ofType: typeName,
            for: saveOperation
        ) { [weak self] writeError in
            guard let self else {
                completionHandler(writeError)
                return
            }
            self.activeSave = nil
            guard writeError == nil else {
                self.synchronizeEditedState(self.editorBackend.persistenceState)
                completionHandler(writeError)
                return
            }
            guard Self.establishesSavePoint(saveOperation) else {
                // Save To, Duplicate, and recovery autosaves write a copy;
                // native NSDocument semantics do not make that copy the
                // source document's new save point.
                self.synchronizeEditedState(self.editorBackend.persistenceState)
                completionHandler(nil)
                return
            }
            let target = EVDocumentIdentity.canonicalURL(url)
            self.recordFileBaseline(snapshot.data, at: target)
            if self.recoveryTarget != target {
                // A successful Save As already adopted its native target even
                // when a newer edit makes the core acknowledgement stale.
                self.configureRecovery(for: target)
            }
            do {
                try self.editorBackend.acknowledgeNativeSave(snapshot)
                self.wasRecovered = false
                self.captureRecoveryNow()
                self.synchronizeEditedState(self.editorBackend.persistenceState)
                completionHandler(nil)
            } catch {
                // The file write itself succeeded, but a stale exact-revision
                // acknowledgement must not make a newer core snapshot clean.
                self.synchronizeEditedState(self.editorBackend.persistenceState)
                completionHandler(error)
            }
        }
    }

    private static func establishesSavePoint(_ operation: NSDocument.SaveOperationType) -> Bool {
        switch operation {
        case .saveOperation, .saveAsOperation, .autosaveInPlaceOperation:
            true
        default:
            false
        }
    }

    public static func sourceFormat(forTypeName typeName: String) -> EVSourceFormat? {
        let lowered = typeName.lowercased()
        if lowered == markdownSourceType { return .markdownSource }
        if lowered == htmlSourceType { return .htmlSource }
        if [".html", ".htm", htmlType].contains(lowered) || lowered.hasSuffix(".html") {
            return .html
        }
        if lowered == rtfType || lowered == ".rtf" || lowered.hasSuffix(".rtf") {
            return .rtf
        }
        if lowered.contains("markdown") || lowered == ".md" || lowered.hasSuffix(".md") {
            return .markdown
        }

        if let type = UTType(typeName) {
            if type.conforms(to: .html) { return .html }
            if type.conforms(to: .rtf) { return .rtf }
            if let markdown = UTType(filenameExtension: "md"),
               type == markdown || type.conforms(to: markdown)
            {
                return .markdown
            }
            if type.conforms(to: .plainText) {
                return .plainText
            }
        }

        if lowered == plainTextType.lowercased()
            || lowered.contains("plain-text")
            || lowered == ".txt"
            || lowered.hasSuffix(".txt")
        {
            return .plainText
        }
        return nil
    }

    @discardableResult
    private static func validateSerializationType(
        _ typeName: String,
        currentFormat: EVSourceFormat
    ) throws -> EVSourceFormat {
        guard let requestedFormat = sourceFormat(forTypeName: typeName) else {
            throw EVDocumentSerializationError.unsupportedWritableType(typeName)
        }
        guard requestedFormat.hasSameSerialization(as: currentFormat) else {
            throw EVDocumentSerializationError.formatConversionUnavailable(
                current: currentFormat,
                requested: requestedFormat
            )
        }
        return requestedFormat
    }

    private func configureBackendCallbacks() {
        editorBackend.sourceDidChange = { [weak self] in
            self?.updateChangeCount(.changeDone)
            self?.scheduleRecovery()
        }
        editorBackend.persistenceStateDidChange = { [weak self] state in
            self?.synchronizeEditedState(state)
            self?.scheduleRecovery()
        }
        synchronizeEditedState(editorBackend.persistenceState)
    }

    private func synchronizeEditedState(_ state: EVDocumentPersistenceState) {
        // The source adapter can change through an undoable status option.
        // Native save validation must follow that current buffer state.
        fileType = Self.typeName(for: editorBackend.sourceFormat)
        isReadOnly = state.isReadOnly
        if !state.isDirty { wasRecovered = false }
        if state.isDirty {
            if !isDocumentEdited {
                updateChangeCount(.changeDone)
            }
        } else if isDocumentEdited {
            updateChangeCount(.changeCleared)
        }
    }

    private nonisolated func onMainActor<T>(
        _ operation: @MainActor @escaping () throws -> T
    ) throws -> T {
        if Thread.isMainThread {
            return try MainActor.assumeIsolated(operation)
        }
        return try DispatchQueue.main.sync {
            try MainActor.assumeIsolated(operation)
        }
    }
}
