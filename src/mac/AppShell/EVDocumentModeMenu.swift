import AppKit
import CViemCore

public struct EVCodeLanguage: Decodable, Equatable, Sendable {
    public let id: String
    public let name: String
    public static let all: [EVCodeLanguage] = (try? EVDocumentModeJSON.read {
        viem_copy_code_languages_json($0, $1, $2)
    }) ?? []
}

public struct EVDocumentModeState: Decodable, Sendable {
    public let documentId: UInt64
    public let documentRevision: UInt64
    public let format: String
    public let automatic: Bool
    public let language: String?
    public let detectedLanguage: String?
    public let detectedName: String
}

public enum EVDocumentModeChoice: Equatable, Sendable {
    case automatic, plainText, markdown, code(String)
    public var code: UInt32 {
        switch self {
        case .automatic: UInt32(VIEM_DOCUMENT_MODE_AUTO)
        case .plainText: UInt32(VIEM_DOCUMENT_MODE_PLAIN_TEXT)
        case .markdown: UInt32(VIEM_DOCUMENT_MODE_MARKDOWN)
        case .code: UInt32(VIEM_DOCUMENT_MODE_CODE)
        }
    }
    public var language: String { if case let .code(id) = self { id } else { "" } }
}

@MainActor
public protocol EVDocumentModeMenuProviding: AnyObject {
    func currentDocumentMode() -> EVDocumentModeState?
    func selectDocumentMode(_ choice: EVDocumentModeChoice, expected: EVDocumentModeState)
}

public enum EVDocumentModeJSON {
    public static func read<T: Decodable>(_ copy: (UnsafeMutablePointer<UInt8>?, UInt64, UnsafeMutablePointer<UInt64>) -> UInt32) throws -> T {
        var required: UInt64 = 0
        let status = copy(nil, 0, &required)
        guard (status == VIEM_STATUS_OK || status == VIEM_STATUS_BUFFER_TOO_SMALL), required <= 1024 * 1024 else {
            throw CocoaError(.coderReadCorrupt)
        }
        var bytes = [UInt8](repeating: 0, count: Int(required))
        let copied = bytes.withUnsafeMutableBufferPointer { copy($0.baseAddress, UInt64($0.count), &required) }
        guard copied == VIEM_STATUS_OK else { throw CocoaError(.coderReadCorrupt) }
        return try JSONDecoder().decode(T.self, from: Data(bytes))
    }
}

final class EVDocumentModeMenuAction: NSObject {
    let choice: EVDocumentModeChoice
    var expected: EVDocumentModeState?
    init(_ choice: EVDocumentModeChoice) { self.choice = choice }
}
