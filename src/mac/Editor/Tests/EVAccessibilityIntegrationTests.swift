import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

final class EVAccessibilityIntegrationTests: XCTestCase {
    @MainActor
    func testCoreTextAndUnicodeRangesAreExposedInUTF16Coordinates() throws {
        let text = "A😀e\u{301}\n漢字"
        let (surface, _, window) = try makeSurface(text: text)
        let view = surface.editorView

        XCTAssertEqual(view.accessibilityValue() as? String, text)
        XCTAssertEqual(view.accessibilityNumberOfCharacters(), 8)
        XCTAssertEqual(view.accessibilitySelectedText(), "")
        XCTAssertEqual(view.accessibilitySelectedTextRange(), NSRange(location: 0, length: 0))
        XCTAssertEqual(view.accessibilityInsertionPointLineNumber(), 0)

        XCTAssertEqual(view.accessibilityRange(for: 1), NSRange(location: 1, length: 2))
        XCTAssertEqual(view.accessibilityRange(for: 2), NSRange(location: 1, length: 2))
        XCTAssertEqual(view.accessibilityRange(for: 3), NSRange(location: 3, length: 2))
        XCTAssertEqual(view.accessibilityRange(for: 4), NSRange(location: 3, length: 2))
        XCTAssertEqual(
            view.accessibilityString(for: NSRange(location: 1, length: 4)),
            "😀e\u{301}"
        )
        XCTAssertNil(
            view.accessibilityString(for: NSRange(location: 2, length: 0)),
            "a UTF-16 offset inside a surrogate pair is not a logical boundary"
        )
        view.setAccessibilitySelectedTextRange(NSRange(location: 2, length: 0))
        XCTAssertEqual(
            view.accessibilitySelectedTextRange(),
            NSRange(location: 0, length: 0),
            "an invalid UTF-16 boundary must not become core selection state"
        )

        XCTAssertEqual(view.accessibilityRange(forLine: 0), NSRange(location: 0, length: 6))
        XCTAssertEqual(view.accessibilityRange(forLine: 1), NSRange(location: 6, length: 2))
        XCTAssertEqual(view.accessibilityLine(for: 6), 1)
        XCTAssertEqual(view.accessibilityVisibleCharacterRange(), NSRange(location: 0, length: 8))

        let attributed = try XCTUnwrap(
            view.accessibilityAttributedString(for: NSRange(location: 0, length: 8))
        )
        XCTAssertEqual(attributed.string, text)
        let font = try XCTUnwrap(
            attributed.attribute(.accessibilityFont, at: 0, effectiveRange: nil)
                as? [NSAccessibility.FontAttributeKey: Any]
        )
        XCTAssertEqual(
            try XCTUnwrap((font[.fontSize] as? NSNumber)?.doubleValue),
            14,
            accuracy: 0.01
        )
        XCTAssertEqual(font[.fontFamily] as? String, "SF Pro")
        XCTAssertNotNil(
            attributed.attribute(.accessibilityForegroundColor, at: 0, effectiveRange: nil)
        )
        withExtendedLifetime(window) {}
    }

