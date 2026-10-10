import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVPendingTypingStyleTests: XCTestCase {
  func testCommandFormattingShortcutsWorkWithoutMenuItemsAndRespectFocus() throws {
    for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
      var shortcuts: [(String, String, UInt16, NSEvent.ModifierFlags, EVMenuCommand)] = [
        ("b", "b", 11, [.command], .bold), ("i", "i", 34, [.command], .italic),
      ]
      if type == EVDocument.markdownType {
        shortcuts.append(("c", "c", 8, [.option, .command], .characterCode))
      } else {
        shortcuts.append(("+", "=", 24, [.control, .command, .shift], .superscript))
        shortcuts.append(("*", "*", 28, [.shift, .command], .bulletedList))
      }
      for (characters, key, code, modifiers, command) in shortcuts {
        let source = Data("word".utf8)
        let backend = EVCoreDocumentBackend()
        try backend.read(source: source, typeName: type)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let window = EVTestFocusWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 400),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = surface
        window.setKeyWindowForTesting(true)
        defer { window.close() }
        XCTAssertTrue(window.makeFirstResponder(surface.editorView))
        surface.formattingToolbar.isHidden = true
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 4))
        let event = try XCTUnwrap(NSEvent.keyEvent(
          with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: 0,
          windowNumber: window.windowNumber, context: nil, characters: characters,
          charactersIgnoringModifiers: key, isARepeat: false, keyCode: code))
        XCTAssertTrue(surface.editorView.performKeyEquivalent(with: event), type)
        XCTAssertEqual(surface.presentation(for: command).state, .on, type)
        XCTAssertNotEqual(try backend.serializedSource(typeName: type), source, type)
        let menuItem = NSMenuItem(title: "Formatting",
          action: #selector(EVFormattingCommandRouting.performEditorFormattingCommand(_:)), keyEquivalent: "")
        menuItem.tag = command.rawValue
        XCTAssertTrue(surface.editorView.validateMenuItem(menuItem), type)
        XCTAssertTrue(try XCTUnwrap(window.firstResponder).tryToPerform(try XCTUnwrap(menuItem.action), with: menuItem))
        XCTAssertEqual(surface.presentation(for: command).state, .off, type)
        XCTAssertEqual(try backend.serializedSource(typeName: type), source, type)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(surface.presentation(for: command).state, .on, type)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: type), source, type)

        let field = NSTextField(string: "style name")
        surface.view.addSubview(field)
        XCTAssertTrue(window.makeFirstResponder(field))
        XCTAssertFalse(surface.editorView.performKeyEquivalent(with: event), type)
        XCTAssertFalse(surface.editorView.validateMenuItem(menuItem), type)
        _ = try XCTUnwrap(window.firstResponder).tryToPerform(try XCTUnwrap(menuItem.action), with: menuItem)
        XCTAssertEqual(try backend.serializedSource(typeName: type), source, type)
      }
    }
  }

  func testFormattingAcceleratorsLeavePendingCommandsAndCompositionUntouched() throws {
    for pending in ["operator", "literal", "prompt", "composition"] {
      let backend = EVCoreDocumentBackend()
      let source = Data("word".utf8)
      try backend.read(source: source, typeName: EVDocument.markdownType)
      let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
      let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 400),
                            styleMask: [.titled], backing: .buffered, defer: false)
      window.isReleasedWhenClosed = false
      window.contentViewController = surface
      defer { window.close() }
      XCTAssertTrue(window.makeFirstResponder(surface.editorView))
      let session = try XCTUnwrap(surface.session)
      switch pending {
      case "operator": surface.performInput { _ = try session.sendText("d") }
      case "literal":
        surface.performInput {
          _ = try session.sendText("i")
          _ = try session.sendKey(kind: UInt32(VIEM_KEY_CONTROL_CHARACTER), codepoint: 118)
        }
      case "prompt": surface.performInput { _ = try session.sendText(":") }
      default:
        surface.performInput { _ = try session.sendText("i") }
        surface.editorView.setMarkedText("draft", selectedRange: NSRange(location: 5, length: 0),
          replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(surface.editorView.hasMarkedText())
      }
      let before = surface.viewPresentation
      let command = EVMenuCommand.characterCode
      let shortcut = try XCTUnwrap(command.formattingShortcut)
      let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
        modifierFlags: shortcut.modifiers, timestamp: 0, windowNumber: window.windowNumber, context: nil,
        characters: shortcut.key, charactersIgnoringModifiers: shortcut.key, isARepeat: false,
        keyCode: 8))
      XCTAssertFalse(surface.editorView.performKeyEquivalent(with: event), "\(pending): \(command)")
      let item = NSMenuItem(title: "Formatting",
        action: #selector(EVFormattingCommandRouting.performEditorFormattingCommand(_:)), keyEquivalent: shortcut.key)
      item.tag = command.rawValue
      XCTAssertFalse(surface.editorView.validateMenuItem(item), "\(pending): \(command)")
      surface.editorView.performEditorFormattingCommand(item)
      XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source, "\(pending): \(command)")
      XCTAssertEqual(surface.viewPresentation.mode, before.mode, "\(pending): \(command)")
      XCTAssertEqual(surface.viewPresentation.flags, before.flags, "\(pending): \(command)")
      if pending == "composition" { XCTAssertTrue(surface.editorView.hasMarkedText()) }
    }
  }

  func testUnavailableFormattingAcceleratorsDoNotMutateSourceOrInvokeHeadings() throws {
    for (type, source, readOnly, command) in [
      (EVDocument.markdownType, "word", true, EVMenuCommand.characterCode),
      (EVDocument.markdownSourceType, "word", true, .bold),
      (EVDocument.markdownType, "```\nword\n```", false, .superscript),
      (EVDocument.plainTextType, "word", false, .bulletedList),
      (EVDocument.codeType, "word", false, .characterCode),
    ] {
      let backend = EVCoreDocumentBackend()
      try backend.read(source: Data(source.utf8), typeName: type)
      try backend.setReadOnly(readOnly)
      let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
      let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 400),
                            styleMask: [.titled], backing: .buffered, defer: false)
      window.isReleasedWhenClosed = false
      window.contentViewController = surface
      defer { window.close() }
      XCTAssertTrue(window.makeFirstResponder(surface.editorView))
      let text = try backend.formattedText() as NSString
      surface.editorView.setAccessibilitySelectedTextRange(text.range(of: "word"))
      XCTAssertEqual(surface.documentState.flags & UInt32(VIEM_DOCUMENT_STATE_READ_ONLY) != 0, readOnly)
      let shortcut = try XCTUnwrap(command.formattingShortcut)
      let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
        modifierFlags: shortcut.modifiers, timestamp: 0, windowNumber: window.windowNumber, context: nil,
        characters: shortcut.key, charactersIgnoringModifiers: shortcut.key, isARepeat: false,
        keyCode: command == .bulletedList ? 28 : command == .numberedList ? 26 : 0))
      XCTAssertFalse(surface.presentation(for: command).isEnabled, "\(type): \(command)")
      XCTAssertTrue(surface.editorView.performKeyEquivalent(with: event), "\(type): \(command)")
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8), "\(type): \(command)")
      XCTAssertFalse(surface.canUndo)
    }
  }

  func testFormattingMenuCannotFallThroughANativeFieldInAnotherWindow() throws {
    let backend = EVCoreDocumentBackend()
    let source = Data("word".utf8)
    try backend.read(source: source, typeName: EVDocument.markdownType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    let documentWindow = EVTestFocusWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 400),
                                         styleMask: [.titled], backing: .buffered, defer: false)
    documentWindow.isReleasedWhenClosed = false
    documentWindow.contentViewController = surface
    defer { documentWindow.close() }
    documentWindow.setKeyWindowForTesting(true)
    XCTAssertTrue(documentWindow.makeFirstResponder(surface.editorView))
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 4))
    let item = NSMenuItem(title: "Bold", action: #selector(EVFormattingCommandRouting.performEditorFormattingCommand(_:)), keyEquivalent: "b")
    item.tag = EVMenuCommand.bold.rawValue
    XCTAssertTrue(surface.editorView.validateMenuItem(item))

    let settingsWindow = EVTestFocusWindow(contentRect: NSRect(x: 0, y: 0, width: 300, height: 150),
                                         styleMask: [.titled], backing: .buffered, defer: false)
    settingsWindow.isReleasedWhenClosed = false
    defer { settingsWindow.close() }
    let field = NSTextField(string: "style name")
    settingsWindow.contentView?.addSubview(field)
    documentWindow.setKeyWindowForTesting(false)
    settingsWindow.setKeyWindowForTesting(true)
    XCTAssertTrue(settingsWindow.makeFirstResponder(field))
    XCTAssertTrue(surface.editorView.isActiveTextSurface, "The main window retains its first responder while a separate panel has focus")
    XCTAssertFalse(surface.editorView.validateMenuItem(item))
    surface.editorView.performEditorFormattingCommand(item)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
  }

  func testOptionItalicKeepsSourceCleanUntilTypedContentAndUndoIsExact() throws {
    for (type, source) in [
      (EVDocument.markdownType, "word"), (EVDocument.markdownSourceType, "word"),
    ] {
      let backend = EVCoreDocumentBackend()
      try backend.read(source: Data(source.utf8), typeName: type)
      let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
      surface.loadViewIfNeeded()
      let session = try XCTUnwrap(surface.session)
      surface.performInput { _ = try session.sendText("i") }
      let revision = try backend.documentState().document_revision
      let event = try XCTUnwrap(
        NSEvent.keyEvent(
          with: .keyDown, location: .zero, modifierFlags: .option, timestamp: 0, windowNumber: 0,
          context: nil, characters: "ˆ", charactersIgnoringModifiers: "i", isARepeat: false,
          keyCode: 34))
      surface.editorView.keyDown(with: event)
      XCTAssertEqual(surface.presentation(for: .italic).state, .on, type)
      XCTAssertEqual(try backend.documentState().document_revision, revision, type)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      let implicit = NSRange(location: NSNotFound, length: 0)
      surface.editorView.insertText("A", replacementRange: implicit)
      surface.editorView.insertText("b", replacementRange: implicit)
      XCTAssertEqual(surface.statusBarState.message, "", type)
      XCTAssertEqual(surface.presentation(for: .italic).state, .on, type)
      XCTAssertNotEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8), type)
    }
  }

}
