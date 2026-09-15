import Foundation
import XCTest
@testable import ViemAppShell

@MainActor
final class EVExternalFileReviewStateTests: XCTestCase {
    func testReturnToBaselineClearsAcknowledgementWithoutReleasingPendingReview() {
        let state = EVExternalFileReviewState()
        let first = Data("first".utf8)
        let second = Data("second".utf8)
        XCTAssertNotNil(state.begin(token: first, canReload: true, isDirty: false))
        state.finish(token: first, acknowledge: true)
        state.clearAcknowledged()
        XCTAssertNotNil(state.begin(token: first, canReload: true, isDirty: false))
        state.finish(token: first, acknowledge: true)
        XCTAssertNotNil(state.begin(token: second, canReload: true, isDirty: false))
        state.clearAcknowledged()
        XCTAssertNil(state.begin(token: first, canReload: true, isDirty: false))
        state.finish(token: second, acknowledge: false)
        XCTAssertNotNil(state.begin(token: first, canReload: true, isDirty: false))
    }

    func testBridgePreservesReviewActionsAcknowledgementAndReset() throws {
        let state = EVExternalFileReviewState()
        let changed = Data([0, 255, 128, 0])
        let review = try XCTUnwrap(state.begin(token: changed, canReload: true, isDirty: true))
        XCTAssertTrue(review.canReload)
        XCTAssertTrue(review.discardsUnsavedChanges)
        state.finish(token: changed, acknowledge: true)
        XCTAssertNil(state.begin(token: changed, canReload: true, isDirty: true))

        state.reset()
        let missing = try XCTUnwrap(state.begin(token: changed, canReload: false, isDirty: true))
        XCTAssertFalse(missing.canReload)
        XCTAssertFalse(missing.discardsUnsavedChanges)
    }

    func testBridgeRetriesFailedReviewAndIgnoresOtherCompletion() throws {
        let state = EVExternalFileReviewState()
        let first = Data("first".utf8)
        let second = Data("second".utf8)
        XCTAssertNotNil(state.begin(token: first, canReload: true, isDirty: false))
        state.finish(token: second, acknowledge: true)
        XCTAssertNil(state.begin(token: second, canReload: true, isDirty: false))
        state.finish(token: first, acknowledge: false)
        let retried = try XCTUnwrap(state.begin(token: first, canReload: true, isDirty: false))
        XCTAssertTrue(retried.canReload)
        XCTAssertFalse(retried.discardsUnsavedChanges)
    }
}
