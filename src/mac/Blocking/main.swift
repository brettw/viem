import Darwin
import Foundation
import ViemBlockingTransport

let arguments = Array(CommandLine.arguments.dropFirst())
if arguments.count != 1 || arguments.first == "--help" || arguments.first == "-h" {
    FileHandle.standardError.write(Data("Usage: blocking-viem <file>\n".utf8))
    exit(arguments.count == 1 ? 0 : 1)
}
do {
    let status = try EVBlockingEditor.run(file: arguments[0])
    exit(status)
} catch {
    FileHandle.standardError.write(Data("blocking-viem: \(error.localizedDescription)\n".utf8))
    exit(1)
}
