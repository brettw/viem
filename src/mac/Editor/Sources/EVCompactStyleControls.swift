import AppKit
import CViemCore
import ViemAppShell
import ViemCoreTextProvider

/// Static captions activate an inherited property without asking AppKit to edit
/// or dispatch an action through a non-editable text field.
@MainActor
final class EVStyleControlLabel: NSTextField {
    var onClick: () -> Void = {}

    override func mouseDown(with event: NSEvent) { onClick() }
}

/// A disabled native control remains inspectable and keeps its standard click
/// behavior. The enclosing view intercepts only an inherited control's first
/// click, declares its inherited value, and forwards that same event to AppKit.
@MainActor
final class EVInheritedStyleControl: NSStackView {
    var canActivate: () -> Bool = { false }
    var activate: () -> Void = {}
    var beginGesture: () -> Void = {}
    var endGesture: (_ editingText: Bool) -> Void = { _ in }

    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let hit = super.hitTest(point) else { return nil }
        guard canActivate() else { return hit }
        if isActivationOnly(hit) { return self }
        guard let control = enclosingControl(hit), !control.isEnabled else { return hit }
        return self
    }

    override func mouseDown(with event: NSEvent) {
        let point = superview?.convert(event.locationInWindow, from: nil) ?? event.locationInWindow
        guard let hit = super.hitTest(point) else { return }
        if isActivationOnly(hit) { activateWithoutPerform(); return }
        guard let control = enclosingControl(hit) else { return }
        activateAndPerform(control) {
            if let well = control as? EVStyleColorWell {
                _ = well.sendAction(well.pulldownAction, to: well.pulldownTarget)
            } else { control.mouseDown(with: event) }
        }
    }

    /// The same dispatch is shared by mouse input and interaction tests; native
    /// controls receive the event only after a successful declaration refresh.
    func activateAndPerform(_ control: NSControl, action: () -> Void) {
        guard canActivate() else { return }
        if isActivationOnly(control) { activateWithoutPerform(); return }
        let priorState = (control as? NSButton)?.state
        // Finish the previous field before opening this gesture. Otherwise its
        // delayed editing-end callback can close the new field's undo group.
        control.window?.makeFirstResponder(nil)
        beginGesture()
        defer { endGesture((control as? NSTextField)?.currentEditor() != nil) }
        activate()
        guard control.isEnabled else { return }
        if let priorState, let button = control as? NSButton { button.state = priorState }
        action()
    }

    private func isActivationOnly(_ view: NSView) -> Bool {
        if view is NSImageView { return true }
        if let label = view as? NSTextField { return !label.isEditable && !label.isSelectable }
        return false
    }

    private func activateWithoutPerform() {
        guard canActivate() else { return }
        window?.makeFirstResponder(nil)
        beginGesture()
        activate()
        endGesture(false)
    }

    private func enclosingControl(_ hit: NSView) -> NSControl? {
        var candidate: NSView? = hit
        while let view = candidate, view !== self {
            if let control = view as? NSControl { return control }
            candidate = view.superview
        }
        return nil
    }
}

