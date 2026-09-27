import AppKit
import CViemCore
import ViemAppShell

@MainActor
private enum EVFormattingClipboard {
  static var snapshot: EVSelectionFormatting?
}

extension EVEditorSurfaceController {
  var canEditParagraphFormatting: Bool {
    canInspectTypography && !isVisualBlockMode
      && (try? session?.listSelection()) != nil
  }

  func performAdditionalFormatCommand(_ command: EVMenuCommand) -> Bool {
    switch command {
    case .superscript, .subscriptText:
      guard canEditTypography, let session else { return true }
      performInput {
        let target: UInt32 = command == .superscript ? 1 : 2
        let current = try session.selectedTypography()
        _ = try session.editDirectProperty(.characterScriptPosition,
          value: .scriptPosition(current.scriptPosition == target && !current.scriptMixed ? 0 : target),
          expected: session.listSelection())
      }
    case .defaultLigatures, .allLigatures, .noLigatures:
      guard canEditTypography, let session else { return true }
      performInput {
        var features = try session.selectedTypography().features.filter { !Self.ligatureTags.contains($0.tag) }
        features += Self.ligatureSettings(for: command)
        _ = try session.editDirectProperty(.characterOpenTypeFeatures,
          value: .openTypeFeatures(features.sorted { $0.tag < $1.tag }), expected: session.listSelection())
      }
    case .copyStyle:
      guard canInspectTypography else { return true }
      performInput {
        guard let selected = try self.session?.selectedFormatting() else { return }
        EVFormattingClipboard.snapshot = EVSelectionFormatting(values: selected.values.map { property, value in
          // Copy appearance: no background is an explicit transparent value
          // when pasted over a differently inherited background.
          (property, property == .characterBackground && value == nil
            ? .color(.init(red: 0, green: 0, blue: 0, alpha: 0)) : value)
        }, mixed: selected.mixed)
      }
    case .pasteStyle:
      guard canEditTypography, let session, let copied = EVFormattingClipboard.snapshot else { return true }
      guard let values = compatiblePasteValues(copied) else { return true }
      performInput { _ = try session.editDirectProperties(values, expected: session.listSelection()) }
    case .clearDirectCharacterFormatting, .clearDirectParagraphFormatting, .clearAllDirectFormatting:
      guard let session, command == .clearDirectParagraphFormatting ? canEditParagraphFormatting : canEditTypography else { return true }
      let properties = command == .clearDirectCharacterFormatting ? EVStyleProperty.characterProperties
        : command == .clearDirectParagraphFormatting ? EVStyleProperty.paragraphProperties
        : EVStyleProperty.characterProperties + EVStyleProperty.paragraphProperties
      performInput { _ = try session.editDirectProperties(properties.map { ($0, nil) }, expected: session.listSelection()) }
    case .paragraphSpacing, .lineSpacingCustom:
      guard canEditParagraphFormatting else { return true }
      showParagraphSpacing(customLineSpacing: command == .lineSpacingCustom)
    default: return false
    }
    return true
  }

  func additionalFormatPresentation(_ command: EVMenuCommand) -> EVMenuItemPresentation? {
    switch command {
    case .superscript, .subscriptText:
      let style = try? session?.selectedTypography()
      let target: UInt32 = command == .superscript ? 1 : 2
      return EVMenuItemPresentation(isEnabled: canEditTypography,
        state: style?.scriptMixed == true ? .mixed : style?.scriptPosition == target ? .on : .off)
    case .defaultLigatures, .allLigatures, .noLigatures:
      let style = try? session?.selectedTypography()
      let expected = Self.ligatureSettings(for: command)
      let selected = command == .defaultLigatures
        ? !(style?.features.contains { Self.ligatureTags.contains($0.tag) } ?? false)
        : expected.allSatisfy { expected in
          style?.features.first { $0.tag == expected.tag }?.setting == expected.setting
        }
      let mixed = (try? session?.selectedFormatting())?.mixed.contains(.characterOpenTypeFeatures) == true
      return EVMenuItemPresentation(isEnabled: canEditTypography,
        state: mixed ? .mixed : selected ? .on : .off)
    case .copyStyle: return EVMenuItemPresentation(isEnabled: canInspectTypography)
    case .pasteStyle: return EVMenuItemPresentation(isEnabled: canEditTypography
      && EVFormattingClipboard.snapshot.flatMap { compatiblePasteValues($0) } != nil)
    case .clearDirectCharacterFormatting, .clearAllDirectFormatting:
      return EVMenuItemPresentation(isEnabled: canEditTypography)
    case .clearDirectParagraphFormatting, .paragraphSpacing, .lineSpacingCustom:
      return EVMenuItemPresentation(isEnabled: canEditParagraphFormatting)
    default: return nil
    }
  }

  func paragraphFormattingPresentation(_ command: EVMenuCommand) -> EVMenuItemPresentation {
    let formatting = try? session?.selectedFormatting()
    let edit = directParagraphEdit(for: command)
    let mixed = edit.map { formatting?.mixed.contains($0.0) == true } == true
    let selected = edit.map { property, value in
      formatting?[property] == (command == .directionAutomatic ? .writingDirection(0) : value)
    } == true
    return EVMenuItemPresentation(isEnabled: canEditParagraphFormatting
      && (command != .directionAutomatic || canUseAutomaticParagraphDirection),
      state: mixed ? .mixed : selected ? .on : .off)
  }

