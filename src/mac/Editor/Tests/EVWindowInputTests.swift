import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVWindowInputTests: XCTestCase {
    private let noReplacement = NSRange(location: NSNotFound, length: 0)

    private func fixture() throws -> (EVCoreDocumentBackend, EVDocument, EVDocumentWindowController) {
        EVFrontendRegistry.install { EVCoreDocumentBackend() }
        let backend = EVCoreDocumentBackend()
        let document = EVDocument(editorBackend: backend)
        try document.read(from: Data("first line\nsecond line\nthird".utf8), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        document.makeWindowControllers()
        let window = try XCTUnwrap(document.windowControllers.first as? EVDocumentWindowController)
        window.showWindow(nil)
        return (backend, document, window)
    }

    private func surface(_ window: EVDocumentWindowController) throws -> EVEditorSurfaceController {
        try XCTUnwrap(window.editorSurface as? EVEditorSurfaceController)
    }

    private func control(_ letter: String, in surface: EVEditorSurfaceController, keyCode: UInt16 = 0) throws {
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: [.control], timestamp: 0, windowNumber: 0, context: nil,
            characters: letter, charactersIgnoringModifiers: letter, isARepeat: false, keyCode: keyCode))
        surface.editorView.keyDown(with: event)
    }

    private func type(_ text: String, in surface: EVEditorSurfaceController) {
        surface.editorView.insertText(text, replacementRange: noReplacement)
    }

    func testHeightUnitUsesBaseParagraphSpacingRatherThanMixedFontsAndIncludesZoom() throws {
        for typeName in [EVDocument.plainTextType, EVDocument.markdownType] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data("# Large title\n\nBody".utf8), typeName: typeName)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            let session = try XCTUnwrap(surface.session)
            func edit(_ key: EVStyleKey, _ mutation: EVStyleMutation) throws {
                _ = try session.editStyle(key: key, expected: backend.styleSheetSnapshot().identity, mutation: mutation)
                surface.refreshPresentation()
            }
            try edit(.baseParagraph, .setDeclaration(.characterSize, .float(20)))
            try edit(.baseParagraph, .setDeclaration(.paragraphLineSpacing, .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER), value: 1.5))))
            if typeName == EVDocument.markdownType {
                try edit(EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1")), .setDeclaration(.characterSize, .float(80)))
            }
            XCTAssertEqual(try XCTUnwrap(surface.defaultLineHeight), 30, accuracy: 0.001)
            _ = try session.setScale(2)
            surface.refreshPresentation()
            XCTAssertEqual(try XCTUnwrap(surface.defaultLineHeight), 60, accuracy: 0.001)
            XCTAssertEqual(try backend.serializedSource(typeName: typeName), Data("# Large title\n\nBody".utf8))
        }
    }

    func testCodeHeightUnitUsesTheCurrentCodeSheetAndViewZoom() throws {
        let original = try EVCodeStyleSession.exportGlobalJSON()
        defer {
            _ = original.withUnsafeBytes { bytes in
                viem_code_replace_style_json(bytes.bindMemory(to: UInt8.self).baseAddress, UInt64(bytes.count))
            }
        }
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("code".utf8), typeName: EVDocument.codeType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        for mutation in [EVStyleMutation.setDeclaration(.characterSize, .float(24)),
            .setDeclaration(.paragraphLineSpacing, .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER), value: 1.5)))] {
            try EVCoreStyleBridge.applyCodeStyle(key: .baseParagraph, expected: EVCoreStyleBridge.copyStyleSheet(core: nil).identity, mutation: mutation)
        }
        XCTAssertEqual(try XCTUnwrap(surface.defaultLineHeight), 36, accuracy: 0.001)
        _ = try session.setScale(2)
        XCTAssertEqual(try XCTUnwrap(surface.defaultLineHeight), 72, accuracy: 0.001)
    }

    func testCountedSplitAndNewPaneKeepTheirBufferAndSetEditorHeightInRows() throws {
        let (backend, document, window) = try fixture()
        defer { window.close(); document.close() }
        let original = try surface(window)
        type("2", in: original)
        try control("w", in: original)
        type("3", in: original)
        try control("v", in: original)
        XCTAssertEqual(window.paneCount, 2, "CTRL-W CTRL-V must split rather than enter Visual Block")
        let split = try surface(window)
        XCTAssertTrue(split.backend === backend)
        XCTAssertFalse(split === original)
        XCTAssertEqual(split.view.superview!.bounds.width, 100, accuracy: 2)
        XCTAssertEqual(original.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))

        type("iX", in: split)
        _ = try XCTUnwrap(split.session).sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        split.refreshPresentation()
        XCTAssertTrue(backend.persistenceState.isDirty)
        try control("w", in: split)
        type("5", in: split)
        try control("n", in: split)
        XCTAssertEqual(window.paneCount, 3, "new pane keeps even an unsaved current buffer open")
        let fresh = try surface(window)
        let freshDocument = try XCTUnwrap(window.activeDocument)
        defer { freshDocument.close() }
        XCTAssertFalse(fresh.backend === backend)
        XCTAssertEqual(try fresh.backend.formattedText(), "")
        XCTAssertEqual(try backend.formattedText(), "Xfirst line\nsecond line\nthird")
        XCTAssertEqual(fresh.editorView.bounds.height, 5 * (try XCTUnwrap(fresh.defaultLineHeight)), accuracy: 2)
    }

    func testWindowControlAliasesCountsAndCancelSurviveNativeRouting() throws {
        let (_, document, window) = try fixture()
        defer { window.close(); document.close() }
        let original = try surface(window)
        try control("w", in: original)
        type("s", in: original)
        let split = try surface(window)
        for letter in ["h", "l"] {
            try control("w", in: split)
            try control(letter, in: split)
            XCTAssertTrue(window.editorSurface === split)
            XCTAssertNil(split.commandOutput)
        }
        try control("w", in: split)
        let backspace = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
            characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: 51))
        split.editorView.keyDown(with: backspace)
        XCTAssertTrue(window.editorSurface === split)
        XCTAssertNil(split.commandOutput)

        try control("w", in: split)
        type("4", in: split)
        try control("_", in: split)
        XCTAssertEqual(split.editorView.bounds.height, 4 * (try XCTUnwrap(split.defaultLineHeight)), accuracy: 2)
        try control("w", in: split)
        type("2", in: split)
        try control("c", in: split)
        XCTAssertEqual(window.paneCount, 2)
        XCTAssertNil(split.commandOutput)
        try control("w", in: split)
        type("1w", in: split)
        XCTAssertTrue(window.editorSurface === original)
    }

    func testVerticalExSplitsArrowFocusWidthCountsAndIndexedResizeKeepSource() throws {
        let (backend, document, window) = try fixture()
        defer { window.close(); document.close() }
        window.window?.setContentSize(NSSize(width:920,height:680))
        func ex(_ command: String, in target: EVEditorSurfaceController) throws {
            type(":" + command, in: target)
            let enter = try XCTUnwrap(NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[],timestamp:0,windowNumber:0,context:nil,characters:"\r",charactersIgnoringModifiers:"\r",isARepeat:false,keyCode:36))
            target.editorView.keyDown(with:enter)
        }
        let original = try surface(window)
        let bytes = try backend.serializedSource(typeName:EVDocument.plainTextType)
        let revision = backend.persistenceState.documentRevision
        try ex("vs",in:original)
        let right = try surface(window)
        XCTAssertFalse(right === original)
        XCTAssertTrue(right.backend === backend)
        try control("w",in:right)
        let left = try XCTUnwrap(NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[],timestamp:0,windowNumber:0,context:nil,characters:"\u{f702}",charactersIgnoringModifiers:"\u{f702}",isARepeat:false,keyCode:123))
        right.editorView.keyDown(with:left)
        XCTAssertTrue(window.editorSurface === original)
        let unit = try XCTUnwrap(right.defaultColumnWidth)
        try ex("vert 2resize 24",in:original)
        XCTAssertTrue(window.editorSurface === original,"indexed resizing does not change focus")
        XCTAssertEqual(right.view.superview!.bounds.width,max(100,unit*24),accuracy:1)
        try ex("vert resize +3",in:original)
        XCTAssertNil(original.commandOutput)
        try ex("wincmd l",in:original)
        XCTAssertTrue(window.editorSurface === right)
        try ex("new",in:right)
        let empty = try surface(window)
        let emptyDocument = try XCTUnwrap(window.activeDocument)
        defer { emptyDocument.close() }
        XCTAssertEqual(window.paneCount,3)
        XCTAssertFalse(empty.backend === backend)
        try ex("resize 0",in:empty)
        XCTAssertEqual(empty.view.bounds.height,0,accuracy:1)
        try ex("wincmd K",in:empty)
        XCTAssertEqual(empty.view.superview!.bounds.width,920,accuracy:1)
        XCTAssertEqual(try backend.serializedSource(typeName:EVDocument.plainTextType),bytes)
        XCTAssertEqual(backend.persistenceState.documentRevision,revision)
    }

    func testSplittingACollapsedPaneReportsNoRoomWithoutChangingTheBuffer() throws {
        let (backend, document, window) = try fixture()
        defer { window.close(); document.close() }
        let original = try surface(window)
        try control("w", in: original)
        type("s", in: original)
        let collapsed = try surface(window)
        try control("w", in: collapsed)
        type("100000-", in: collapsed)
        XCTAssertEqual(collapsed.editorView.bounds.height, 0, accuracy: 1)
        let before = backend.persistenceState
        try control("w", in: collapsed)
        type("s", in: collapsed)
        XCTAssertEqual(window.paneCount, 2)
        XCTAssertTrue(window.editorSurface === collapsed)
        XCTAssertEqual(collapsed.commandOutput, "No room to split the current view.")
        XCTAssertEqual(backend.persistenceState.documentRevision, before.documentRevision)
        XCTAssertEqual(backend.persistenceState.isDirty, before.isDirty)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data("first line\nsecond line\nthird".utf8))
    }

    func testVisualBlockWindowPrefixKeepsSelectionAndColonOpensItsRange() throws {
        let (_, document, window) = try fixture()
        defer { window.close(); document.close() }
        let original = try surface(window)
        try control("v", in: original)
        type("jl", in: original)
        XCTAssertEqual(original.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_BLOCK))
        let cursor = original.viewPresentation.cursor_utf8_offset
        try control("w", in: original)
        try control("c", in: original)
        XCTAssertEqual(original.viewPresentation.mode, UInt32(VIEM_MODE_VISUAL_BLOCK))
        XCTAssertEqual(original.viewPresentation.cursor_utf8_offset, cursor)
        try control("w", in: original)
        type(":", in: original)
        XCTAssertEqual(original.statusBarState.commandLine?.text, "'<,'>")
        XCTAssertEqual(original.viewPresentation.mode, UInt32(VIEM_MODE_COMMAND_LINE))
        XCTAssertNil(original.commandOutput)
    }
}
