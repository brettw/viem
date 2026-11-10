import AppKit
import XCTest
@testable import ViemAppShell

final class EVStatusBarStateTests: XCTestCase {
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

    @MainActor
    func testDiffBaseSegmentShowsItsWholeTitleFileAndPosition() throws {
        let bar = EVStatusBarView(frame: NSRect(x: 0, y: 0, width: 800, height: EVStatusBarView.preferredHeight))
        bar.diffBase = .init(title: "BASE @ HEAD", fileURL: URL(fileURLWithPath: "/tmp/old.txt"),
                             location: "Ln 23, Col 14", width: 400)
        bar.apply(EVStatusBarState(mode: "NORMAL", location: "Ln 1, Col 1"))
        bar.layoutSubtreeIfNeeded()
        func descendants(_ view: NSView) -> [NSView] { view.subviews.flatMap { [$0] + descendants($0) } }
        let labels = descendants(bar).compactMap { $0 as? NSTextField }.filter { !$0.isHidden }
        for text in ["BASE @ HEAD", "/tmp/old.txt", "Ln 23, Col 14"] {
            let label = try XCTUnwrap(labels.first { $0.stringValue == text }, text)
            XCTAssertGreaterThanOrEqual(label.frame.width, ceil(label.intrinsicContentSize.width), "\(text) is not clipped")
            XCTAssertLessThanOrEqual(label.frame.maxX, 400, "\(text) stays over the base side")
        }
    }
}
