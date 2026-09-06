import AppKit
import CEvimCore
import EvimAppShell
import EvimCoreTextProvider
import XCTest

@testable import EvimEditor

final class EVTypographySessionTests: XCTestCase {
  @MainActor
  func testShowFontsOpensVisibleNativePanelForArabicVisualSelection() throws {
    for (type, source) in [
      (EVDocument.htmlType, "<p>Arabic word مرحبا in the middle</p>"),
      (
        EVDocument.rtfType,
        #"{\rtf1 Arabic word \u1605?\u1585?\u1581?\u1576?\u1575? in the middle}"#
      ),
    ] {
      let backend = EVCoreDocumentBackend()
      try backend.read(source: Data(source.utf8), typeName: type)
      let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
      surface.loadViewIfNeeded()
      let session = try XCTUnwrap(surface.session)
      surface.perform(menuCommand: .selectAll, sender: nil)
      XCTAssertTrue(surface.presentation(for: .showFonts).isEnabled)
      _ = try session.selectedTypography()
      NSFontManager.shared.fontPanel(false)?.orderOut(nil)
      surface.perform(menuCommand: .showFonts, sender: nil)
      let panel = try XCTUnwrap(NSFontManager.shared.fontPanel(false))
      defer { panel.orderOut(nil) }
      XCTAssertTrue(panel.isVisible)
      XCTAssertTrue(panel.canBecomeKey)
      XCTAssertNotNil(panel.screen)
      XCTAssertTrue(NSApplication.shared.windows.contains { $0 === panel })
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      panel.orderOut(nil)
      surface.perform(menuCommand: .showColors, sender: nil)
      let colors = NSColorPanel.shared
      defer { colors.orderOut(nil) }
      XCTAssertTrue(colors.isVisible)
      XCTAssertTrue(colors.canBecomeKey)
      XCTAssertTrue(NSScreen.screens.contains { $0.visibleFrame.intersects(colors.frame) })
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      colors.color = NSColor(srgbRed: 0.75, green: 0.25, blue: 0.5, alpha: 1)
      XCTAssertNotEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
      _ = try session.undo()
      XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
    }
  }

  @MainActor
  func testShowFontsResponderActionOpensPanelForVimWordSelection() throws {
    let backend = EVCoreDocumentBackend()
    let source = "<p>Arabic word مرحبا in the middle</p>\n<p>This is a longer paragraph.</p>"
    try backend.read(source: Data(source.utf8), typeName: EVDocument.htmlType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let document = EVDocument()
    let controller = EVDocumentWindowController(document: document, editorSurface: surface)
    let window = try XCTUnwrap(controller.window)
    window.makeKeyAndOrderFront(nil)
    window.makeFirstResponder(surface.editorView)
    defer { window.orderOut(nil) }
    let session = try XCTUnwrap(surface.session)
    _ = try session.sendText("gg0viw")
    surface.refreshPresentation()
    XCTAssertEqual(surface.statusBarState.mode, "VISUAL")
    XCTAssertTrue(surface.presentation(for: .showFonts).isEnabled)
    _ = try session.selectedTypography()
    let item = NSMenuItem(
      title: "Show Fonts",
      action: #selector(EVEditorCommandRouting.performEditorMenuCommand(_:)), keyEquivalent: "t")
    item.tag = EVMenuCommand.showFonts.rawValue
    NSFontManager.shared.fontPanel(false)?.orderOut(nil)
    // XCTest does not run an NSApplication event loop, so key-window discovery
    // is unavailable; begin at the same editor first responder explicitly.
    XCTAssertTrue(surface.editorView.tryToPerform(try XCTUnwrap(item.action), with: item))
    let panel = try XCTUnwrap(NSFontManager.shared.fontPanel(false))
    defer { panel.orderOut(nil) }
    XCTAssertTrue(panel.isVisible)
    XCTAssertTrue(panel.canBecomeKey)
    XCTAssertTrue(NSScreen.screens.contains { $0.visibleFrame.intersects(panel.frame) })
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), Data(source.utf8))
  }

