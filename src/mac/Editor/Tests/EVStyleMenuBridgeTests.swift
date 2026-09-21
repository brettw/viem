import AppKit
import CViemCore
@testable import ViemAppShell
import XCTest

@testable import ViemEditor

final class EVStyleMenuBridgeTests: XCTestCase {
  @MainActor
  func testParagraphShortcutValidationRetainsCurrentStyleCheckmark() throws {
    let backend = EVCoreDocumentBackend()
    let source = Data("<h2>Title</h2><p>Body</p>".utf8)
    try backend.read(source: source, typeName: EVDocument.htmlType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let session = try XCTUnwrap(surface.session)
    let content = EVDocumentContentViewController(editorSurface: surface)
    func check(_ command: EVMenuCommand, state: NSControl.StateValue) {
      let item = NSMenuItem(title: "Style", action: #selector(EVEditorCommandRouting.performEditorMenuCommand(_:)), keyEquivalent: "")
      item.tag = command.rawValue
      XCTAssertTrue(content.validateMenuItem(item))
      XCTAssertEqual(item.state, state)
    }
    check(.heading2, state: .on)
    check(.heading0, state: .off)
    check(.heading1, state: .off)
    surface.performInput { _ = try session.sendText("G") }
    check(.heading2, state: .off)
    check(.heading0, state: .on)

    let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
    let item = NSMenuItem(title: "Default Paragraph", action: #selector(EVStyleMenuActionRouting.performEditorStyleMenuAction(_:)), keyEquivalent: "")
    item.representedObject = EVStyleMenuAction(kind: .assign, role: .character, stableID: "", documentID: catalogue.documentID, documentRevision: catalogue.documentRevision, styleSheetRevision: catalogue.styleSheetRevision)
    _ = surface.editorView.validateMenuItem(item)
    XCTAssertEqual(item.state, .on)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), source)
  }

  @MainActor
  func testListIndentMenuValidatesAndDispatchesFromTheMiddleOfAnItem() throws {
    let source = "<ol data-keep='yes'><li>One</li><li><b>Second</b></li></ol><!--keep-->"
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data(source.utf8), typeName: EVDocument.htmlType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let session = try XCTUnwrap(surface.session)
    let content = EVDocumentContentViewController(editorSurface: surface)
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 6, length: 0))

    func item(_ command: EVMenuCommand) -> NSMenuItem {
      let item = NSMenuItem(
        title: command == .increaseIndent ? "Indent" : "Unindent",
        action: #selector(EVEditorCommandRouting.performEditorMenuCommand(_:)),
        keyEquivalent: "")
      item.tag = command.rawValue
      return item
    }

