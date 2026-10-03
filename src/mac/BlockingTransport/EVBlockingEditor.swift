import Darwin
import Foundation
import MachO

/// Launches the GUI directly so its existing single-instance broker also handles
/// editor invocations. The short-lived forwarding process never owns completion.
public enum EVBlockingEditor {
    public static func run(file: String) throws -> Int32 {
        try run(file: file, workingDirectory: FileManager.default.currentDirectoryPath,
                wrapperExecutable: currentExecutableURL())
    }

    static func run(file: String, workingDirectory: String, wrapperExecutable: URL,
                    startupTimeout: TimeInterval = 30) throws -> Int32 {
        guard !file.isEmpty, !file.contains("\0"), workingDirectory.hasPrefix("/") else {
            throw EVBlockingFailure("Expected a file path.")
        }
        let path = URL(fileURLWithPath: file, relativeTo:
            URL(fileURLWithPath: workingDirectory, isDirectory: true)).standardizedFileURL.path
        var isDirectory: ObjCBool = false
        if FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), isDirectory.boolValue {
            throw EVBlockingFailure("Expected a file, not a directory.")
        }
        let executable = wrapperExecutable.resolvingSymlinksInPath()
            .deletingLastPathComponent().appendingPathComponent("Viem")
        let listener = try EVBlockingEditListener()
        let process = Process()
        process.executableURL = executable
        process.arguments = ["--blocking-edit", listener.endpoint, path]
        process.currentDirectoryURL = URL(fileURLWithPath: workingDirectory, isDirectory: true)
        // A persistent GUI must not retain the terminal's input or output pipe
        // after the blocking caller returns. Diagnostics belong to this helper.
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        return try listener.waitForCompletion(startupTimeout: startupTimeout) {
            guard !process.isRunning, process.terminationStatus != 0 else { return nil }
            return "Viem failed to open the requested file (launch status \(process.terminationStatus))."
        }
    }

    static func currentExecutableURL() throws -> URL {
        var size: UInt32 = 0
        _ = _NSGetExecutablePath(nil, &size)
        var path = [CChar](repeating: 0, count: Int(size))
        guard _NSGetExecutablePath(&path, &size) == 0 else {
            throw EVBlockingFailure("Could not locate blocking-viem.")
        }
        return URL(fileURLWithPath: String(cString: path)).resolvingSymlinksInPath()
    }
}