  @MainActor
  func testFontBatchIsOneVerifiedUndoAndOpenTypeMenuUsesTheSelectedFontsCatalogue() throws {
    for type in [EVDocument.htmlType, EVDocument.rtfType] {
      let source =
        type == EVDocument.htmlType ? "<p>Text</p><!--keep-->" : #"{\rtf1 Text{\*\opaque keep}}"#
      let backend = EVCoreDocumentBackend()
      try backend.read(source: Data(source.utf8), typeName: type)
      let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
      surface.loadViewIfNeeded()
      let session = try XCTUnwrap(surface.session)
      surface.perform(menuCommand: .selectAll, sender: nil)
      let selected = try session.listSelection()
      let original = try backend.serializedSource(typeName: type)
      try session.setDirectCharacterProperties(
        [
          (.characterFontFamilies, .stringList(["AvenirNext-UltraLight"])),
          (.characterWeight, .unsigned(200)), (.characterBold, .boolean(true)),
          (.characterSize, .float(24)), (.characterSlant, .fontSlant(1)),
        ], expected: selected)
      let typography = try session.selectedTypography()
      XCTAssertEqual(typography.fontFamily, "AvenirNext-UltraLight")
      XCTAssertEqual(typography.baseWeight, 200)
      XCTAssertEqual(typography.weight, 500)
      XCTAssertTrue(typography.bold)
      XCTAssertEqual(typography.size, 24)
      XCTAssertEqual(typography.slant, 1)
      XCTAssertThrowsError(
        try session.setDirectCharacterProperties([(.characterSize, .float(12))], expected: selected)
      )
      let menu = NSMenu()
      surface.populateOpenTypeFeatureMenu(menu)
      let catalogue = EVFontCatalog.features(for: typography.fontFamily)
      XCTAssertEqual(Array(menu.items.prefix(catalogue.count)).map(\.title), catalogue.map(\.label))
      let featureItem = try XCTUnwrap(menu.items.first)
      XCTAssertTrue(featureItem.isEnabled)
      XCTAssertTrue(
        NSApplication.shared.sendAction(
          try XCTUnwrap(featureItem.action), to: featureItem.target, from: featureItem))
      XCTAssertFalse(try session.selectedTypography().features.isEmpty)
      _ = try session.undo()
      _ = try session.undo()
      XCTAssertEqual(try backend.serializedSource(typeName: type), original)
    }
  }
  @MainActor
  func testSelectionQueryIncludesFaceBaseEmphasisMixedAndFeatures() throws {
    let backend = EVCoreDocumentBackend()
    try backend.read(
      source: Data(
        "<p style='font-family:AvenirNext-UltraLight;font-weight:200;font-size:20pt;font-feature-settings: \"liga\" 0'><b>A</b>B</p>"
          .utf8), typeName: EVDocument.htmlType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let session = try XCTUnwrap(surface.session)
    var style = try session.selectedTypography()
    XCTAssertEqual(style.fontFamily, "AvenirNext-UltraLight")
    XCTAssertEqual(style.baseWeight, 200)
    XCTAssertEqual(style.weight, 500)
    XCTAssertTrue(style.bold)
    XCTAssertEqual(style.size, 20)
    XCTAssertEqual(style.features, [EVOpenTypeFeature(tag: "liga", setting: 0)])
    XCTAssertNil(style.foreground)
    surface.perform(menuCommand: .selectAll, sender: nil)
    style = try session.selectedTypography()
    XCTAssertTrue(style.mixed)
    let before = try backend.serializedSource(typeName: EVDocument.htmlType)
    surface.perform(menuCommand: .bold, sender: nil)
    style = try session.selectedTypography()
    XCTAssertTrue(style.bold)
    XCTAssertEqual(style.weight, 500)
    XCTAssertFalse(style.mixed)
    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.htmlType), before)
  }

  @MainActor
  func testEmptyParagraphQueriesItsFontWithoutLayout() throws {
    let backend = EVCoreDocumentBackend()
    try backend.read(
      source: Data("<p style='font-size:36pt;font-family:Georgia'></p>".utf8),
      typeName: EVDocument.htmlType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let session = try XCTUnwrap(surface.session)
    let before = surface.presentationRefreshCount
    XCTAssertEqual(try session.selectedTypography().size, 36)
    XCTAssertEqual(try session.currentFontEnWidth(), 18)
    XCTAssertEqual(surface.presentationRefreshCount, before)
  }
}
