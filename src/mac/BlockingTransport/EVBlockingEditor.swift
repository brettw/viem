import Foundation

/// Launches the GUI directly so its existing single-instance broker also handles
/// editor invocations. The short-lived forwarding process never owns completion.
public enum EVBlockingEditor {
    public static func run(file: String) throws -> Int32 {
        try run(file: file, workingDirectory: FileManager.default.currentDirectoryPath,
                wrapperExecutable: EVEditorLaunch.currentExecutableURL())
    }

    static func run(file: String, workingDirectory: String, wrapperExecutable: URL,
                    startupTimeout: TimeInterval = 30) throws -> Int32 {
        guard !file.isEmpty, !file.contains("\0") else {
            throw EVBlockingFailure("Expected a file path.")
        }
        try EVEditorLaunch.validateWorkingDirectory(workingDirectory)
        let path = URL(fileURLWithPath: file, relativeTo:
            URL(fileURLWithPath: workingDirectory, isDirectory: true)).standardizedFileURL.path
        var isDirectory: ObjCBool = false
        if FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), isDirectory.boolValue {
            throw EVBlockingFailure("Expected a file, not a directory.")
        }
        let listener = try EVBlockingEditListener()
        let process = try EVEditorLaunch.start(arguments: ["--blocking-edit", listener.endpoint, path],
            workingDirectory: workingDirectory, wrapperExecutable: wrapperExecutable)
        return try listener.waitForCompletion(startupTimeout: startupTimeout) {
            guard !process.isRunning, process.terminationStatus != 0 else { return nil }
            return "Viem failed to open the requested file (launch status \(process.terminationStatus))."
        }
    }
}
