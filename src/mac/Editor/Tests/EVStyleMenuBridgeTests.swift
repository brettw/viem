import AppKit
import CEvimCore
import EvimAppShell
import XCTest

@testable import EvimEditor

final class EVStyleMenuBridgeTests: XCTestCase {
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
    XCTAssertTrue(internalStyles.allSatisfy { $0.name.hasPrefix("*") && !$0.capabilities.contains(.assign) })
    let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
    XCTAssertEqual(catalogue.entries.first { $0.stableID == "Heading2" }?.presentation.state, .on)
    XCTAssertEqual(catalogue.entries.first { $0.stableID == "Character" }?.presentation.state, .on)
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
    surface.performInput { _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE)) }
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
    XCTAssertTrue(String(decoding: saved, as: UTF8.self).contains("--evim-style-deleted"))
    let reopened = EVCoreDocumentBackend()
    try reopened.read(source: saved, typeName: EVDocument.htmlType)
    XCTAssertNil(try reopened.styleSheetSnapshot().definition(for: heading.key))
    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(original.utf8))
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
      let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
      XCTAssertTrue(
        try XCTUnwrap(catalogue.entries.first { $0.stableID == "List2" }).presentation.isEnabled)
      surface.perform(
        styleMenuAction: EVStyleMenuAction(
          kind: .assign, role: .paragraph, stableID: "List2",
          documentID: catalogue.documentID, documentRevision: catalogue.documentRevision,
          styleSheetRevision: catalogue.styleSheetRevision), sender: nil)
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
    XCTAssertEqual(catalogue.entries.count, coreSnapshot.definitions.count)
    XCTAssertEqual(
      catalogue.entries.filter { $0.role == .character }.map(\.stableID),
      [
        "Character", "Code"
      ])
    XCTAssertEqual(
      Set(catalogue.entries.filter { $0.role == .paragraph }.map(\.stableID)),
      Set(
        [
          "Paragraph", "Code Block", "Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6",
        ] + (1...3).map { "List\($0)" }))
    XCTAssertEqual(
      catalogue.entries.filter { $0.role == .document }.map(\.stableID),
      [
        "Document"
      ])
    XCTAssertTrue(catalogue.entries.allSatisfy { !$0.presentation.isEnabled })
    XCTAssertEqual(catalogue.entries.first { $0.stableID == "Paragraph" }?.presentation.state, .on)
    XCTAssertEqual(catalogue.entries.first { $0.stableID == "Character" }?.presentation.state, .on)
    XCTAssertTrue(catalogue.entries.filter { !["Paragraph", "Character"].contains($0.stableID) }.allSatisfy { $0.presentation.state == .off })
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
