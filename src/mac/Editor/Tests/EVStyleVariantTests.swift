import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVStyleVariantTests: XCTestCase {

    private func descendants(_ view: NSView) -> [NSView] { view.subviews.flatMap { [$0] + descendants($0) } }

}
