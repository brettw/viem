import Foundation

/// Opens or activates the GUI, returning as soon as the owning application has
/// accepted the command-line request. Document closure is not awaited.
public enum EVCommandLineLauncher {
    public static func run(arguments: [String]) throws -> Int32 {
        try run(arguments: arguments, workingDirectory: FileManager.default.currentDirectoryPath,
                wrapperExecutable: EVEditorLaunch.currentExecutableURL())
    }

    static func run(arguments: [String], workingDirectory: String, wrapperExecutable: URL,
                    startupTimeout: TimeInterval = 30, completionTimeout: TimeInterval = 30) throws -> Int32 {
        let listener = try EVBlockingEditListener()
        let process = try EVEditorLaunch.start(arguments: ["--cli-launch", listener.endpoint] + arguments,
            workingDirectory: workingDirectory, wrapperExecutable: wrapperExecutable)
        return try listener.waitForCompletion(startupTimeout: startupTimeout, completionTimeout: completionTimeout) {
            guard !process.isRunning, process.terminationStatus != 0 else { return nil }
            return "Viem failed to accept the launch request (launch status \(process.terminationStatus))."
        }
    }
}
