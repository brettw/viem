import XCTest
@testable import ViemEditor

final class EVEditorCompositionTests: XCTestCase {
    @MainActor
    func testFrontendCompositionInstalls() {
        EVEditorComposition.install()
    }
}
