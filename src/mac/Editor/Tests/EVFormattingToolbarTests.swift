import AppKit
import CViemCore
@testable import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVFormattingToolbarTests: XCTestCase {
  private func surface(_ source: String, type: String? = nil) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-toolbar-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
    let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory))
    try backend.read(source: Data(source.utf8), typeName: type ?? EVDocument.markdownType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    surface.view.frame = NSRect(x: 0, y: 0, width: 900, height: 300)
    surface.viewDidLayout()
    return (backend, surface)
  }

  private func choose(_ id: String, from popup: NSPopUpButton, toolbar: EVFormattingToolbarView) throws {
    let item = try XCTUnwrap(popup.itemArray.first { ($0.representedObject as? EVStyleMenuAction)?.stableID == id })
    popup.select(item)
    toolbar.chooseStyle(popup)
  }

  func testFormattedViewTogglePreservesSourceTracksSharedHistoryAndRestoresFocus() throws {
    let source = Data("# Heading\n\n**Words**".utf8)
    let (backend, first) = try surface(String(decoding: source, as: UTF8.self), type: EVDocument.markdownSourceType)
    let second = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    let document = EVDocument(editorBackend: backend)
    let firstWindow = EVDocumentWindowController(document: document, editorSurface: first, configuration: backend.configuration)
    let secondWindow = EVDocumentWindowController(document: document, editorSurface: second, configuration: backend.configuration)
    defer { firstWindow.close(); secondWindow.close() }
    firstWindow.showWindow(nil)
    secondWindow.showWindow(nil)
    let button = first.formattingToolbar.formattedView
    let otherButton = second.formattingToolbar.formattedView
    let image = try XCTUnwrap(button.image)
    XCTAssertEqual(button.state, .off)
    XCTAssertEqual(button.accessibilityLabel(), "Formatted view")
    XCTAssertEqual(button.toolTip, "Formatted view (WYSIWYG)")
    XCTAssertFalse(button.allowsMixedState)
    XCTAssertTrue(button.isEnabled)

    button.performClick(nil)
    XCTAssertEqual(backend.sourceFormat, .markdown)
    XCTAssertEqual(button.state, .on)
    XCTAssertEqual(otherButton.state, .on)
    XCTAssertEqual(try backend.formattedText(), "Heading\nWords")
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
    XCTAssertTrue(firstWindow.window?.firstResponder === first.editorView)
    XCTAssertTrue(button.image === image)

    second.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(backend.sourceFormat, .markdownSource)
    XCTAssertEqual(button.state, .off)
    XCTAssertEqual(otherButton.state, .off)
    second.perform(menuCommand: .redo, sender: nil)
    XCTAssertEqual(button.state, .on)
    XCTAssertEqual(otherButton.state, .on)
    button.performClick(nil)
    XCTAssertEqual(backend.sourceFormat, .markdownSource)
    XCTAssertEqual(button.state, .off)
    XCTAssertEqual(otherButton.state, .off)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), source)

    let wasVisible = backend.configuration.showFormattingToolbar(for: .markdown)
    defer { try? backend.configuration.setShowFormattingToolbar(wasVisible, for: .markdown) }
    try backend.configuration.setShowFormattingToolbar(false, for: .markdown)
    let field = NSTextField(string: "Another control")
    first.view.addSubview(field)
    XCTAssertTrue(firstWindow.window?.makeFirstResponder(field) == true)
    button.performClick(nil)
    XCTAssertNil(button.window, "The target mode retains its hidden-toolbar preference")
    XCTAssertTrue(firstWindow.window?.firstResponder === first.editorView)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
  }

  func testSelectorsTrackCaretMixedSelectionAssignmentAndUndo() throws {
    let source = "# Heading\n\nplain `code`"
    let (backend, surface) = try surface(source, type: EVDocument.markdownType)
    let toolbar = surface.formattingToolbar
    XCTAssertEqual(toolbar.paragraphStyle.titleOfSelectedItem, "Heading 1")
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 8, length: 0))
    toolbar.refresh()
    XCTAssertEqual(toolbar.paragraphStyle.titleOfSelectedItem, "Base Paragraph")
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 14, length: 4))
    toolbar.refresh()
    XCTAssertEqual(toolbar.characterStyle.titleOfSelectedItem, "Code")
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 18))
    toolbar.refresh()
    XCTAssertEqual(toolbar.paragraphStyle.titleOfSelectedItem, "Mixed")
    XCTAssertEqual(toolbar.characterStyle.titleOfSelectedItem, "Mixed")
    try choose("Heading2", from: toolbar.paragraphStyle, toolbar: toolbar)
    XCTAssertNil(surface.commandOutput)
    XCTAssertEqual(toolbar.paragraphStyle.titleOfSelectedItem, "Heading 2")
    let changed = try backend.serializedSource(typeName: EVDocument.markdownType)
    surface.perform(menuCommand: .undo, sender: nil)
    toolbar.refresh()
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    XCTAssertEqual(toolbar.paragraphStyle.titleOfSelectedItem, surface.currentStyleMenuCatalogue()?.entries.first {
      $0.role == .paragraph && $0.presentation.state == .on
    }?.displayName ?? "Mixed", "Undo restores the core's history caret/selection, which the selector follows")
    surface.perform(menuCommand: .redo, sender: nil)
    toolbar.refresh()
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), changed)
    XCTAssertEqual(toolbar.paragraphStyle.titleOfSelectedItem, "Heading 2")
  }

  func testCharacterCodeTogglesAndCodeBlockAvailability() throws {
    for (type, source) in [(EVDocument.markdownType, "Words")] {
      let (_, surface) = try surface(source, type: type)
      let toolbar = surface.formattingToolbar
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 5))
      toolbar.refresh()
      XCTAssertFalse(toolbar.characterCode.isHidden)
      toolbar.toggleCharacterCode(toolbar.characterCode)
      XCTAssertNil(surface.commandOutput)
      XCTAssertEqual(toolbar.characterCode.state, .on)
      XCTAssertEqual(toolbar.characterStyle.titleOfSelectedItem, "Code")
      toolbar.toggleCharacterCode(toolbar.characterCode)
      XCTAssertNil(surface.commandOutput)
      XCTAssertEqual(toolbar.characterCode.state, .off)
      XCTAssertEqual(toolbar.characterStyle.titleOfSelectedItem, "Default Paragraph")
      XCTAssertFalse(toolbar.codeBlock.isHidden)
    }
  }

  func testMarkdownCodeBlockToolbarTogglesAndRestoresExactSourceWithUndo() throws {
    for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
      let source = "Before\n\nWords\n\nAfter"
      let (backend, surface) = try surface(source, type: type)
      let toolbar = surface.formattingToolbar
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 8, length: 0))
      toolbar.refresh()
      XCTAssertFalse(toolbar.codeBlock.isHidden)
      toolbar.toggleCodeBlock(toolbar.codeBlock)
      XCTAssertNil(surface.commandOutput)
      XCTAssertEqual(toolbar.codeBlock.state, .on)
      let fenced = Data("Before\n\n```\nWords\n```\n\nAfter".utf8)
      XCTAssertEqual(try backend.serializedSource(typeName: type), fenced)
      toolbar.toggleCodeBlock(toolbar.codeBlock)
      XCTAssertNil(surface.commandOutput)
      XCTAssertEqual(toolbar.codeBlock.state, .off)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: type), fenced)
      surface.perform(menuCommand: .redo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
    }
  }

  func testBlockQuoteToolbarTogglesStructureAndPreservesInnerTreatments() throws {
    for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
      for source in ["Words", "# Heading", "- Item"] {
        let (backend, surface) = try surface(source, type: type)
        let toolbar = surface.formattingToolbar
        XCTAssertFalse(toolbar.blockQuote.isHidden)
        XCTAssertEqual(toolbar.blockQuote.state, .off)
        let group = try XCTUnwrap(toolbar.codeBlock.superview as? NSStackView)
        let index = try XCTUnwrap(group.arrangedSubviews.firstIndex(of: toolbar.codeBlock))
        XCTAssertTrue(group.arrangedSubviews[index - 1] === toolbar.blockQuote)
        toolbar.blockQuote.performClick(nil)
        XCTAssertNil(surface.commandOutput)
        XCTAssertEqual(toolbar.blockQuote.state, .on)
        let quoted = source.split(separator: "\n", omittingEmptySubsequences: false).map { "> \($0)" }.joined(separator: "\n")
        XCTAssertEqual(try backend.serializedSource(typeName: type), Data(quoted.utf8))
        toolbar.blockQuote.performClick(nil)
        XCTAssertNil(surface.commandOutput)
        XCTAssertEqual(toolbar.blockQuote.state, .off)
        XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: type), Data(quoted.utf8))
        surface.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      }
    }
    let (_, mixed) = try surface("> One\n\nTwo", type: EVDocument.markdownType)
    mixed.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 7))
    mixed.formattingToolbar.refresh()
    XCTAssertEqual(mixed.formattingToolbar.blockQuote.state, .mixed)
    mixed.formattingToolbar.blockQuote.performClick(nil)
    XCTAssertNil(mixed.commandOutput)
    XCTAssertEqual(mixed.formattingToolbar.blockQuote.state, .on)
  }

  func testSourceTableDelimiterAndPrefixesDisableBlockFormatting() throws {
    let source = "> | Quoted item | Value |\n> | --- | ---: |\n> | Inside the quotation | 7 |"
    let (backend, surface) = try surface(source, type: EVDocument.markdownSourceType)
    let toolbar = surface.formattingToolbar
    let text = try backend.formattedText() as NSString
    for offset in 0..<text.length {
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: offset, length: 0))
      toolbar.refresh()
      XCTAssertFalse(toolbar.paragraphStyle.isEnabled, "at \(offset)")
      for command in [EVMenuCommand.bulletedList, .numberedList, .increaseIndent, .decreaseIndent] {
        XCTAssertFalse(try XCTUnwrap(toolbar.commandButtons[command]).isEnabled, "\(command) at \(offset)")
        XCTAssertFalse(surface.presentation(for: command).isEnabled)
      }
      XCTAssertTrue(toolbar.codeBlock.isHidden)
      XCTAssertFalse(toolbar.blockQuote.isHidden)
      XCTAssertFalse(toolbar.blockQuote.isEnabled)
    }
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data(source.utf8))
    XCTAssertNil(surface.commandOutput)
  }

  func testBlockQuoteStaysVisibleButDisabledForCodeBlocksAndMixedSelections() throws {
    for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
      for code in ["```mermaid\ngraph LR\n    Writing --> Editing\n```", "    indented code", "> ```\n> quoted code\n> ```", "```\n\n```"] {
        let source = "Before\n\n\(code)\n\nAfter"
        let (backend, surface) = try surface(source, type: type)
        let toolbar = surface.formattingToolbar
        let text = try backend.formattedText() as NSString
        let start = text.range(of: "Before").length + 1
        let end = text.range(of: "After").location
        for range in [NSRange(location: start, length: 0), NSRange(location: 0, length: end)] {
          surface.editorView.setAccessibilitySelectedTextRange(range)
          toolbar.refresh()
          XCTAssertFalse(toolbar.blockQuote.isHidden, code)
          XCTAssertFalse(toolbar.blockQuote.isEnabled, code)
          toolbar.toggleBlockQuote(toolbar.blockQuote)
          XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
          XCTAssertFalse(surface.canUndo)
        }
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: text.range(of: "After").location, length: 0))
        toolbar.refresh()
        XCTAssertTrue(toolbar.blockQuote.isEnabled)
      }
    }
  }

  func testUndoInDemoMermaidBlockPreservesVisibleTextPosition() throws {
    let checkout = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
      .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
      .deletingLastPathComponent()
    let source = try String(contentsOf: checkout.appendingPathComponent("docs/markdown_demo.md"), encoding: .utf8)
    for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
      let (backend, surface) = try surface(source, type: type)
      let session = try XCTUnwrap(surface.session)
      let text = try backend.formattedText() as NSString
      let at = text.range(of: "Writing --> Editing").location
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: at, length: 0))
      func baseline() throws -> Float {
        let caret = try session.presentation().cursor_utf8_offset
        let row = try XCTUnwrap(try session.layoutExport().rows.first { $0.text_start <= caret && caret < $0.text_end })
        return row.baseline - surface.viewportState.top
      }
      let rowBaseline = try baseline() + surface.viewportState.top
      surface.requestVerticalViewport(top: CGFloat(rowBaseline - 120))
      // The toolbar now forbids this, but existing undo history can contain it.
      surface.performInput { _ = try session.setBlockQuote(true, expected: session.listSelection()) }
      let before = try baseline()
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertNil(surface.commandOutput)
      XCTAssertEqual(try baseline(), before, accuracy: 0.1)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      surface.perform(menuCommand: .redo, sender: nil)
      XCTAssertNil(surface.commandOutput)
      XCTAssertEqual(try baseline(), before, accuracy: 0.1)
    }
  }

  func testNativeSourceEnterBetweenQuoteMarkerAndSpaceContinuesQuote() throws {
    let source = "> Foo\n>\n> Bar"
    let (backend, surface) = try surface(source, type: EVDocument.markdownSourceType)
    let session = try XCTUnwrap(surface.session)
    surface.performInput { _ = try session.sendText("i") }
    let text = try backend.formattedText() as NSString
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: text.range(of: "Bar").location - 1, length: 0))
    surface.editorView.doCommand(by: #selector(NSResponder.insertNewline(_:)))
    XCTAssertNil(surface.commandOutput)
    let saved = try backend.serializedSource(typeName: EVDocument.markdownSourceType)
    XCTAssertTrue(String(decoding: saved, as: UTF8.self).hasSuffix("> Bar"))
    let (_, reopened) = try self.surface(String(decoding: saved, as: UTF8.self), type: EVDocument.markdownType)
    let body = try reopened.backend.formattedText() as NSString
    reopened.editorView.setAccessibilitySelectedTextRange(NSRange(location: body.range(of: "Bar").location, length: 0))
    XCTAssertEqual(reopened.formattingToolbar.blockQuote.state, .on)
    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data(source.utf8))
    surface.perform(menuCommand: .redo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), saved)
  }

  func testNativeEnterAtQuotePrefixSplitsProseAndTablesIdentically() throws {
    let checkout = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
      .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
      .deletingLastPathComponent()
    let demo = try String(contentsOf: checkout.appendingPathComponent("docs/markdown_demo.md"), encoding: .utf8)
    for source in [
      "> foo\n> bar",
      "> | Quoted item | Value |\n> | --- | ---: |\n> | Inside the quotation | 7 |",
      demo,
    ] {
      for afterSpace in [false, true] {
        let (backend, surface) = try surface(source, type: EVDocument.markdownSourceType)
        let session = try XCTUnwrap(surface.session)
        let text = try backend.formattedText() as NSString
        let firstLine = source.contains("> | Quoted item") ? "> | Quoted item" : "> foo"
        let at = text.range(of: firstLine).location + (afterSpace ? 2 : 1)
        // Accessibility selections address materialized geometry; first bring
        // the demo's table into the viewport, as a user would before clicking.
        surface.goToLine(UInt64(text.substring(to: at).filter { $0 == "\n" }.count + 1))
        surface.performInput { _ = try session.sendText("i") }
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: at, length: 0))
        XCTAssertEqual(surface.editorView.accessibilitySelectedTextRange(), NSRange(location: at, length: 0))
        surface.editorView.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        XCTAssertNil(surface.commandOutput)
        let prefix = afterSpace ? "> " : ">"
        let expected = source.replacingOccurrences(of: firstLine, with: "\(prefix)\n\(firstLine)")
        let saved = try backend.serializedSource(typeName: EVDocument.markdownSourceType)
        XCTAssertEqual(saved, Data(expected.utf8))
        XCTAssertEqual(surface.editorView.accessibilitySelectedTextRange(), NSRange(location: at + 1 + prefix.utf16.count, length: 0))
        XCTAssertTrue(try backend.formattedText().contains("\(prefix)\n\(firstLine)"))
        let (_, fresh) = try self.surface(expected, type: EVDocument.markdownSourceType)
        XCTAssertEqual(try backend.formattedText(), try fresh.backend.formattedText())
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data(source.utf8))
        surface.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), saved)
      }
    }
  }

  func testMouseAndReturnAtQuotePrefixes() throws {
    let source = "> foo\n>\n> bar\n\n> | foo | bar |\n> |---|---|\n> |a|b"
    for target in ["> foo", "> | foo"] {
      let (backend, surface) = try surface(source, type: EVDocument.markdownSourceType)
      let session = try XCTUnwrap(surface.session)
      surface.performInput { _ = try session.sendText("i") }
      let text = try backend.formattedText() as NSString
      let at = text.range(of: target).location + 1
      let snapshot = try session.layoutExport()
      let caret = try XCTUnwrap(snapshot.carets.first { $0.text_offset == UInt64(at) })
      let row = try XCTUnwrap(snapshot.rows.first { $0.row_index == caret.row_index })
      let local = surface.editorView.viewPoint(fromLayoutPoint: CGPoint(x: CGFloat(caret.x), y: CGFloat(row.baseline - row.ascent / 2)))
      let click = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown,
        location: surface.editorView.convert(local, to: nil), modifierFlags: [], timestamp: 0,
        windowNumber: 0, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
      surface.editorView.mouseDown(with: click)
      XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64(at))
      let enter = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
        modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil,
        characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36))
      surface.editorView.keyDown(with: enter)
      XCTAssertNil(surface.commandOutput)
      let saved = try backend.serializedSource(typeName: EVDocument.markdownSourceType)
      XCTAssertEqual(saved, Data(source.replacingOccurrences(of: target, with: ">\n" + target).utf8))
      XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, UInt64(at + 2))
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data(source.utf8))
      surface.perform(menuCommand: .redo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), saved)
    }
  }

  func testMarkdownSourceListToolbarRemovalSeparatesTheMiddleParagraph() throws {
    for (command, marker) in [(EVMenuCommand.bulletedList, "- "), (.numberedList, "3. ")] {
      let source = "\(marker)Before\n\(marker)Middle\n\(marker)After"
      let (backend, surface) = try surface(source, type: EVDocument.markdownSourceType)
      let text = try backend.formattedText()
      let at = try XCTUnwrap(text.range(of: "Middle"))
      let location = text.distance(from: text.startIndex, to: at.lowerBound)
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: location, length: 0))
      let toolbar = surface.formattingToolbar
      toolbar.refresh()
      let button = try XCTUnwrap(toolbar.commandButtons[command])
      XCTAssertEqual(button.state, .on)
      toolbar.performCommand(button)
      XCTAssertNil(surface.commandOutput)
      XCTAssertEqual(button.state, .off)
      XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType),
        Data("\(marker)Before\n\nMiddle\n\n\(command == .numberedList ? "1. " : marker)After".utf8))
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data(source.utf8))
    }
  }

  func testNumberedListToolbarRefreshesSplitAndRejoinedSourceNumbers() throws {
    let source = "1. First\n2. Middle\n3. Third\n4. Fourth"
    let (backend, surface) = try surface(source, type: EVDocument.markdownSourceType)
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 12, length: 0))
    let toolbar = surface.formattingToolbar
    toolbar.refresh()
    let numbered = try XCTUnwrap(toolbar.commandButtons[.numberedList])
    toolbar.performCommand(numbered)
    XCTAssertNil(surface.commandOutput)
    XCTAssertEqual(numbered.state, .off)
    let split = Data("1. First\n\nMiddle\n\n1. Third\n2. Fourth".utf8)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), split)
    toolbar.performCommand(numbered)
    XCTAssertNil(surface.commandOutput)
    XCTAssertEqual(numbered.state, .on)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType),
      Data("1. First\n\n2. Middle\n\n3. Third\n4. Fourth".utf8))
    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), split)
    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownSourceType), Data(source.utf8))
  }

  func testMarkdownOmitsUnsupportedPropertiesAndButtonsShareMenuActions() throws {
    let (backend, surface) = try surface("Words", type: EVDocument.markdownType)
    let toolbar = surface.formattingToolbar
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 5))
    toolbar.refresh()
    XCTAssertEqual(Set(toolbar.commandButtons.keys), Set([
      .bold, .italic, .strikethrough, .bulletedList, .numberedList, .increaseIndent, .decreaseIndent
    ]))
    XCTAssertFalse(try XCTUnwrap(toolbar.commandButtons[.strikethrough]).isHidden)
    let bold = try XCTUnwrap(toolbar.commandButtons[.bold])
    toolbar.performCommand(bold)
    XCTAssertNil(surface.commandOutput)
    XCTAssertEqual(bold.state, .on)
    XCTAssertEqual(try backend.formattedText(), "Words")
    toolbar.performCommand(bold)
    XCTAssertEqual(bold.state, .off)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("Words".utf8))
  }

  func testNormalCaretTracksInlineTraitsAndTogglesEnterInsertWithoutSourceChanges() throws {
    for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
      for (source, command) in [("**ab** plain", EVMenuCommand.bold), ("*ab* plain", .italic), ("~~ab~~ plain", .strikethrough)] {
        let (backend, surface) = try surface(source, type: type)
        let session = try XCTUnwrap(surface.session)
        let text = try backend.formattedText() as NSString
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: text.range(of: "ab").location + 1, length: 0))
        let toolbar = surface.formattingToolbar
        toolbar.refresh()
        let button = try XCTUnwrap(toolbar.commandButtons[command])
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
        XCTAssertEqual(button.state, .on, source)
        XCTAssertTrue(button.isEnabled)
        button.performClick(nil)
        XCTAssertNil(surface.commandOutput)
        XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        XCTAssertEqual(button.state, .off)
        XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        XCTAssertFalse(surface.canUndo)
        surface.editorView.insertText("X", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertNil(surface.commandOutput)
        XCTAssertEqual(button.state, .off)
        let changed = try backend.serializedSource(typeName: type)
        XCTAssertNotEqual(changed, Data(source.utf8))
        surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        surface.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: type), changed)
        let (_, reopened) = try self.surface(String(decoding: changed, as: UTF8.self), type: EVDocument.markdownType)
        let rich = try reopened.backend.formattedText() as NSString
        reopened.editorView.setAccessibilitySelectedTextRange(NSRange(location: rich.range(of: "X").location, length: 1))
        XCTAssertEqual(reopened.presentation(for: command).state, .off, "Inserted text must reopen without the disabled trait")
      }
    }
  }

  func testLinkToolbarTracksCaretSplitsTypingAndRemovesSelectedTreatment() throws {
    for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
      let source = "before [label](<next.md> \"Title\") after"
      let (backend, surface) = try surface(source, type: type)
      let session = try XCTUnwrap(surface.session)
      let text = try backend.formattedText() as NSString
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: text.range(of: "label").location + 2, length: 0))
      let toolbar = surface.formattingToolbar
      toolbar.refresh()
      XCTAssertEqual(toolbar.insertLink.state, .on)
      XCTAssertTrue(toolbar.insertLink.isEnabled)
      toolbar.insertLink.performClick(nil)
      XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
      XCTAssertEqual(toolbar.insertLink.state, .off)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      XCTAssertFalse(surface.canUndo)
      surface.editorView.insertText("X", replacementRange: NSRange(location: NSNotFound, length: 0))
      XCTAssertNil(surface.commandOutput)
      XCTAssertEqual(toolbar.insertLink.state, .off)
      let saved = try backend.serializedSource(typeName: type)
      let (fresh, reopened) = try self.surface(String(decoding: saved, as: UTF8.self), type: EVDocument.markdownType)
      XCTAssertEqual(try fresh.formattedText(), "before laXbel after")
      reopened.editorView.setAccessibilitySelectedTextRange(NSRange(location: 9, length: 0))
      XCTAssertNil(try reopened.session?.inlineContentContext(.link).item)
      surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      surface.perform(menuCommand: .redo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: type), saved)

      let (selectedBackend, selectedSurface) = try self.surface(source, type: type)
      let selectedText = try selectedBackend.formattedText() as NSString
      let selection = type == EVDocument.markdownType ? selectedText.range(of: "label")
        : selectedText.range(of: "[label](<next.md> \"Title\")")
      selectedSurface.editorView.setAccessibilitySelectedTextRange(selection)
      let selectedToolbar = selectedSurface.formattingToolbar
      selectedToolbar.refresh()
      XCTAssertEqual(selectedToolbar.insertLink.state, .on)
      XCTAssertTrue(selectedToolbar.insertLink.isEnabled)
      selectedToolbar.insertLink.performClick(nil)
      XCTAssertNil(selectedSurface.commandOutput)
      XCTAssertEqual(try selectedBackend.serializedSource(typeName: type), Data("before label after".utf8))
      selectedSurface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try selectedBackend.serializedSource(typeName: type), Data(source.utf8))
    }
  }

  func testImageToolbarIsActiveDisabledForCurrentOrSelectedImageAndOffAtTypingCaret() throws {
    let source = "A![alt](local.png)B"
    let (backend, surface) = try surface(source, type: EVDocument.markdownType)
    let session = try XCTUnwrap(surface.session)
    let toolbar = surface.formattingToolbar
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 1, length: 0))
    toolbar.refresh()
    XCTAssertEqual(toolbar.insertImage.state, .on)
    XCTAssertFalse(toolbar.insertImage.isEnabled)
    surface.performInput { _ = try session.sendText("i") }
    toolbar.refresh()
    XCTAssertEqual(toolbar.insertImage.state, .off)
    XCTAssertTrue(toolbar.insertImage.isEnabled)
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 1, length: 1))
    toolbar.refresh()
    XCTAssertEqual(toolbar.insertImage.state, .on)
    XCTAssertFalse(toolbar.insertImage.isEnabled)
    toolbar.insertImage.performClick(nil)
    XCTAssertFalse(surface.imagePopover.isEditing)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 2, length: 0))
    toolbar.refresh()
    XCTAssertEqual(toolbar.insertImage.state, .off)
    XCTAssertTrue(toolbar.insertImage.isEnabled)
    let revision = try backend.revision()
    try backend.setReadOnly(true)
    toolbar.refresh()
    XCTAssertEqual(try backend.revision(), revision)
    XCTAssertEqual(toolbar.insertImage.state, .off)
    XCTAssertFalse(toolbar.insertImage.isEnabled)
    try backend.setReadOnly(false)
    toolbar.refresh()
    XCTAssertTrue(toolbar.insertImage.isEnabled)
  }

  func testInlineButtonsRetainReadOnlyStateAndDisableLiteralCodeBlockTyping() throws {
    for type in [EVDocument.markdownType, EVDocument.markdownSourceType] {
      let source = "**bold** *italic* ~~strike~~ [link](next.md) `code`"
      let (backend, surface) = try surface(source, type: type)
      let text = try backend.formattedText() as NSString
      try backend.setReadOnly(true)
      for (word, command) in [("bold", EVMenuCommand.bold), ("italic", .italic), ("strike", .strikethrough)] {
        surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: text.range(of: word).location, length: 0))
        surface.formattingToolbar.refresh()
        let button = try XCTUnwrap(surface.formattingToolbar.commandButtons[command])
        XCTAssertEqual(button.state, .on)
        XCTAssertFalse(button.isEnabled)
      }
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: text.range(of: "link").location, length: 0))
      surface.formattingToolbar.refresh()
      XCTAssertEqual(surface.formattingToolbar.insertLink.state, .on)
      XCTAssertFalse(surface.formattingToolbar.insertLink.isEnabled)
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: text.range(of: "code").location, length: 0))
      surface.formattingToolbar.refresh()
      XCTAssertEqual(surface.formattingToolbar.characterCode.state, .on)
      XCTAssertFalse(surface.formattingToolbar.characterCode.isHidden)
      XCTAssertFalse(surface.formattingToolbar.characterCode.isEnabled)
      XCTAssertFalse(surface.formattingToolbar.characterStyle.isEnabled)
      XCTAssertEqual(surface.formattingToolbar.characterStyle.titleOfSelectedItem, "Code")
      surface.formattingToolbar.toggleCharacterCode(surface.formattingToolbar.characterCode)
      XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      try backend.setReadOnly(false)
      surface.formattingToolbar.refresh()
      XCTAssertEqual(surface.formattingToolbar.characterCode.state, .on)
      XCTAssertTrue(surface.formattingToolbar.characterCode.isEnabled)
      XCTAssertTrue(surface.formattingToolbar.characterStyle.isEnabled)

      let (_, code) = try self.surface("```\nliteral\n```", type: type)
      let codeText = try code.backend.formattedText() as NSString
      code.editorView.setAccessibilitySelectedTextRange(NSRange(location: codeText.range(of: "literal").location, length: 0))
      code.formattingToolbar.refresh()
      for command in [EVMenuCommand.bold, .italic, .strikethrough] {
        XCTAssertFalse(try XCTUnwrap(code.formattingToolbar.commandButtons[command]).isEnabled)
      }
      XCTAssertFalse(code.formattingToolbar.characterCode.isHidden)
      XCTAssertFalse(code.formattingToolbar.characterCode.isEnabled)
      XCTAssertFalse(code.formattingToolbar.characterStyle.isEnabled)
      code.formattingToolbar.toggleCharacterCode(code.formattingToolbar.characterCode)
      XCTAssertEqual(code.viewPresentation.mode, UInt32(VIEM_MODE_NORMAL))
    }
    for type in [EVDocument.plainTextType, EVDocument.codeType] {
      let (_, literal) = try surface("literal", type: type)
      literal.formattingToolbar.refresh()
      XCTAssertTrue(literal.formattingToolbar.characterCode.isHidden)
    }
  }

  func testListTogglesUseStructuralStateAcrossNestingAndMixedParagraphs() throws {
    let (_, surface) = try surface("- One\n  - Two\n\nPlain", type: EVDocument.markdownType)
    let toolbar = surface.formattingToolbar
    let bullets = try XCTUnwrap(toolbar.commandButtons[.bulletedList])
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 7))
    toolbar.refresh()
    XCTAssertEqual(toolbar.paragraphStyle.titleOfSelectedItem, "Mixed")
    XCTAssertEqual(bullets.state, .on)
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 13))
    toolbar.refresh()
    XCTAssertEqual(bullets.state, .mixed)
    XCTAssertEqual(toolbar.commandButtons[.numberedList]?.state, .off)
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 0))
    toolbar.refresh()
    toolbar.performCommand(bullets)
    XCTAssertNil(surface.commandOutput)
    XCTAssertEqual(bullets.state, .off)
    toolbar.performCommand(bullets)
    XCTAssertNil(surface.commandOutput)
    XCTAssertEqual(bullets.state, .on)
  }

  func testWindowRefreshKeepsToolbarCurrentWithoutPollingAndFitsNarrowWindows() throws {
    let (backend, surface) = try surface("# Head\n\nWords", type: EVDocument.markdownType)
    let document = EVDocument(editorBackend: backend)
    let controller = EVDocumentWindowController(document: document, editorSurface: surface)
    defer { controller.close() }
    controller.showWindow(nil)
    let toolbar = surface.formattingToolbar
    XCTAssertEqual(toolbar.paragraphStyle.titleOfSelectedItem, "Heading 1")
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 5, length: 0))
    XCTAssertEqual(toolbar.paragraphStyle.titleOfSelectedItem, "Base Paragraph")
    let window = try XCTUnwrap(controller.window)
    window.setContentSize(NSSize(width: 920, height: 300))
    window.contentView?.superview?.layoutSubtreeIfNeeded()
    let button = toolbar.formattedView
    XCTAssertEqual(button.frame.maxX, toolbar.bounds.maxX - 10, accuracy: 1)
    let lastFormattingButton = try XCTUnwrap(toolbar.commandButtons[.decreaseIndent])
    let lastFrame = lastFormattingButton.convert(lastFormattingButton.bounds, to: toolbar)
    XCTAssertGreaterThan(button.frame.minX - lastFrame.maxX, 12)
    if let directory = ProcessInfo.processInfo.environment["VIEM_TOOLBAR_SCREENSHOT_DIR"] {
      window.setContentSize(NSSize(width: 920, height: 300))
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 5, length: 5))
      for appearance in [NSAppearance.Name.aqua, .darkAqua] {
        window.appearance = NSAppearance(named: appearance)
        let frame = try XCTUnwrap(window.contentView?.superview)
        frame.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let bitmap = try XCTUnwrap(frame.bitmapImageRepForCachingDisplay(in: frame.bounds))
        frame.cacheDisplay(in: frame.bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent("toolbar-\(appearance.rawValue).png"))
      }
    }
    window.setContentSize(NSSize(width: 480, height: 300))
    window.contentView?.superview?.layoutSubtreeIfNeeded()
    XCTAssertLessThanOrEqual(toolbar.frame.width, window.frame.width)
    let scroll = try XCTUnwrap(toolbar.subviews.first as? NSScrollView)
    XCTAssertGreaterThan(try XCTUnwrap(scroll.documentView).frame.width, scroll.contentSize.width)
    XCTAssertTrue(scroll.hasHorizontalScroller)
    XCTAssertEqual(button.frame.maxX, toolbar.bounds.maxX - 10, accuracy: 1)
    XCTAssertGreaterThanOrEqual(button.frame.minX - scroll.frame.maxX, 12)
    XCTAssertFalse(button.isHidden)
  }
}
