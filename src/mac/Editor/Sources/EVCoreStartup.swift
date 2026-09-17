import Foundation
import CViemCore
import ViemAppShell

/// Native file access is separate from the portable command parser. The
/// callback copies each borrowed diagnostic while the startup call is active.
enum EVCoreStartup {
    private final class Diagnostics {
        var messages: [String] = []
        let path: String
        init(path: String) { self.path = path }
    }

    static func initialize(core: ViemCoreHandle, file: EVStartupFile) -> [String] {
        let diagnostics = Diagnostics(path: file.url.path)
        diagnostics.messages = file.diagnostics
        let bytes = Array(file.text.utf8)
        let status = bytes.withUnsafeBufferPointer { input in
            viem_core_initialize_startup(core, input.baseAddress, UInt64(input.count), { context, line, message, length in
                guard let context, let message else { return }
                let result = Unmanaged<Diagnostics>.fromOpaque(context).takeUnretainedValue()
                let text = String(decoding: UnsafeBufferPointer(start: message, count: Int(length)), as: UTF8.self)
                result.messages.append("\(result.path):\(line): \(text)")
            }, Unmanaged.passUnretained(diagnostics).toOpaque())
        }
        if status != UInt32(VIEM_STATUS_OK) {
            diagnostics.messages.append("\(file.url.path): Unable to load startup commands (core status \(status)).")
        }
        return diagnostics.messages
    }
}