/// A compact, declaration-aware typography palette. Unchecked overrides remove
/// declarations; inherited values are displayed only by the live preview.
@MainActor
final class EVCompactStyleControls: NSObject, NSTextFieldDelegate, NSComboBoxDelegate {
    let characterView = NSStackView()
    let paragraphView = NSStackView()
    let blockView = NSStackView()
    var onMutations: (([EVStyleMutation]) -> Void)?
    var onEditBegan: (() -> Void)?
    var onEditEnded: (() -> Void)?
    private(set) var hasInvalidDraft = false
    private var definition: EVStyleDefinition?
    private var documentID: UInt64?
    private var theme = EVTheme.midnight
    private let alignmentValues: [UInt32] = [UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_START), UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER), UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_END)]
    private var updating = false
    private var publishingFontChange = false
    private var endFontEditAfterPublication = false
    private var editable = false
    private var inheritedGestureActive = false
    private let family = NSComboBox()
    private let face = NSPopUpButton()
    private let sizeUnit = NSPopUpButton()
    private var fontSizeBasis: Float?
    private var allowsPercentageSize = true
    private let fallbackButton = NSButton()
    private(set) var fallbackPopover: NSPopover?
    private let featureButton = NSButton()
    private var fields: [EVStyleProperty: NSTextField] = [:]
    private var steppers: [EVStyleProperty: EVStyleStepper] = [:]
    private var lastLineValues: [UInt32: Float] = [:]
    private var buttons: [EVStyleProperty: NSButton] = [:]
    private var wells: [EVStyleProperty: EVStyleColorWell] = [:]
    private var directions: [EVStyleProperty: NSPopUpButton] = [:]
    private var overrideButtons: [EVStyleProperty: NSButton] = [:]
    private let alignment = NSSegmentedControl()
    private let lineKind = NSPopUpButton()
    private let lineValue = NSTextField()
    private let lineUnit = EVStyleControlLabel(labelWithString: "")
    private var fontFaces: [EVFontFace] = []
    private var isBaseParagraph: Bool { definition?.flags.contains(.baseParagraph) == true }
    private let sectionSpacing = NSFont.systemFontSize

    /// Non-selectable divider row between the picker's two portable system
    /// entries and the sorted list of installed fonts. NSComboBox has no
    /// native separator item, so a selection of this text is ignored instead.
    private static let familyListSeparator = String(repeating: "─", count: 10)

    private static func familyPickerItems() -> [String] {
        let installed = NSFontManager.shared.availableFontFamilies.sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
        return [
            EVFontCatalog.displayFamilyName(for: EVFontCatalog.systemDefaultFamily),
            EVFontCatalog.displayFamilyName(for: EVFontCatalog.systemMonospaceFamily),
            familyListSeparator,
        ] + installed
    }

    override init() {
        super.init()
        family.addItems(withObjectValues: Self.familyPickerItems())
        family.numberOfVisibleItems = 20
        family.completes = true
        family.usesDataSource = false
        family.delegate = self
        family.target = self
        family.action = #selector(familyChanged(_:))
        family.setAccessibilityLabel("Font family")
        family.widthAnchor.constraint(greaterThanOrEqualToConstant: 180).isActive = true
        family.setContentHuggingPriority(.defaultLow, for: .horizontal)
        face.target = self
        face.action = #selector(faceChanged(_:))
        face.setAccessibilityLabel("Font face")
        face.widthAnchor.constraint(equalToConstant: 140).isActive = true
        fallbackButton.title = "…"
        fallbackButton.bezelStyle = .texturedRounded
        fallbackButton.target = self
        fallbackButton.action = #selector(showFallbacks(_:))
        fallbackButton.toolTip = "Edit ordered fallback fonts"
        fallbackButton.setAccessibilityLabel("Fallback fonts")
        fallbackButton.widthAnchor.constraint(equalToConstant: 28).isActive = true
        let familyRow = row([
            overrideGroup(.characterFontFamilies, control: row([family, fallbackButton, face], spacing: 4)),
            numeric(.characterSize, title: "Size", width: 44, showsLabel: false),
        ])
        let emphasis = row([
            toggle(.characterBold, title: "B", font: .boldSystemFont(ofSize: 14)),
            toggle(.characterSlant, title: "I", font: NSFontManager.shared.convert(.systemFont(ofSize: 14), toHaveTrait: .italicFontMask)),
            toggle(.characterUnderline, title: "U"), toggle(.characterStrikethrough, title: "S̶"),
        ], spacing: sectionSpacing)
        featureButton.image = EVStyleIcons.image(.features)
        featureButton.bezelStyle = .texturedRounded
        featureButton.target = self
        featureButton.action = #selector(showFeatures(_:))
        featureButton.toolTip = "OpenType features supported by this font"
        featureButton.setAccessibilityLabel("OpenType features")
        featureButton.widthAnchor.constraint(equalToConstant: 32).isActive = true
        featureButton.heightAnchor.constraint(equalToConstant: 27).isActive = true
        let appearance = row([
            labeled("", control: emphasis),
            color(.characterForeground, title: "Text Color"),
            color(.characterBackground, title: "Background Color"),
            NSView(),
            labeled("OpenType", control: overrideGroup(.characterOpenTypeFeatures, control: featureButton), property: .characterOpenTypeFeatures),
        ])
        let metrics = row([
            numeric(.characterLetterSpacing, title: "Tracking", icon: .tracking),
            direction(.characterDirection, title: "Direction"),
        ])
        configure(characterView, rows: [familyRow, axisRows, appearance, separator(), metrics])

        alignment.segmentCount = 3
        alignment.trackingMode = .selectOne
        for (index, symbol) in [EVStyleIcons.Symbol.alignStart, .alignCenter, .alignEnd].enumerated() {
            alignment.setImage(EVStyleIcons.image(symbol), forSegment: index)
            alignment.setWidth(36, forSegment: index)
            alignment.setToolTip(["Align start", "Center", "Align end"][index], forSegment: index)
        }
        alignment.target = self
        alignment.action = #selector(alignmentChanged(_:))
        alignment.setAccessibilityLabel("Paragraph alignment")
        let paraToolbar = row([overrideGroup(.paragraphAlignment, control: alignment), NSView(), direction(.paragraphBaseDirection, title: "Direction")])
        let indents = row([
            numeric(.paragraphLeadingIndent, title: "Start indent", icon: .leadingIndent),
            numeric(.paragraphTrailingIndent, title: "End indent", icon: .trailingIndent),
            numeric(.paragraphFirstLineIndent, title: "First line", icon: .firstIndent),
        ])
        lineKind.addItems(withTitles: ["Normal", "Multiple", "At least", "Exactly"])
        for (index, value) in [VIEM_STYLE_LINE_SPACING_NORMAL, VIEM_STYLE_LINE_SPACING_MULTIPLIER, VIEM_STYLE_LINE_SPACING_AT_LEAST, VIEM_STYLE_LINE_SPACING_EXACT].enumerated() {
            lineKind.item(at: index)?.tag = Int(value)
        }
        lineKind.target = self
        lineKind.action = #selector(lineSpacingChanged(_:))
        lineKind.setAccessibilityLabel("Line spacing kind")
        lineValue.delegate = self
        lineValue.tag = Int(EVStyleProperty.paragraphLineSpacing.rawValue)
        lineValue.setAccessibilityLabel("Line spacing value")
        lineValue.widthAnchor.constraint(equalToConstant: 48).isActive = true
        lineUnit.onClick = { [weak self] in
            guard let self else { return }
            self.activateFromLabel(.paragraphLineSpacing, label: self.lineUnit)
        }
        lineUnit.textColor = .secondaryLabelColor
        lineUnit.font = .systemFont(ofSize: 11)
        lineUnit.setAccessibilityLabel("Line spacing unit")
        lineUnit.setContentCompressionResistancePriority(.required, for: .horizontal)
        lineUnit.widthAnchor.constraint(equalToConstant: 14).isActive = true
        let lineControls = row([
            icon(.lineSpacing), lineKind, lineValue,
            stepper(.paragraphLineSpacing, title: "Line spacing value"), lineUnit,
        ], spacing: 3)
        let line = labeled(
            "Line spacing",
            control: overrideGroup(.paragraphLineSpacing, control: lineControls),
            property: .paragraphLineSpacing
        )
        configure(paragraphView, rows: [paraToolbar, separator(), indents, row([line, NSView()])])

        // Each row describes one physical side of the CSS box. Reuse the same
        // declaration-aware native controls and color chooser as text highlight.
        let background = row([color(.blockBackground, title: "Background Color"), NSView()])
        let sides: [(String, EVStyleProperty, EVStyleProperty, EVStyleProperty, EVStyleProperty)] = [
            ("Top", .blockMarginTop, .blockPaddingTop, .blockBorderTopWidth, .blockBorderTopColor),
            ("Right", .blockMarginRight, .blockPaddingRight, .blockBorderRightWidth, .blockBorderRightColor),
            ("Bottom", .blockMarginBottom, .blockPaddingBottom, .blockBorderBottomWidth, .blockBorderBottomColor),
            ("Left", .blockMarginLeft, .blockPaddingLeft, .blockBorderLeftWidth, .blockBorderLeftColor),
        ]
        let sideRows = sides.map { side, margin, padding, border, borderColor in
            let label = NSTextField(labelWithString: side)
            label.font = .systemFont(ofSize: 11, weight: .medium)
            label.widthAnchor.constraint(equalToConstant: 42).isActive = true
            return row([label,
                numeric(margin, title: margin.displayName, width: 45, showsLabel: false),
                numeric(padding, title: padding.displayName, width: 45, showsLabel: false),
                numeric(border, title: border.displayName, width: 45, showsLabel: false),
                color(borderColor, title: "", showsLabel: false), NSView()], spacing: 12)
        }
        let columns = row([NSTextField(labelWithString: ""),
            NSTextField(labelWithString: "Margin"), NSTextField(labelWithString: "Padding"),
            NSTextField(labelWithString: "Border weight"), NSTextField(labelWithString: "Color"), NSView()], spacing: 12)
        for (index, width) in [CGFloat(42), 109, 109, 109, 52].enumerated() {
            columns.arrangedSubviews[index].widthAnchor.constraint(equalToConstant: width).isActive = true
            (columns.arrangedSubviews[index] as? NSTextField)?.font = .systemFont(ofSize: 10, weight: .medium)
        }
        configure(blockView, rows: [background, columns] + sideRows)
        blockView.spacing = 5
        blockView.edgeInsets = NSEdgeInsets(top: 5, left: 10, bottom: 5, right: 10)
    }

    func configure(_ definition: EVStyleDefinition?, theme: EVTheme = .midnight, documentID: UInt64? = nil, fontSizeBasis: Float? = nil, allowsPercentageSize: Bool = true) {
        fallbackPopover?.close()
        fallbackPopover = nil
        let activeLineEditor = lineValue.currentEditor() as? NSTextView
        let activeLineDraft = activeLineEditor.map { ($0.string, $0.selectedRange()) }
        self.theme = theme
        let wasUpdating = updating
        updating = true
        defer { updating = wasUpdating }
        if self.definition?.key != definition?.key || self.documentID != documentID {
            lastLineValues = [:]
            for well in wells.values { well.dismissColorControls() }
        }
        self.documentID = documentID
        self.definition = definition
        self.fontSizeBasis = fontSizeBasis
        self.allowsPercentageSize = allowsPercentageSize
        editable = definition?.capabilities.contains(.declarations) == true
        hasInvalidDraft = false
        refreshFontControls()
        fallbackButton.isEnabled = isOverridden(.characterFontFamilies)
        let fallbackCount = max(0, stringList(.characterFontFamilies).count - 1)
        fallbackButton.toolTip = "Edit ordered fallback fonts (\(fallbackCount))"
        buttons[.characterBold]?.state = isOverridden(.characterBold) && boolean(.characterBold) ? .on : .off
        buttons[.characterSlant]?.state = isOverridden(.characterSlant) && unsigned(.characterSlant) != 0 ? .on : .off
        for property in [EVStyleProperty.characterUnderline, .characterStrikethrough] { buttons[property]?.state = isOverridden(property) && boolean(property) ? .on : .off }
        for (property, button) in buttons { button.isEnabled = isOverridden(property); setHelp(button, property) }
        sizeUnit.item(withTitle: "%")?.isEnabled = canUsePercentageSize
        sizeUnit.item(withTitle: "%")?.isHidden = !allowsPercentageSize
        if isOverridden(.characterSize) {
            sizeUnit.selectItem(withTitle: declaredSizePercentage == nil ? "pt" : "%")
        } else { sizeUnit.select(nil) }
        sizeUnit.isEnabled = isOverridden(.characterSize)
        setHelp(sizeUnit, .characterSize)
        for (property, field) in fields {
            let displayValue = property == .characterSize ? sizeDisplayValue : number(property)
            field.stringValue = isOverridden(property) ? Self.numberText(displayValue) : ""
            field.isEnabled = isOverridden(property)
            field.textColor = .labelColor
            setHelp(field, property)
            synchronizeStepper(property, value: Double(displayValue), enabled: field.isEnabled)
        }
        refreshThemeColors(theme)
        for (property, popup) in directions { popup.selectItem(at: isOverridden(property) ? min(2, Int(unsigned(property))) : -1); popup.isEnabled = isOverridden(property); setHelp(popup, property) }
        for (property, button) in overrideButtons {
            button.isEnabled = canEdit(property) && !isBaseParagraph
            button.state = isBaseParagraph || definition?.properties[property]?.isDeclared == true
            || property == .characterFontFamilies && (definition?.properties[.characterWeight]?.isDeclared == true || definition?.properties[.characterFontAxes]?.isDeclared == true) ? .on : .off
        }
        alignment.selectedSegment = isOverridden(.paragraphAlignment) ? (alignmentValues.firstIndex(of: unsigned(.paragraphAlignment)) ?? 0) : -1
        alignment.isEnabled = isOverridden(.paragraphAlignment)
        lineKind.isEnabled = isOverridden(.paragraphLineSpacing)
        if case let .lineSpacing(value)? = definition?.properties[.paragraphLineSpacing]?.effective {
            let displayKind = Self.displayLineSpacingKind(value)
            let displayValue = displayKind == UInt32(VIEM_STYLE_LINE_SPACING_NORMAL)
                ? Float(1) : Self.roundedLineSpacingValue(value.value)
            lineKind.selectItem(withTag: Int(displayKind))
            if value.kind != UInt32(VIEM_STYLE_LINE_SPACING_NORMAL) {
                lastLineValues[value.kind] = Self.roundedLineSpacingValue(value.value)
            }
            lineValue.stringValue = Self.lineSpacingNumberText(displayValue)
        } else { lineKind.selectItem(at: 0); lineValue.stringValue = "1" }
        if !isOverridden(.paragraphLineSpacing) { lineKind.select(nil); lineValue.stringValue = "" }
        lineValue.isEnabled = isOverridden(.paragraphLineSpacing) && lineKind.indexOfSelectedItem >= 0
        updateLineSpacingUnit(lineKind.selectedItem.map { UInt32($0.tag) })
        synchronizeStepper(.paragraphLineSpacing, value: Double(Float(lineValue.stringValue) ?? 0), enabled: lineValue.isEnabled)
        if let activeLineEditor, let (draft, selection) = activeLineDraft {
            activeLineEditor.string = draft
            let start = min(selection.location, draft.utf16.count)
            activeLineEditor.setSelectedRange(NSRange(
                location: start,
                length: min(selection.length, draft.utf16.count - start)
            ))
        }
    }

    private var listedFontFaces: [EVFontFace]?
    private var listedFontInstances: [EVFontInstance]?
    private var listedVariableFont = false
    private var rebuildFaceMenu = false
    private func refreshFontControls() {
        let wasUpdating = updating
        updating = true
        defer { updating = wasUpdating }
        let chosen = stringList(.characterFontFamilies).first ?? "Helvetica"
        fontFaces = EVFontCatalog.faces(for: chosen)
        let displayFamily = EVFontCatalog.displayFamilyName(for: chosen)
        if family.objectValues.contains(where: { ($0 as? String) == displayFamily }) {
            family.selectItem(withObjectValue: displayFamily)
        } else if family.indexOfSelectedItem >= 0 {
            family.deselectItem(at: family.indexOfSelectedItem)
        }
        setFamilyText(isOverridden(.characterFontFamilies) ? displayFamily : "")
        family.isEnabled = isOverridden(.characterFontFamilies)
        let info = EVFontVariations.info(for: chosen)
        rebuildFaceMenu = listedFontFaces != fontFaces || listedFontInstances != info.instances || listedVariableFont != !info.axes.isEmpty
        if rebuildFaceMenu {
            face.removeAllItems()
            for member in fontFaces { face.addItem(withTitle: member.styleName) }
            listedFontFaces = fontFaces; listedFontInstances = info.instances; listedVariableFont = !info.axes.isEmpty
        }
        // A popup selects its first item automatically when populated. An
        // unresolved request must not appear to select that unrelated face.
        face.select(nil)
        if isOverridden(.characterFontFamilies), let current = currentFontFace, let index = fontFaces.firstIndex(of: current) {
            face.selectItem(at: index)
        }
        face.isEnabled = isOverridden(.characterFontFamilies) && !fontFaces.isEmpty
        refreshAxisControls(chosen)
        featureButton.isEnabled = isOverridden(.characterOpenTypeFeatures) && !EVFontCatalog.features(for: chosen).isEmpty
    }

    private var currentFontFace: EVFontFace? {
        let chosen = stringList(.characterFontFamilies).first ?? "Helvetica"
        let weight = number(.characterWeight, fallback: 400)
        if let exact = fontFaces.first(where: { $0.postScriptName == chosen }), Float(exact.weight) == weight || !EVFontVariations.info(for: chosen).axes.isEmpty { return exact }
        let italic = EVFontCatalog.face(named: chosen)?.italic ?? false
        let matching = fontFaces.filter { Float($0.weight) == weight && $0.italic == italic }
        return matching.first { $0.postScriptName == chosen } ?? matching.first
    }

    private let axisRows = NSStackView()
    private var axisControls: [String: (EVFontAxisSlider, NSTextField)] = [:]
    private var axisIdentity = ""
    private var fontInstances: [EVFontInstance] = []
    private var axisValues: [String: Double] {
        if case let .string(value)? = definition?.properties[.characterFontAxes]?.effective { return EVFontVariations.decode(value) }
        return [:]
    }
    private func setFontAxes(_ values: [String: Double]) {
        let weight = UInt32(min(1000, max(1, (values["wght"] ?? Double(number(.characterWeight, fallback: 400))).rounded())))
        publishFontMutations([.setDeclaration(.characterFontFamilies, .stringList(stringList(.characterFontFamilies))), .setDeclaration(.characterFontAxes, .string(EVFontVariations.encode(values))), .setDeclaration(.characterWeight, .unsigned(weight))])
    }
    private func refreshAxisControls(_ chosen: String) {
        let info = EVFontVariations.info(for: chosen)
        fontInstances = info.instances
        if !info.axes.isEmpty {
            if rebuildFaceMenu {
                for instance in fontInstances { face.addItem(withTitle: instance.name) }
                face.addItem(withTitle: "Custom")
            }
            if let index = fontInstances.firstIndex(where: { instance in info.axes.allSatisfy { abs(instance.values[$0.tag]! - (axisValues[$0.tag] ?? $0.defaultValue)) < 0.001 } }) {
                face.selectItem(at: fontFaces.count + index)
            } else { face.selectItem(at: face.numberOfItems - 1) }
        }
        let identity = chosen
        if axisIdentity != identity {
            axisIdentity = identity
            for view in axisRows.arrangedSubviews { axisRows.removeArrangedSubview(view); view.removeFromSuperview() }
            axisControls.removeAll()
            axisRows.orientation = .vertical; axisRows.alignment = .leading; axisRows.spacing = 6
            let visible = info.axes.filter { !$0.hidden && $0.minimum < $0.maximum }
            for start in stride(from: 0, to: visible.count, by: 2) {
                var columns: [NSView] = []
                for axis in visible[start..<min(start + 2, visible.count)] {
                    let label = NSTextField(labelWithString: axis.name)
                    label.font = .systemFont(ofSize: 11)
                    let slider = EVFontAxisSlider()
                    slider.minValue = axis.minimum; slider.maxValue = axis.maximum; slider.isContinuous = true
                    slider.setAccessibilityLabel(axis.name)
                    slider.axisTag = axis.tag; slider.fontIdentity = identity; slider.target = self; slider.action = #selector(axisChanged(_:))
                    slider.widthAnchor.constraint(equalToConstant: 240).isActive = true
                    slider.beginGesture = { [weak self] in guard let self, !self.inheritedGestureActive else { return }; self.onEditEnded?(); self.onEditBegan?() }
                    slider.endGesture = { [weak self] in guard let self, !self.inheritedGestureActive else { return }; self.onEditEnded?() }
                    let column = NSStackView(views: [label, slider]); column.orientation = .vertical; column.alignment = .leading; column.spacing = 2
                    axisControls[axis.tag] = (slider, label); columns.append(overrideGroup(.characterFontFamilies, control: column, showsCheckbox: false))
                }
                axisRows.addArrangedSubview(row(columns, spacing: 20))
            }
        }
        let saved = axisValues
        let bold: Bool
        if case let .boolean(value)? = definition?.properties[.characterBold]?.effective { bold = value } else { bold = false }
        let effective = EVFontVariations.effective(info, saved: saved, weight: Double(number(.characterWeight, fallback: 400)) + (bold ? 300 : 0), bold: bold, slant: unsigned(.characterSlant))
        for axis in info.axes {
            guard let (slider, label) = axisControls[axis.tag] else { continue }
            let value = min(axis.maximum, max(axis.minimum, saved[axis.tag] ?? axis.defaultValue))
            slider.doubleValue = value; slider.isEnabled = isOverridden(.characterFontFamilies)
            let actual = effective[axis.tag] ?? value
            label.stringValue = "\(axis.name) \(String(format: "%.0f", value))" + (abs(actual - value) > 0.001 ? " (effective \(String(format: "%.0f", actual)))" : "")
        }
        axisRows.isHidden = axisControls.isEmpty
    }
    @objc private func axisChanged(_ sender: EVFontAxisSlider) {
        guard !updating, editable, sender.fontIdentity == axisIdentity else { return }
        let chosen = stringList(.characterFontFamilies).first ?? "Helvetica"
        var values = EVFontVariations.info(for: chosen).defaults
        values.merge(axisValues) { _, saved in saved }
        values[sender.axisTag] = min(sender.maxValue, max(sender.minValue, sender.doubleValue.rounded()))
        setFontAxes(values)
    }

    private func setFamilyText(_ value: String) {
        let wasUpdating = updating
        updating = true
        defer { updating = wasUpdating }
        family.stringValue = value
        // During a combo selection AppKit may still own an older field-editor
        // string. Keep it in sync so a later action/end notification cannot
        // restore the previous family after the selected value was committed.
        if let editor = family.currentEditor() as? NSTextView, editor.string != value {
            let selection = editor.selectedRange()
            editor.string = value
            let start = min(selection.location, value.utf16.count)
            editor.setSelectedRange(NSRange(location: start, length: min(selection.length, value.utf16.count - start)))
        }
    }

    func refreshThemeColors(_ theme: EVTheme) {
        self.theme = theme
        for (property, well) in wells {
            let resolved = definition?.properties[property]
            if property == .characterForeground && (resolved?.usesThemeDefault == true || resolved == nil) {
                well.color = theme.foreground.color
            } else if case let .color(value)? = resolved?.effective { well.color = value.appKitColor }
            else if property == .blockBackground { well.color = newBlockBackground.appKitColor }
            else { well.color = .clear }
            well.isEnabled = isOverridden(property)
            setHelp(well, property)
        }
    }

    private func configure(_ stack: NSStackView, rows: [NSView]) {
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.edgeInsets = NSEdgeInsets(top: 10, left: 10, bottom: 10, right: 10)
        for row in rows {
            stack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -20).isActive = true
        }
    }
    private func row(_ views: [NSView], spacing: CGFloat? = nil) -> NSStackView {
        let stack = NSStackView(views: views)
        stack.orientation = .horizontal
        stack.alignment = .centerY
        stack.spacing = spacing ?? sectionSpacing
        return stack
    }
    private func labeled(_ title: String, control: NSView, property: EVStyleProperty? = nil) -> NSStackView {
        let label = EVStyleControlLabel(labelWithString: title)
        label.onClick = { [weak self, weak label] in
            guard let property else { return }
            self?.activateFromLabel(property, label: label)
        }
        label.font = .systemFont(ofSize: 10, weight: .medium)
        label.textColor = .secondaryLabelColor
        // Empty captions reserve the same space above the emphasis buttons.
        label.heightAnchor.constraint(equalToConstant: ceil(label.font!.ascender - label.font!.descender + label.font!.leading)).isActive = true
        let stack = NSStackView(views: [label, control])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 4
        return stack
    }
    private func icon(_ symbol: EVStyleIcons.Symbol) -> NSImageView {
        let view = NSImageView(image: EVStyleIcons.image(symbol))
        view.widthAnchor.constraint(equalToConstant: 20).isActive = true
        return view
    }
    private func numeric(_ property: EVStyleProperty, title: String, icon symbol: EVStyleIcons.Symbol? = nil, width: CGFloat = 65, showsLabel: Bool = true) -> NSView {
        let field = NSTextField()
        field.delegate = self
        field.tag = Int(property.rawValue)
        field.setAccessibilityLabel(title)
        field.alignment = .right
        field.widthAnchor.constraint(equalToConstant: width).isActive = true
        fields[property] = field
        let unit: NSView
        if property == .characterSize {
            sizeUnit.addItems(withTitles: ["pt", "%"])
            sizeUnit.menu?.autoenablesItems = false
            sizeUnit.target = self
            sizeUnit.action = #selector(sizeUnitChanged(_:))
            sizeUnit.setAccessibilityLabel("Font size unit")
            sizeUnit.setContentCompressionResistancePriority(.required, for: .horizontal)
            unit = sizeUnit
        } else {
            let label = EVStyleControlLabel(labelWithString: "pt")
            label.onClick = { [weak self, weak label] in self?.activateFromLabel(property, label: label) }
            label.textColor = .secondaryLabelColor
            label.font = .systemFont(ofSize: 11)
            label.setContentCompressionResistancePriority(.required, for: .horizontal)
            unit = label
        }
        var items: [NSView] = symbol.map { [icon($0)] } ?? []
        items += [field, stepper(property, title: title), unit]
        let controls = overrideGroup(property, control: row(items, spacing: 3))
        return showsLabel ? labeled(title, control: controls, property: property) : controls
    }
    private func stepper(_ property: EVStyleProperty, title: String) -> EVStyleStepper {
        let control = EVStyleStepper()
        control.controlSize = .small
        control.setContentCompressionResistancePriority(.required, for: .horizontal)
        control.valueWraps = false
        control.autorepeat = true
        control.isContinuous = true
        control.tag = Int(property.rawValue)
        control.target = self
        control.action = #selector(stepperChanged(_:))
        control.setAccessibilityLabel("Adjust \(title.lowercased())")
        control.onGestureBegan = { [weak self] in
            guard let self, !self.inheritedGestureActive else { return }
            self.onEditEnded?()
            self.onEditBegan?()
        }
        control.onGestureEnded = { [weak self] in
            guard let self, !self.inheritedGestureActive else { return }
            self.onEditEnded?()
        }
        steppers[property] = control
        return control
    }

    private func synchronizeStepper(_ property: EVStyleProperty, value: Double, enabled: Bool) {
        guard let control = steppers[property] else { return }
        if property == .characterSize, sizeUnit.titleOfSelectedItem == "%" {
            control.increment = 1
            control.minValue = 10
            control.maxValue = 1000
            control.doubleValue = value
            control.isEnabled = enabled
            control.toolTip = "Font size percentage: a whole number from 10 to 1000."
            return
        }
        let relativeLineSpacing = property == .paragraphLineSpacing
            && (lineKind.selectedItem?.tag == Int(VIEM_STYLE_LINE_SPACING_NORMAL)
                || lineKind.selectedItem?.tag == Int(VIEM_STYLE_LINE_SPACING_MULTIPLIER))
        let positive = property == .characterSize || relativeLineSpacing
        control.increment = relativeLineSpacing || property == .characterLetterSpacing ? 0.1 : 1
        control.minValue = positive ? min(max(value, Double(Float.leastNormalMagnitude)), 0.1)
            : property == .paragraphLineSpacing || EVStyleProperty.nonnegativeBlockProperties.contains(property) ? 0 : -Double(Float.greatestFiniteMagnitude)
        control.maxValue = Double(Float.greatestFiniteMagnitude)
        control.doubleValue = value
        control.isEnabled = enabled
        setHelp(control, property)
    }

    private func updateLineSpacingUnit(_ kind: UInt32?) {
        if kind == UInt32(VIEM_STYLE_LINE_SPACING_NORMAL)
            || kind == UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER) {
            lineUnit.stringValue = "×"
            return
        }
        if kind == UInt32(VIEM_STYLE_LINE_SPACING_AT_LEAST) || kind == UInt32(VIEM_STYLE_LINE_SPACING_EXACT) {
            lineUnit.stringValue = "pt"
            return
        }
        lineUnit.stringValue = ""
    }

    @objc private func stepperChanged(_ sender: EVStyleStepper) {
        guard !updating, editable, let property = EVStyleProperty(rawValue: UInt32(sender.tag)) else { return }
        let field = property == .paragraphLineSpacing ? lineValue : fields[property]
        guard let field else { return }
        field.stringValue = property == .paragraphLineSpacing
            ? Self.lineSpacingNumberText(Float(sender.doubleValue))
            : Self.numberText(Float(sender.doubleValue))
        if property == .paragraphLineSpacing { lineSpacingChanged(sender); return }
        controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: field))
    }

    private func toggle(_ property: EVStyleProperty, title: String, font: NSFont = .systemFont(ofSize: 14)) -> NSView {
        let button = NSButton(title: title, target: self, action: #selector(toggleChanged(_:)))
        button.setButtonType(.pushOnPushOff)
        button.bezelStyle = .texturedRounded
        button.font = font
        if property == .characterUnderline {
            button.attributedTitle = NSAttributedString(string: title, attributes: [
                .font: font, .foregroundColor: NSColor.labelColor,
                .underlineStyle: NSUnderlineStyle.single.rawValue,
            ])
        }
        button.tag = Int(property.rawValue)
        button.setAccessibilityLabel(property == .characterBold ? "Bold" : property == .characterSlant ? "Italic" : property.displayName)
        button.widthAnchor.constraint(equalToConstant: 31).isActive = true
        button.heightAnchor.constraint(equalToConstant: 27).isActive = true
        buttons[property] = button
        return overrideGroup(property, control: button)
    }


    private func overrideGroup(_ property: EVStyleProperty, control: NSView, showsCheckbox: Bool = true) -> NSView {
        let checkbox = NSButton(checkboxWithTitle: "", target: self, action: #selector(overrideChanged(_:)))
        checkbox.controlSize = .small
        checkbox.setContentCompressionResistancePriority(.required, for: .horizontal)
        checkbox.tag = Int(property.rawValue)
        checkbox.toolTip = "Override inherited"
        checkbox.setAccessibilityLabel("Override \(property.displayName.lowercased())")
        if showsCheckbox { overrideButtons[property] = checkbox }
        let wrapper = EVInheritedStyleControl(views: [control])
        wrapper.orientation = .horizontal
        wrapper.spacing = 0
        wrapper.canActivate = { [weak self] in
            guard let self else { return false }
            return self.canEdit(property) && !self.isOverridden(property)
        }
        wrapper.activate = { [weak self] in self?.enableOverride(property) }
        wrapper.beginGesture = { [weak self] in
            guard let self else { return }
            self.onEditEnded?()
            self.onEditBegan?()
            self.inheritedGestureActive = true
        }
        wrapper.endGesture = { [weak self] editingText in
            self?.inheritedGestureActive = false
            // A text click begins a continuing field edit. Keep activation and
            // subsequent typing in the same undo gesture until editing ends.
            if !editingText { self?.onEditEnded?() }
        }
        return showsCheckbox ? row([checkbox, wrapper], spacing: 3) : wrapper
    }

    private func canEdit(_ property: EVStyleProperty) -> Bool {
        editable && (!EVStyleProperty.paragraphProperties.contains(property) || definition?.kind != .character)
            && (!EVStyleProperty.blockProperties.contains(property) || definition?.kind != .character)
    }

    private func isOverridden(_ property: EVStyleProperty) -> Bool {
        canEdit(property) && (isBaseParagraph || definition?.properties[property]?.isDeclared == true
            || property == .characterFontFamilies && (definition?.properties[.characterWeight]?.isDeclared == true || definition?.properties[.characterFontAxes]?.isDeclared == true))
    }

    // A missing fill is different from an explicitly transparent color. Seed
    // a new fill with an opaque canvas color so RGB-only picker gestures are
    // visible; retain any inherited or previously chosen alpha unchanged.
    private var newBlockBackground: EVStyleColor {
        EVStyleColor(red: Float(theme.background.red), green: Float(theme.background.green),
            blue: Float(theme.background.blue), alpha: 1)
    }

    private func enableOverride(_ property: EVStyleProperty) {
        guard canEdit(property), !isOverridden(property) else { return }
        var value = definition?.properties[property]?.effective
        if property == .blockBackground, value == nil { value = .color(newBlockBackground) }
        if property == .characterBackground, value == nil {
            value = .color(EVStyleColor(red: 0, green: 0, blue: 0, alpha: 0))
        }
        if EVStyleProperty.blockBorderColors.contains(property), value == nil {
            value = .color(EVStyleColor(red: 0, green: 0, blue: 0, alpha: 1))
        }
        if EVStyleProperty.blockProperties.contains(property), value == nil { value = .float(0) }
        if property == .characterForeground, definition?.properties[property]?.usesThemeDefault == true {
            let color = theme.foreground
            value = .color(EVStyleColor(red: Float(color.red), green: Float(color.green), blue: Float(color.blue), alpha: Float(color.alpha)))
        }
        if property == .characterFontFamilies, let value {
            send([.setDeclaration(property, value), .setDeclaration(.characterWeight, definition?.properties[.characterWeight]?.effective ?? .unsigned(400)), .setDeclaration(.characterFontAxes, definition?.properties[.characterFontAxes]?.effective ?? .string("{}"))])
        } else if let value { send([.setDeclaration(property, value)]) }
    }

    private func activateFromLabel(_ property: EVStyleProperty, label: NSView?) {
        guard canEdit(property), !isOverridden(property) else { return }
        label?.window?.makeFirstResponder(nil)
        onEditEnded?()
        onEditBegan?()
        enableOverride(property)
        onEditEnded?()
    }

    @objc private func overrideChanged(_ sender: NSButton) {
        guard !isBaseParagraph, let property = EVStyleProperty(rawValue: UInt32(sender.tag)) else { return }
        if sender.state == .on { enableOverride(property) }
        else { send([.clearDeclaration(property)]) }
    }

    private func color(_ property: EVStyleProperty, title: String, showsLabel: Bool = true) -> NSView {
        let well = EVStyleColorWell(frame: .zero)
        well.target = self
        well.action = #selector(colorChanged(_:))
        well.tag = Int(property.rawValue)
        well.setAccessibilityLabel(property == .characterForeground ? "Text color" : property == .characterBackground ? "Background color" : property.displayName)
        well.widthAnchor.constraint(equalToConstant: 31).isActive = true
        well.heightAnchor.constraint(equalToConstant: 27).isActive = true
        wells[property] = well
        let controls = overrideGroup(property, control: well)
        return showsLabel ? labeled(title, control: controls, property: property) : controls
    }
    private func direction(_ property: EVStyleProperty, title: String) -> NSView {
        let popup = NSPopUpButton()
        popup.addItems(withTitles: ["Automatic", "Left to right", "Right to left"])
        popup.target = self
        popup.action = #selector(directionChanged(_:))
        popup.tag = Int(property.rawValue)
        popup.setAccessibilityLabel("\(property == .characterDirection ? "Character" : "Paragraph") direction")
        directions[property] = popup
        return labeled(title, control: overrideGroup(property, control: popup), property: property)
    }
    private func separator() -> NSView { let view = NSBox(); view.boxType = .separator; return view }
    private func setHelp(_ control: NSView, _ property: EVStyleProperty) {
        control.toolTip = isBaseParagraph ? "Defined by Base Paragraph." : definition?.properties[property]?.isDeclared == true ? "Declared in this style. Uncheck the override to inherit." : "Inherited from the parent style."
    }
    private func number(_ property: EVStyleProperty, fallback: Float = 0) -> Float {
        switch definition?.properties[property]?.effective { case let .float(value)?: value; case let .unsigned(value)?: Float(value); default: fallback }
    }
    private var canUsePercentageSize: Bool {
        allowsPercentageSize && !isBaseParagraph
            && (definition?.kind == .character || definition?.parentKey != nil)
    }
    private var declaredSizePercentage: UInt32? {
        if case let .percentage(value)? = definition?.properties[.characterSize]?.declared { return value }
        return nil
    }
    private var sizeDisplayValue: Float {
        declaredSizePercentage.map { Float($0) } ?? number(.characterSize, fallback: 14)
    }
    @objc private func sizeUnitChanged(_ sender: NSPopUpButton) {
        guard !updating, isOverridden(.characterSize) else { return }
        let points = number(.characterSize, fallback: 14)
        let value: EVStyleValue
        if sender.titleOfSelectedItem == "%" {
            guard canUsePercentageSize, let basis = fontSizeBasis, basis.isFinite, basis > 0 else {
                sender.selectItem(withTitle: "pt")
                return
            }
            // Unit changes retain the preview's appearance as closely as the
            // integer percentage range allows. Resolved point sizes stay exact.
            value = .percentage(UInt32(min(1000, max(10, (Double(points) / Double(basis) * 100).rounded()))))
        } else {
            value = .float(points)
        }
        hasInvalidDraft = false
        send([.setDeclaration(.characterSize, value)])
    }
    private func unsigned(_ property: EVStyleProperty) -> UInt32 {
        switch definition?.properties[property]?.effective { case let .fontSlant(value)?, let .writingDirection(value)?, let .paragraphAlignment(value)?: value; default: 0 }
    }
    private func boolean(_ property: EVStyleProperty) -> Bool { if case let .boolean(value)? = definition?.properties[property]?.effective { return value }; return false }
    private func string(_ property: EVStyleProperty) -> String { if case let .string(value)? = definition?.properties[property]?.effective { return value }; return "" }
    private func stringList(_ property: EVStyleProperty) -> [String] { if case let .stringList(value)? = definition?.properties[property]?.effective { return value }; return [] }
    private static func numberText(_ value: Float) -> String { String(format: "%g", value) }
    private static func roundedLineSpacingValue(_ value: Float) -> Float {
        guard value.isFinite else { return value }
        return Float((Double(value) * 10).rounded() / 10)
    }
    private static func lineSpacingNumberText(_ value: Float) -> String {
        let rounded = roundedLineSpacingValue(value)
        guard rounded.isFinite else { return numberText(rounded) }
        if rounded.rounded() == rounded { return numberText(rounded) }
        return String(format: "%.1f", rounded)
    }
    private static func displayLineSpacingKind(_ value: EVLineSpacing) -> UInt32 {
        let normal = UInt32(VIEM_STYLE_LINE_SPACING_NORMAL)
        let multiplier = UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER)
        guard value.kind == multiplier else { return value.kind }
        return roundedLineSpacingValue(value.value) == 1 ? normal : multiplier
    }
    private func send(_ mutations: [EVStyleMutation]) { guard !updating, editable else { return }; onMutations?(mutations) }

    @objc private func familyChanged(_ sender: Any?) {
        changeFamily(to: family.currentEditor()?.string ?? family.stringValue)
    }
    func comboBoxSelectionDidChange(_ notification: Notification) {
        guard notification.object as? NSComboBox === family, !updating, !publishingFontChange,
              let value = family.objectValueOfSelectedItem as? String else { return }
        guard value != Self.familyListSeparator else {
            refreshFontControls()
            return
        }
        setFamilyText(value)
        changeFamily(to: value)
    }
    private func changeFamily(to proposed: String) {
        guard !updating, !publishingFontChange, editable else { return }
        let value = proposed.trimmingCharacters(in: .whitespacesAndNewlines)
        let chosen = stringList(.characterFontFamilies).first ?? "Helvetica"
        guard !value.isEmpty,
              value.caseInsensitiveCompare(chosen) != .orderedSame,
              value.caseInsensitiveCompare(EVFontCatalog.displayFamilyName(for: chosen)) != .orderedSame else {
            refreshFontControls()
            return
        }
        // The two portable system entries store their generic request token
        // rather than a concrete resolved face, so the same declaration
        // renders correctly on Windows too.
        if let portable = EVFontCatalog.portableFamily(forDisplayName: value) {
            publishFontMutations([.setDeclaration(.characterFontFamilies, .stringList(replacingPrimaryFamily(with: portable))), .setDeclaration(.characterFontAxes, .string("{}")), .setDeclaration(.characterWeight, .unsigned(400))])
            return
        }
        if let member = EVFontCatalog.faceForFamilyChange(to: value, currentFace: currentFontFace) {
            chooseFace(member)
        } else {
            publishFontMutations([.setDeclaration(.characterFontFamilies, .stringList(replacingPrimaryFamily(with: value))), .setDeclaration(.characterFontAxes, .string("{}")), .setDeclaration(.characterWeight, .unsigned(400))])
        }
    }
    @objc private func faceChanged(_ sender: Any?) {
        guard !updating, !publishingFontChange else { return }
        let index = face.indexOfSelectedItem
        if fontFaces.indices.contains(index) { chooseFace(fontFaces[index]) }
        else if fontInstances.indices.contains(index - fontFaces.count) { setFontAxes(fontInstances[index - fontFaces.count].values) }
    }
    private func chooseFace(_ member: EVFontFace) {
        var values = EVFontVariations.info(for: member.postScriptName).defaults
        if values["wght"] != nil { values["wght"] = Double(member.weight) }
        publishFontMutations([.setDeclaration(.characterFontFamilies, .stringList(replacingPrimaryFamily(with: member.postScriptName))), .setDeclaration(.characterWeight, .unsigned(UInt32(member.weight))), .setDeclaration(.characterFontAxes, .string(EVFontVariations.encode(values)))])
    }
    private func publishFontMutations(_ mutations: [EVStyleMutation]) {
        guard !updating, !publishingFontChange, editable else { return }
        publishingFontChange = true
        defer {
            publishingFontChange = false
            if endFontEditAfterPublication {
                endFontEditAfterPublication = false
                onEditEnded?()
            }
        }
        send(mutations)
        // The owner synchronously publishes a committed definition. Re-render
        // that authority after both success and rejection, including its active
        // field editor, rather than leaving a tentative family/face on screen.
        refreshFontControls()
    }
    private func replacingPrimaryFamily(with value: String) -> [String] {
        [value] + stringList(.characterFontFamilies).dropFirst()
    }
    @objc private func showFallbacks(_ sender: NSButton) {
        guard editable else { return }
        let families = stringList(.characterFontFamilies)
        let primary = families.first ?? "SF Pro"
        let editor = EVFallbackFontsController(primary: primary, fallbacks: Array(families.dropFirst()))
        let popover = NSPopover()
        popover.behavior = .transient
        popover.contentViewController = editor
        popover.contentSize = NSSize(width: 380, height: 290)
        editor.onApply = { [weak self, weak popover] values in
            popover?.close()
            self?.send([.setDeclaration(.characterFontFamilies, .stringList([primary] + values))])
        }
        editor.onCancel = { [weak popover] in popover?.close() }
        fallbackPopover = popover
        popover.show(relativeTo: sender.bounds, of: sender, preferredEdge: .maxY)
    }
    @objc private func toggleChanged(_ sender: NSButton) {
        guard let property = EVStyleProperty(rawValue: UInt32(sender.tag)) else { return }
        if property == .characterSlant { send([.setDeclaration(property, .fontSlant(sender.state == .on ? 1 : 0))]) }
        else { send([.setDeclaration(property, .boolean(sender.state == .on))]) }
    }
    @objc private func colorChanged(_ sender: NSColorWell) {
        guard let property = EVStyleProperty(rawValue: UInt32(sender.tag)), let rgb = sender.color.usingColorSpace(.sRGB) else { return }
        let color = EVStyleColor(red: Float(rgb.redComponent), green: Float(rgb.greenComponent), blue: Float(rgb.blueComponent), alpha: Float(rgb.alphaComponent))
        send([.setDeclaration(property, .color(color))])
    }
    @objc private func directionChanged(_ sender: NSPopUpButton) { guard let property = EVStyleProperty(rawValue: UInt32(sender.tag)) else { return }; send([.setDeclaration(property, .writingDirection(UInt32(sender.indexOfSelectedItem)))]) }
    @objc private func alignmentChanged(_ sender: NSSegmentedControl) {
        guard alignmentValues.indices.contains(sender.selectedSegment) else { return }
        send([.setDeclaration(.paragraphAlignment, .paragraphAlignment(alignmentValues[sender.selectedSegment]))])
    }
    @objc private func lineSpacingChanged(_ sender: Any?) {
        guard let item = lineKind.selectedItem else { return }
        let selectedKind = UInt32(item.tag)
        let normalKind = UInt32(VIEM_STYLE_LINE_SPACING_NORMAL)
        let multiplierKind = UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER)
        let relative = selectedKind == normalKind || selectedKind == multiplierKind
        let previousDisplayKind: UInt32? = {
            guard case let .lineSpacing(value)? = definition?.properties[.paragraphLineSpacing]?.effective else { return nil }
            return Self.displayLineSpacingKind(value)
        }()
        var draftValue = Float(lineValue.stringValue)
        if sender is NSPopUpButton {
            if selectedKind == normalKind {
                draftValue = 1
            } else if selectedKind == multiplierKind {
                let prior = lastLineValues[multiplierKind] ?? 1.1
                draftValue = previousDisplayKind == normalKind || Self.roundedLineSpacingValue(prior) == 1 ? 1.1 : prior
            } else {
                draftValue = lastLineValues[selectedKind] ?? number(.characterSize, fallback: 14) * 1.2
            }
        }
        let rawValid = draftValue.map { $0.isFinite && (relative ? $0 > 0 : $0 >= 0) } == true
        guard rawValid, let rawValue = draftValue else {
            lineValue.textColor = .systemRed
            steppers[.paragraphLineSpacing]?.isEnabled = false
            hasInvalidDraft = true
            return
        }
        let value = Self.roundedLineSpacingValue(rawValue)
        guard !relative || value > 0 else {
            lineValue.textColor = .systemRed
            steppers[.paragraphLineSpacing]?.isEnabled = false
            hasInvalidDraft = true
            return
        }
        let kind = relative && value == 1 ? normalKind : (relative ? multiplierKind : selectedKind)
        lineKind.selectItem(withTag: Int(kind))
        let isActiveTextDraft = sender as? NSTextField === lineValue && lineValue.currentEditor() != nil
        if !isActiveTextDraft { lineValue.stringValue = Self.lineSpacingNumberText(value) }
        updateLineSpacingUnit(kind)
        if kind != normalKind { lastLineValues[kind] = value }
        hasInvalidDraft = false
        lineValue.textColor = .labelColor
        let valueEnabled = isOverridden(.paragraphLineSpacing)
        lineValue.isEnabled = valueEnabled
        synchronizeStepper(.paragraphLineSpacing, value: Double(value), enabled: valueEnabled)
        send([.setDeclaration(.paragraphLineSpacing, .lineSpacing(EVLineSpacing(kind: kind, value: kind == normalKind ? 0 : value)))])
    }

    func controlTextDidBeginEditing(_ notification: Notification) {
        guard !updating, !publishingFontChange else { return }
        onEditBegan?()
    }
    func controlTextDidEndEditing(_ notification: Notification) {
        if publishingFontChange {
            endFontEditAfterPublication = true
            return
        }
        if !updating, notification.object as? NSComboBox === family { familyChanged(family) }
        if !updating, notification.object as? NSTextField === lineValue, !hasInvalidDraft,
           case let .lineSpacing(value)? = definition?.properties[.paragraphLineSpacing]?.effective {
            let displayKind = Self.displayLineSpacingKind(value)
            let displayValue = displayKind == UInt32(VIEM_STYLE_LINE_SPACING_NORMAL)
                ? Float(1) : Self.roundedLineSpacingValue(value.value)
            lineKind.selectItem(withTag: Int(displayKind))
            lineValue.stringValue = Self.lineSpacingNumberText(displayValue)
            updateLineSpacingUnit(displayKind)
            synchronizeStepper(.paragraphLineSpacing, value: Double(displayValue), enabled: lineValue.isEnabled)
        }
        // AppKit may end a temporary field editor while forwarding the first
        // native click. The wrapper owns that transition's gesture lifetime.
        if !inheritedGestureActive { onEditEnded?() }
    }
    func controlTextDidChange(_ notification: Notification) {
        guard let field = notification.object as? NSTextField, !(field is NSComboBox), let property = EVStyleProperty(rawValue: UInt32(field.tag)), !updating else { return }
        if property == .paragraphLineSpacing { lineSpacingChanged(field); return }
        if property == .characterSize, sizeUnit.titleOfSelectedItem == "%" {
            guard canUsePercentageSize, let value = UInt32(field.stringValue), (10...1000).contains(value) else {
                field.textColor = .systemRed
                field.toolTip = "Enter a whole-number percentage from 10 to 1000."
                steppers[property]?.isEnabled = false
                hasInvalidDraft = true
                return
            }
            field.textColor = .labelColor
            hasInvalidDraft = false
            send([.setDeclaration(property, .percentage(value))])
            return
        }
        guard let value = Float(field.stringValue), value.isFinite, (property != .characterSize || value > 0), (!EVStyleProperty.nonnegativeBlockProperties.contains(property) || value >= 0) else { field.textColor = .systemRed; steppers[property]?.isEnabled = false; hasInvalidDraft = true; return }
        field.textColor = .labelColor
        hasInvalidDraft = false
        send([.setDeclaration(property, .float(value))])
    }
    @objc private func showFeatures(_ sender: NSButton) {
        let menu = NSMenu()
        menu.autoenablesItems = false
        let family = stringList(.characterFontFamilies).first ?? "Helvetica"
        var settings: [String: UInt32] = [:]
        if case let .openTypeFeatures(values)? = definition?.properties[.characterOpenTypeFeatures]?.effective { for value in values { settings[value.tag] = value.setting } }
        for feature in EVFontCatalog.features(for: family) {
            let item = NSMenuItem(title: feature.label, action: #selector(featureChanged(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = feature.tag
            item.state = (settings[feature.tag] ?? feature.defaultValue) == 0 ? .off : .on
            menu.addItem(item)
        }
        menu.addItem(.separator())
        let reset = NSMenuItem(title: "Use Font Defaults", action: #selector(resetFeatures(_:)), keyEquivalent: "")
        reset.target = self
        menu.addItem(reset)
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: sender.bounds.maxY + 3), in: sender)
    }
    @objc private func featureChanged(_ sender: NSMenuItem) {
        guard let tag = sender.representedObject as? String else { return }
        var values: [EVOpenTypeFeature] = []
        if case let .openTypeFeatures(current)? = definition?.properties[.characterOpenTypeFeatures]?.effective { values = current }
        values.removeAll { $0.tag == tag }
        values.append(EVOpenTypeFeature(tag: tag, setting: sender.state == .on ? 0 : 1))
        send([.setDeclaration(.characterOpenTypeFeatures, .openTypeFeatures(values.sorted { $0.tag < $1.tag }))])
    }
    @objc private func resetFeatures(_ sender: Any?) { send([.setDeclaration(.characterOpenTypeFeatures, .openTypeFeatures([]))]) }
}

