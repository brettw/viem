import CViemCore
import Foundation
import ViemAppShell

@MainActor
enum EVCoreLaunchArguments {
    private struct ParseResult: Decodable {
        var arguments: EVLaunchArguments?
        var error: String?
    }

    static func parse(_ arguments: [String]) throws -> EVLaunchArguments {
        let input = try JSONEncoder().encode(arguments)
        return try input.withUnsafeBytes { raw in
            let bytes = raw.bindMemory(to: UInt8.self)
            var required: UInt64 = 0
            let countStatus = viem_parse_launch_arguments(bytes.baseAddress, UInt64(bytes.count), nil, 0, &required)
            guard countStatus == UInt32(VIEM_STATUS_BUFFER_TOO_SMALL),
                  let count = Int(exactly: required) else {
                throw EVCoreFrontendError.core(operation: "Parse command-line arguments", status: countStatus)
            }
            var output = [UInt8](repeating: 0, count: count)
            let status = output.withUnsafeMutableBufferPointer { buffer in
                viem_parse_launch_arguments(bytes.baseAddress, UInt64(bytes.count), buffer.baseAddress, UInt64(buffer.count), &required)
            }
            guard status == UInt32(VIEM_STATUS_OK), required == UInt64(count) else {
                throw EVCoreFrontendError.core(operation: "Parse command-line arguments", status: status)
            }
            let result = try JSONDecoder().decode(ParseResult.self, from: Data(output))
            if let message = result.error { throw EVLaunchArgumentError(message) }
            guard let parsed = result.arguments else {
                throw EVLaunchArgumentError("The command-line parser returned no arguments.")
            }
            return parsed
        }
    }
}

extension EVEditorSurfaceController {
    public func goToLine(_ line: UInt64) {
        performInput {
            try attachToCore()
            guard let session else { return }
            // Normal G already owns grapheme-safe, clamped logical line
            // navigation. Startup is a fresh Normal view, with no pending input.
            let sequence = line == UInt64.max ? "G" : "\(max(1, min(line, UInt64(Int.max))))G"
            _ = try session.sendText(sequence)
        }
    }
}
