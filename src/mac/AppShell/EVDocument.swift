import AppKit
import UniformTypeIdentifiers

public enum EVSourceFormat: String, CaseIterable, Equatable, Sendable {
    case plainText
    case markdown
    case markdownSource
    case html
    case rtf

    public var displayName: String {
        switch self {
        case .plainText: "Plain Text"
        case .markdown: "Markdown WYSIWYG"
        case .markdownSource: "Markdown"
        case .html: "HTML"
        case .rtf: "RTF"
        }
    }

    public func hasSameSerialization(as other: Self) -> Bool {
        self == other || ([Self.markdown, .markdownSource].contains(self)
            && [Self.markdown, .markdownSource].contains(other))
    }
}

public enum EVDocumentSerializationError: LocalizedError, Equatable {
    case unsupportedWritableType(String)
    case formatConversionUnavailable(current: EVSourceFormat, requested: EVSourceFormat)

    public var errorDescription: String? {
        switch self {
        case let .unsupportedWritableType(typeName):
            "eVim cannot serialize the requested document type ‘\(typeName)’ safely."
        case let .formatConversionUnavailable(current, requested):
            "Saving \(current.displayName) as \(requested.displayName) requires a document format conversion, which is not available yet."
        }
    }
}

@MainActor
public final class EVDocument: NSDocument {
    public static let plainTextType = UTType.plainText.identifier
    public static let markdownType = UTType(filenameExtension: "md")?.identifier
        ?? "net.daringfireball.markdown"
    public static let markdownSourceType = "com.evim.markdown-source"
    public static let htmlType = UTType.html.identifier
    public static let rtfType = UTType.rtf.identifier

    nonisolated(unsafe) public let editorBackend: any EVDocumentBackend
    private struct ActiveSave {
        let snapshot: EVDocumentSaveSnapshot
        let sourceFormat: EVSourceFormat
    }

    private var activeSave: ActiveSave?

    public override convenience init() {
        self.init(editorBackend: EVFrontendRegistry.makeDocumentBackend())
    }

    /// Dependency-injected construction keeps NSDocument's native read/write
    /// boundary directly testable without changing the production registry.
    init(editorBackend: any EVDocumentBackend) {
        self.editorBackend = editorBackend
        super.init()
        configureBackendCallbacks()
        hasUndoManager = false
    }

    public override class var autosavesInPlace: Bool {
        true
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

    private static func typeName(for format: EVSourceFormat) -> String {
        switch format {
        case .plainText: plainTextType
        case .markdown, .markdownSource: markdownType
        case .html: htmlType
        case .rtf: rtfType
        }
    }

    /// Interactive eVim layout is deliberately continuous and unpaginated.
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
        }
    }

    public override nonisolated func data(ofType typeName: String) throws -> Data {
        try onMainActor {
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
        let snapshot: EVDocumentSaveSnapshot
        let sourceFormat: EVSourceFormat
        do {
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
        to url: URL,
        ofType typeName: String,
        for saveOperation: NSDocument.SaveOperationType,
        completionHandler: @escaping (Error?) -> Void
    ) {
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
            do {
                try self.editorBackend.acknowledgeNativeSave(snapshot)
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
        }
        editorBackend.persistenceStateDidChange = { [weak self] state in
            self?.synchronizeEditedState(state)
        }
        synchronizeEditedState(editorBackend.persistenceState)
    }

    private func synchronizeEditedState(_ state: EVDocumentPersistenceState) {
        // The source adapter can change through an undoable status option.
        // Native save validation must follow that current buffer state.
        fileType = Self.typeName(for: editorBackend.sourceFormat)
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
