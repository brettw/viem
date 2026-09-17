import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVFunctionKeyMappingTests: XCTestCase {
    private func fixture(startup: String) throws
        -> (EVCoreDocumentBackend, EVDocument, EVDocumentWindowController, EVEditorSurfaceController)
    {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-function-mappings-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data(startup.utf8).write(to: directory.appendingPathComponent("startup.viem"))
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let document = EVDocument(editorBackend: backend)
        try document.read(from: Data("first line\nsecond line".utf8), ofType: EVDocument.plainTextType)
        document.fileType = EVDocument.plainTextType
        document.makeWindowControllers()
        let controller = try XCTUnwrap(document.windowControllers.first as? EVDocumentWindowController)
        controller.showWindow(nil)
        let surface = try XCTUnwrap(controller.editorSurface as? EVEditorSurfaceController)
        let window = try XCTUnwrap(controller.window)
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        return (backend, document, controller, surface)
    }

    private func functionKey(_ number: UInt32, modifiers: NSEvent.ModifierFlags = [],
                             window: NSWindow? = nil) throws -> NSEvent {
        let character = String(try XCTUnwrap(UnicodeScalar(UInt32(NSF1FunctionKey) + number - 1)))
        return try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: modifiers.union(.function), timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window?.windowNumber ?? 0, context: nil,
            characters: character, charactersIgnoringModifiers: character,
            isARepeat: false, keyCode: number == 2 ? 120 : 0))
    }

    func testStartupControlF2MappingLeavesExCommandReadyForReturn() throws {
        let (backend, document, controller, surface) = try fixture(startup: "map <C-F2> :sp\n")
        defer { controller.close(); document.close() }
        let window = try XCTUnwrap(controller.window)

        window.sendEvent(try functionKey(2, modifiers: [.control], window: window))

        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_COMMAND_LINE))
        XCTAssertEqual(surface.commandLine?.text, "sp")
        XCTAssertEqual(controller.paneCount, 1)
        XCTAssertEqual(try backend.formattedText(), "first line\nsecond line")
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertNil(surface.commandOutput)
    }

    func testStartupControlF2MappingWithReturnSplitsTheNativeWindow() throws {
        let (backend, document, controller, surface) = try fixture(startup: "map <C-F2> :sp<CR>\n")
        defer { controller.close(); document.close() }
        let window = try XCTUnwrap(controller.window)

        window.sendEvent(try functionKey(2, modifiers: [.control], window: window))

        XCTAssertEqual(controller.paneCount, 2)
        let split = try XCTUnwrap(controller.editorSurface as? EVEditorSurfaceController)
        XCTAssertFalse(split === surface)
        XCTAssertTrue(split.backend === backend)
        XCTAssertEqual(split.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertEqual(try backend.formattedText(), "first line\nsecond line")
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertNil(split.commandOutput)
    }

    func testUnmodifiedFunctionKeyIsMappedInsteadOfInsertedAsPrivateUseText() throws {
        let (backend, document, controller, surface) = try fixture(startup: "map <F2> l\n")
        defer { controller.close(); document.close() }

        surface.editorView.keyDown(with: try functionKey(2))

        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 1)
        XCTAssertEqual(try backend.formattedText(), "first line\nsecond line")
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertNil(surface.commandOutput)
    }

    func testFunctionModifiersAndHighFunctionNumbersReachTheirDistinctMappings() throws {
        let startup = """
        map <F2> l
        map <S-F2> 2l
        map <C-F2> 3l
        map <A-F2> 4l
        map <D-F2> 5l
        map <S-C-A-D-F35> 6l
        """
        let (backend, document, controller, surface) = try fixture(startup: startup)
        defer { controller.close(); document.close() }
        let inputs: [(UInt32, NSEvent.ModifierFlags, UInt64)] = [
            (2, [], 1), (2, [.shift], 2), (2, [.control], 3),
            (2, [.option], 4), (2, [.command], 5),
            (35, [.shift, .control, .option, .command], 6),
        ]
        for (number, modifiers, expectedOffset) in inputs {
            surface.editorView.insertText("0", replacementRange: NSRange(location: NSNotFound, length: 0))
            surface.editorView.keyDown(with: try functionKey(number, modifiers: modifiers))
            XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, expectedOffset)
            XCTAssertNil(surface.commandOutput)
        }
        XCTAssertEqual(try backend.formattedText(), "first line\nsecond line")
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    func testQuotedFunctionKeyKeepsLiteralCaptureAheadOfMappings() throws {
        let (backend, document, controller, surface) = try fixture(startup: "map <C-F2> :sp<CR>\n")
        defer { controller.close(); document.close() }
        let session = try XCTUnwrap(surface.session)
        surface.performInput {
            _ = try session.sendText("i")
            _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 118)
        }

        surface.editorView.keyDown(with: try functionKey(2, modifiers: [.control]))

        XCTAssertEqual(try backend.formattedText(), "<C-F2>first line\nsecond line")
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(surface.viewPresentation.flags & UInt32(VIEM_VIEW_PRESENTATION_LITERAL_INPUT_PENDING), 0)
        XCTAssertEqual(controller.paneCount, 1)
        XCTAssertNil(surface.commandOutput)
    }

    func testMarkedTextKeepsFunctionKeysInTheInputMethod() throws {
        let (backend, document, controller, surface) = try fixture(startup: "map <C-F2> :sp<CR>\n")
        defer { controller.close(); document.close() }
        let view = FunctionKeyCompositionView(surface: surface)
        view.setMarkedText("かな", selectedRange: NSRange(location: 2, length: 0),
                           replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(view.hasMarkedText())

        view.keyDown(with: try functionKey(2, modifiers: [.control]))

        XCTAssertEqual(view.handledFunctionKeyCount, 1)
        XCTAssertTrue(view.hasMarkedText())
        XCTAssertEqual(controller.paneCount, 1)
        XCTAssertEqual(try backend.formattedText(), "first line\nsecond line")
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    func testCommandLineTypingMapsAndReplacesTheCurrentSelection() throws {
        let (backend, document, controller, surface) = try fixture(startup: "cmap x hello\n")
        defer { controller.close(); document.close() }
        let view = surface.editorView
        view.insertText(":abc", replacementRange: NSRange(location: NSNotFound, length: 0))
        view.selectAll(nil)
        XCTAssertEqual(surface.commandLine?.selectedUTF8Range, 0..<3)
        let window = try XCTUnwrap(controller.window)
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
            modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window.windowNumber, context: nil, characters: "x",
            charactersIgnoringModifiers: "x", isARepeat: false, keyCode: 7))

        window.sendEvent(event)

        XCTAssertEqual(surface.commandLine?.text, "hello")
        XCTAssertEqual(surface.commandLine?.selectedUTF8Range, 5..<5)
        XCTAssertEqual(try backend.formattedText(), "first line\nsecond line")
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertNil(surface.commandOutput)
    }

    func testCommandLineExplicitReplacementAndIMECommitKeepTheirLiteralPayloads() throws {
        let (backend, document, controller, surface) = try fixture(startup: "cmap x hello\n")
        defer { controller.close(); document.close() }
        let view = surface.editorView
        view.insertText(":abc", replacementRange: NSRange(location: NSNotFound, length: 0))

        view.insertText("x", replacementRange: NSRange(location: 0, length: 3))
        XCTAssertEqual(surface.commandLine?.text, "x")
        view.setMarkedText("x", selectedRange: NSRange(location: 1, length: 0),
                           replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(view.hasMarkedText())
        view.insertText("x", replacementRange: NSRange(location: NSNotFound, length: 0))

        XCTAssertFalse(view.hasMarkedText())
        XCTAssertEqual(surface.commandLine?.text, "xx")
        XCTAssertEqual(try backend.formattedText(), "first line\nsecond line")
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertNil(surface.commandOutput)
    }
}

@MainActor
private final class FunctionKeyCompositionView: EVEditorView {
    private lazy var compositionInputContext = FunctionKeyCompositionContext(client: self)
    override var inputContext: NSTextInputContext? { compositionInputContext }
    var handledFunctionKeyCount: Int { compositionInputContext.handledFunctionKeyCount }
}

@MainActor
private final class FunctionKeyCompositionContext: NSTextInputContext {
    private(set) var handledFunctionKeyCount = 0
    override func handleEvent(_ event: NSEvent) -> Bool {
        handledFunctionKeyCount += 1
        return true
    }
}
