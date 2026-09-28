import AppKit
import XCTest
@testable import ViemAppShell

final class EVStatusBarStateTests: XCTestCase {
    func testNewDocumentStatusDefaults() {
        let state = EVStatusBarState()
        XCTAssertEqual(state.mode, "NORMAL")
        XCTAssertEqual(state.location, "Ln 1, Col 1")
    }

    @MainActor
    func testStatusBarDisplaysModeFileMessageAndLocation() throws {
        let bar = EVStatusBarView(frame: .zero)
        func descendants(_ view: NSView) -> [NSView] {
            view.subviews.flatMap { [$0] + descendants($0) }
        }
        var options: [EVStatusBarOption] = []
        bar.optionDidChange = { options.append($0) }

        bar.fileURL = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
            .appendingPathComponent("docs/notes.md")
        bar.apply(EVStatusBarState(mode: "INSERT", message: "Saved", location: "Ln 4, Col 2"))
        let controls = descendants(bar)
        XCTAssertTrue(controls.compactMap { $0 as? NSPopUpButton }.isEmpty)
        let labels = controls.compactMap { $0 as? NSTextField }
        XCTAssertEqual(labels.map(\.stringValue), ["INSERT", bar.filePath, "Saved"])

        let location = try XCTUnwrap(descendants(bar).compactMap { $0 as? NSButton }
            .first { $0.title == "Ln 4, Col 2" })
        location.performClick(nil)
        XCTAssertEqual(options, [.lineMode(.physicalSource)])
        bar.apply(EVStatusBarState(lineMode: .physicalSource))
        location.performClick(nil)
        XCTAssertEqual(options, [.lineMode(.physicalSource), .lineMode(.visual)])
    }
}
