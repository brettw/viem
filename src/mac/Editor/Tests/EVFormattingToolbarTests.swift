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
    try backend.read(source: Data(source.utf8), typeName: type ?? EVDocument.markdownType,
      filename: "notes.md", allowAutomaticCode: false)
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

  func testCharacterAndTableGroupsHaveRequestedOrderAndStandardGaps() throws {
    let (_, surface) = try surface("Words")
    let toolbar = surface.formattingToolbar
    let viewport = try XCTUnwrap(toolbar.subviews.first)
    let row = try XCTUnwrap(viewport.subviews.first as? NSStackView)
    let bold = try XCTUnwrap(toolbar.commandButtons[.bold])
    let character = try XCTUnwrap(bold.superview as? NSStackView)
    XCTAssertEqual(character.arrangedSubviews.compactMap { ($0 as? NSButton)?.title },
      ["Bold", "Italic", "Underline", "Strikethrough"])
    let scripts = try XCTUnwrap(toolbar.commandButtons[.superscript]?.superview as? NSStackView)
    XCTAssertEqual(scripts.arrangedSubviews.compactMap { ($0 as? NSButton)?.title }, ["Superscript", "Subscript"])
    XCTAssertTrue(toolbar.commandButtons.values.allSatisfy { $0.image != nil }, "Every toolbar command must have a visible icon")
    var scriptNumberCenters: [CGFloat] = []
    for command in [EVMenuCommand.superscript, .subscript] {
      let image = try XCTUnwrap(toolbar.commandButtons[command]?.image)
      XCTAssertTrue(image.isTemplate)
      let bitmap = try XCTUnwrap(NSBitmapImageRep(data: try XCTUnwrap(image.tiffRepresentation)))
      var numberPixels: [CGFloat] = []
      for y in 0..<bitmap.pixelsHigh {
        for x in (bitmap.pixelsWide * 5 / 9)..<bitmap.pixelsWide {
          if (bitmap.colorAt(x: x, y: y)?.alphaComponent ?? 0) > 0.25 {
            numberPixels.append(CGFloat(y) / CGFloat(bitmap.pixelsHigh))
          }
        }
      }
      XCTAssertFalse(numberPixels.isEmpty, "The script icon must draw its numeral")
      XCTAssertLessThan((numberPixels.max() ?? 0) - (numberPixels.min() ?? 0), 0.36,
        "The script numeral stays small within the 18-point icon")
      scriptNumberCenters.append(numberPixels.reduce(0, +) / CGFloat(max(1, numberPixels.count)))
    }
    XCTAssertGreaterThan(abs(scriptNumberCenters[0] - scriptNumberCenters[1]), 0.25,
      "Superscript and subscript must visibly place their numerals at different heights")
    let codeLink = try XCTUnwrap(toolbar.characterCode.superview as? NSStackView)
    XCTAssertEqual(codeLink.arrangedSubviews.compactMap { ($0 as? NSButton)?.title }, ["Code (Character)", "Link"])
    let characterIndex = try XCTUnwrap(row.arrangedSubviews.firstIndex(of: character))
    let scriptIndex = try XCTUnwrap(row.arrangedSubviews.firstIndex(of: scripts))
    let codeIndex = try XCTUnwrap(row.arrangedSubviews.firstIndex(of: codeLink))
    XCTAssertEqual(characterIndex, 0)
    XCTAssertEqual(scriptIndex, characterIndex + 1)
    XCTAssertEqual(codeIndex, scriptIndex + 1)
    XCTAssertFalse(row.arrangedSubviews.contains { $0 is NSBox })
    let tableIndex = try XCTUnwrap(row.arrangedSubviews.firstIndex(of: toolbar.insertTable))
    XCTAssertTrue(toolbar.insertImage.superview === row.arrangedSubviews[tableIndex - 1])
    XCTAssertTrue(toolbar.commandButtons[.increaseIndent]?.superview === row.arrangedSubviews[tableIndex + 1])
    XCTAssertTrue(toolbar.paragraphStyle === row.arrangedSubviews[tableIndex + 2])
    XCTAssertTrue(toolbar.characterStyle === row.arrangedSubviews[tableIndex + 3])
    XCTAssertTrue(row.arrangedSubviews.last === toolbar.characterStyle)
    toolbar.layoutSubtreeIfNeeded()
    let standardGap = row.arrangedSubviews[codeIndex + 1].frame.minX - codeLink.frame.maxX
    XCTAssertEqual(standardGap, row.spacing, accuracy: 0.01)
    for index in [scriptIndex, codeIndex, tableIndex, tableIndex + 1] {
      XCTAssertEqual(row.arrangedSubviews[index].frame.minX - row.arrangedSubviews[index - 1].frame.maxX,
        standardGap, accuracy: 0.01)
    }
    XCTAssertTrue(toolbar.paragraphStyle.itemArray.allSatisfy { $0.image != nil })
    XCTAssertTrue(toolbar.characterStyle.itemArray.allSatisfy { $0.image != nil })
  }

  func testPreviewParagraphSelectorShowsOnlySupportedAssignmentsAndKeepsCatalogue() throws {
    let (backend, surface) = try surface("Words")
    let toolbar = surface.formattingToolbar
    let snapshot = try backend.styleSheetSnapshot()
    let supported = Set(snapshot.definitions.filter { $0.key.namespace == .block && $0.capabilities.contains(.assign) }.map { $0.key.id.rawValue })
    let expected: Set<String> = ["Paragraph", "Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6", "Block quote", "Code Block"]
    XCTAssertEqual(supported, expected)
    let displayed = Set(toolbar.paragraphStyle.itemArray.compactMap { ($0.representedObject as? EVStyleMenuAction)?.stableID })
    XCTAssertEqual(displayed, supported)
    XCTAssertTrue(displayed.isDisjoint(with: ["Table", "Table cell", "Table header", "Image"]))
    for definition in snapshot.definitions where definition.flags.contains(.internalList)
      || definition.kind == .list {
      XCTAssertEqual(EVStyleIcons.type(for: definition), .paragraph, definition.name)
    }
    toolbar.formattedView.performClick(nil)
    toolbar.refresh()
    XCTAssertEqual(backend.sourceFormat, .code)
    XCTAssertTrue(toolbar.paragraphStyle.isHiddenOrHasHiddenAncestor, "Preview off hides the formatting controls")
  }

  func testPreviewListCaretShowsItsExactParagraphStyleWithoutOfferingAssignment() throws {
    for source in ["- item", "1. item"] {
      let (backend, surface) = try surface(source)
      let toolbar = surface.formattingToolbar
      let selected = try XCTUnwrap(surface.session?.selectedNamedStyles().paragraph)
      let definition = try XCTUnwrap(backend.styleSheetSnapshot().definition(namespace: .block, id: selected))
      XCTAssertTrue(definition.flags.contains(.internalList))
      let item = try XCTUnwrap(toolbar.paragraphStyle.selectedItem)
      XCTAssertEqual((item.representedObject as? EVStyleMenuAction)?.stableID, selected.rawValue)
      XCTAssertEqual(item.title, definition.name)
      XCTAssertFalse(item.isEnabled)
      XCTAssertEqual(item.image?.accessibilityDescription, "Paragraph style")
      XCTAssertTrue(toolbar.paragraphStyle.isEnabled)
      let entries = toolbar.paragraphStyle.itemArray.compactMap { ($0.representedObject as? EVStyleMenuAction)?.stableID }
      XCTAssertEqual(entries.filter { $0.hasPrefix("BulletedList") || $0.hasPrefix("NumberedList") }, [selected.rawValue])
    }
  }

  func testUnderlineAndScriptToolbarActionsRoundTripAndUndo() throws {
    for (command, markup) in [(EVMenuCommand.underline, "ins"), (.superscript, "sup"), (.subscript, "sub")] {
      let (backend, surface) = try surface("Words")
      surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 5))
      let toolbar = surface.formattingToolbar
      let button = try XCTUnwrap(toolbar.commandButtons[command])
      toolbar.refresh()
      XCTAssertTrue(button.isEnabled)
      button.performClick(nil)
      XCTAssertNil(surface.commandOutput)
      XCTAssertEqual(button.state, .on)
      let changed = try backend.serializedSource(typeName: EVDocument.markdownType)
      XCTAssertEqual(changed, Data("<\(markup)>Words</\(markup)>".utf8))
      let (_, reopened) = try self.surface(String(decoding: changed, as: UTF8.self))
      reopened.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 5))
      XCTAssertEqual(reopened.presentation(for: command).state, .on)
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("Words".utf8))
      surface.perform(menuCommand: .redo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), changed)
    }
  }

  func testMarkdownPreviewTogglePreservesSourceTracksSharedHistoryAndRestoresFocus() throws {
    let source = Data("# Heading\n\n**Words**".utf8)
    let (backend, first) = try surface(String(decoding: source, as: UTF8.self), type: EVDocument.codeType)
    let second = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    let document = EVDocument(editorBackend: backend)
    let firstWindow = EVDocumentWindowController(document: document, editorSurface: first, configuration: backend.configuration)
    let secondWindow = EVDocumentWindowController(document: document, editorSurface: second, configuration: backend.configuration)
    defer { firstWindow.close(); secondWindow.close() }
    firstWindow.showWindow(nil)
    secondWindow.showWindow(nil)
    let button = first.formattingToolbar.formattedView
    let otherButton = second.formattingToolbar.formattedView
    let titleBarPreview = firstWindow.formattingToolbar.previewButton
    // Literal Markdown has no toolbar; the title bar offers Preview in the
    // toolbar toggle's place. Preview restores the toolbar and its toggle.
    func showsLiteralChrome(_ window: EVDocumentWindowController, _ surface: EVEditorSurfaceController) -> Bool {
      window.formattingToolbar.toggleButton.isHidden && !window.formattingToolbar.previewButton.isHidden
        && window.formattingToolbar.previewButton.window === window.window && surface.formattingToolbar.window == nil
    }
    func showsPreviewChrome(_ window: EVDocumentWindowController, _ surface: EVEditorSurfaceController) -> Bool {
      !window.formattingToolbar.toggleButton.isHidden && window.formattingToolbar.previewButton.isHidden
        && window.formattingToolbar.toggleButton.state == .on && surface.formattingToolbar.window === window.window
    }
    XCTAssertTrue(showsLiteralChrome(firstWindow, first))
    XCTAssertTrue(showsLiteralChrome(secondWindow, second))
    let image = try XCTUnwrap(button.image)
    XCTAssertEqual(button.state, .off)
    XCTAssertEqual(button.accessibilityLabel(), "Markdown Preview")
    XCTAssertEqual(button.toolTip, "Markdown Preview")
    XCTAssertFalse(button.allowsMixedState)
    XCTAssertTrue(button.isEnabled)
    XCTAssertEqual(titleBarPreview.accessibilityLabel(), "Markdown Preview")
    XCTAssertEqual(titleBarPreview.toolTip, "Markdown Preview")

    titleBarPreview.performClick(nil)
    XCTAssertEqual(backend.sourceFormat, .markdown)
    XCTAssertEqual(button.state, .on)
    XCTAssertEqual(otherButton.state, .on)
    XCTAssertTrue(showsPreviewChrome(firstWindow, first))
    XCTAssertTrue(showsPreviewChrome(secondWindow, second))
    XCTAssertEqual(try backend.formattedText(), "Heading\nWords")
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
    XCTAssertTrue(firstWindow.window?.firstResponder === first.editorView)
    XCTAssertTrue(button.image === image)

    // Detached toolbars refresh when Preview installs them again.
    second.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(backend.sourceFormat, .code)
    XCTAssertTrue(showsLiteralChrome(firstWindow, first), "undoing a view switch removes the toolbar again")
    XCTAssertTrue(showsLiteralChrome(secondWindow, second))
    second.perform(menuCommand: .redo, sender: nil)
    XCTAssertEqual(button.state, .on)
    XCTAssertEqual(otherButton.state, .on)
    XCTAssertTrue(showsPreviewChrome(firstWindow, first), "redoing a view switch restores the toolbar")
    XCTAssertTrue(showsPreviewChrome(secondWindow, second))
    button.performClick(nil)
    XCTAssertEqual(backend.sourceFormat, .code)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
    XCTAssertTrue(showsLiteralChrome(firstWindow, first), "Preview off hides the toolbar and offers Preview in the title bar")
    XCTAssertTrue(showsLiteralChrome(secondWindow, second))
    XCTAssertTrue(firstWindow.window?.firstResponder === first.editorView)

    // Preview from the title bar keeps a hidden toolbar hidden.
    try backend.configuration.setShowFormattingToolbar(false, for: .markdown)
    XCTAssertTrue(showsLiteralChrome(firstWindow, first), "literal Markdown offers Preview whatever the toolbar preference")
    titleBarPreview.performClick(nil)
    XCTAssertEqual(backend.sourceFormat, .markdown)
    XCTAssertFalse(firstWindow.formattingToolbar.toggleButton.isHidden)
    XCTAssertTrue(titleBarPreview.isHidden)
    XCTAssertEqual(firstWindow.formattingToolbar.toggleButton.state, .off)
    XCTAssertNil(first.formattingToolbar.window)
    XCTAssertTrue(firstWindow.window?.firstResponder === first.editorView)
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
    for type in [EVDocument.markdownType] {
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
    for type in [EVDocument.markdownType] {
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

  func testBlockQuoteStaysVisibleButDisabledForCodeBlocksAndMixedSelections() throws {
    for type in [EVDocument.markdownType] {
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
    for type in [EVDocument.markdownType] {
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
      let viewport = surface.viewportState.top
      let codeStart = UInt64(text.substring(to: text.range(of: "graph LR").location).utf8.count)
      let unchangedRows = try session.layoutExport().rows.filter {
        $0.text_end <= codeStart && $0.y >= viewport && $0.text_start < $0.text_end
      }.map { (offset: $0.text_start, baseline: $0.baseline - viewport) }
      XCTAssertFalse(unchangedRows.isEmpty)
      func assertViewport() throws {
        XCTAssertEqual(surface.viewportState.top, viewport, accuracy: 0.1)
        let rows = try session.layoutExport().rows
        for before in unchangedRows {
          let row = try XCTUnwrap(rows.first { $0.text_start == before.offset })
          XCTAssertEqual(row.baseline - surface.viewportState.top, before.baseline, accuracy: 0.1,
            "Undo and redo preserve surrounding text, while the edited block reflows naturally")
        }
      }
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertNil(surface.commandOutput)
      try assertViewport()
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      surface.perform(menuCommand: .redo, sender: nil)
      XCTAssertNil(surface.commandOutput)
      try assertViewport()
    }
  }

  func testMarkdownOmitsUnsupportedPropertiesAndButtonsShareMenuActions() throws {
    let (backend, surface) = try surface("Words", type: EVDocument.markdownType)
    let toolbar = surface.formattingToolbar
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 5))
    toolbar.refresh()
    XCTAssertEqual(Set(toolbar.commandButtons.keys), Set([
      .bold, .italic, .underline, .strikethrough, .superscript, .subscript, .bulletedList, .numberedList, .increaseIndent, .decreaseIndent
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
    for type in [EVDocument.markdownType] {
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
    for type in [EVDocument.markdownType] {
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
    for type in [EVDocument.markdownType] {
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
    let lastFrame = toolbar.characterStyle.convert(toolbar.characterStyle.bounds, to: toolbar)
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
    let viewport = try XCTUnwrap(toolbar.subviews.first)
    let row = try XCTUnwrap(viewport.subviews.first as? NSStackView)
    window.setContentSize(NSSize(width: 650, height: 300))
    window.contentView?.superview?.layoutSubtreeIfNeeded()
    let indent = try XCTUnwrap(toolbar.commandButtons[.decreaseIndent])
    let indentFrame = indent.convert(indent.bounds, to: viewport)
    let paragraphFrame = toolbar.paragraphStyle.convert(toolbar.paragraphStyle.bounds, to: viewport)
    let characterFrame = toolbar.characterStyle.convert(toolbar.characterStyle.bounds, to: viewport)
    XCTAssertGreaterThanOrEqual(indentFrame.minX, viewport.bounds.minX)
    XCTAssertLessThanOrEqual(indentFrame.maxX, viewport.bounds.maxX)
    XCTAssertGreaterThan(paragraphFrame.maxX, viewport.bounds.maxX)
    XCTAssertGreaterThan(characterFrame.minX, viewport.bounds.maxX)
    XCTAssertEqual(button.frame.maxX, toolbar.bounds.maxX - 10, accuracy: 1)
    XCTAssertGreaterThanOrEqual(button.frame.minX - viewport.frame.maxX, 12)
    window.setContentSize(NSSize(width: 480, height: 300))
    window.contentView?.superview?.layoutSubtreeIfNeeded()
    XCTAssertLessThanOrEqual(toolbar.frame.width, window.frame.width)
    XCTAssertGreaterThan(row.frame.width, viewport.bounds.width)
    XCTAssertTrue(viewport.clipsToBounds)
    XCTAssertFalse(toolbar.subviews.contains { $0 is NSScrollView })
    let rowOrigin = row.frame.origin
    let viewportOrigin = viewport.bounds.origin
    toolbar.characterStyle.scrollToVisible(toolbar.characterStyle.bounds)
    window.contentView?.superview?.layoutSubtreeIfNeeded()
    XCTAssertEqual(row.frame.origin, rowOrigin)
    XCTAssertEqual(viewport.bounds.origin, viewportOrigin)
    XCTAssertEqual(button.frame.maxX, toolbar.bounds.maxX - 10, accuracy: 1)
    XCTAssertGreaterThanOrEqual(button.frame.minX - viewport.frame.maxX, 12)
    XCTAssertFalse(button.isHidden)
  }
}
