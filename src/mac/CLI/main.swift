import Darwin
import Foundation
import ViemBlockingTransport

let arguments = Array(CommandLine.arguments.dropFirst())
if arguments.count == 1, arguments[0] == "--help" || arguments[0] == "-h" {
    FileHandle.standardOutput.write(Data("Usage: viem-cli [-o[count]] [-O[count]] [+line] [--] [file ...]\n".utf8))
    exit(0)
}
do {
    exit(try EVCommandLineLauncher.run(arguments: arguments))
} catch {
    FileHandle.standardError.write(Data("viem-cli: \(error.localizedDescription)\n".utf8))
    exit(1)
}