/// Edits the ordered fallback tail as one draft. Apply is one style mutation;
/// dismissal discards the draft and leaves the source declaration unchanged.
@MainActor
final class EVFallbackFontsController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    var onApply: (([String]) -> Void)?
    var onCancel: (() -> Void)?
    private let primary: String
    private var fallbacks: [String]
    private let table = NSTableView()
    private let entry = NSComboBox()
    private let removeButton = NSButton()
    private let upButton = NSButton()
    private let downButton = NSButton()

    init(primary: String, fallbacks: [String]) {
        self.primary = primary
        self.fallbacks = fallbacks
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { nil }

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 380, height: 290))
        let title = NSTextField(labelWithString: "Fallback Fonts")
        title.font = .systemFont(ofSize: 14, weight: .semibold)
        let explanation = NSTextField(wrappingLabelWithString: "Tried in order after \(EVFontCatalog.displayFamilyName(for: primary)) when a character needs another font.")
        explanation.font = .systemFont(ofSize: 11)
        explanation.textColor = .secondaryLabelColor
        table.addTableColumn(NSTableColumn(identifier: NSUserInterfaceItemIdentifier("family")))
        table.headerView = nil
        table.rowHeight = 24
        table.delegate = self
        table.dataSource = self
        table.allowsEmptySelection = true
        table.setAccessibilityLabel("Ordered fallback fonts")
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder
        scroll.heightAnchor.constraint(equalToConstant: 106).isActive = true
        entry.addItems(withObjectValues: NSFontManager.shared.availableFontFamilies.sorted())
        entry.numberOfVisibleItems = 20
        entry.completes = true
        entry.usesDataSource = false
        entry.placeholderString = "Font family"
        entry.setAccessibilityLabel("Add fallback family")
        entry.target = self
        entry.action = #selector(add(_:))
        entry.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let addButton = button("Add", label: "Add fallback font", action: #selector(add(_:)))
        let entryRow = NSStackView(views: [entry, addButton])
        entryRow.spacing = 6
        configure(removeButton, title: "−", label: "Remove fallback font", action: #selector(remove(_:)))
        configure(upButton, title: "↑", label: "Move fallback up", action: #selector(moveFallbackUp(_:)))
        configure(downButton, title: "↓", label: "Move fallback down", action: #selector(moveFallbackDown(_:)))
        let cancel = button("Cancel", label: "Cancel fallback changes", action: #selector(cancel(_:)))
        let apply = button("Apply", label: "Apply fallback fonts", action: #selector(apply(_:)))
        apply.keyEquivalent = "\r"
        let actions = NSStackView(views: [removeButton, upButton, downButton, NSView(), cancel, apply])
        actions.spacing = 6
        let stack = NSStackView(views: [title, explanation, scroll, entryRow, actions])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 9
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 14),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -14),
            stack.topAnchor.constraint(equalTo: view.topAnchor, constant: 14),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: view.bottomAnchor, constant: -14),
        ])
        for child in [explanation, scroll, entryRow, actions] {
            child.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        }
        refreshSelection()
    }

    private func configure(_ button: NSButton, title: String, label: String, action: Selector) {
        button.title = title
        button.bezelStyle = .rounded
        button.target = self
        button.action = action
        button.setAccessibilityLabel(label)
    }
    private func button(_ title: String, label: String, action: Selector) -> NSButton {
        let button = NSButton()
        configure(button, title: title, label: label, action: action)
        return button
    }
    func numberOfRows(in tableView: NSTableView) -> Int { fallbacks.count }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let label = NSTextField(labelWithString: "\(row + 1).  \(fallbacks[row])")
        label.lineBreakMode = .byTruncatingTail
        return label
    }
    func tableViewSelectionDidChange(_ notification: Notification) { refreshSelection() }
    private func refreshSelection() {
        let selected = table.selectedRow
        removeButton.isEnabled = fallbacks.indices.contains(selected)
        upButton.isEnabled = selected > 0
        downButton.isEnabled = selected >= 0 && selected + 1 < fallbacks.count
    }
    private func refresh(select row: Int) {
        table.reloadData()
        if fallbacks.indices.contains(row) { table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false) }
        else { table.deselectAll(nil) }
        refreshSelection()
    }
    @objc private func add(_ sender: Any?) {
        let value = entry.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return }
        fallbacks.append(value)
        entry.stringValue = ""
        refresh(select: fallbacks.count - 1)
    }
    @objc private func remove(_ sender: Any?) {
        let selected = table.selectedRow
        guard fallbacks.indices.contains(selected) else { return }
        fallbacks.remove(at: selected)
        refresh(select: min(selected, fallbacks.count - 1))
    }
    @objc private func moveFallbackUp(_ sender: Any?) { move(by: -1) }
    @objc private func moveFallbackDown(_ sender: Any?) { move(by: 1) }
    private func move(by delta: Int) {
        let selected = table.selectedRow
        guard fallbacks.indices.contains(selected), fallbacks.indices.contains(selected + delta) else { return }
        fallbacks.swapAt(selected, selected + delta)
        refresh(select: selected + delta)
    }
    @objc private func apply(_ sender: Any?) { onApply?(fallbacks) }
    @objc private func cancel(_ sender: Any?) { onCancel?() }
}

private final class EVFontAxisSlider: NSSlider {
    var axisTag = ""
    var fontIdentity = ""
    var beginGesture: (() -> Void)?
    var endGesture: (() -> Void)?
    override func mouseDown(with event: NSEvent) { beginGesture?(); defer { endGesture?() }; super.mouseDown(with: event) }
    override func keyDown(with event: NSEvent) { beginGesture?(); defer { endGesture?() }; super.keyDown(with: event) }
}
