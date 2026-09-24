import AppKit
import CViemCore
import XCTest

@testable import ViemEditor

final class EVTextInputClientTests: XCTestCase {
    @MainActor
    func testEscapeDiscardsAccentPickerStateWithoutMarkedText() throws {
        for route in 0..<3 {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data("base".utf8), typeName: "public.plain-text")
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            let session = try XCTUnwrap(surface.session)
            surface.performInput { _ = try session.sendText("i") }
            let view = RecordingEditorView(surface: surface)
            surface.view = view
            let window = textInputWindow(view)
            defer { window.contentView = nil }

            // A long press starts with an ordinary letter. The system can
            // retain an accent candidate without marking any client text.
            view.insertText("o", replacementRange: NSRange(location: NSNotFound, length: 0))
            XCTAssertFalse(view.hasMarkedText())
            let revision = try backend.revision()
            switch route {
            case 0: view.keyDown(with: keyEvent(keyCode: 53))
            case 1: view.doCommand(by: #selector(NSResponder.cancelOperation(_:)))
            default: view.cancelOperation(nil)
            }

            XCTAssertEqual(view.inputContextDiscardCount, 1, "route \(route)")
            XCTAssertTrue(view.inputContextHandledKeyCodes.isEmpty)
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
            XCTAssertEqual(try backend.formattedText(), "obase")
            XCTAssertEqual(try backend.revision(), revision)
            _ = try session.undo()
            XCTAssertEqual(try backend.formattedText(), "base")
            _ = try session.redo()
            XCTAssertEqual(try backend.formattedText(), "obase")
        }
    }

    @MainActor
    func testDirectCancelActionAndControlBracketCancelEveryMarkedTargetLocally() throws {
        for command in ["i", "R", ":draft", "/draft", "?draft"] {
            for directAction in [true, false] {
                let backend = EVCoreDocumentBackend()
                try backend.read(source: Data("base".utf8), typeName: "public.plain-text")
                let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
                surface.loadViewIfNeeded()
                let session = try XCTUnwrap(surface.session)
                surface.performInput { if !command.isEmpty { _ = try session.sendText(command) } }
                let view = RecordingEditorView(surface: surface)
                surface.view = view
                let window = textInputWindow(view)
                defer { window.contentView = nil }
                let mode = surface.viewPresentation.mode
                let prompt = surface.commandLine?.text
                let revision = try backend.revision()

                view.setMarkedText("ô", selectedRange: NSRange(location: 1, length: 0),
                                   replacementRange: NSRange(location: NSNotFound, length: 0))
                XCTAssertTrue(view.hasMarkedText(), command)
                if directAction { view.cancelOperation(nil) }
                else { view.keyDown(with: keyEvent(keyCode: 33, modifiers: .control, characters: "[")) }

                XCTAssertFalse(view.hasMarkedText(), command)
                XCTAssertFalse(session.hasActiveComposition, command)
                XCTAssertEqual(view.inputContextDiscardCount, 1, command)
                XCTAssertTrue(view.inputContextHandledKeyCodes.isEmpty)
                XCTAssertEqual(surface.viewPresentation.mode, mode, command)
                XCTAssertEqual(surface.commandLine?.text, prompt, command)
                XCTAssertEqual(try backend.revision(), revision, command)
                XCTAssertEqual(try backend.formattedText(), "base", command)
            }
        }
    }

    @MainActor
    func testAccentReplacementCancellationRetainsOriginalLetterAndUndoGroup() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("base".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendText("i") }
        let view = RecordingEditorView(surface: surface)
        surface.view = view
        let window = textInputWindow(view)
        defer { window.contentView = nil }
        view.insertText("o", replacementRange: NSRange(location: NSNotFound, length: 0))
        let revision = try backend.revision()
        view.setMarkedText("ô", selectedRange: NSRange(location: 1, length: 0),
                           replacementRange: NSRange(location: 0, length: 1))
        XCTAssertEqual(surface.fullPresentedText(), "ôbase")

        view.cancelOperation(nil)

