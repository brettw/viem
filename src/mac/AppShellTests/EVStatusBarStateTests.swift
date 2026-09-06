import XCTest
@testable import EvimAppShell

final class EVStatusBarStateTests: XCTestCase {
    func testNewDocumentStatusDefaults() {
        let state = EVStatusBarState()
        XCTAssertEqual(state.mode, "NORMAL")
        XCTAssertEqual(state.location, "Ln 1, Col 1")
        XCTAssertEqual(state.encoding, "UTF-8")
        XCTAssertEqual(state.lineEnding, "LF")
        XCTAssertEqual(state.format, "Plain Text")
    }
}
