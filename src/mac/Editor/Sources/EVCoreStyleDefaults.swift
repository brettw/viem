import Foundation
import CViemCore

/// Copies borrowed diagnostics during the synchronous defaults import. Native
/// presentation adds the file location; validation and recovery stay in core.
enum EVCoreStyleDefaults {
    private final class Diagnostics {
        var messages: [String] = []
        let path: String
        init(path: String) { self.path = path }
    }

    static func initialize(core: ViemCoreHandle, revision: UInt64, json: Data,
                           path: String) -> (status: UInt32, messages: [String]) {
        let diagnostics = Diagnostics(path: path)
        let status = json.withUnsafeBytes { raw in
            viem_core_initialize_style_defaults(core, revision,
                raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count), { context, message, length in
                    guard let context, let message else { return }
                    let result = Unmanaged<Diagnostics>.fromOpaque(context).takeUnretainedValue()
                    let text = String(decoding: UnsafeBufferPointer(start: message, count: Int(length)), as: UTF8.self)
                    result.messages.append("\(result.path): \(text)")
                }, Unmanaged.passUnretained(diagnostics).toOpaque())
        }
        return (status, diagnostics.messages)
    }
}