        XCTAssertFalse(view.hasMarkedText())
        XCTAssertEqual(view.inputContextDiscardCount, 1)
        XCTAssertEqual(surface.fullPresentedText(), "obase")
        XCTAssertEqual(try backend.revision(), revision)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        // A second Escape now performs the ordinary mode transition.
        view.keyDown(with: keyEvent(keyCode: 53))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        _ = try session.undo()
        XCTAssertEqual(try backend.formattedText(), "base")
        _ = try session.redo()
        XCTAssertEqual(try backend.formattedText(), "obase")
    }

    @MainActor
    func testActiveCompositionRoutesEditingKeysThroughAppKitInterpretation() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("base".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendText("i") }
        let view = RecordingEditorView(surface: surface)

        view.setMarkedText(
            "かな",
            selectedRange: NSRange(location: 2, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )
        XCTAssertTrue(view.hasMarkedText())
        XCTAssertTrue(session.hasActiveComposition)

        for keyCode: UInt16 in [123, 51, 117, 36, 48, 115, 119] {
            view.keyDown(with: keyEvent(keyCode: keyCode))
        }

        XCTAssertEqual(view.inputContextHandledKeyCodes, [123, 51, 117, 36, 48, 115, 119])
        XCTAssertTrue(view.interpretedKeyCodes.isEmpty)

        view.inputContextConsumesEvents = false
        view.keyDown(with: keyEvent(keyCode: 124))
        XCTAssertEqual(view.inputContextHandledKeyCodes.last, 124)
        XCTAssertEqual(view.interpretedKeyCodes, [124])
        XCTAssertTrue(view.hasMarkedText())
        XCTAssertTrue(session.hasActiveComposition)
        XCTAssertEqual(try backend.formattedText(), "base")

        view.unmarkText()
    }

    @MainActor
    func testInsertTextHonorsExplicitUTF16ReplacementAsOneUndoUnit() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("left😀right".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendText("i") }
        let client: NSTextInputClient = surface.editorView

        client.insertText("é", replacementRange: NSRange(location: 4, length: 2))

        XCTAssertEqual(try backend.formattedText(), "leftéright")
        _ = try session.undo()
        XCTAssertEqual(try backend.formattedText(), "left😀right")
    }

    @MainActor
    func testEmptyInsertTextDeletesExplicitReplacementRange() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("abc".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendText("i") }
        let client: NSTextInputClient = surface.editorView

        client.insertText("", replacementRange: NSRange(location: 1, length: 1))

        XCTAssertEqual(try backend.formattedText(), "ac")
    }

    @MainActor
    func testInsertTextRetargetsAnActiveCompositionWhenRangeIsExplicit() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("abcd".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendText("i") }
        let client: NSTextInputClient = surface.editorView

        client.setMarkedText(
            "X",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: 1, length: 1)
        )
        client.insertText("Y", replacementRange: NSRange(location: 2, length: 1))

        XCTAssertFalse(client.hasMarkedText())
        XCTAssertEqual(try backend.formattedText(), "abYd")
    }

    @MainActor
    func testSelectedRangeUsesMarkedTextRelativeSelection() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("A😀B".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendText("i") }
        let client: NSTextInputClient = surface.editorView

        client.setMarkedText(
            "漢😀字",
            selectedRange: NSRange(location: 1, length: 2),
            replacementRange: NSRange(location: 1, length: 2)
        )

        XCTAssertEqual(client.markedRange(), NSRange(location: 1, length: 4))
        XCTAssertEqual(client.selectedRange(), NSRange(location: 2, length: 2))
        XCTAssertEqual(try backend.formattedText(), "A😀B")

        _ = try session.cancelComposition()
    }

    @MainActor
    func testDocumentCompositionUsesProjectedLayoutGeometryAndAccessibility() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(
            source: Data("left middle right".utf8),
            typeName: "public.plain-text"
        )
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let view = surface.editorView
        view.frame = NSRect(x: 0, y: 0, width: 110, height: 150)
        surface.viewDidLayout()
        let window = NSWindow(
            contentRect: view.frame,
            styleMask: .borderless,
            backing: .buffered,
            defer: false
        )
        window.contentView = view
        XCTAssertTrue(window.makeFirstResponder(view))
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendText("i") }
        let client: NSTextInputClient = view

        client.setMarkedText(
            "漢😀字",
            selectedRange: NSRange(location: 1, length: 2),
            replacementRange: NSRange(location: 5, length: 6)
        )

        XCTAssertEqual(try backend.formattedText(), "left middle right")
        XCTAssertEqual(surface.fullPresentedText(), "left 漢😀字 right")
        XCTAssertEqual(client.markedRange(), NSRange(location: 5, length: 4))
        XCTAssertEqual(client.selectedRange(), NSRange(location: 6, length: 2))
        XCTAssertEqual(view.accessibilityValue() as? String, "left 漢😀字 right")
        XCTAssertEqual(view.accessibilityNumberOfCharacters(), 15)
        XCTAssertEqual(view.accessibilitySelectedText(), "😀")
        XCTAssertEqual(view.accessibilitySelectedTextRange(), client.selectedRange())

        let overlay = try XCTUnwrap(surface.compositionOverlay)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertEqual(snapshot.rows.last?.hard_line_end, overlay.info.utf8_length)
        XCTAssertGreaterThan(snapshot.rows.count, 1)
        let laidOutSuffix = snapshot.clusters
            .filter { $0.text_start >= overlay.info.marked_end }
            .compactMap {
                surface.layoutText(in: Int($0.text_start) ..< Int($0.text_end))
            }
            .joined()
        XCTAssertTrue(laidOutSuffix.contains("right"))

        var actual = NSRange(location: NSNotFound, length: 0)
        let rect = client.firstRect(
            forCharacterRange: NSRange(location: 6, length: 0),
            actualRange: &actual
        )
        XCTAssertEqual(actual, NSRange(location: 6, length: 0))
        XCTAssertGreaterThan(rect.height, 0)
        XCTAssertEqual(client.characterIndex(for: NSPoint(x: rect.minX, y: rect.midY)), 6)

        view.keyDown(with: keyEvent(keyCode: 53))
        XCTAssertFalse(client.hasMarkedText())
        XCTAssertEqual(try backend.formattedText(), "left middle right")
        XCTAssertEqual(view.accessibilityValue() as? String, "left middle right")
    }

    @MainActor
    func testMalformedUTF16RangesAreRejectedWithoutMutationOrTraps() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("A😀B".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let client: NSTextInputClient = surface.editorView
        let malformed = [
            NSRange(location: -1, length: 1),
            NSRange(location: 0, length: -1),
            NSRange(location: Int.max - 2, length: 8),
            NSRange(location: 2, length: 0),
        ]

        for range in malformed {
            XCTAssertNil(client.attributedSubstring(forProposedRange: range, actualRange: nil))
            client.insertText("X", replacementRange: range)
        }
        client.setMarkedText(
            "😀",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )

        XCTAssertFalse(client.hasMarkedText())
        XCTAssertEqual(try backend.formattedText(), "A😀B")
    }

    @MainActor
    func testDirectCoreCancellationImmediatelyClearsNativeMarkedState() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("abc".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendText("i") }
        let view = RecordingEditorView(surface: surface)
        surface.view = view
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 320, height: 180),
            styleMask: .borderless,
            backing: .buffered,
            defer: false
        )
        window.contentView = view
        XCTAssertTrue(window.makeFirstResponder(view))
        let client: NSTextInputClient = view

        client.setMarkedText(
            "é",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )
        XCTAssertTrue(client.hasMarkedText())

        _ = try session.sendKey(kind: UInt32(VIEM_KEY_RIGHT))

        XCTAssertFalse(client.hasMarkedText())
        XCTAssertEqual(client.markedRange().location, NSNotFound)
        XCTAssertFalse(session.hasActiveComposition)
        XCTAssertEqual(view.inputContextDiscardCount, 1)
        XCTAssertEqual(try backend.formattedText(), "abc")
    }

    @MainActor
    func testRejectedExplicitReplacementKeepsNativeAndCoreMarkedStateAligned() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data([0xE9]), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        surface.performInput { _ = try session.sendText("i") }
        let client: NSTextInputClient = surface.editorView

        client.insertText("😀", replacementRange: NSRange(location: 0, length: 1))

        XCTAssertTrue(client.hasMarkedText())
        XCTAssertTrue(session.hasActiveComposition)
        XCTAssertEqual(client.markedRange(), NSRange(location: 0, length: 2))
        XCTAssertEqual(client.selectedRange(), NSRange(location: 2, length: 0))
        XCTAssertEqual(try backend.formattedText(), "é")

        _ = try session.cancelComposition()
    }

    @MainActor
    func testInsertModeCompositionUpdatesCommitsAndCancelsThroughCore() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("base".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        let client: NSTextInputClient = surface.editorView
        surface.performInput { _ = try session.sendText("i") }

        client.setMarkedText(
            "か",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )
        client.setMarkedText(
            "かな",
            selectedRange: NSRange(location: 2, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )

        XCTAssertTrue(client.hasMarkedText())
        XCTAssertTrue(session.hasActiveComposition)
        XCTAssertEqual(try backend.formattedText(), "base")

        client.unmarkText()
        XCTAssertFalse(client.hasMarkedText())
        XCTAssertFalse(session.hasActiveComposition)
        XCTAssertEqual(try backend.formattedText(), "かなbase")

        client.setMarkedText(
            "没",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )
        surface.editorView.keyDown(with: keyEvent(keyCode: 53))
        XCTAssertFalse(client.hasMarkedText())
        XCTAssertFalse(session.hasActiveComposition)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(try backend.formattedText(), "かなbase")
    }

    @MainActor
    func testReplaceModeCompositionUsesCoreAndHonorsExplicitReplacement() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("abcd".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        let client: NSTextInputClient = surface.editorView
        surface.performInput { _ = try session.sendText("R") }

        client.setMarkedText(
            "X",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: 0, length: 1)
        )
        client.setMarkedText(
            "YZ",
            selectedRange: NSRange(location: 2, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )
        XCTAssertTrue(session.hasActiveComposition)
        XCTAssertEqual(try backend.formattedText(), "abcd")

        client.insertText("YZ", replacementRange: client.markedRange())
        XCTAssertFalse(client.hasMarkedText())
        XCTAssertFalse(session.hasActiveComposition)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_REPLACE))
        XCTAssertEqual(try backend.formattedText(), "YZbcd")

        client.setMarkedText(
            "Q",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: 2, length: 1)
        )
        surface.editorView.keyDown(with: keyEvent(keyCode: 53))
        XCTAssertFalse(client.hasMarkedText())
        XCTAssertFalse(session.hasActiveComposition)
        XCTAssertEqual(try backend.formattedText(), "YZbcd")
    }

    @MainActor
    func testNormalAndEveryVisualModeRejectNativeCompositionAndAccentReplacement() throws {
        for command in ["", "v", "V", "block", "d", "r", "f", "ctrl-o"] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data("base\nnext".utf8), typeName: "public.plain-text")
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            let session = try XCTUnwrap(surface.session)
            surface.performInput {
                if command == "block" {
                    _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 118)
                } else if command == "ctrl-o" {
                    _ = try session.sendText("i")
                    _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 111)
                } else if !command.isEmpty {
                    _ = try session.sendText(command)
                }
            }
            let view = surface.editorView
            let mode = surface.viewPresentation.mode
            let cursor = surface.viewPresentation.cursor_utf8_offset
            let anchor = surface.viewPresentation.visual_anchor_utf8_offset
            let revision = try backend.revision()
            XCTAssertNil(view.inputContext, command)
            for replacement in [NSRange(location: NSNotFound, length: 0), NSRange(location: 0, length: 1)] {
                view.setMarkedText("かな", selectedRange: NSRange(location: 2, length: 0), replacementRange: replacement)
                XCTAssertFalse(view.hasMarkedText(), command)
                XCTAssertEqual(view.markedRange().location, NSNotFound, command)
                XCTAssertFalse(session.hasActiveComposition, command)
            }
            view.unmarkText()
            // A late callback from an old accent session must not execute i.
            view.insertText("i", replacementRange: NSRange(location: 0, length: 1))
            XCTAssertEqual(surface.viewPresentation.mode, mode, command)
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, cursor, command)
            XCTAssertEqual(surface.viewPresentation.visual_anchor_utf8_offset, anchor, command)
            XCTAssertEqual(try backend.revision(), revision, command)
            XCTAssertEqual(try backend.formattedText(), "base\nnext", command)
        }
    }

    @MainActor
    func testRepeatedCommandKeysBypassNativeTextInterpretation() throws {
        for command in ["", "v", "V", "block"] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data("abcdef\nghijkl".utf8), typeName: "public.plain-text")
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            let session = try XCTUnwrap(surface.session)
            surface.performInput {
                if command == "block" {
                    _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 118)
                } else if !command.isEmpty { _ = try session.sendText(command) }
            }
            let view = RecordingEditorView(surface: surface)
            let mode = surface.viewPresentation.mode
            view.keyDown(with: keyEvent(keyCode: 37, characters: "l"))
            view.keyDown(with: keyEvent(keyCode: 37, characters: "l", isRepeat: true))
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 2, command)
            XCTAssertEqual(surface.viewPresentation.mode, mode, command)
            XCTAssertNil(view.inputContext, command)
            XCTAssertTrue(view.interpretedKeyCodes.isEmpty, command)
            XCTAssertTrue(view.inputContextHandledKeyCodes.isEmpty, command)
            XCTAssertEqual(try backend.formattedText(), "abcdef\nghijkl", command)
        }
    }

    @MainActor
    func testInputContextTracksTextEntryAndDiscardsUnmarkedCandidatesOnModeExit() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("base".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let view = RecordingEditorView(surface: surface)
        surface.view = view
        let window = textInputWindow(view)
        defer { window.contentView = nil }
        XCTAssertNil(view.inputContext)
        view.keyDown(with: keyEvent(keyCode: 34, characters: "i"))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertNotNil(view.inputContext)
        view.keyDown(with: keyEvent(keyCode: 31, characters: "o"))
        view.keyDown(with: keyEvent(keyCode: 31, characters: "o", isRepeat: true))
        XCTAssertEqual(view.interpretedKeyCodes, [31, 31])
        view.insertText("o", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertFalse(view.hasMarkedText())
        // Ctrl-C changes mode without using the Escape/cancel responder path.
        view.keyDown(with: keyEvent(keyCode: 8, modifiers: .control, characters: "c"))
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertNil(view.inputContext)
        XCTAssertEqual(view.inputContextDiscardCount, 1)
        XCTAssertEqual(try backend.formattedText(), "obase")
        view.keyDown(with: keyEvent(keyCode: 34, characters: "i"))
        XCTAssertNotNil(view.inputContext)
        view.keyDown(with: keyEvent(keyCode: 53))
        XCTAssertNil(view.inputContext)
        XCTAssertEqual(view.inputContextDiscardCount, 2)
    }

    @MainActor
    func testStatusLineKeepsNativeInterpretationAndAccentReplacement() throws {
        for command in [":", "/", "?", "v:"] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data("base".utf8), typeName: "public.plain-text")
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            let session = try XCTUnwrap(surface.session)
            surface.performInput { _ = try session.sendText(command) }
            let view = RecordingEditorView(surface: surface)
            XCTAssertNotNil(view.inputContext, command)
            view.keyDown(with: keyEvent(keyCode: 31, characters: "o"))
            view.keyDown(with: keyEvent(keyCode: 31, characters: "o", isRepeat: true))
            XCTAssertEqual(view.interpretedKeyCodes, [31, 31], command)
            let offset = try XCTUnwrap(surface.commandLine?.text.utf16.count)
            view.insertText("o", replacementRange: NSRange(location: NSNotFound, length: 0))
            view.setMarkedText("ô", selectedRange: NSRange(location: 1, length: 0),
                               replacementRange: NSRange(location: offset, length: 1))
            XCTAssertTrue(view.hasMarkedText(), command)
            XCTAssertFalse(session.hasActiveComposition, command)
            view.unmarkText()
            XCTAssertEqual(surface.commandLine?.text.utf16.last, 0xF4, command)
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_COMMAND_LINE), command)
            XCTAssertEqual(try backend.formattedText(), "base", command)
        }
    }

    @MainActor
    func testUnicodeCommandOperandsStillReachCoreWithoutNativeComposition() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("aéz".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let view = RecordingEditorView(surface: surface)
        view.keyDown(with: keyEvent(keyCode: 3, characters: "f"))
        view.keyDown(with: keyEvent(keyCode: 14, characters: "é"))
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 1)
        view.keyDown(with: keyEvent(keyCode: 15, characters: "r"))
        view.keyDown(with: keyEvent(keyCode: 14, characters: "e\u{301}"))
        XCTAssertEqual(try backend.formattedText(), "ae\u{301}z")
        XCTAssertTrue(view.interpretedKeyCodes.isEmpty)
    }

    @MainActor
    func testExCommandLineCompositionIsLocalUntilCommitAndRendersMarkedSelection() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("document".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.editorView.frame = NSRect(x: 0, y: 0, width: 420, height: 220)
        let session = try XCTUnwrap(surface.session)
        let client: NSTextInputClient = surface.editorView
        let sourceRevision = try backend.revision()
        surface.performInput {
            _ = try session.sendText(":")
            _ = try session.sendText("abcd")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_LEFT))
        }
        XCTAssertEqual(client.selectedRange(), NSRange(location: 3, length: 0))

        client.setMarkedText(
            "漢字",
            selectedRange: NSRange(location: 1, length: 1),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )

        XCTAssertTrue(client.hasMarkedText())
        XCTAssertFalse(session.hasActiveComposition)
        XCTAssertEqual(surface.commandLine?.text, "abcd")
        XCTAssertEqual(client.markedRange(), NSRange(location: 3, length: 2))
        XCTAssertEqual(client.selectedRange(), NSRange(location: 4, length: 1))
        let markedState = try XCTUnwrap(surface.editorView.commandLineRenderState())
        XCTAssertEqual(markedState.text, "abc漢字d")
        XCTAssertEqual(markedState.displayText, ":abc漢字d")
        XCTAssertEqual(markedState.markedDisplayRange, NSRange(location: 4, length: 2))
        XCTAssertEqual(markedState.selectedDisplayRange, NSRange(location: 5, length: 1))
        XCTAssertEqual(try backend.revision(), sourceRevision)
        XCTAssertEqual(try backend.formattedText(), "document")

        client.setMarkedText(
            "かな",
            selectedRange: NSRange(location: 2, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )
        XCTAssertEqual(surface.commandLine?.text, "abcd")
        XCTAssertEqual(surface.editorView.commandLineRenderState()?.text, "abcかなd")

        client.unmarkText()
        XCTAssertFalse(client.hasMarkedText())
        XCTAssertFalse(session.hasActiveComposition)
        XCTAssertEqual(surface.commandLine?.text, "abcかなd")
        XCTAssertEqual(try backend.revision(), sourceRevision)
        XCTAssertEqual(try backend.formattedText(), "document")

        client.setMarkedText(
            "没",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )
        surface.editorView.keyDown(with: keyEvent(keyCode: 53))
        XCTAssertFalse(client.hasMarkedText())
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_COMMAND_LINE))
        XCTAssertEqual(surface.commandLine?.text, "abcかなd")
        XCTAssertEqual(try backend.formattedText(), "document")
    }

    @MainActor
    func testSearchCommandLineCompositionCommitsViaCommandInputAndCancelsLocally() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("café document".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        let client: NSTextInputClient = surface.editorView
        surface.performInput {
            _ = try session.sendText("/")
            _ = try session.sendText("A😀B")
        }

        client.setMarkedText(
            "猫",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: 1, length: 2)
        )
        XCTAssertEqual(surface.commandLine?.text, "A😀B")
        XCTAssertEqual(surface.editorView.commandLineRenderState()?.text, "A猫B")
        XCTAssertFalse(session.hasActiveComposition)

        client.setMarkedText(
            "狐",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )
        XCTAssertEqual(surface.commandLine?.text, "A😀B")
        XCTAssertEqual(surface.editorView.commandLineRenderState()?.text, "A狐B")

        client.insertText("犬", replacementRange: client.markedRange())
        XCTAssertFalse(client.hasMarkedText())
        XCTAssertEqual(surface.commandLine?.text, "A犬B")
        XCTAssertEqual(try backend.formattedText(), "café document")

        client.setMarkedText(
            "鳥",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )
        surface.editorView.keyDown(with: keyEvent(keyCode: 53))
        XCTAssertFalse(client.hasMarkedText())
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_COMMAND_LINE))
        XCTAssertEqual(surface.commandLine?.info.identity.kind, UInt32(VIEM_COMMAND_LINE_KIND_SEARCH_FORWARD))
        XCTAssertEqual(surface.commandLine?.text, "A犬B")
        XCTAssertEqual(try backend.formattedText(), "café document")
    }

    @MainActor
    func testStaleCommandLineDraftCannotOverwriteNewerCoreInput() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("document".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        let client: NSTextInputClient = surface.editorView
        surface.performInput {
            _ = try session.sendText(":")
            _ = try session.sendText("abc")
        }
        client.setMarkedText(
            "draft",
            selectedRange: NSRange(location: 5, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )

        // Simulate a core-owned command-line change that arrives before the
        // surface has republished its cached presentation.
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_LEFT))
        client.unmarkText()

        XCTAssertFalse(client.hasMarkedText())
        XCTAssertEqual(surface.commandLine?.text, "abc")
        XCTAssertEqual(surface.commandLine?.info.cursor_utf8_offset, 2)
        XCTAssertEqual(try backend.formattedText(), "document")
    }

    @MainActor
    func testMouseDragStartsAndMouseUpStopsSelectionAutoscroll() throws {
        let backend = EVCoreDocumentBackend()
        let text = (0..<400).map { "line \($0)" }.joined(separator: "\n")
        try backend.read(source: Data(text.utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 220, height: 120)
        surface.viewDidLayout()
        let view = RecordingEditorView(surface: surface)
        view.frame = NSRect(x: 0, y: 0, width: 220, height: 120)

        view.mouseDragged(
            // NSEvent locations use the window's bottom-up coordinates; a
            // negative y is below this flipped editor view.
            with: mouseEvent(type: .leftMouseDragged, location: NSPoint(x: 250, y: -30))
        )

        XCTAssertEqual(view.autoscrollCallCount, 1)
        XCTAssertTrue(view.isDragAutoscrollActive)

        let initialTop = surface.viewportState.top
        XCTAssertTrue(view.performDragAutoscrollStep())
        XCTAssertGreaterThan(surface.viewportState.top, initialTop)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_SELECTION_CHARACTER))

        view.mouseUp(with: mouseEvent(type: .leftMouseUp, location: NSPoint(x: 250, y: -30)))
        XCTAssertFalse(view.isDragAutoscrollActive)
    }
}

