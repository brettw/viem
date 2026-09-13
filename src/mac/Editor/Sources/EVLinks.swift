import AppKit
import CViemCore

/// A menu retains an exact source revision, never a destination or a naked
/// offset that could be interpreted in a different document after an edit.
struct EVLinkMenuTarget {
    let documentID: UInt64
    let revision: UInt64
    let offset: UInt64
}

extension EVCoreDocumentBackend {
    func linkDestination(at target: EVLinkMenuTarget) throws -> String? {
        var required: UInt64 = 0
        var found: UInt8 = 0
        let query = viem_core_copy_link_destination(core, target.documentID, target.revision,
            target.offset, nil, 0, &required, &found)
        guard query == 0 || query == 9 else {
            throw EVCoreFrontendError.core(operation: "Read link", status: query)
        }
        guard found != 0 else { return nil }
        guard let count = Int(exactly: required) else { throw EVCoreFrontendError.invalidUTF8 }
        var bytes = [UInt8](repeating: 0, count: count)
        let status = bytes.withUnsafeMutableBufferPointer {
            viem_core_copy_link_destination(core, target.documentID, target.revision,
                target.offset, $0.baseAddress, UInt64($0.count), &required, &found)
        }
        guard status == 0 else { throw EVCoreFrontendError.core(operation: "Read link", status: status) }
        guard let text = String(bytes: bytes, encoding: .utf8) else { throw EVCoreFrontendError.invalidUTF8 }
        return found == 0 ? nil : text
    }
}

enum EVLinkError: LocalizedError {
    case unsupportedDestination
    case browserUnavailable
    var errorDescription: String? {
        switch self {
        case .unsupportedDestination: "This link does not have a supported web or file destination."
        case .browserUnavailable: "The default web browser could not be opened."
        }
    }
}

@MainActor
enum EVLinkOpener {
    private static func escapedURL(_ text: String) -> String {
        // Encode only bytes that cannot appear literally in a URI. In
        // particular, preserve existing %HH escapes even when a different
        // character (a space or backtick, for example) needs escaping.
        let literal = Set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~:/?#[]@!$&'()*+,;=".utf8)
        let bytes = Array(text.utf8)
        let hex = Array("0123456789ABCDEF".utf8)
        func isHex(_ byte: UInt8) -> Bool {
            (48...57).contains(byte) || (65...70).contains(byte) || (97...102).contains(byte)
        }
        var result: [UInt8] = []
        var at = 0
        while at < bytes.count {
            let byte = bytes[at]
            if byte == 37, at + 2 < bytes.count, isHex(bytes[at + 1]), isHex(bytes[at + 2]) {
                result.append(contentsOf: bytes[at...at + 2]); at += 3
            } else {
                if literal.contains(byte) { result.append(byte) }
                else { result.append(contentsOf: [37, hex[Int(byte >> 4)], hex[Int(byte & 15)]]) }
                at += 1
            }
        }
        return String(decoding: result, as: UTF8.self)
    }

    static func destinationURL(_ destination: String, relativeTo base: URL?) throws -> URL {
        guard !destination.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
              let url = URL(string: escapedURL(destination), relativeTo: base)?.absoluteURL,
              let scheme = url.scheme?.lowercased(),
              ["https", "http", "file"].contains(scheme),
              scheme == "file" || url.host?.isEmpty == false
        else { throw EVLinkError.unsupportedDestination }
        return url
    }

    static func open(_ url: URL, completion: @escaping @MainActor (Error?) -> Void) {
        // URLs are passed as structured values. No shell, command string,
        // subprocess interpolation, or additional percent-decoding is involved.
        guard let browser = NSWorkspace.shared.urlForApplication(toOpen: URL(string: "https://example.com")!) else {
            completion(EVLinkError.browserUnavailable)
            return
        }
        NSWorkspace.shared.open([url], withApplicationAt: browser,
                                configuration: NSWorkspace.OpenConfiguration()) { _, error in
            Task { @MainActor in completion(error) }
        }
    }
}
