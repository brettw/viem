import AppKit
import CViemCore
@testable import ViemAppShell
import XCTest

@testable import ViemEditor

final class EVStyleMenuBridgeTests: XCTestCase {
  @MainActor
  func testParagraphShortcutValidationRetainsCurrentStyleCheckmark() throws {
    let backend = EVCoreDocumentBackend()
    let source = Data("## Title\n\nBody".utf8)
    try backend.read(source: source, typeName: EVDocument.markdownType)
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
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
  }

  @MainActor
  func testListIndentMenuValidatesAndDispatchesFromTheMiddleOfAnItem() throws {
    let source = "1. One\n2. **Second**"
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data(source.utf8), typeName: EVDocument.markdownType)
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
    let indented = try backend.serializedSource(typeName: EVDocument.markdownType)
    XCTAssertNotEqual(indented, Data(source.utf8))

    content.performEditorMenuCommand(unindent)
    XCTAssertNil(surface.commandOutput)
    XCTAssertEqual(try backend.formattedText(), "One\nSecond")
    XCTAssertEqual(try session.selectedNamedStyles().paragraph?.rawValue, "NumberedList1")
    XCTAssertTrue(content.validateMenuItem(indent))
    XCTAssertFalse(content.validateMenuItem(unindent))
    let unindented = try backend.serializedSource(typeName: EVDocument.markdownType)

    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), indented)
    surface.perform(menuCommand: .redo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), unindented)
  }

  @MainActor
  func testHeadingMenuWritesFormatMarkersAndRejectsStaleActions() throws {
    for (type, source, expected) in [
      (EVDocument.markdownSourceType, "Paragraph", "## Paragraph"),
      (EVDocument.markdownType, "Paragraph", "## Paragraph"),
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
  func testLiveCoreCatalogueExposesOnlyBasePlainTextStylesAndDisablesAssignment() throws {
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
      [""])
    XCTAssertEqual(
      Set(catalogue.entries.filter { $0.role == .paragraph }.map(\.stableID)),
      Set(["Paragraph"]))
    XCTAssertFalse(catalogue.entries.contains { $0.displayName == "Base Document" || $0.displayName == "Base Character" })
    XCTAssertTrue(catalogue.entries.allSatisfy { !$0.presentation.isEnabled })
    XCTAssertEqual(catalogue.entries.first { $0.stableID == "Paragraph" }?.presentation.state, .on)
    XCTAssertEqual(catalogue.entries.first { $0.stableID == "" }?.presentation.state, .on)
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
    XCTAssertNotEqual(coordinator.inspection?.targetCoreDocumentID, catalogue.documentID)
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
