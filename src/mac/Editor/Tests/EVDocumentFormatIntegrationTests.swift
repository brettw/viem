import CViemCore
import ViemAppShell
import Foundation
import Testing
@testable import ViemEditor

@MainActor
struct EVDocumentFormatIntegrationTests {
    @Test
    func plainTextOpensAndRoundTripsEveryInitialEncoding() throws {
        let fixtures: [(Data, String, String)] = [
            (Data("café\r\nnext".utf8), "café\nnext", "UTF-8"),
            (Data([0x63, 0x61, 0x66, 0xe9, 0x0d, 0x0a, 0x6e, 0x65, 0x78, 0x74]), "café\nnext", "Latin-1"),
            (utf16Data("café\r\nnext", littleEndian: true), "café\nnext", "UTF-16 LE"),
            (utf16Data("café\r\nnext", littleEndian: false), "café\nnext", "UTF-16 BE"),
        ]

        for (source, formatted, encodingLabel) in fixtures {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: source, typeName: "public.plain-text")

            #expect(try backend.formattedText() == formatted)
            #expect(try backend.serializedSource(typeName: "public.plain-text") == source)
            #expect(backend.encodingLabel == encodingLabel)
            #expect(backend.formatLabel == "Plain Text")
            #expect(backend.lineEndingLabel == "CRLF")
        }
    }

    @Test
    func markdownUsesCoreProjectionAndPreservesPhysicalSource() throws {
        let source = Data("# café\r\nThis is **bold**.\r\n".utf8)
        let backend = EVCoreDocumentBackend()

        try backend.read(source: source, typeName: "net.daringfireball.markdown")

        #expect(try backend.formattedText() == "café\nThis is bold.")
        #expect(try backend.serializedSource(typeName: "net.daringfireball.markdown") == source)
        #expect(backend.encodingLabel == "UTF-8")
        #expect(backend.formatLabel == "Markdown WYSIWYG")
        #expect(backend.lineEndingLabel == "CRLF")
    }

    @Test
    func malformedUTF8AfterBOMUsesCoreBOMFirstDetectionWithoutLosingBytes() throws {
        let source = Data([0xEF, 0xBB, 0xBF, 0xFF])
        let backend = EVCoreDocumentBackend()

        try backend.read(source: source, typeName: "public.plain-text")

        #expect(backend.encodingLabel == "UTF-8")
        #expect(
            backend.currentDocumentState.flags & UInt32(VIEM_DOCUMENT_STATE_HAS_BOM) != 0
        )
        #expect(try backend.formattedText() == "\u{FFFD}")
        #expect(try backend.serializedSource(typeName: "public.plain-text") == source)
    }

    @Test
    func backendRejectsCrossFormatSerializationAndKeepsSameFormatSnapshotsExact() throws {
        let fixtures: [(Data, String, String, EVSourceFormat, EVSourceFormat)] = [
            (
                Data("plain source\r\n".utf8),
                EVDocument.plainTextType,
                EVDocument.markdownType,
                .plainText,
                .markdown
            ),
            (
                Data("# Markdown source\r\n".utf8),
                EVDocument.markdownType,
                EVDocument.plainTextType,
                .markdown,
                .plainText
            ),
        ]

        for (source, sourceType, requestedType, sourceFormat, requestedFormat) in fixtures {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: source, typeName: sourceType)
            let revision = backend.currentDocumentState.document_revision
            let expectedError = EVDocumentSerializationError.formatConversionUnavailable(
                current: sourceFormat,
                requested: requestedFormat
            )

            #expect(backend.sourceFormat == sourceFormat)
            do {
                _ = try backend.serializedSource(typeName: requestedType)
                Issue.record("Cross-format serialization unexpectedly succeeded")
            } catch {
                #expect((error as? EVDocumentSerializationError) == expectedError)
            }
            do {
                _ = try backend.nativeSaveSnapshot(typeName: requestedType)
                Issue.record("Cross-format native snapshot unexpectedly succeeded")
            } catch {
                #expect((error as? EVDocumentSerializationError) == expectedError)
            }

            #expect(backend.currentDocumentState.document_revision == revision)
            #expect(try backend.serializedSource(typeName: sourceType) == source)
            let snapshot = try backend.nativeSaveSnapshot(typeName: sourceType)
            #expect(snapshot.data == source)
            #expect(snapshot.documentRevision == revision)
        }
    }

    private func utf16Data(_ string: String, littleEndian: Bool) -> Data {
        var bytes: [UInt8] = littleEndian ? [0xff, 0xfe] : [0xfe, 0xff]
        for codeUnit in string.utf16 {
            if littleEndian {
                bytes.append(UInt8(truncatingIfNeeded: codeUnit))
                bytes.append(UInt8(truncatingIfNeeded: codeUnit >> 8))
            } else {
                bytes.append(UInt8(truncatingIfNeeded: codeUnit >> 8))
                bytes.append(UInt8(truncatingIfNeeded: codeUnit))
            }
        }
        return Data(bytes)
    }
}
