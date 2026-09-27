import AppKit
import ViemAppShell

/// One process-wide handle on the selected theme file, independent of the
/// number or size of buffers. The file is read at startup and thereafter only
/// when the user reloads it or an in-app write needs its conflict check; it is
/// never polled. File reads run off the main actor and are capped before
/// allocating their data.
@MainActor
final class EVCodeStyleFileMonitor {
    private struct Stamp: Equatable, Sendable {
        let size: UInt64
        let inode: UInt64
        let modified: Date
    }
    private enum ReadResult: Sendable {
        case unchanged, retry, removed
        case contents(Data, Stamp)
        case failed(String, Stamp?)
    }
    private let configuration: EVConfigurationStore
    private let file: URL
    private let apply: @MainActor (Data) throws -> Void
    private var baseline: Stamp?
    private var generation: UInt64 = 0
    private var inFlight = false
    private var forceRequested = false
    private var completions: [@MainActor (String?) -> Void] = []

    init(configuration: EVConfigurationStore, apply: @escaping @MainActor (Data) throws -> Void) {
        self.configuration = configuration
        file = configuration.selectedThemeURL!
        self.apply = apply
        baseline = try? Self.stamp(file)
        EVCodePreferences.shared.reportLoadDiagnostics([], source: "code_styles_file")
    }

    func didWriteFile() {
        generation &+= 1
        baseline = try? Self.stamp(file)
        report(nil)
    }

    func validateBeforeWrite() throws {
        guard try Self.stamp(file) == baseline else {
            checkForChanges()
            throw NSError(domain: "ViemCodeStyle", code: 3, userInfo: [NSLocalizedDescriptionKey: "The theme changed outside Viem and is being reloaded. Retry the edit after the styles refresh."])
        }
    }

    /// `force` re-reads and re-applies the file even when its stamp is
    /// unchanged, so an explicit Reload is never a silent no-op.
    func checkForChanges(force: Bool = false, completion: (@MainActor (String?) -> Void)? = nil) {
        if let completion { completions.append(completion) }
        if force { forceRequested = true }
        guard !inFlight else { return }
        inFlight = true
        let forcing = forceRequested
        forceRequested = false
        let requestedGeneration = generation
        let baseline = baseline
        let file = file
        DispatchQueue.global(qos: .utility).async { [weak self] in
            let result = Self.readChange(file, baseline: baseline, force: forcing)
            DispatchQueue.main.async {
                guard let self else { completion?(nil); return }
                self.inFlight = false
                guard requestedGeneration == self.generation else {
                    if forcing { self.forceRequested = true }
                    self.checkForChanges()
                    return
                }
                // nil is success: the caller reports the failure itself, since
                // load diagnostics otherwise surface only in Settings.
                var outcome: String?
                switch result {
                case .unchanged: break
                case .retry:
                    // Only reachable when the file kept changing under every
                    // read, so an explicit reload must not claim it applied.
                    if forcing { outcome = "\(self.file.lastPathComponent) is being written right now. Reload again once the writer finishes." }
                case .removed:
                    self.baseline = nil
                    do { try self.configuration.ensureCurrentThemeExists() }
                    catch { outcome = error.localizedDescription }
                case let .failed(message, stamp):
                    self.baseline = stamp
                    outcome = message
                case let .contents(data, stamp):
                    self.baseline = stamp
                    do { try self.apply(data) }
                    catch { outcome = "Unable to reload \(self.file.lastPathComponent): \(error.localizedDescription) The last valid theme remains active." }
                }
                self.report(outcome)
                if self.forceRequested { self.checkForChanges(); return }
                let callbacks = self.completions
                self.completions.removeAll()
                for callback in callbacks { callback(outcome) }
            }
        }
    }

    private func report(_ message: String?) {
        EVCodePreferences.shared.reportLoadDiagnostics(message.map { [$0] } ?? [], source: "code_styles_file")
    }

    private nonisolated static func stamp(_ file: URL) throws -> Stamp? {
        do {
            let attributes = try FileManager.default.attributesOfItem(atPath: file.resolvingSymlinksInPath().path)
            return Stamp(size: (attributes[.size] as? NSNumber)?.uint64Value ?? 0,
                inode: (attributes[.systemFileNumber] as? NSNumber)?.uint64Value ?? 0,
                modified: attributes[.modificationDate] as? Date ?? .distantPast)
        } catch let error as CocoaError where error.code == .fileReadNoSuchFile || error.code == .fileNoSuchFile { return nil }
    }

    private nonisolated static func readChange(_ file: URL, baseline: Stamp?, force: Bool) -> ReadResult {
        for _ in 0..<4 {
            let result = readOnce(file, baseline: baseline, force: force)
            if case .retry = result { continue }
            return result
        }
        return .retry
    }

    private nonisolated static func readOnce(_ file: URL, baseline: Stamp?, force: Bool) -> ReadResult {
        var observed: Stamp?
        do {
            observed = try stamp(file)
            // A forced read still reports a missing file rather than comparing
            // one absent stamp against another and calling it unchanged.
            if !force, observed == baseline { return .unchanged }
            guard let observed else { return .removed }
            let limit = 20 * 1024 * 1024
            guard observed.size <= limit else { return .failed("\(file.lastPathComponent) exceeds 20 MiB. The last valid theme remains active.", observed) }
            let handle = try FileHandle(forReadingFrom: file)
            defer { try? handle.close() }
            var data = Data()
            while data.count <= limit {
                let chunk = try handle.read(upToCount: min(64 * 1024, limit + 1 - data.count)) ?? Data()
                if chunk.isEmpty { break }
                data.append(chunk)
            }
            guard data.count <= limit else { return .failed("\(file.lastPathComponent) exceeds 20 MiB. The last valid theme remains active.", observed) }
            guard try stamp(file) == observed else { return .retry }
            return .contents(data, observed)
        } catch { return .failed("Unable to read \(file.lastPathComponent): \(error.localizedDescription)", observed) }
    }
}
