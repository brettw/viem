import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

final class EVCoreStateMenuIntegrationTests: XCTestCase {
    @MainActor
    func testStandardNativeEditSelectorsReachCoreWhenDocumentViewIsFocused() throws {
        let (backend, surface, _) = try makeSurface("Text")
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 640, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = surface
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        func item(_ command: EVMenuCommand) throws -> NSMenuItem {
            let item = NSMenuItem(title: "Edit", action: try XCTUnwrap(command.nativeEditAction), keyEquivalent: "")
            item.tag = command.rawValue
            return item
        }
        func send(_ command: EVMenuCommand) throws {
            let action = try item(command)
            XCTAssertTrue(surface.editorView.validateMenuItem(action))
            XCTAssertTrue(window.firstResponder === surface.editorView)
            XCTAssertTrue(try XCTUnwrap(window.firstResponder).tryToPerform(try XCTUnwrap(action.action), with: action))
        }
        XCTAssertFalse(surface.editorView.validateMenuItem(try item(.undo)))
        try send(.selectAll)
        XCTAssertEqual(surface.editorView.selectedRange(), NSRange(location: 0, length: 4))
        try send(.delete)
        XCTAssertEqual(try backend.formattedText(), "")
        let undo = try item(.undo)
        XCTAssertTrue(surface.editorView.validateMenuItem(undo))
        XCTAssertTrue(undo.title.hasPrefix("Undo "))
        try send(.undo)
        XCTAssertEqual(try backend.formattedText(), "Text")
        try send(.redo)
        XCTAssertEqual(try backend.formattedText(), "")
    }

    @MainActor
    private final class Pasteboard: EVPasteboardAccess {
        var text: String?
        var generation: UInt64 = 1

        var viemGeneration: UInt64 { generation }
        var viemIsWritable: Bool { true }
        func viemString() -> String? { text }
        func viemCanReadString() -> Bool { text != nil }
        func viemClearContents() -> Int {
            text = nil
            generation &+= 1
            return Int(generation)
        }
        func viemSetString(_ string: String) -> Bool {
            text = string
            generation &+= 1
            return true
        }
    }

    @MainActor
    func testHistoryAvailabilityTitlesAndDirtyStateComeFromCore() throws {
        let (backend, surface, session) = try makeSurface("base")

        XCTAssertFalse(surface.canUndo)
        XCTAssertFalse(surface.canRedo)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertEqual(surface.presentation(for: .undo), .init(isEnabled: false, title: "Undo"))
        XCTAssertEqual(surface.presentation(for: .redo), .init(isEnabled: false, title: "Redo"))

        _ = try session.sendText("i")
        _ = try session.sendText("X")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        surface.refreshPresentation()

        XCTAssertEqual(surface.formattedText, "Xbase")
        XCTAssertTrue(surface.canUndo)
        XCTAssertFalse(surface.canRedo)
        XCTAssertTrue(backend.persistenceState.isDirty)
        XCTAssertEqual(
            surface.presentation(for: .undo),
            .init(isEnabled: true, title: "Undo Text Change")
        )

        surface.perform(menuCommand: .undo, sender: nil)

        XCTAssertEqual(surface.formattedText, "base")
        XCTAssertFalse(surface.canUndo)
        XCTAssertTrue(surface.canRedo)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertEqual(
            surface.presentation(for: .redo),
            .init(isEnabled: true, title: "Redo Text Change")
        )
    }

    @MainActor
    func testBackendAcknowledgesOnlyTheExactCurrentSaveRevision() throws {
        let (backend, surface, session) = try makeSurface("base")
        var dirtyTransitions: [Bool] = []
        backend.persistenceStateDidChange = { dirtyTransitions.append($0.isDirty) }

        _ = try session.sendText("i")
        _ = try session.sendText("X")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        let snapshot = try backend.nativeSaveSnapshot(typeName: "public.plain-text")

        XCTAssertEqual(snapshot.data, Data("Xbase".utf8))
        XCTAssertEqual(snapshot.documentRevision, try backend.documentState().document_revision)
        XCTAssertTrue(backend.persistenceState.isDirty)

        try backend.acknowledgeNativeSave(snapshot)

        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertEqual(
            surface.documentState.flags & UInt32(VIEM_DOCUMENT_STATE_IS_DIRTY),
            0,
            "save acknowledgement refreshes every surface's cached menu state"
        )
        XCTAssertEqual(dirtyTransitions, [true, false])
        XCTAssertTrue(surface.canUndo, "marking a save point must not discard history")

        _ = try session.sendText("A")
        _ = try session.sendText("Y")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        XCTAssertEqual(surface.formattedText, "XbaseY")
        XCTAssertTrue(backend.persistenceState.isDirty)

        XCTAssertThrowsError(try backend.acknowledgeNativeSave(snapshot)) { error in
            guard case let EVCoreFrontendError.core(_, status) = error else {
                return XCTFail("unexpected stale-save error: \(error)")
            }
            XCTAssertEqual(status, UInt32(VIEM_STATUS_STALE_REVISION))
        }
        XCTAssertTrue(backend.persistenceState.isDirty)
        XCTAssertEqual(try backend.serializedSource(typeName: "public.plain-text"), Data("XbaseY".utf8))
    }

