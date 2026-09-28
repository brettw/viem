import AppKit
import CViemCore
@testable import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVFormattingToolbarTests: XCTestCase {
  private func surface(_ source: String, type: String? = nil) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
    let backend = EVCoreDocumentBackend()
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
    let firstWindow = EVDocumentWindowController(document: document, editorSurface: first)
    let secondWindow = EVDocumentWindowController(document: document, editorSurface: second)
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
