import AppKit
import ViemAppShell

/// One process-wide settings watch, independent of the number or size of buffers.
/// File reads run off the main actor and are capped before allocating their data.
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
    private let file: URL
    private let apply: @MainActor (Data) throws -> Void
    private var baseline: Stamp?
    private var generation: UInt64 = 0
    private var inFlight = false
    private var timer: Timer?
    private var completions: [@MainActor () -> Void] = []

    init(configuration: EVConfigurationStore, apply: @escaping @MainActor (Data) throws -> Void) {
        file = configuration.directory.appendingPathComponent("code_style.json")
        self.apply = apply
        baseline = try? Self.stamp(file)
        EVCodePreferences.shared.reportLoadDiagnostics([], source: "code_styles_file")
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.checkForChanges() }
        }
        if let timer { RunLoop.main.add(timer, forMode: .common) }
    }

    deinit { timer?.invalidate() }

    func didWriteFile() {
        generation &+= 1
        baseline = try? Self.stamp(file)
        report(nil)
    }

    func validateBeforeWrite() throws {
        guard try Self.stamp(file) == baseline else {
            checkForChanges()
            throw NSError(domain: "ViemCodeStyle", code: 3, userInfo: [NSLocalizedDescriptionKey: "Code styles changed outside Viem and are being reloaded. Retry the edit after the styles refresh."])
        }
    }

    func checkForChanges(completion: (@MainActor () -> Void)? = nil) {
        if let completion { completions.append(completion) }
        guard !inFlight else { return }
        inFlight = true
        let requestedGeneration = generation
        let baseline = baseline
        let file = file
        DispatchQueue.global(qos: .utility).async { [weak self] in
            let result = Self.readChange(file, baseline: baseline)
            DispatchQueue.main.async {
                guard let self else { completion?(); return }
                self.inFlight = false
                guard requestedGeneration == self.generation else { self.checkForChanges(); return }
                switch result {
                case .unchanged, .retry: break
                case .removed:
                    self.baseline = nil
                    self.report("code_style.json was removed. The last valid Code styles remain active; use Restore Defaults to reset them.")
                case let .failed(message, stamp):
                    self.baseline = stamp
                    self.report(message)
                case let .contents(data, stamp):
                    self.baseline = stamp
                    do { try self.apply(data); self.report(nil) }
                    catch { self.report("Unable to reload code_style.json: \(error.localizedDescription) The last valid Code styles remain active.") }
                }
                let callbacks = self.completions
                self.completions.removeAll()
                for callback in callbacks { callback() }
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

    private nonisolated static func readChange(_ file: URL, baseline: Stamp?) -> ReadResult {
        var observed: Stamp?
        do {
            observed = try stamp(file)
            guard observed != baseline else { return .unchanged }
            guard let observed else { return .removed }
            let limit = 4 * 1024 * 1024
            guard observed.size <= limit else { return .failed("code_style.json exceeds 4 MiB. The last valid Code styles remain active.", observed) }
            let handle = try FileHandle(forReadingFrom: file)
            defer { try? handle.close() }
            var data = Data()
            while data.count <= limit {
                let chunk = try handle.read(upToCount: min(64 * 1024, limit + 1 - data.count)) ?? Data()
                if chunk.isEmpty { break }
                data.append(chunk)
            }
            guard data.count <= limit else { return .failed("code_style.json exceeds 4 MiB. The last valid Code styles remain active.", observed) }
            guard try stamp(file) == observed else { return .retry }
            return .contents(data, observed)
        } catch { return .failed("Unable to read code_style.json: \(error.localizedDescription)", observed) }
    }
}
