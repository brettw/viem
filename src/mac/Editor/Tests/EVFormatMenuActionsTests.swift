import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVFormatMenuActionsTests: XCTestCase {
  func testScriptsAreExclusiveToggleOffAndUndoInRTF() throws {
    for (type, text) in [(EVDocument.rtfType, #"{\rtf1{\pard text}{\*\comment keep}}"#),
                         (EVDocument.rtfType, #"{\rtf1 text{\*\opaque keep}}"#)] {
      let surface = try makeSurface(text, type: type)
      let session = try XCTUnwrap(surface.session)
      XCTAssertFalse(surface.presentation(for: .superscript).isEnabled)
      surface.perform(menuCommand: .selectAll, sender: nil)
      for (command, expected) in [(EVMenuCommand.superscript, UInt32(1)), (.subscriptText, 2), (.subscriptText, 0)] {
        surface.perform(menuCommand: .selectAll, sender: nil)
        XCTAssertTrue(surface.presentation(for: command).isEnabled)
        let before = try surface.backend.serializedSource(typeName: type)
        surface.perform(menuCommand: command, sender: nil)
        XCTAssertEqual(try session.selectedTypography().scriptPosition, expected)
        XCTAssertEqual(surface.presentation(for: .superscript).state, expected == 1 ? .on : .off)
        XCTAssertEqual(surface.presentation(for: .subscriptText).state, expected == 2 ? .on : .off)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(try surface.backend.serializedSource(typeName: type), before)
        surface.perform(menuCommand: .selectAll, sender: nil)
        surface.perform(menuCommand: command, sender: nil)
      }
    }
  }

  func testScriptStateIgnoresOtherMixedPropertiesAndDetectsMixedScript() throws {
    let same = try makeSurface(#"{\rtf1{\pard {\super A}{\super\b B}}}"#)
    same.perform(menuCommand: .selectAll, sender: nil)
    XCTAssertTrue(try XCTUnwrap(same.session).selectedTypography().mixed)
    XCTAssertEqual(same.presentation(for: .superscript).state, .on)
    same.perform(menuCommand: .superscript, sender: nil)
    XCTAssertEqual(try XCTUnwrap(same.session).selectedTypography().scriptPosition, 0)
    let mixed = try makeSurface(#"{\rtf1{\pard {\super A}{\sub B}}}"#)
    mixed.perform(menuCommand: .selectAll, sender: nil)
    XCTAssertEqual(mixed.presentation(for: .superscript).state, .mixed)
    mixed.perform(menuCommand: .superscript, sender: nil)
    XCTAssertEqual(mixed.presentation(for: .superscript).state, .on)
    XCTAssertEqual(mixed.presentation(for: .subscriptText).state, .off)
  }

  func testLigatureCommandsRetainUnrelatedFeaturesAndRestoreFontDefaults() throws {
    let surface = try makeSurface(#"{\rtf1{\pard Text}{\*\comment keep}}"#)
    let session = try XCTUnwrap(surface.session)
    try session.editStyle(key: .baseParagraph, expected: surface.backend.styleSheetSnapshot().identity,
      mutation: .setDeclaration(.characterOpenTypeFeatures, .openTypeFeatures([.init(tag: "smcp", setting: 1)])))
    surface.perform(menuCommand: .selectAll, sender: nil)
    let original = try surface.backend.serializedSource(typeName: EVDocument.rtfType)
    XCTAssertEqual(surface.presentation(for: .defaultLigatures).state, .on)
    for command in [EVMenuCommand.noLigatures, .allLigatures, .defaultLigatures] {
      surface.perform(menuCommand: command, sender: nil)
      let features = try session.selectedTypography().features
      XCTAssertTrue(features.contains(EVOpenTypeFeature(tag: "smcp", setting: 1)))
      for tag in ["liga", "clig", "dlig", "hlig"] {
        XCTAssertEqual(features.first { $0.tag == tag }?.setting,
          command == .defaultLigatures ? nil : command == .allLigatures ? 1 : 0)
      }
      XCTAssertEqual(surface.presentation(for: command).state, .on)
    }
    for _ in 0..<3 { surface.perform(menuCommand: .undo, sender: nil) }
    XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.rtfType), original)
  }

  func testOpenTypeFontDefaultsOverrideInheritedFeatureDeclarations() throws {
    let surface = try makeSurface(#"{\rtf1\ansi\deff0{\fonttbl{\f0 Times New Roman;}{\f1 Courier New;}{\f2 AvenirNext-Regular;}}{\pard \f2 Text}}"#)
    let session = try XCTUnwrap(surface.session)
    let snapshot = try surface.backend.styleSheetSnapshot()
    _ = try session.editStyle(key: .baseParagraph, expected: snapshot.identity,
      mutation: .setDeclaration(.characterOpenTypeFeatures,
        .openTypeFeatures([.init(tag: "liga", setting: 0)])))
    surface.perform(menuCommand: .selectAll, sender: nil)
    XCTAssertEqual(try session.selectedTypography().features, [.init(tag: "liga", setting: 0)])
    let original = try surface.backend.serializedSource(typeName: EVDocument.rtfType)
    let menu = NSMenu()
    surface.populateOpenTypeFeatureMenu(menu)
    let defaults = try XCTUnwrap(menu.item(withTitle: "Use Font Defaults"))
    XCTAssertTrue(defaults.isEnabled)
    XCTAssertTrue(NSApplication.shared.sendAction(try XCTUnwrap(defaults.action), to: defaults.target, from: defaults))
    XCTAssertNil(surface.commandOutput)
    XCTAssertEqual(try session.selectedTypography().features, [])
    XCTAssertEqual(try surface.backend.styleSheetSnapshot().definition(for: .baseParagraph)?
      .properties[.characterOpenTypeFeatures]?.declared, .openTypeFeatures([.init(tag: "liga", setting: 0)]))
    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.rtfType), original)
  }

  func testParagraphMenuActionsAndSpacingAreCheckedAndUndoable() throws {
    let surface = try makeSurface(#"{\rtf1{\pard Text}{\*\comment keep}}"#)
    let session = try XCTUnwrap(surface.session)
    for command in [EVMenuCommand.alignCenter, .alignEnd, .alignStart, .directionRightToLeft,
                    .directionLeftToRight, .lineSpacingSingle,
                    .lineSpacingOneAndHalf, .lineSpacingDouble, .lineSpacingNormal] {
      let original = try surface.backend.serializedSource(typeName: EVDocument.rtfType)
      XCTAssertTrue(surface.presentation(for: command).isEnabled)
      surface.perform(menuCommand: command, sender: nil)
      XCTAssertEqual(surface.presentation(for: command).state, .on, "\(command): \(surface.statusBarState.message)")
      surface.perform(menuCommand: .undo, sender: nil)
      XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.rtfType), original)
    }
    let original = try surface.backend.serializedSource(typeName: EVDocument.rtfType)
    surface.applyParagraphSpacing(before: 7, after: 13, expected: try session.listSelection())
    let formatted = try session.selectedFormatting()
    XCTAssertEqual(formatted[.blockMarginTop], .float(7))
    XCTAssertEqual(formatted[.blockMarginBottom], .float(13))
    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try surface.backend.serializedSource(typeName: EVDocument.rtfType), original,
                   "Both spacing fields form one undo unit")
  }

  func testPastedNoBackgroundOverridesAnInheritedBackground() throws {
    let source = try makeSurface(#"{\rtf1{\pard source}}"#)
    source.perform(menuCommand: .copyStyle, sender: nil)
    let target = try makeSurface(#"{\rtf1{\pard target}}"#)
    target.perform(menuCommand: .selectAll, sender: nil)
    target.perform(menuCommand: .pasteStyle, sender: nil)
    XCTAssertNil(target.commandOutput)
    XCTAssertEqual(try XCTUnwrap(target.session).selectedTypography().background?.alpha, 0)
  }

  func testMixedFormattingCheckmarksAreSpecificToEachProperty() throws {
    let surface = try makeSurface(#"{\rtf1\pard\qc AB\par\pard\qr C}"#)
    surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 0, length: 2))
    XCTAssertEqual(surface.presentation(for: .alignCenter).state, .on)
    XCTAssertEqual(surface.presentation(for: .defaultLigatures).state, .on)
    surface.perform(menuCommand: .selectAll, sender: nil)
    XCTAssertEqual(surface.presentation(for: .alignCenter).state, .mixed)
    XCTAssertEqual(surface.presentation(for: .alignEnd).state, .mixed)
    XCTAssertEqual(surface.presentation(for: .defaultLigatures).state, .on)
  }

  func testCustomSpacingSheetUsesExactAndMultipleABIValues() async throws {
    let surface = try makeSurface(#"{\rtf1{\pard Text}}"#)
    let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 640, height: 400),
      styleMask: [.titled, .closable], backing: .buffered, defer: false)
    window.contentViewController = surface
    window.makeKeyAndOrderFront(nil)
    defer { window.orderOut(nil) }
    let session = try XCTUnwrap(surface.session)
    for (title, value, kind) in [("Exactly (pt)", "24", UInt32(VIEM_STYLE_LINE_SPACING_EXACT)),
                               ("Multiple", "1.75", UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER))] {
      surface.perform(menuCommand: .lineSpacingCustom, sender: nil)
      let sheet = try XCTUnwrap(window.attachedSheet)
      let controls = descendants(try XCTUnwrap(sheet.contentView))
      let popup = try XCTUnwrap(controls.first { $0 is NSPopUpButton } as? NSPopUpButton)
      popup.selectItem(withTitle: title)
      let input = try XCTUnwrap(controls.first { $0 is NSTextField && $0.accessibilityLabel() == "Value" } as? NSTextField)
      input.stringValue = value
      let apply = try XCTUnwrap(controls.first { ($0 as? NSButton)?.title == "Apply" } as? NSButton)
      apply.performClick(nil)
      for _ in 0..<30 where window.attachedSheet != nil { try await Task.sleep(for: .milliseconds(20)) }
      XCTAssertEqual(try session.selectedFormatting()[.paragraphLineSpacing], .lineSpacing(.init(kind: kind, value: Float(value)!)))
    }
  }

  func testCopyPasteStyleAndClearFormattingUseSingleAtomicUndo() throws {
    let source = try makeSurface(#"{\rtf1\ansi\deff0{\fonttbl{\f0 Times New Roman;}{\f1 Courier New;}{\f2 Georgia;}}{\colortbl;\red51\green102\blue153;}{\pard \f2 \fs38 \cf1 \qc \sa180 {\super source}}}"#)
    source.perform(menuCommand: .copyStyle, sender: nil)
    let target = try makeSurface(#"{\rtf1{\pard target}{\*\comment untouched}}"#)
    target.perform(menuCommand: .selectAll, sender: nil)
    let session = try XCTUnwrap(target.session)
    let original = try target.backend.serializedSource(typeName: EVDocument.rtfType)
    XCTAssertTrue(target.presentation(for: .pasteStyle).isEnabled)
    target.perform(menuCommand: .pasteStyle, sender: nil)
    XCTAssertNil(target.commandOutput)
    let style = try session.selectedFormatting()
    XCTAssertEqual(style[.characterFontFamilies], .stringList(["Georgia"]))
    XCTAssertEqual(style[.characterSize], .float(19))
    XCTAssertEqual(style[.characterScriptPosition], .scriptPosition(1))
    XCTAssertEqual(style[.paragraphAlignment], .paragraphAlignment(UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER)))
    target.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try target.backend.serializedSource(typeName: EVDocument.rtfType), original)
    target.perform(menuCommand: .selectAll, sender: nil)
    target.perform(menuCommand: .pasteStyle, sender: nil)
    let pasted = try target.backend.serializedSource(typeName: EVDocument.rtfType)
    XCTAssertTrue(target.presentation(for: .clearAllDirectFormatting).isEnabled)
    target.perform(menuCommand: .clearAllDirectFormatting, sender: nil)
    XCTAssertNil(target.commandOutput)
    XCTAssertEqual(try session.selectedTypography().scriptPosition, 0)
    target.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try target.backend.serializedSource(typeName: EVDocument.rtfType), pasted)
  }

  func testPasteStyleIntoRTFUsesPortableFormattingAndRejectsUnavailableAutomaticOverride() throws {
    let source = try makeSurface(#"{\rtf1\ansi\deff0{\fonttbl{\f0 Times New Roman;}{\f1 Courier New;}{\f2 Georgia;}}{\colortbl;\red51\green102\blue153;}{\pard \f2 \fs38 \cf1 \qc {\super source}}}"#)
    source.perform(menuCommand: .copyStyle, sender: nil)
    let target = try makeSurface(#"{\rtf1 target{\*\opaque keep}}"#, type: EVDocument.rtfType)
    target.perform(menuCommand: .selectAll, sender: nil)
    XCTAssertTrue(target.presentation(for: .pasteStyle).isEnabled)
    let original = try target.backend.serializedSource(typeName: EVDocument.rtfType)
    target.perform(menuCommand: .pasteStyle, sender: nil)
    XCTAssertNil(target.commandOutput)
    let style = try XCTUnwrap(target.session).selectedTypography()
    XCTAssertEqual(style.fontFamily, "Georgia")
    XCTAssertEqual(style.size, 19)
    XCTAssertEqual(style.scriptPosition, 1)
    target.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try target.backend.serializedSource(typeName: EVDocument.rtfType), original)
    let rtl = try makeSurface(#"{\rtf1\rtlpar target}"#, type: EVDocument.rtfType)
    rtl.perform(menuCommand: .selectAll, sender: nil)
    XCTAssertFalse(rtl.presentation(for: .pasteStyle).isEnabled)
  }

  func testPanelInspectionIsAvailableAtNormalCaretAndUnsupportedFormattingStaysDisabled() throws {
    let rich = try makeSurface(#"{\rtf1{\pard text}}"#)
    for command in [EVMenuCommand.showFonts, .showColors, .textColor, .highlightColor] {
      XCTAssertTrue(rich.presentation(for: command).isEnabled)
    }
    for type in [EVDocument.plainTextType, EVDocument.markdownType, EVDocument.codeType] {
      let surface = try makeSurface("text", type: type)
      surface.perform(menuCommand: .selectAll, sender: nil)
      for command in [EVMenuCommand.superscript, .subscriptText, .allLigatures,
                      .showFonts, .showColors, .paragraphSpacing, .lineSpacingCustom,
                      .clearAllDirectFormatting, .pasteStyle] {
        XCTAssertFalse(surface.presentation(for: command).isEnabled, "\(type): \(command)")
      }
    }
  }

  private func makeSurface(_ text: String, type: String? = nil) throws -> EVEditorSurfaceController {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data(text.utf8), typeName: type ?? EVDocument.rtfType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 300)
    surface.viewDidLayout()
    return surface
  }

  private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
}