    let indent = item(.increaseIndent)
    let unindent = item(.decreaseIndent)
    XCTAssertTrue(content.validateMenuItem(indent))
    XCTAssertFalse(content.validateMenuItem(unindent))
    content.performEditorMenuCommand(indent)
    XCTAssertNil(surface.commandOutput)
    XCTAssertEqual(try backend.formattedText(), "One\nSecond")
    XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "NumberedList2")

    XCTAssertFalse(content.validateMenuItem(indent))
    XCTAssertTrue(content.validateMenuItem(unindent))
    let indented = try backend.serializedSource(typeName: EVDocument.htmlType)
    XCTAssertNotEqual(indented, Data(source.utf8))
    XCTAssertTrue(String(decoding: indented, as: UTF8.self).contains("data-keep='yes'"))
    XCTAssertTrue(String(decoding: indented, as: UTF8.self).contains("<!--keep-->"))

    content.performEditorMenuCommand(unindent)
    XCTAssertNil(surface.commandOutput)
    XCTAssertEqual(try backend.formattedText(), "One\nSecond")
    XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "NumberedList1")
    XCTAssertTrue(content.validateMenuItem(indent))
    XCTAssertFalse(content.validateMenuItem(unindent))
    let unindented = try backend.serializedSource(typeName: EVDocument.htmlType)

    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), indented)
    surface.perform(menuCommand: .redo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), unindented)
  }

  @MainActor
  func testBlankHTMLParagraphListStyleMenuAllowsTypingWithNativeMarkup() throws {
    for (command, styleID, tag) in [
      (EVMenuCommand.bulletedList, "BulletedList1", "ul"),
      (EVMenuCommand.numberedList, "NumberedList1", "ol"),
    ] {
      for source in ["", "<p></p>"] {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: EVDocument.htmlType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        try session.sendText("i")
        XCTAssertFalse(try XCTUnwrap(surface.currentStyleMenuCatalogue()).entries.contains {
          $0.stableID == styleID
        })
        XCTAssertTrue(surface.presentation(for: command).isEnabled)
        surface.perform(menuCommand: command, sender: nil)
        XCTAssertEqual(surface.statusBarState.message, "")
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data("<\(tag)><li></li></\(tag)>".utf8))
        XCTAssertEqual(surface.currentStyleMenuCatalogue()?.entries.first { $0.stableID == styleID }?.presentation.state, .on)

        surface.editorView.insertText("a", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(surface.statusBarState.message, "")
        XCTAssertEqual(try backend.formattedText(), "a")
        let saved = try backend.serializedSource(typeName: EVDocument.htmlType)
        XCTAssertEqual(saved, Data("<\(tag)><li>a</li></\(tag)>".utf8))
        let reopened = EVCoreDocumentBackend()
        try reopened.read(source: saved, typeName: EVDocument.htmlType)
        XCTAssertEqual(try reopened.formattedText(), "a")
        try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data("<\(tag)><li></li></\(tag)>".utf8))
        surface.perform(menuCommand: .redo, sender: nil)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), saved)
      }
    }
  }

  @MainActor
  func testHTMLSourceInternalStylesStayInEditorAndOutOfAssignmentMenus() throws {
    let source = "<h2 title='hello'>Body &amp; <em>words</em></h2>"
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data(source.utf8), typeName: EVDocument.htmlSourceType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let snapshot = try backend.styleSheetSnapshot()
    let internalStyles = snapshot.definitions.filter { $0.flags.contains(.internalSyntax) }
    XCTAssertGreaterThanOrEqual(internalStyles.count, 7)
    XCTAssertTrue(internalStyles.allSatisfy { !$0.capabilities.contains(.assign) })
    XCTAssertTrue(internalStyles.filter { $0.key.id.rawValue.hasPrefix("* HTML ") }.allSatisfy { $0.name.hasPrefix("*") })
    let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
    XCTAssertEqual(catalogue.entries.first { $0.stableID == "Heading2" }?.presentation.state, .on)
    XCTAssertEqual(catalogue.entries.first { $0.stableID == "" }?.presentation.state, .on)
    let internalIDs = Set(internalStyles.map { $0.key.id.rawValue })
    XCTAssertFalse(catalogue.entries.contains { internalIDs.contains($0.stableID) })
    let definition = try XCTUnwrap(internalStyles.first { $0.key.id.rawValue == "* HTML Tag name" })
    let editor = EVStyleEditorViewController()
    editor.retarget(document: surface, styleKey: definition.key)
    XCTAssertEqual(editor.inspection.selectedStyleKey, definition.key)
    let color = EVStyleColor(red: 0.2, green: 0.4, blue: 0.8, alpha: 1)
    XCTAssertTrue(editor.setPropertyForTesting(.characterForeground, value: .color(color)), editor.inspection.diagnostic)
    let saved = try backend.serializedSource(typeName: EVDocument.htmlType)
    XCTAssertEqual(saved, Data(source.utf8), "Syntax colors are buffer configuration, not source declarations")
    let session = try XCTUnwrap(surface.session)
    surface.editorView.insertText("A", replacementRange: NSRange(location: NSNotFound, length: 0))
    surface.editorView.insertText(" ", replacementRange: NSRange(location: NSNotFound, length: 0))
    XCTAssertNotEqual(try backend.serializedSource(typeName: EVDocument.htmlType), saved)
    XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: definition.key)?.properties[.characterForeground]?.declared, .color(color))
    surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE)) }
    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), saved)
    XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: definition.key)?.properties[.characterForeground]?.declared, .color(color))
    let fresh = try XCTUnwrap(surface.currentStyleMenuCatalogue())
    surface.perform(styleMenuAction: EVStyleMenuAction(kind: .assign, role: .character, stableID: definition.key.id.rawValue, documentID: fresh.documentID, documentRevision: fresh.documentRevision, styleSheetRevision: fresh.styleSheetRevision), sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), saved, "A forged assignment action cannot apply an automatic syntax style")
    withExtendedLifetime(surface) {}
  }
  @MainActor
  func testBuiltinHeadingDeletionPersistsAndDisablesAbsentShortcut() throws {
    let backend = EVCoreDocumentBackend()
    let original = "<h1 data-keep='x'>Title</h1><p>Body</p><!--keep-->"
    try backend.read(source: Data(original.utf8), typeName: EVDocument.htmlType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let session = try XCTUnwrap(surface.session)
    try session.setIncludeStyleDefinitionsInFile(true, expected: backend.documentState())
    let beforeDelete = try backend.serializedSource(typeName: EVDocument.htmlType)
    let sheet = try backend.styleSheetSnapshot()
    let heading = try XCTUnwrap(
      sheet.definition(namespace: .block, id: EVStyleID(rawValue: "Heading1")))
    XCTAssertTrue(heading.capabilities.contains(.delete))
    XCTAssertFalse(
      try XCTUnwrap(sheet.definition(namespace: .block, id: EVStyleID(rawValue: "Paragraph")))
        .capabilities.contains(.delete))
    try session.deleteStyle(heading.key, identity: sheet.identity)
    XCTAssertFalse(surface.headingShortcutPresentation(level: 1).isEnabled)
    XCTAssertNil(try backend.styleSheetSnapshot().definition(for: heading.key))
    let saved = try backend.serializedSource(typeName: EVDocument.htmlType)
    XCTAssertTrue(String(decoding: saved, as: UTF8.self).contains("--viem-style-deleted"))
    let reopened = EVCoreDocumentBackend()
    try reopened.read(source: saved, typeName: EVDocument.htmlType)
    XCTAssertNil(try reopened.styleSheetSnapshot().definition(for: heading.key))
    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), beforeDelete)
    XCTAssertTrue(surface.headingShortcutPresentation(level: 1).isEnabled)
  }

  @MainActor
  func testDefaultListParagraphStyleMenuWritesSourceWithoutChangingText() throws {
    for (type, source) in [
      (EVDocument.htmlType, "<p data-keep='x'>Words</p><!--keep-->"),
      (EVDocument.rtfType, "{\\rtf1 Words{\\*\\unknown keep}}"),
    ] {
      let backend = EVCoreDocumentBackend()
      try backend.read(source: Data(source.utf8), typeName: type)
      let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
      surface.loadViewIfNeeded()
      XCTAssertTrue(surface.presentation(for: .numberedList).isEnabled)
      surface.perform(menuCommand: .numberedList, sender: nil)
      XCTAssertEqual(surface.currentStyleMenuCatalogue()?.entries.first {
        $0.stableID == "NumberedList1"
      }?.presentation.state, .on)
      XCTAssertEqual(surface.statusBarState.message, "")
      XCTAssertNotEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
    }
  }

  @MainActor
  func testHeadingMenuWritesFormatMarkersAndRejectsStaleActions() throws {
    for (type, source, expected) in [
      (EVDocument.markdownSourceType, "Paragraph", "## Paragraph"),
      (EVDocument.markdownType, "Paragraph", "## Paragraph"),
      (
        EVDocument.htmlType, "<p data-keep='yes'>Paragraph</p>",
        "<h2 data-keep='yes'>Paragraph</h2>"
      ),
    ] {
      let backend = EVCoreDocumentBackend()
      try backend.read(source: Data(source.utf8), typeName: type)
      let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
      surface.loadViewIfNeeded()
      let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
      XCTAssertTrue(
        try XCTUnwrap(catalogue.entries.first { $0.stableID == "Heading2" }).presentation.isEnabled)
      let action = EVStyleMenuAction(
        kind: .assign, role: .paragraph, stableID: "Heading2",
        documentID: catalogue.documentID, documentRevision: catalogue.documentRevision,
        styleSheetRevision: catalogue.styleSheetRevision)
      surface.perform(styleMenuAction: action, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(expected.utf8))
      XCTAssertEqual(surface.statusBarState.message, "")
      let stale = EVStyleMenuAction(
        kind: .assign, role: .paragraph, stableID: "Paragraph",
        documentID: catalogue.documentID, documentRevision: catalogue.documentRevision,
        styleSheetRevision: catalogue.styleSheetRevision)
      surface.perform(styleMenuAction: stale, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(expected.utf8))
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
    }
  }

  @MainActor
  func testLiveCoreCatalogueExposesEveryRoleAndDisablesUnsupportedPlainTextAssignment() throws {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data("plain text".utf8), typeName: "public.plain-text")
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()

    let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
    let coreSnapshot = try backend.styleSheetSnapshot()

    XCTAssertEqual(catalogue.documentID, coreSnapshot.identity.documentID)
    XCTAssertEqual(catalogue.documentRevision, coreSnapshot.identity.documentRevision)
    XCTAssertEqual(catalogue.styleSheetRevision, coreSnapshot.identity.styleSheetRevision)
    XCTAssertEqual(catalogue.entries.count, coreSnapshot.definitions.filter { !$0.flags.contains(.internalList) && !$0.flags.contains(.internalSyntax) }.count + 1)
    XCTAssertEqual(
      catalogue.entries.filter { $0.role == .character }.map(\.stableID),
      [
        "", "Code"
      ])
    XCTAssertEqual(
      Set(catalogue.entries.filter { $0.role == .paragraph }.map(\.stableID)),
      Set(
        [
          "Paragraph", "Block quote", "Code Block", "Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6",
        ]))
    XCTAssertFalse(catalogue.entries.contains { $0.displayName == "Base Document" || $0.displayName == "Base Character" })
    XCTAssertTrue(catalogue.entries.allSatisfy { !$0.presentation.isEnabled })
    XCTAssertEqual(catalogue.entries.first { $0.stableID == "Paragraph" }?.presentation.state, .on)
    XCTAssertEqual(catalogue.entries.first { $0.stableID == "" }?.presentation.state, .on)
    XCTAssertTrue(catalogue.entries.filter { !["Paragraph", ""].contains($0.stableID) }.allSatisfy { $0.presentation.state == .off })
    XCTAssertTrue(catalogue.canEditStyles)
  }

  @MainActor
  func testEditActionRetargetsSingletonToExactStableStyleIdentity() throws {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data("# Heading".utf8), typeName: "public.markdown")
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
    let coordinator = EVStyleEditorCoordinator.shared
    coordinator.close()
    defer { coordinator.close() }

    surface.perform(
      styleMenuAction: EVStyleMenuAction(
        kind: .edit,
        role: .paragraph,
        stableID: "Heading2",
        documentID: catalogue.documentID,
        documentRevision: catalogue.documentRevision,
        styleSheetRevision: catalogue.styleSheetRevision
      ),
      sender: nil
    )

    XCTAssertEqual(
      coordinator.inspection?.selectedStyleKey,
      EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading2"))
    )
    XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, catalogue.documentID)
  }

  @MainActor
  func testAssignmentPayloadCannotBypassDisabledAdapterCapability() throws {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data("plain text".utf8), typeName: "public.plain-text")
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let before = try backend.nativeSaveSnapshot(typeName: "public.plain-text")
    let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
    let coordinator = EVStyleEditorCoordinator.shared
    coordinator.close()

    surface.perform(
      styleMenuAction: EVStyleMenuAction(
        kind: .assign,
        role: .paragraph,
        stableID: "Heading1",
        documentID: catalogue.documentID,
        documentRevision: catalogue.documentRevision,
        styleSheetRevision: catalogue.styleSheetRevision
      ),
      sender: nil
    )

    XCTAssertEqual(try backend.nativeSaveSnapshot(typeName: "public.plain-text"), before)
    XCTAssertNil(coordinator.styleWindow)
  }
}
