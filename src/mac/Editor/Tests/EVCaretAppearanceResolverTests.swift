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

    func testSystemCaretColorResolvesInTheViewsEffectiveAppearance() {
        let resolver = EVCaretAppearanceResolver(
            notificationCenter: NotificationCenter(),
            workspaceNotificationCenter: NotificationCenter()
        )
        let view = NSView()
        view.appearance = NSAppearance(named: .darkAqua)

        let color = resolver.color(for: view)
        XCTAssertNotNil(color.usingColorSpace(.deviceRGB))
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

    func testRedUsesBlackBecauseLinearLuminanceMakesItsContrastHigher() {
        let redLuminance = EVCaretAppearanceResolver.relativeLuminance(
            sRGBRed: 1,
            green: 0,
            blue: 0
        )
        let contrastWithBlack = EVCaretAppearanceResolver.contrastRatio(
            between: redLuminance,
            and: 0
        )
        let contrastWithWhite = EVCaretAppearanceResolver.contrastRatio(
            between: redLuminance,
            and: 1
        )

        XCTAssertEqual(redLuminance, 0.2126, accuracy: 0.0001)
        XCTAssertGreaterThan(contrastWithBlack, contrastWithWhite)
        XCTAssertGreaterThanOrEqual(contrastWithBlack, 4.5)
        XCTAssertLessThan(contrastWithWhite, 4.5)
    }
}
