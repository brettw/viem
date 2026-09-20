import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVTypographyPanelTrackingTests: XCTestCase {
  func testColorsFollowCurrentForegroundAndBackgroundAfterCoalescingWithoutEditingSource() async throws {
    let surface = try makeSurface()
    let original = try surface.backend.serializedSource(typeName: EVDocument.htmlType)
    let panel = NSColorPanel.shared
    defer { panel.close() }
    select(0, in: surface)
    surface.perform(menuCommand: .highlightColor, sender: nil)
    assertColor(panel.color, red: 1, green: 1, blue: 0)
    let before = EVTypographyPanels.shared.synchronizationCount
    select(1, in: surface)
    select(0, in: surface)
    select(1, in: surface)
    assertColor(panel.color, red: 1, green: 1, blue: 0)
    try await Task.sleep(for: .milliseconds(230))
    XCTAssertEqual(EVTypographyPanels.shared.synchronizationCount, before + 1)
    assertColor(panel.color, red: 0, green: 1, blue: 1)
    XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.htmlType), original)
    surface.perform(menuCommand: .textColor, sender: nil)
    assertColor(panel.color, red: 0, green: 0, blue: 1)
    select(0, in: surface)
    try await Task.sleep(for: .milliseconds(230))
    assertColor(panel.color, red: 1, green: 0, blue: 0)
    XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.htmlType), original)
  }

  func testColorGestureBeforeDelayUsesCurrentExactSelection() throws {
    let surface = try makeSurface()
    let session = try XCTUnwrap(surface.session)
    select(0, in: surface)
    surface.perform(menuCommand: .textColor, sender: nil)
    defer { NSColorPanel.shared.close() }
    select(1, in: surface)
    NSColorPanel.shared.color = NSColor(srgbRed: 0.25, green: 0.5, blue: 0.75, alpha: 1)
    XCTAssertEqual(try session.selectedTypography().foreground, EVStyleColor(red: 0.25, green: 0.5, blue: 0.75, alpha: 1))
    select(0, in: surface)
    XCTAssertEqual(try session.selectedTypography().foreground, EVStyleColor(red: 1, green: 0, blue: 0, alpha: 1))
  }

  func testFontPanelFollowsFamilyAndSize() async throws {
    let surface = try makeSurface()
    let original = try surface.backend.serializedSource(typeName: EVDocument.htmlType)
    select(0, in: surface)
    surface.perform(menuCommand: .showFonts, sender: nil)
    defer { NSFontManager.shared.fontPanel(false)?.close() }
    XCTAssertEqual(NSFontManager.shared.selectedFont?.pointSize, 16)
    select(1, in: surface)
    try await Task.sleep(for: .milliseconds(230))
    XCTAssertEqual(NSFontManager.shared.selectedFont?.pointSize, 24)
    XCTAssertEqual(NSFontManager.shared.selectedFont?.familyName, "Helvetica")
    XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.htmlType), original)
  }

  func testClosedPanelStopsFollowing() async throws {
    let surface = try makeSurface()
    select(0, in: surface)
    surface.perform(menuCommand: .showColors, sender: nil)
    NSColorPanel.shared.close()
    let before = EVTypographyPanels.shared.synchronizationCount
    select(1, in: surface)
    try await Task.sleep(for: .milliseconds(230))
    XCTAssertEqual(EVTypographyPanels.shared.synchronizationCount, before)
  }

  func testTransparentHighlightIsAnExplicitOverrideOfInheritedBackground() throws {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data("<p style='background-color:yellow'>text</p>".utf8), typeName: EVDocument.htmlType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    select(0, in: surface)
    surface.perform(menuCommand: .highlightColor, sender: nil)
    defer { NSColorPanel.shared.close() }
    NSColorPanel.shared.color = NSColor(srgbRed: 0, green: 0, blue: 0, alpha: 0)
    let style = try XCTUnwrap(surface.session).selectedTypography()
    XCTAssertEqual(style.background?.alpha, 0)
  }

  func testFontSizeGestureBeforeDelayUsesNewCaretFamilyAndPreservesSemanticBold() throws {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data("<p><span style='font-family:Georgia'>A</span><b style='font-family:Helvetica'>B</b></p>".utf8), typeName: EVDocument.htmlType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let session = try XCTUnwrap(surface.session)
    select(0, in: surface)
    surface.perform(menuCommand: .showFonts, sender: nil)
    defer { NSFontManager.shared.fontPanel(false)?.close() }
    select(1, in: surface)
    XCTAssertTrue(surface.canEditTypography)
    let sizeUp = NSMenuItem(title: "Size Up", action: nil, keyEquivalent: "")
    sizeUp.tag = 3 // NSSizeUpFontAction, dispatched by the real shared manager.
    NSFontManager.shared.modifyFont(sizeUp)
    XCTAssertNil(surface.commandOutput)
    let style = try session.selectedTypography()
    XCTAssertTrue(style.fontFamily.hasPrefix("Helvetica"))
    XCTAssertEqual(style.size, 15)
    XCTAssertEqual(style.baseWeight, 400)
    XCTAssertTrue(style.bold)
    XCTAssertEqual(style.weight, 700)
  }

  private func makeSurface() throws -> EVEditorSurfaceController {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data("<p><span style='color:red;background-color:yellow;font-family:Georgia;font-size:16pt'>A</span><span style='color:blue;background-color:#00ffff;font-family:Helvetica;font-size:24pt'>B</span></p>".utf8), typeName: EVDocument.htmlType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    return surface
  }
  private func select(_ offset: Int, in surface: EVEditorSurfaceController) {
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: offset, length: 1))
  }
  private func assertColor(_ color: NSColor, red: CGFloat, green: CGFloat, blue: CGFloat,
                           file: StaticString = #filePath, line: UInt = #line) {
    guard let color = color.usingColorSpace(.sRGB) else { return XCTFail("No RGB color", file: file, line: line) }
    XCTAssertEqual(color.redComponent, red, accuracy: 0.001, file: file, line: line)
    XCTAssertEqual(color.greenComponent, green, accuracy: 0.001, file: file, line: line)
    XCTAssertEqual(color.blueComponent, blue, accuracy: 0.001, file: file, line: line)
  }
}
