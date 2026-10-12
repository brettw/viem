import Foundation
import MachO

/// Both terminal helpers launch the GUI beside their actual executable, even
/// when invoked through a link elsewhere on PATH.
enum EVEditorLaunch {
    static func currentExecutableURL() throws -> URL {
        var size: UInt32 = 0
        _ = _NSGetExecutablePath(nil, &size)
        var path = [CChar](repeating: 0, count: Int(size))
        guard _NSGetExecutablePath(&path, &size) == 0 else {
            throw EVBlockingFailure("Could not locate the Viem launcher.")
        }
        return URL(fileURLWithPath: String(cString: path)).resolvingSymlinksInPath()
    }

    static func validateWorkingDirectory(_ workingDirectory: String) throws {
        var isDirectory: ObjCBool = false
        guard workingDirectory.hasPrefix("/"), !workingDirectory.contains("\0"),
              FileManager.default.fileExists(atPath: workingDirectory, isDirectory: &isDirectory),
              isDirectory.boolValue else {
            throw EVBlockingFailure("The current working directory is unavailable.")
        }
    }

    static func start(arguments: [String], workingDirectory: String, wrapperExecutable: URL) throws -> Process {
        try validateWorkingDirectory(workingDirectory)
        guard arguments.allSatisfy({ !$0.contains("\0") }) else {
            throw EVBlockingFailure("Viem arguments cannot contain a null character.")
        }
        // Foundation keeps NUL percent-encoded even in URL.path. Decode the
        // encoded path once so real "%00" filenames remain valid.
        guard wrapperExecutable.isFileURL,
              let wrapperPath = wrapperExecutable.path(percentEncoded: true).removingPercentEncoding,
              !wrapperPath.contains("\0") else {
            throw EVBlockingFailure("Could not locate the Viem launcher.")
        }
        let executable = wrapperExecutable.resolvingSymlinksInPath()
            .deletingLastPathComponent().appendingPathComponent("Viem")
        guard FileManager.default.isExecutableFile(atPath: executable.path) else {
            throw EVBlockingFailure("Could not find the Viem GUI beside this launcher.")
        }
        let process = Process()
        process.executableURL = executable
        process.arguments = arguments
        process.currentDirectoryURL = URL(fileURLWithPath: workingDirectory, isDirectory: true)
        // A persistent GUI must not retain the terminal's input or output pipe
        // after its caller returns. Diagnostics belong to the terminal helper.
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        do { try process.run() }
        catch { throw EVBlockingFailure("Could not launch the Viem GUI.") }
        return process
    }
}
