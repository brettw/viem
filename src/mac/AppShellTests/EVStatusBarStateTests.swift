import AppKit
import XCTest
@testable import ViemAppShell

final class EVStatusBarStateTests: XCTestCase {
    func testNewDocumentStatusDefaults() {
        let state = EVStatusBarState()
        XCTAssertEqual(state.mode, "NORMAL")
        XCTAssertEqual(state.location, "Ln 1, Col 1")
        XCTAssertEqual(state.format, "Plain Text")
    }

    @MainActor
    func testStatusBarRetainsOnlyFormatPickerAndLineModeWithStatusText() throws {
        let bar = EVStatusBarView(frame: .zero)
        bar.apply(EVStatusBarState(mode: "INSERT", message: "Saved", location: "Ln 4, Col 2", format: "Markdown WYSIWYG"))
        func descendants(_ view: NSView) -> [NSView] {
            view.subviews.flatMap { [$0] + descendants($0) }
        }
        let controls = descendants(bar)
        let pickers = controls.compactMap { $0 as? NSPopUpButton }
        XCTAssertEqual(pickers.count, 1)
        let format = try XCTUnwrap(pickers.first)
        XCTAssertEqual(format.title, "Markdown WYSIWYG")
        XCTAssertEqual(format.itemTitles, ["Markdown Source", "Markdown WYSIWYG"])
        let labels = controls.compactMap { ($0 as? NSTextField)?.stringValue }
        XCTAssertTrue(labels.contains("INSERT"))
        XCTAssertTrue(labels.contains("Saved"))
        let location = try XCTUnwrap(controls.compactMap { $0 as? NSButton }.first { $0.title == "Ln 4, Col 2" })
        var options: [EVStatusBarOption] = []
        bar.optionDidChange = { options.append($0) }
        location.performClick(nil)
        format.selectItem(withTitle: "Markdown Source")
        _ = format.sendAction(format.action, to: format.target)
        XCTAssertEqual(options, [.lineMode(.physicalSource), .format(.markdownSource)])
        for name in ["Plain Text", "Code"] {
            bar.apply(EVStatusBarState(format: name))
            XCTAssertTrue(format.isHidden)
            XCTAssertTrue(descendants(bar).compactMap { $0 as? NSTextField }
                .contains { !$0.isHidden && $0.stringValue == name })
        }
    }
}
