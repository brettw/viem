import Foundation

/// Completion belongs to the shared document, not a launch process or window.
/// Pending opens are tracked too, so an error exit releases every caller.
@MainActor
final class EVBlockingEditSessions {
    static let shared = EVBlockingEditSessions()
    private var completions: [UUID: EVBlockingEditCompletion] = [:]
    private var documents: [EVDocument: Set<UUID>] = [:]

    var count: Int { completions.count }

    func begin(_ completion: EVBlockingEditCompletion) throws -> UUID {
        guard completions.count < 128 else {
            throw EVLaunchArgumentError("Too many pending blocking editor requests.")
        }
        let token = UUID()
        completions[token] = completion
        return token
    }

    func attach(_ token: UUID, to document: EVDocument) {
        guard completions[token] != nil else { return }
        documents[document, default: []].insert(token)
    }

    func fail(_ token: UUID) {
        completions.removeValue(forKey: token)?.finish(exitCode: 1)
    }

    func released(_ document: EVDocument) {
        guard let tokens = documents.removeValue(forKey: document) else { return }
        for token in tokens { completions.removeValue(forKey: token)?.finish(exitCode: 0) }
    }

    func abort(exitCode: Int32) {
        let pending = completions.values
        completions.removeAll()
        documents.removeAll()
        for completion in pending { completion.finish(exitCode: exitCode) }
    }
}
