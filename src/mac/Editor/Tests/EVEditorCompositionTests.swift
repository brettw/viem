import XCTest
@testable import EvimEditor

final class EVEditorCompositionTests: XCTestCase {
    @MainActor
    func testFrontendCompositionInstalls() {
        EVEditorComposition.install()
    }
}
