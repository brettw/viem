import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVMarkdownSwitchIntegrationTests: XCTestCase {
    func testLargeMarkdownModeSwitchPublishesOneViewportAndPreservesSource() throws {
        let fixture = (0..<1_000).map { index in
            """
            ## Section \(index)

            Human-language **writing** with *emphasis* and `code`, including café.
            This source continuation flows into the same paragraph.

            - First item with **strong words**.
              A continuation of the item.
            - Second item.

            ```text
                first line

                last line
            ```

            """
        }.joined(separator: "\n")
        // An optional real document makes the same end-to-end path useful for
        // profiling without making the regression depend on repository prose.
        let source: Data
        if let path = ProcessInfo.processInfo.environment["VIEM_PROFILE_MARKDOWN_PATH"] {
            source = try Data(contentsOf: URL(fileURLWithPath: path))
        } else {
            source = Data(fixture.utf8)
        }
        let backend = EVCoreDocumentBackend()
        try backend.read(source: source, typeName: EVDocument.markdownSourceType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        _ = try session.resize(width: 900, height: 600)
        surface.refreshPresentation()

        for format: EVSourceFormat in [.markdown, .markdownSource, .markdown, .markdownSource] {
            backend.resetFormattedAccessCounters()
            let refreshes = surface.presentationRefreshCount
            let start = ContinuousClock.now
            surface.perform(statusOption: .format(format))
            let elapsed = start.duration(to: .now)
            print("Markdown switch \(source.count) bytes → \(format.displayName): \(elapsed)")

            XCTAssertEqual(backend.sourceFormat, format)
            XCTAssertNil(surface.commandOutput)
            XCTAssertEqual(surface.presentationRefreshCount, refreshes + 1)
            XCTAssertEqual(backend.formattedAccessCounters.fullRangeReadCalls, 0)
            XCTAssertLessThan(backend.formattedAccessCounters.requestedUTF8Bytes, UInt64(source.count / 4))
            XCTAssertFalse(try session.layoutExport().rows.isEmpty)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
        }
    }
}
