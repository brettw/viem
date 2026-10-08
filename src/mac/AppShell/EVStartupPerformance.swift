import AppKit

/// Opt-in tracing of the shipped startup path. Normal launches retain no trace
/// and perform no report I/O. Milestones are timestamps, not additive scopes.
@MainActor
public enum EVStartupPerformance {
    private static let environment = ProcessInfo.processInfo.environment
    private static let report = environment["VIEM_STARTUP_REPORT"]
    private static let origin = ProcessInfo.processInfo.systemUptime
    private static var events: [[String: Any]] = []
    private static var completed = false

    public static var isEnabled: Bool { report != nil && !completed }

    public static func mark(_ name: String) {
        guard report != nil, !completed, events.count < 256 else { return }
        let started = origin
        events.append(["name": name, "milliseconds": (ProcessInfo.processInfo.systemUptime - started) * 1_000])
    }

    public static func firstDraw(document: URL?, width: CGFloat, height: CGFloat) {
        guard let report, !completed else { return }
        if let target = environment["VIEM_STARTUP_DOCUMENT"],
           document?.standardizedFileURL.path != URL(fileURLWithPath: target).standardizedFileURL.path { return }
        mark("editor.firstDraw")
        completed = true
        let elapsed = (ProcessInfo.processInfo.systemUptime - origin) * 1_000
        let launchTime = environment["VIEM_STARTUP_LAUNCH_TIME"].flatMap(Double.init)
        let result: [String: Any] = [
            "mainToFirstDrawMilliseconds": elapsed,
            "processToFirstDrawMilliseconds": launchTime.map { (Date.timeIntervalSinceReferenceDate - $0) * 1_000 } ?? elapsed,
            "document": document?.path ?? "", "width": width, "height": height,
            "events": events,
        ]
        // Finish the draw before diagnostic I/O and optional profiling exit.
        DispatchQueue.main.async {
            do {
                try JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
                    .write(to: URL(fileURLWithPath: report), options: .atomic)
            } catch { NSLog("Startup report: %@", error.localizedDescription) }
            if environment["VIEM_STARTUP_EXIT"] == "1" { NSApplication.shared.terminate(nil) }
        }
    }
}