    @MainActor
    func testVisualLineAndBlockSelectionsExposeExactLogicalSegments() throws {
        let (surface, session, window) = try makeSurface(text: "ab\ncd")
        let view = surface.editorView

        surface.performInput { _ = try session.sendText("V") }
        XCTAssertEqual(view.accessibilitySelectedText(), "ab\n")
        XCTAssertEqual(view.accessibilitySelectedTextRange(), NSRange(location: 0, length: 3))
        XCTAssertEqual(
            view.accessibilitySelectedTextRanges()?.map(\.rangeValue),
            [NSRange(location: 0, length: 3)]
        )

        view.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 3))
        XCTAssertEqual(surface.selectedUTF8Ranges(), [0 ..< 3])
        XCTAssertEqual(
            surface.viewPresentation.mode,
            UInt32(VIEM_MODE_VISUAL_LINE),
            "an unrepresentable hard-break endpoint must not replace an exact core selection"
        )

        surface.performInput {
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.sendKey(
                kind: UInt32(VIEM_KEY_CONTROL_CHARACTER),
                codepoint: UInt32(Character("v").asciiValue!)
            )
        }
        surface.performInput {
            _ = try session.sendKey(
                kind: UInt32(VIEM_KEY_CHARACTER),
                codepoint: UInt32(Character("l").asciiValue!)
            )
            _ = try session.sendKey(
                kind: UInt32(VIEM_KEY_CHARACTER),
                codepoint: UInt32(Character("j").asciiValue!)
            )
        }

        XCTAssertEqual(view.accessibilitySelectedText(), "ab\ncd")
        XCTAssertEqual(
            view.accessibilitySelectedTextRanges()?.map(\.rangeValue),
            [
                NSRange(location: 0, length: 2),
                NSRange(location: 3, length: 2),
            ]
        )
        XCTAssertEqual(
            view.accessibilitySelectedTextRange(),
            NSRange(location: 0, length: 2),
            "the singular AppKit attribute must not invent a destructive bounding range"
        )
        view.setAccessibilitySelectedText("X")
        XCTAssertEqual(try surface.backend.formattedText(), "ab\ncd")
        XCTAssertEqual(view.accessibilitySelectedText(), "ab\ncd")
        withExtendedLifetime(window) {}
    }

    @MainActor
    func testRangeAndPointGeometryUseExactLayoutAndSelectionExports() throws {
        let (surface, _, window) = try makeSurface(text: "A😀B")
        let view = surface.editorView
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let emoji = try XCTUnwrap(
            snapshot.clusters.first { $0.text_start == 1 && $0.text_end == 5 }
        )
        let emojiRange = NSRange(location: 1, length: 2)
        let expectedClusterFrame = window.convertToScreen(
            view.convert(view.viewRect(emoji.typographic_bounds), to: nil)
        )

        assertRect(view.accessibilityFrame(for: emojiRange), equals: expectedClusterFrame)
        let queried = view.accessibilityRange(
            for: NSPoint(x: expectedClusterFrame.midX, y: expectedClusterFrame.midY)
        )
        XCTAssertEqual(queried, emojiRange)

        view.setAccessibilitySelectedTextRange(emojiRange)
        let selection = try XCTUnwrap(surface.visualSelection)
        XCTAssertEqual(surface.selectedUTF8Ranges(), [1 ..< 5])
        let expectedSelectionFrame = try XCTUnwrap(
            selection.rectangles
                .map { view.viewRect($0.rect) }
                .reduce(nil as NSRect?) { partial, rectangle in
                    partial.map { $0.union(rectangle) } ?? rectangle
                }
        )
        assertRect(
            view.accessibilityFrame(for: emojiRange),
            equals: window.convertToScreen(view.convert(expectedSelectionFrame, to: nil))
        )
        withExtendedLifetime(window) {}
    }

    @MainActor
    func testAccessibilitySelectionAndEditsRouteThroughCoreIntentions() throws {
        let (surface, session, window) = try makeSurface(text: "A😀B")
        let view = surface.editorView

        view.setAccessibilitySelectedTextRange(NSRange(location: 1, length: 2))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_SELECTION_CHARACTER))
        XCTAssertEqual(surface.selectedUTF8Ranges(), [1 ..< 5])
        XCTAssertEqual(view.accessibilitySelectedText(), "😀")
        XCTAssertFalse(view.hasMarkedText())

        view.setAccessibilitySelectedText("é")
        XCTAssertEqual(try surface.backend.formattedText(), "AéB")
        XCTAssertFalse(view.hasMarkedText())
        _ = try session.undo()
        XCTAssertEqual(try surface.backend.formattedText(), "A😀B")

        view.setAccessibilityValue("whole 👩🏽‍💻")
        XCTAssertEqual(try surface.backend.formattedText(), "whole 👩🏽‍💻")
        XCTAssertFalse(view.hasMarkedText())
        withExtendedLifetime(window) {}
    }

    @MainActor
    func testStaleAndOutsideCoverageSelectionAndGeometryRequestsAreRejected() throws {
        let text = (0..<2_000).map { "line \($0)" }.joined(separator: "\n")
        let (surface, session, window) = try makeSurface(
            text: text,
            size: NSSize(width: 280, height: 110)
        )
        let view = surface.editorView
        let initial = try session.presentation()
        let finalCharacter = NSRange(location: text.utf16.count - 1, length: 1)

        XCTAssertEqual(view.accessibilityString(for: finalCharacter), "9")
        XCTAssertEqual(view.accessibilityFrame(for: finalCharacter), .zero)
        view.setAccessibilitySelectedTextRange(finalCharacter)
        let afterOutsideRequest = try session.presentation()
        XCTAssertEqual(afterOutsideRequest.cursor_utf8_offset, initial.cursor_utf8_offset)
        XCTAssertEqual(afterOutsideRequest.mode, initial.mode)

        let firstCharacter = NSRange(location: 0, length: 1)
        XCTAssertNotEqual(view.accessibilityFrame(for: firstCharacter), .zero)
        _ = try session.setWrap(false)
        XCTAssertEqual(
            view.accessibilityFrame(for: firstCharacter),
            .zero,
            "a cached layout is rejected after its live identity changes"
        )
        view.setAccessibilitySelectedTextRange(NSRange(location: 1, length: 0))
        let afterStaleRequest = try session.presentation()
        XCTAssertEqual(afterStaleRequest.cursor_utf8_offset, initial.cursor_utf8_offset)
        XCTAssertEqual(afterStaleRequest.mode, initial.mode)
        withExtendedLifetime(window) {}
    }

    @MainActor
    private func makeSurface(
        text: String,
        size: NSSize = NSSize(width: 520, height: 260)
    ) throws -> (EVEditorSurfaceController, EVCoreViewSession, NSWindow) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-accessibility-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        try EVStyleTestFixtures.configure(configuration, format: .plainText, declarations: [
            (.baseParagraph, .characterSize, .float(14)),
            (.baseParagraph, .characterFontFamilies, .stringList(["system-ui"])),
            (.baseParagraph, .characterFontAxes, .string("{}")),
            (.baseParagraph, .characterWeight, .unsigned(400)),
        ])
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(text.utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(origin: .zero, size: size)
        surface.viewDidLayout()
        let window = NSWindow(
            contentRect: surface.view.bounds,
            styleMask: .borderless,
            backing: .buffered,
            defer: false
        )
        window.contentView = surface.view
        return (surface, try XCTUnwrap(surface.session), window)
    }

    private func assertRect(
        _ actual: NSRect,
        equals expected: NSRect,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertEqual(actual.minX, expected.minX, accuracy: 0.01, file: file, line: line)
        XCTAssertEqual(actual.minY, expected.minY, accuracy: 0.01, file: file, line: line)
        XCTAssertEqual(actual.width, expected.width, accuracy: 0.01, file: file, line: line)
        XCTAssertEqual(actual.height, expected.height, accuracy: 0.01, file: file, line: line)
    }
}
