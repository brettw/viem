import CViemCore
import Foundation
import ViemAppShell

extension EVCoreViewSession {
    func setHostRegister(_ name: UInt32, text: String) throws {
        let bytes = Array(text.utf8)
        let status = bytes.withUnsafeBufferPointer {
            viem_core_view_set_host_register(document.core, viewID, name, $0.baseAddress, UInt64($0.count))
        }
        guard status == UInt32(VIEM_STATUS_OK) else { throw EVCoreFrontendError.core(operation: "Set register context", status: status) }
    }
    func jumpToFileMark(_ target: EVFileMarkTarget, linewise: Bool) throws {
        _ = try performCoreOperation("Jump to file mark") { viem_core_view_jump_file_mark(document.core, viewID, target.name, target.generation, linewise ? 1 : 0, $0) }
    }
}

extension EVEditorSurfaceController {
    public func setAlternateFile(_ path: String?) {
        alternateFilePath = path
        do { try session?.setHostRegister(35, text: path ?? "") } catch { report(error) }
    }
    public func resolveFileMark(_ name: UInt32) throws -> EVFileMarkTarget {
        var info = ViemFileMarkTargetV1()
        var status = viem_file_mark_target(name, 0, &info, nil, 0)
        guard status == UInt32(VIEM_STATUS_OK), let count = Int(exactly: info.path_length) else {
            throw EVCoreFrontendError.core(operation: "Find file mark", status: status)
        }
        var bytes = [UInt8](repeating: 0, count: count)
        let generation = info.generation
        status = bytes.withUnsafeMutableBufferPointer { viem_file_mark_target(name, generation, &info, $0.baseAddress, UInt64($0.count)) }
        guard status == UInt32(VIEM_STATUS_OK), let actualCount = Int(exactly: info.path_length), actualCount <= bytes.count,
              let path = String(bytes: bytes.prefix(actualCount), encoding: .utf8) else {
            throw EVCoreFrontendError.core(operation: "Read file mark", status: status)
        }
        return EVFileMarkTarget(name: name, generation: info.generation, documentID: info.document_id, path: path)
    }
    public func jumpToFileMark(_ target: EVFileMarkTarget, linewise: Bool) throws {
        try attachToCore()
        guard let session else { throw EVDocumentHostError.staleRequest }
        try session.jumpToFileMark(target, linewise: linewise)
        refreshPresentation()
    }
}
extension EVCoreViewSession {
    func dropText(_ text: String) throws -> ViemCoreOutcomeV1 {
        let bytes = Array(text.utf8)
        return try performHostEffectTurn("Drop text") { outcome, effects in
            withCommandTurnContext { context in
                bytes.withUnsafeBufferPointer { viem_core_view_drop_text(document.core, viewID, $0.baseAddress, UInt64($0.count), context, outcome, effects) }
            }
        }
    }
}
extension EVEditorSurfaceController {
    func acceptTextDrop(_ text: String) -> Bool {
        do {
            try attachToCore()
            guard let session else { return false }
            let result = try session.dropText(text)
            refreshPresentation()
            return result.command_status == UInt32(VIEM_COMMAND_STATUS_COMPLETE)
        } catch { report(error); return false }
    }
}