    @MainActor
    func testWordWrapUsesCoreViewStateIncludingExChanges() throws {
        let (_, surface, session) = try makeSurface("alpha beta gamma")
        XCTAssertEqual(surface.presentation(for: .wordWrap).state, .on)
        surface.perform(menuCommand: .wordWrap, sender: nil)
        XCTAssertEqual(surface.presentation(for: .wordWrap).state, .off)
        surface.perform(menuCommand: .wordWrap, sender: nil)
        XCTAssertEqual(surface.presentation(for: .wordWrap).state, .on)

        try send(Array(":set nowrap"), through: session)
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        surface.refreshPresentation()
        XCTAssertEqual(surface.presentation(for: .wordWrap).state, .off)
        try send(Array(":set wrap"), through: session)
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        surface.refreshPresentation()
        XCTAssertEqual(surface.presentation(for: .wordWrap).state, .on)
    }

    @MainActor
    func testLineEndingRadioActionIsAtomicUndoableAndShared() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("one\r\ntwo\r\n".utf8), typeName: "public.plain-text")
        let first = try makeSurface(backend)
        let second = try makeSurface(backend)

        XCTAssertEqual(first.surface.presentation(for: .lineEndingWindows).state, .on)
        XCTAssertEqual(first.surface.presentation(for: .lineEndingUnix).state, .off)

        first.surface.perform(menuCommand: .lineEndingUnix, sender: nil)

        XCTAssertEqual(try backend.serializedSource(typeName: "public.plain-text"), Data("one\ntwo\n".utf8))
        XCTAssertEqual(first.surface.formattedText, "one\ntwo\n")
        XCTAssertEqual(second.surface.formattedText, "one\ntwo\n")
        XCTAssertEqual(second.surface.presentation(for: .lineEndingUnix).state, .on)
        XCTAssertEqual(first.surface.presentation(for: .lineEndingUnix).state, .on)
        XCTAssertEqual(
            first.surface.presentation(for: .undo).title,
            "Undo Line Endings"
        )

        first.surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: "public.plain-text"), Data("one\r\ntwo\r\n".utf8))
        XCTAssertEqual(second.surface.presentation(for: .lineEndingWindows).state, .on)
    }

    @MainActor
    func testEncodingMenuConversionsAreCheckedSharedAndExactlyUndoable() throws {
        let source = "café\r\nnext"
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.plainTextType)
        let first = try makeSurface(backend)
        let second = try makeSurface(backend)
        let choices: [(EVMenuCommand, UInt32)] = [
            (.encodingLatin1, UInt32(VIEM_ENCODING_LATIN1)),
            (.encodingUTF16LE, UInt32(VIEM_ENCODING_UTF16_LE)),
            (.encodingUTF16BE, UInt32(VIEM_ENCODING_UTF16_BE)),
            (.encodingUTF8, UInt32(VIEM_ENCODING_UTF8)),
        ]
        XCTAssertEqual(first.surface.presentation(for: .encodingUTF8).state, .on)
        for (command, encoding) in choices {
            let previous = try backend.serializedSource(typeName: EVDocument.plainTextType)
            let previousEncoding = backend.currentDocumentState.encoding
            first.surface.perform(menuCommand: command, sender: nil)
            XCTAssertEqual(backend.currentDocumentState.encoding, encoding)
            XCTAssertEqual(first.surface.formattedText, "café\nnext")
            XCTAssertEqual(second.surface.formattedText, "café\nnext")
            XCTAssertEqual(first.surface.statusBarState.message, "")
            for (choice, _) in choices {
                for surface in [first.surface, second.surface] {
                    let presentation = surface.presentation(for: choice)
                    XCTAssertTrue(presentation.isEnabled)
                    XCTAssertEqual(presentation.state, choice == command ? .on : .off)
                }
            }
            let converted = try backend.serializedSource(typeName: EVDocument.plainTextType)
            XCTAssertNotEqual(converted, previous)
            let reopened = EVCoreDocumentBackend()
            // Conversion preserves BOM absence, so reopen with the captured
            // source interpretation instead of guessing BOM-less UTF-16.
            try reopened.restoreRecovery(backend.recoverySnapshot())
            XCTAssertEqual(try reopened.formattedText(), "café\nnext")
            XCTAssertEqual(reopened.currentDocumentState.encoding, encoding)
            XCTAssertEqual(reopened.currentDocumentState.file_format, UInt32(VIEM_FILE_FORMAT_DOS))
            first.surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), previous)
            XCTAssertEqual(backend.currentDocumentState.encoding, previousEncoding)
            first.surface.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), converted)
            XCTAssertEqual(second.surface.presentation(for: command).state, .on)
        }
    }

    @MainActor
    func testRejectedLineEndingConversionChangesNeitherBytesStateNorHistory() throws {
        let backend = EVCoreDocumentBackend()
        // In unix mode the CR is literal content. Converting to classic-Mac
        // would reinterpret it as another logical line boundary and is rejected.
        try backend.read(source: Data("one\rtwo\nthree".utf8), typeName: "public.plain-text")
        let (surface, session) = try makeSurface(backend)
        let before = try backend.documentState()
        let bytes = try backend.serializedSource(typeName: "public.plain-text")

        XCTAssertThrowsError(
            try session.setFileFormat(UInt32(VIEM_FILE_FORMAT_MAC), expected: before)
        )
        surface.refreshPresentation()
        let after = try backend.documentState()

        XCTAssertEqual(after.document_revision, before.document_revision)
        XCTAssertEqual(after.file_format, before.file_format)
        XCTAssertEqual(after.flags, before.flags)
        XCTAssertEqual(try backend.serializedSource(typeName: "public.plain-text"), bytes)
        XCTAssertEqual(surface.presentation(for: .lineEndingUnix).state, .on)
        XCTAssertFalse(surface.canUndo)
    }

    @MainActor
    func testFindActionsEnterCoreCommandLinesAndRepeatSearch() throws {
        let (_, surface, session) = try makeSurface("one two one")

        surface.perform(menuCommand: .find, sender: nil)
        XCTAssertEqual(surface.commandLine?.prompt, "/")
        XCTAssertEqual(surface.commandLine?.text, "")
        _ = try session.sendText("one")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
        surface.refreshPresentation()
        let firstMatch = surface.viewPresentation.cursor_utf8_offset

        surface.perform(menuCommand: .findPrevious, sender: nil)
        let previousMatch = surface.viewPresentation.cursor_utf8_offset
        surface.perform(menuCommand: .findNext, sender: nil)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, firstMatch)
        XCTAssertNotEqual(previousMatch, firstMatch)

        surface.perform(menuCommand: .findAndReplace, sender: nil)
        XCTAssertEqual(surface.commandLine?.prompt, ":")
        XCTAssertEqual(surface.commandLine?.text, "%s/")
    }

    @MainActor
    func testUseSelectionForFindUsesExactCoreSelectionAndNativeFindPasteboard() throws {
        let (_, surface, session) = try makeSurface("a.c abc a.c")
        let findPasteboard = Pasteboard()
        surface.findPasteboard = findPasteboard

        XCTAssertFalse(surface.presentation(for: .useSelectionForFind).isEnabled)
        _ = try session.sendText("v")
        _ = try session.sendText("2")
        _ = try session.sendText("l")
        surface.refreshPresentation()

        XCTAssertEqual(surface.selectionText(), "a.c")
        XCTAssertTrue(surface.presentation(for: .useSelectionForFind).isEnabled)
        XCTAssertTrue(surface.presentation(for: .jumpToSelection).isEnabled)
        let cursor = surface.viewPresentation.cursor_utf8_offset

        surface.perform(menuCommand: .useSelectionForFind, sender: nil)

        XCTAssertEqual(findPasteboard.text, "a.c")
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, cursor)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_CHARACTER))

        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        _ = try session.sendText("0")
        surface.refreshPresentation()
        surface.perform(menuCommand: .findNext, sender: nil)
        XCTAssertEqual(
            surface.viewPresentation.cursor_utf8_offset,
            8,
            "selection text is installed as a literal core search, not a regex"
        )
    }

    @MainActor
    func testJumpToSelectionRevealsCoreCursorAfterSelectionLeavesLayoutCoverage() throws {
        let source = (0 ..< 300).map { "row \($0)" }.joined(separator: "\n")
        let (_, surface, session) = try makeSurface(source)
        surface.view.frame = NSRect(x: 0, y: 0, width: 320, height: 100)
        surface.viewDidLayout()

        try send(Array("250Gv"), through: session)
        surface.refreshPresentation()
        let selectionCursor = surface.viewPresentation.cursor_utf8_offset
        XCTAssertGreaterThan(surface.viewportState.top, 1_000)

        let farState = surface.viewportState
        _ = try session.setViewportOrigin(left: 0, top: 0, expected: farState)
        surface.refreshPresentation()
        XCTAssertEqual(surface.viewportState.top, 0)
        // The logical selection export survives; offscreen geometry is not fabricated.
        XCTAssertEqual(surface.visualSelection?.rectangles.isEmpty, true)
        XCTAssertTrue(surface.presentation(for: .jumpToSelection).isEnabled)
        XCTAssertTrue(surface.presentation(for: .useSelectionForFind).isEnabled)

        surface.perform(menuCommand: .jumpToSelection, sender: nil)

        XCTAssertGreaterThan(surface.viewportState.top, 1_000)
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, selectionCursor)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_CHARACTER))
        XCTAssertNotNil(surface.visualSelection)
    }

    @MainActor
    func testZoomIsExactViewLocalLayoutStateAndActualSizeRestoresOne() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(
            source: Data("A proportional sentence that wraps differently under magnification.".utf8),
            typeName: "public.plain-text"
        )
        let first = try makeSurface(backend)
        let second = try makeSurface(backend)
        first.surface.view.frame = NSRect(x: 0, y: 0, width: 240, height: 180)
        second.surface.view.frame = first.surface.view.frame
        first.surface.viewDidLayout()
        second.surface.viewDidLayout()

        XCTAssertEqual(first.surface.zoomScale, 1)
        XCTAssertEqual(second.surface.zoomScale, 1)
        let documentRevision = first.surface.documentState.document_revision
        let firstConfiguration = first.surface.viewportState.configuration_generation
        let secondConfiguration = second.surface.viewportState.configuration_generation
        let firstRows = try XCTUnwrap(first.surface.layoutSnapshot).rows.count

        first.surface.perform(menuCommand: .zoomIn, sender: nil)

        XCTAssertEqual(first.surface.zoomScale, 1.10, accuracy: 0.0001)
        XCTAssertEqual(second.surface.zoomScale, 1, accuracy: 0.0001)
        XCTAssertGreaterThan(first.surface.viewportState.configuration_generation, firstConfiguration)
        XCTAssertEqual(second.surface.viewportState.configuration_generation, secondConfiguration)
        XCTAssertEqual(first.surface.documentState.document_revision, documentRevision)
        XCTAssertGreaterThanOrEqual(
            try XCTUnwrap(first.surface.layoutSnapshot).rows.count,
            firstRows,
            "zoom reflows exact layout rather than scaling a cached bitmap"
        )

        first.surface.perform(menuCommand: .actualSize, sender: nil)
        XCTAssertEqual(first.surface.zoomScale, 1, accuracy: 0.0001)
        XCTAssertEqual(second.surface.zoomScale, 1, accuracy: 0.0001)
        XCTAssertTrue(first.surface.presentation(for: .zoomIn).isEnabled)
        XCTAssertTrue(first.surface.presentation(for: .zoomOut).isEnabled)
        XCTAssertTrue(first.surface.presentation(for: .actualSize).isEnabled)
    }

    @MainActor
    func testDeleteUsesModeAppropriateCoreIntentions() throws {
        do {
            let (_, surface, _) = try makeSurface("abc")
            surface.perform(menuCommand: .delete, sender: nil)
            XCTAssertEqual(surface.formattedText, "bc")
        }

        do {
            let (_, surface, session) = try makeSurface("abc")
            _ = try session.sendText("i")
            surface.refreshPresentation()
            surface.perform(menuCommand: .delete, sender: nil)
            XCTAssertEqual(surface.formattedText, "bc")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        }

        do {
            let (_, surface, session) = try makeSurface("abc")
            _ = try session.sendText("R")
            surface.refreshPresentation()
            surface.perform(menuCommand: .delete, sender: nil)
            XCTAssertEqual(surface.formattedText, "bc")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_REPLACE))
        }

        do {
            let (_, surface, session) = try makeSurface("abc")
            _ = try session.sendText(":abc")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_HOME))
            surface.refreshPresentation()
            surface.perform(menuCommand: .delete, sender: nil)
            XCTAssertEqual(surface.commandLine?.text, "bc")
        }

        do {
            let (_, surface, session) = try makeSurface("abc")
            _ = try session.sendText("v")
            _ = try session.sendText("l")
            surface.refreshPresentation()
            surface.perform(menuCommand: .delete, sender: nil)
            XCTAssertEqual(surface.formattedText, "c")
        }
    }

    @MainActor
    func testCaseTransformsAreVisualOnlyExceptNormalToggle() throws {
        do {
            let (_, surface, _) = try makeSurface("Abc")
            XCTAssertFalse(surface.presentation(for: .makeUppercase).isEnabled)
            XCTAssertFalse(surface.presentation(for: .makeLowercase).isEnabled)
            XCTAssertTrue(surface.presentation(for: .toggleCase).isEnabled)
            surface.perform(menuCommand: .toggleCase, sender: nil)
            XCTAssertEqual(surface.formattedText, "abc")
        }

        do {
            let (_, surface, session) = try makeSurface("abc")
            _ = try session.sendText("v")
            _ = try session.sendText("l")
            surface.refreshPresentation()
            XCTAssertTrue(surface.presentation(for: .makeUppercase).isEnabled)
            surface.perform(menuCommand: .makeUppercase, sender: nil)
            XCTAssertEqual(surface.formattedText, "ABc")
        }

        do {
            let (_, surface, session) = try makeSurface("ABc")
            _ = try session.sendText("v")
            _ = try session.sendText("l")
            surface.refreshPresentation()
            surface.perform(menuCommand: .makeLowercase, sender: nil)
            XCTAssertEqual(surface.formattedText, "abc")
        }

        do {
            let (_, surface, session) = try makeSurface("abc")
            _ = try session.sendText("i")
            surface.refreshPresentation()
            XCTAssertFalse(surface.presentation(for: .toggleCase).isEnabled)
        }

        do {
            let (_, surface, session) = try makeSurface("ab\ncd")
            try enterVisualBlock(through: session, extendRight: true)
            surface.refreshPresentation()
            surface.perform(menuCommand: .makeUppercase, sender: nil)
            XCTAssertEqual(surface.formattedText, "AB\nCD")
        }
    }

    @MainActor
    func testSelectionCommandsKeepWholeLineAndTrailingNewlineSemantics() throws {
        let (_, surface, session) = try makeSurface("one two\nthree\n")
        _ = try session.sendText("w")
        surface.refreshPresentation()

        surface.perform(menuCommand: .selectVisualRow, sender: nil)
        XCTAssertEqual(surface.selectionText(), "one two")

        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        surface.refreshPresentation()
        surface.perform(menuCommand: .selectAll, sender: nil)
        XCTAssertEqual(surface.selectionText(), "one two\nthree\n")
        // Select All owns the exact full text range, independently of row
        // policy; it does not derive an extent from a final line motion.
        XCTAssertEqual(
            surface.visualSelection?.info.identity.kind,
            UInt32(VIEM_VISUAL_SELECTION_KIND_CHARACTER)
        )
    }

    @MainActor
    func testSelectVisualRowUsesExactSoftWrappedRowEdges() throws {
        let (_, surface, session) = try makeSurface(
            "one two three four five six seven eight nine ten"
        )
        surface.view.frame = NSRect(x: 0, y: 0, width: 155, height: 260)
        surface.viewDidLayout()
        let layout = try XCTUnwrap(surface.layoutSnapshot)
        XCTAssertGreaterThan(layout.rows.count, 2)
        let target = layout.rows[1]

        var point = ViemLayoutCaretPointV1()
        point.struct_size = UInt32(MemoryLayout<ViemLayoutCaretPointV1>.size)
        point.document_revision = surface.viewPresentation.document_revision
        point.text_offset = target.text_start
        point.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
        _ = try session.placeCursor(point, extendSelection: false)
        surface.refreshPresentation()

        surface.perform(menuCommand: .selectVisualRow, sender: nil)

        XCTAssertEqual(
            surface.selectedUTF8Ranges(),
            [Int(target.text_start) ..< Int(target.text_end)]
        )
        XCTAssertNotEqual(surface.selectionText(), surface.formattedText)
    }

    @MainActor
    func testNativeCopyAndCutUpdateCoreRegistersBeforePasteboard() throws {
        let pasteboard = Pasteboard()

        do {
            let (_, surface, session) = try makeSurface("abc")
            surface.pasteboard = pasteboard
            _ = try session.sendText("v")
            _ = try session.sendText("l")
            surface.refreshPresentation()

            surface.perform(menuCommand: .copy, sender: nil)
            XCTAssertEqual(pasteboard.text, "ab")
            XCTAssertEqual(surface.formattedText, "abc")
            // Native Copy keeps the Visual selection; leave it before putting.
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_CHARACTER))
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
            _ = try session.sendText("0P")
            surface.refreshPresentation()
            XCTAssertEqual(surface.formattedText, "ababc")
        }

        _ = pasteboard.viemClearContents()
        do {
            let (_, surface, session) = try makeSurface("abc")
            surface.pasteboard = pasteboard
            _ = try session.sendText("v")
            _ = try session.sendText("l")
            surface.refreshPresentation()

            surface.perform(menuCommand: .cut, sender: nil)
            XCTAssertEqual(pasteboard.text, "ab")
            XCTAssertEqual(surface.formattedText, "c")
            _ = try session.sendText("p")
            surface.refreshPresentation()
            XCTAssertEqual(surface.formattedText, "cab")
        }

        _ = pasteboard.viemClearContents()
        do {
            let (_, surface, session) = try makeSurface("ab\ncd")
            surface.pasteboard = pasteboard
            try enterVisualBlock(through: session, extendRight: false)
            surface.refreshPresentation()

            surface.perform(menuCommand: .cut, sender: nil)
            XCTAssertEqual(pasteboard.text, "a\nc")
            XCTAssertEqual(surface.formattedText, "b\nd")
        }
    }

    @MainActor
    func testNativeCopyUsesTheCoreSelectionWhenItsGeometryIsOffscreen() throws {
        let source = (0 ..< 300).map { "row \($0)" }.joined(separator: "\n")
        let (_, surface, session) = try makeSurface(source)
        let pasteboard = Pasteboard()
        surface.pasteboard = pasteboard
        surface.view.frame = NSRect(x: 0, y: 0, width: 320, height: 100)
        surface.viewDidLayout()

        try send(Array("v4l"), through: session)
        surface.refreshPresentation()
        XCTAssertEqual(surface.selectionText(), "row 0")

        _ = try session.setViewportOrigin(
            left: 0,
            top: 3_000,
            expected: surface.viewportState
        )
        surface.refreshPresentation()
        XCTAssertEqual(surface.visualSelection?.rectangles.isEmpty, true)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_CHARACTER))

        XCTAssertTrue(
            surface.presentation(for: .copy).isEnabled,
            "logical selection commands must not depend on materialized rectangles"
        )
        surface.perform(menuCommand: .copy, sender: nil)

        XCTAssertEqual(pasteboard.text, "row 0")
        XCTAssertEqual(surface.formattedText, source)
    }

    @MainActor
    func testNativePasteCanReplaceAVisualBlockUsingTheCoreRegisterPath() throws {
        let (_, surface, session) = try makeSurface("ab\ncd")
        let pasteboard = Pasteboard()
        pasteboard.text = "X"
        surface.pasteboard = pasteboard
        try enterVisualBlock(through: session, extendRight: false)
        surface.refreshPresentation()

        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_BLOCK))
        XCTAssertTrue(surface.presentation(for: .paste).isEnabled)
        surface.perform(menuCommand: .paste, sender: nil)

        XCTAssertEqual(surface.formattedText, "Xb\nd")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
    }

    @MainActor
    func testMarkdownBoldAndItalicMenusAreSourceBackedToggleableAndUndoable() throws {
        let (backend, surface, session) = try makeMarkdownSurface("alpha beta")
        try send(Array("v4l"), through: session)
        surface.refreshPresentation()

        XCTAssertEqual(
            surface.presentation(for: .bold),
            EVMenuItemPresentation(isEnabled: true, state: .off)
        )
        XCTAssertEqual(
            surface.presentation(for: .italic),
            EVMenuItemPresentation(isEnabled: true, state: .off)
        )

        surface.perform(menuCommand: .bold, sender: nil)

        XCTAssertEqual(surface.formattedText, "alpha beta")
        XCTAssertEqual(
            try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("**alpha** beta".utf8)
        )
        XCTAssertEqual(surface.presentation(for: .bold).state, .on)
        XCTAssertEqual(surface.presentation(for: .italic).state, .off)
        XCTAssertEqual(surface.presentation(for: .undo).title, "Undo Style Change")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_CHARACTER))

        surface.perform(menuCommand: .undo, sender: nil)

        XCTAssertEqual(
            try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("alpha beta".utf8)
        )
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertEqual(surface.presentation(for: .bold), EVMenuItemPresentation(isEnabled: true, state: .off))
        XCTAssertEqual(surface.presentation(for: .redo).title, "Redo Style Change")

        surface.perform(menuCommand: .redo, sender: nil)

        XCTAssertEqual(
            try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("**alpha** beta".utf8)
        )
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertEqual(surface.presentation(for: .bold), EVMenuItemPresentation(isEnabled: true, state: .on))

        try send(Array("0v4l"), through: session)
        surface.refreshPresentation()
        XCTAssertEqual(surface.presentation(for: .bold).state, .on)

        surface.perform(menuCommand: .bold, sender: nil)
        XCTAssertEqual(
            try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("alpha beta".utf8)
        )
        XCTAssertEqual(surface.presentation(for: .bold).state, .off)

        surface.perform(menuCommand: .italic, sender: nil)
        XCTAssertEqual(
            try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("*alpha* beta".utf8)
        )
        XCTAssertEqual(surface.presentation(for: .italic).state, .on)
    }

    @MainActor
    func testSemanticStyleMenusApplyMixedFormattingWithoutChangingUnselectedText() throws {
        let (backend, surface, session) = try makeMarkdownSurface("**alpha** beta")
        try send(Array("v6l"), through: session)
        surface.refreshPresentation()

        XCTAssertEqual(
            surface.presentation(for: .bold),
            EVMenuItemPresentation(isEnabled: true, state: .mixed)
        )

        surface.perform(menuCommand: .bold, sender: nil)

        XCTAssertEqual(surface.formattedText, "alpha beta")
        XCTAssertEqual(
            try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("**alpha b**eta".utf8)
        )
        XCTAssertEqual(surface.presentation(for: .bold), EVMenuItemPresentation(isEnabled: true, state: .on))

        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(
            try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("**alpha** beta".utf8)
        )
        surface.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(
            try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("**alpha b**eta".utf8)
        )
    }

    @MainActor
    func testSemanticStyleMenusEnableMarkdownCaretAndDisablePlainTextAndVisualBlockSelections() throws {
        do {
            let (_, surface, _) = try makeMarkdownSurface("alpha")

            XCTAssertEqual(surface.presentation(for: .bold), EVMenuItemPresentation(isEnabled: true, state: .off))
            XCTAssertEqual(surface.presentation(for: .italic), EVMenuItemPresentation(isEnabled: true, state: .off))
        }

        do {
            let (_, surface, session) = try makeSurface("alpha")
            try send(Array("v4l"), through: session)
            surface.refreshPresentation()

            XCTAssertFalse(surface.presentation(for: .bold).isEnabled)
            XCTAssertFalse(surface.presentation(for: .italic).isEnabled)
        }

        do {
            let (_, surface, session) = try makeMarkdownSurface("ab\ncd")
            try enterVisualBlock(through: session, extendRight: true)
            surface.refreshPresentation()

            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_BLOCK))
            XCTAssertFalse(surface.presentation(for: .bold).isEnabled)
            XCTAssertFalse(surface.presentation(for: .italic).isEnabled)
        }
    }

    @MainActor
    func testSemanticStyleMutationRejectsAChangedLogicalSelectionIdentity() throws {
        let (backend, surface, session) = try makeMarkdownSurface("alpha beta")
        try send(Array("v4l"), through: session)
        surface.refreshPresentation()
        let stale = try session.semanticStylePresentation(
            UInt32(VIEM_SEMANTIC_STYLE_STRONG)
        ).selection

        _ = try session.sendText("l")
        surface.refreshPresentation()

        XCTAssertThrowsError(
            try session.setSemanticStyle(
                UInt32(VIEM_SEMANTIC_STYLE_STRONG),
                enabled: true,
                expected: stale
            )
        ) { error in
            guard case let EVCoreFrontendError.core(_, status) = error else {
                return XCTFail("unexpected stale-selection error: \(error)")
            }
            XCTAssertEqual(status, UInt32(VIEM_STATUS_STALE_REVISION))
        }
        XCTAssertEqual(
            try backend.serializedSource(typeName: EVDocument.markdownType),
            Data("alpha beta".utf8)
        )
        XCTAssertFalse(surface.canUndo)
    }

    @MainActor
    func testMarkdownSemanticStyleMenuUsesLogicalSelectionWhenGeometryIsOffscreen() throws {
        let source = (0 ..< 300).map { "row \($0)" }.joined(separator: "\n\n")
        let (backend, surface, session) = try makeMarkdownSurface(source)
        surface.view.frame = NSRect(x: 0, y: 0, width: 320, height: 100)
        surface.viewDidLayout()

        try send(Array("v4l"), through: session)
        surface.refreshPresentation()
        _ = try session.setViewportOrigin(
            left: 0,
            top: 3_000,
            expected: surface.viewportState
        )
        surface.refreshPresentation()

        XCTAssertEqual(surface.visualSelection?.rectangles.isEmpty, true)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_CHARACTER))
        XCTAssertEqual(surface.presentation(for: .bold).state, .off)
        XCTAssertTrue(surface.presentation(for: .bold).isEnabled)

        surface.perform(menuCommand: .bold, sender: nil)

        XCTAssertEqual(surface.formattedText, source.replacingOccurrences(of: "\n\n", with: "\n"))
        let expectedSource = "**row 0**" + String(source.dropFirst(5))
        XCTAssertEqual(
            try backend.serializedSource(typeName: EVDocument.markdownType),
            Data(expectedSource.utf8)
        )
        XCTAssertEqual(surface.presentation(for: .bold).state, .on)
    }

    @MainActor
    func testUnsupportedFormattingAndHostEffectItemsValidateDisabled() throws {
        let (_, surface, _) = try makeSurface("text")
        for command in [
            EVMenuCommand.bold,
            .italic,
            .printDocument,
        ] {
            XCTAssertFalse(surface.presentation(for: command).isEnabled, "\(command) must not beep")
        }
    }

    @MainActor
    private func makeSurface(
        _ text: String
    ) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(text.utf8), typeName: "public.plain-text")
        let pair = try makeSurface(backend)
        return (backend, pair.surface, pair.session)
    }

    @MainActor
    private func makeMarkdownSurface(
        _ source: String
    ) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownType)
        let pair = try makeSurface(backend)
        return (backend, pair.surface, pair.session)
    }

    @MainActor
    private func makeSurface(
        _ backend: EVCoreDocumentBackend
    ) throws -> (surface: EVEditorSurfaceController, session: EVCoreViewSession) {
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 520, height: 260)
        surface.viewDidLayout()
        return (surface, try XCTUnwrap(surface.session))
    }

    @MainActor
    private func send(_ characters: [Character], through session: EVCoreViewSession) throws {
        for character in characters {
            _ = try session.sendText(String(character))
        }
    }

    @MainActor
    private func enterVisualBlock(
        through session: EVCoreViewSession,
        extendRight: Bool
    ) throws {
        _ = try session.sendKey(
            kind: UInt32(VIEM_KEY_CONTROL_CHARACTER),
            codepoint: UInt32(Character("v").asciiValue!)
        )
        if extendRight {
            _ = try session.sendKey(
                kind: UInt32(VIEM_KEY_CHARACTER),
                codepoint: UInt32(Character("l").asciiValue!)
            )
        }
        _ = try session.sendKey(
            kind: UInt32(VIEM_KEY_CHARACTER),
            codepoint: UInt32(Character("j").asciiValue!)
        )
    }
}