  var canUseAutomaticParagraphDirection: Bool { backend.sourceFormat != .rtf }

  private func compatiblePasteValues(_ copied: EVSelectionFormatting) -> [(EVStyleProperty, EVStyleValue?)]? {
    guard backend.sourceFormat == .rtf else { return copied.values }
    guard let current = try? session?.selectedFormatting() else { return nil }
    var values: [(EVStyleProperty, EVStyleValue?)] = []
    for (property, value) in copied.values {
      if value == .writingDirection(0) {
        // RTF has explicit LTR/RTL controls but no automatic-direction
        // override. An already automatic value needs no source declaration.
        guard current[property] == value, !current.mixed.contains(property) else { return nil }
      } else { values.append((property, value)) }
    }
    return values
  }

  private static let ligatureTags = ["liga", "clig", "dlig", "hlig"]
  private static func ligatureSettings(for command: EVMenuCommand) -> [EVOpenTypeFeature] {
    command == .defaultLigatures ? [] : ligatureTags.map {
      EVOpenTypeFeature(tag: $0, setting: command == .allLigatures ? 1 : 0)
    }
  }

  func applyParagraphSpacing(before: Float, after: Float, expected: ViemLogicalSelectionIdentityV1) {
    guard canEditParagraphFormatting, let session else { return }
    performInput {
      _ = try session.editDirectProperties([
        (.blockMarginTop, .float(before)), (.blockMarginBottom, .float(after))
      ], expected: expected)
    }
  }

  func applyCustomLineSpacing(_ value: EVLineSpacing, expected: ViemLogicalSelectionIdentityV1) {
    guard canEditParagraphFormatting, let session else { return }
    performInput { _ = try session.editDirectProperty(.paragraphLineSpacing,
      value: .lineSpacing(value), expected: expected) }
  }

  private func showParagraphSpacing(customLineSpacing: Bool) {
    guard let session, let expected = try? session.listSelection(),
      let current = try? session.selectedFormatting(), let window = view.window else { return }
    let alert = NSAlert()
    alert.messageText = customLineSpacing ? "Line Spacing" : "Paragraph Spacing"
    alert.addButton(withTitle: "Apply")
    alert.addButton(withTitle: "Cancel")
    let content = NSStackView()
    content.orientation = .vertical; content.alignment = .leading; content.spacing = 8
    func field(_ title: String, value: Float) -> NSTextField {
      let label = NSTextField(labelWithString: title)
      let input = NSTextField(string: String(format: "%g", value))
      input.setAccessibilityLabel(title)
      input.widthAnchor.constraint(equalToConstant: 100).isActive = true
      let row = NSStackView(views: [label, input]); row.spacing = 8
      content.addArrangedSubview(row)
      return input
    }
    func number(_ property: EVStyleProperty) -> Float {
      if case let .float(value)? = current[property] { return value }; return 0
    }
    let before: NSTextField?
    let after: NSTextField?
    let multiplier: NSTextField?
    let kind: NSPopUpButton?
    if customLineSpacing {
      let popup = NSPopUpButton()
      popup.addItems(withTitles: ["Multiple", "At least (pt)", "Exactly (pt)"])
      popup.setAccessibilityLabel("Line spacing kind")
      content.addArrangedSubview(popup)
      let spacing: EVLineSpacing
      if case let .lineSpacing(value)? = current[.paragraphLineSpacing] { spacing = value }
      else { spacing = EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER), value: 1) }
      let kinds = [UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER),
        UInt32(VIEM_STYLE_LINE_SPACING_AT_LEAST), UInt32(VIEM_STYLE_LINE_SPACING_EXACT)]
      popup.selectItem(at: kinds.firstIndex(of: spacing.kind) ?? 0)
      multiplier = field("Value", value: spacing.value > 0 ? spacing.value : 1)
      kind = popup; before = nil; after = nil
    } else {
      before = field("Before (pt)", value: number(.blockMarginTop))
      after = field("After (pt)", value: number(.blockMarginBottom))
      multiplier = nil; kind = nil
    }
    alert.accessoryView = content
    alert.beginSheetModal(for: window) { [weak self] response in
      guard response == .alertFirstButtonReturn, let self else { return }
      if let multiplier, let kind {
        guard let value = Float(multiplier.stringValue), value.isFinite, value > 0 else {
          self.publishHostMessage("Line spacing must be a positive number."); return
        }
        let kinds = [UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER),
          UInt32(VIEM_STYLE_LINE_SPACING_AT_LEAST), UInt32(VIEM_STYLE_LINE_SPACING_EXACT)]
        self.applyCustomLineSpacing(.init(kind: kinds[kind.indexOfSelectedItem], value: value), expected: expected)
      } else if let before, let after {
        guard let first = Float(before.stringValue), let second = Float(after.stringValue),
          first.isFinite, second.isFinite else {
          self.publishHostMessage("Paragraph spacing must be finite numbers in points."); return
        }
        self.applyParagraphSpacing(before: first, after: second, expected: expected)
      }
    }
  }
}