@MainActor
private final class RecordingEditorView: EVEditorView {
    private(set) var interpretedKeyCodes: [UInt16] = []
    private(set) var autoscrollCallCount = 0
    private lazy var recordingInputContext = RecordingTextInputContext(client: self)

    override var nativeTextInputContext: NSTextInputContext? { recordingInputContext }

    var inputContextDiscardCount: Int { recordingInputContext.discardCount }
    var inputContextHandledKeyCodes: [UInt16] { recordingInputContext.handledKeyCodes }
    var inputContextConsumesEvents: Bool {
        get { recordingInputContext.consumesEvents }
        set { recordingInputContext.consumesEvents = newValue }
    }

    override func interpretKeyEvents(_ eventArray: [NSEvent]) {
        interpretedKeyCodes.append(contentsOf: eventArray.map(\.keyCode))
    }

    override func autoscroll(with event: NSEvent) -> Bool {
        autoscrollCallCount += 1
        return true
    }
}

@MainActor
private final class RecordingTextInputContext: NSTextInputContext {
    private(set) var discardCount = 0
    private(set) var handledKeyCodes: [UInt16] = []
    var consumesEvents = true

    override func handleEvent(_ event: NSEvent) -> Bool {
        handledKeyCodes.append(event.keyCode)
        return consumesEvents
    }

    override func discardMarkedText() {
        discardCount += 1
    }
}

@MainActor
private func textInputWindow(_ view: EVEditorView) -> NSWindow {
    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 320, height: 180),
                          styleMask: .borderless, backing: .buffered, defer: false)
    window.contentView = view
    XCTAssertTrue(window.makeFirstResponder(view))
    return window
}

private func keyEvent(keyCode: UInt16, modifiers: NSEvent.ModifierFlags = [], characters: String = "", isRepeat: Bool = false) -> NSEvent {
    NSEvent.keyEvent(
        with: .keyDown,
        location: .zero,
        modifierFlags: modifiers,
        timestamp: 0,
        windowNumber: 0,
        context: nil,
        characters: characters,
        charactersIgnoringModifiers: characters,
        isARepeat: isRepeat,
        keyCode: keyCode
    )!
}

private func mouseEvent(type: NSEvent.EventType, location: NSPoint) -> NSEvent {
    NSEvent.mouseEvent(
        with: type,
        location: location,
        modifierFlags: [],
        timestamp: 0,
        windowNumber: 0,
        context: nil,
        eventNumber: 1,
        clickCount: 1,
        pressure: 1
    )!
}
