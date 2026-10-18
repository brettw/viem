import AppKit
import XCTest
@testable import ViemEditor

@MainActor
final class EVImageSizeControlsTests: XCTestCase {
    func testFirstExpansionUsesNaturalPixelsAndLockedEditsFollowEitherAxis() {
        let controls = EVImageSizeControls()
        controls.setNaturalSize(NSSize(width: 600, height: 400))
        XCTAssertFalse(controls.expanded)
        controls.disclosure.performClick(nil)
        XCTAssertEqual(controls.widthField.stringValue, "600")
        XCTAssertEqual(controls.heightField.stringValue, "400")
        XCTAssertEqual(controls.lockButton.state, .on)
        edit(controls.widthField, "300", in: controls)
        XCTAssertEqual(controls.heightField.stringValue, "200")
        edit(controls.heightField, "100", in: controls)
        XCTAssertEqual(controls.widthField.stringValue, "150")
        XCTAssertEqual(controls.dimensions.width, 150)
        XCTAssertEqual(controls.dimensions.height, 100)
        controls.useImageSizeButton.performClick(nil)
        XCTAssertEqual(controls.widthField.stringValue, "600")
        XCTAssertEqual(controls.heightField.stringValue, "400")
        controls.disclosure.performClick(nil)
        XCTAssertEqual(controls.dimensions.width, 600)
        XCTAssertEqual(controls.dimensions.height, 400)
    }

    func testRelockingPreservesLastEditedAxisAndResetsBetweenShowings() {
        let controls = EVImageSizeControls()
        controls.reset(width: 300, height: 100)
        controls.setNaturalSize(NSSize(width: 600, height: 400))
        XCTAssertTrue(controls.expanded)
        XCTAssertEqual(controls.lockButton.state, .off)
        // No field edits: preserve width.
        controls.lockButton.performClick(nil)
        XCTAssertEqual(controls.heightField.stringValue, "200")
        controls.lockButton.performClick(nil)
        edit(controls.heightField, "80", in: controls)
        controls.lockButton.performClick(nil)
        XCTAssertEqual(controls.widthField.stringValue, "120")
        controls.lockButton.performClick(nil)
        edit(controls.widthField, "75", in: controls)
        controls.lockButton.performClick(nil)
        XCTAssertEqual(controls.heightField.stringValue, "50")
        controls.reset(width: 300, height: 100)
        controls.setNaturalSize(NSSize(width: 600, height: 400))
        controls.lockButton.performClick(nil)
        XCTAssertEqual(controls.widthField.stringValue, "300")
        XCTAssertEqual(controls.heightField.stringValue, "200")
    }

    func testExistingLockUsesEitherOnePixelToleranceAndPreservesOneAxisSource() {
        let controls = EVImageSizeControls()
        for (natural, width, height, locked) in [
            (NSSize(width: 1000, height: 100), UInt32(95), UInt32(10), true),
            (NSSize(width: 100, height: 1000), 10, 95, true),
            (NSSize(width: 100, height: 1000), 10, 80, false),
        ] {
            controls.reset(width: width, height: height)
            controls.setNaturalSize(natural)
            XCTAssertEqual(controls.lockButton.state, locked ? .on : .off)
            XCTAssertEqual(controls.dimensions.width, width)
            XCTAssertEqual(controls.dimensions.height, height)
        }
        controls.reset(width: 200, height: nil)
        controls.setNaturalSize(NSSize(width: 600, height: 400))
        XCTAssertEqual(controls.heightField.stringValue, "133")
        XCTAssertEqual(controls.lockButton.state, .on)
        XCTAssertEqual(controls.dimensions.width, 200)
        XCTAssertNil(controls.dimensions.height)
    }

    func testUnavailableDimensionsAllowManualSizingAndRejectInvalidValues() {
        let controls = EVImageSizeControls()
        controls.disclosure.performClick(nil)
        XCTAssertFalse(controls.lockButton.isEnabled)
        XCTAssertFalse(controls.useImageSizeButton.isEnabled)
        XCTAssertTrue(controls.isValid, "Empty dimensions retain intrinsic sizing")
        edit(controls.widthField, "200", in: controls)
        XCTAssertTrue(controls.isValid)
        for value in ["0", "-1", "1.5", "4294967296", "abc"] {
            edit(controls.heightField, value, in: controls)
            XCTAssertFalse(controls.isValid, value)
        }
        edit(controls.heightField, "100", in: controls)
        controls.setNaturalSize(NSSize(width: 400, height: 200))
        XCTAssertEqual(controls.widthField.stringValue, "200")
        XCTAssertEqual(controls.heightField.stringValue, "100")
    }

    func testFoldingPreservesAuthoredSizeAndEmptyFieldsRemoveIt() {
        let controls = EVImageSizeControls()
        controls.reset(width: 120, height: nil)
        controls.setNaturalSize(NSSize(width: 400, height: 200))
        controls.disclosure.performClick(nil)
        XCTAssertEqual(controls.dimensions.width, 120)
        XCTAssertNil(controls.dimensions.height)
        controls.disclosure.performClick(nil)
        edit(controls.widthField, "", in: controls)
        edit(controls.heightField, "", in: controls)
        XCTAssertTrue(controls.isValid)
        XCTAssertNil(controls.dimensions.width)
        XCTAssertNil(controls.dimensions.height)
    }

    func testProportionalOverflowStaysInvalidInsteadOfChangingAspectRatio() {
        let controls = EVImageSizeControls()
        controls.setNaturalSize(NSSize(width: 1, height: 5000))
        controls.disclosure.performClick(nil)
        edit(controls.widthField, "4294967295", in: controls)
        XCTAssertEqual(controls.heightField.stringValue, "21474836475000")
        XCTAssertFalse(controls.isValid)
    }

    private func edit(_ field: NSTextField, _ text: String, in controls: EVImageSizeControls) {
        field.stringValue = text
        controls.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: field))
    }
}
