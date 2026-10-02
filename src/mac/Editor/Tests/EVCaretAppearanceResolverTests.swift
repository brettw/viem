import AppKit
import XCTest
@testable import ViemEditor

@MainActor
final class EVCaretAppearanceResolverTests: XCTestCase {
    func testSystemAndAccessibilityChangesAdvanceGenerationAndPublish() {
        let center = NotificationCenter()
        let workspaceCenter = NotificationCenter()
        let resolver = EVCaretAppearanceResolver(
            notificationCenter: center,
            workspaceNotificationCenter: workspaceCenter
        )
        var publications = 0
        let observer = center.addObserver(
            forName: .viemCaretAppearanceDidChange,
            object: resolver,
            queue: nil
        ) { _ in publications += 1 }
        defer { center.removeObserver(observer) }

        let initial = resolver.generation
        center.post(name: NSColor.systemColorsDidChangeNotification, object: nil)
        XCTAssertEqual(resolver.generation, initial + 1)
        workspaceCenter.post(
            name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification,
            object: nil
        )
        XCTAssertEqual(resolver.generation, initial + 2)
        resolver.noteEffectiveAppearanceChange()
        XCTAssertEqual(resolver.generation, initial + 3)
        XCTAssertEqual(publications, 3)
    }

    func testBlockGlyphColorChoosesTheHigherWCAGContrastForRepresentativeColors() throws {
        let fixtures: [(name: String, background: NSColor, expected: NSColor)] = [
            (
                "red",
                NSColor(srgbRed: 1, green: 0, blue: 0, alpha: 1),
                .black
            ),
            (
                "mid-gray",
                NSColor(srgbRed: 0.5, green: 0.5, blue: 0.5, alpha: 1),
                .black
            ),
            (
                "dark",
                NSColor(srgbRed: 0.1, green: 0.1, blue: 0.1, alpha: 1),
                .white
            ),
            (
                "light",
                NSColor(srgbRed: 0.9, green: 0.9, blue: 0.9, alpha: 1),
                .black
            ),
        ]

        for fixture in fixtures {
            let actual = try XCTUnwrap(
                EVCaretAppearanceResolver.glyphColor(contrastingWith: fixture.background)
                    .usingColorSpace(.sRGB)
            )
            let expected = try XCTUnwrap(fixture.expected.usingColorSpace(.sRGB))
            XCTAssertEqual(
                actual.redComponent,
                expected.redComponent,
                accuracy: 0.0001,
                fixture.name
            )
            XCTAssertEqual(
                actual.greenComponent,
                expected.greenComponent,
                accuracy: 0.0001,
                fixture.name
            )
            XCTAssertEqual(
                actual.blueComponent,
                expected.blueComponent,
                accuracy: 0.0001,
                fixture.name
            )
        }
    }

}
