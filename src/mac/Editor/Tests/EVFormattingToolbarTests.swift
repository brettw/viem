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
