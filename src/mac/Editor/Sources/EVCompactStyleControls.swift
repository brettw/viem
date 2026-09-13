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
    var onMutations: (([EVStyleMutation]) -> Void)?
    var onEditBegan: (() -> Void)?
    var onEditEnded: (() -> Void)?
    private(set) var hasInvalidDraft = false
    private var definition: EVStyleDefinition?
    private var documentID: UInt64?
    private var theme = EVTheme.paper
    private var sourceFormat = EVSourceFormat.plainText
    private let alignmentValues: [UInt32] = [UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_START), UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_CENTER), UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_END)]
    private var updating = false
    private var publishingFontChange = false
    private var endFontEditAfterPublication = false
    private var editable = false
    private var inheritedGestureActive = false
    private let family = NSComboBox()
    private let face = NSPopUpButton()
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
    private var fontFaces: [EVFontFace] = []
    private var isBaseParagraph: Bool { definition?.flags.contains(.baseParagraph) == true }
    private let sectionSpacing = NSFont.systemFontSize

    override init() {
        super.init()
        family.addItems(withObjectValues: NSFontManager.shared.availableFontFamilies.sorted())
        family.numberOfVisibleItems = 20
        family.completes = true
        family.usesDataSource = false
        family.delegate = self
        family.target = self
        family.action = #selector(familyChanged(_:))
        family.setAccessibilityLabel("Font family")
        family.widthAnchor.constraint(greaterThanOrEqualToConstant: 210).isActive = true
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
            overrideGroup(.characterFontFamilies, control: row([family, fallbackButton], spacing: 4)),
            overrideGroup(.characterWeight, control: face),
            numeric(.characterSize, title: "Size", width: 62, showsLabel: false),
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
            numeric(.characterBaselineShift, title: "Baseline", icon: .baseline),
            direction(.characterDirection, title: "Direction"),
        ])
        configure(characterView, rows: [familyRow, appearance, separator(), metrics])

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
        let line = labeled("Line spacing", control: overrideGroup(.paragraphLineSpacing, control: row([icon(.lineSpacing), lineKind, lineValue, stepper(.paragraphLineSpacing, title: "Line spacing value")], spacing: 3)), property: .paragraphLineSpacing)
        let spacing = row([
            numeric(.paragraphSpacingBefore, title: "Space before", icon: .before),
            numeric(.paragraphSpacingAfter, title: "Space after", icon: .after), line,
        ])
        configure(paragraphView, rows: [paraToolbar, separator(), indents, spacing])
    }

    func configure(_ definition: EVStyleDefinition?, theme: EVTheme = .paper, sourceFormat: EVSourceFormat = .plainText, documentID: UInt64? = nil) {
        fallbackPopover?.close()
        fallbackPopover = nil
        self.theme = theme
        self.sourceFormat = sourceFormat
        let wasUpdating = updating
        updating = true
        defer { updating = wasUpdating }
        if self.definition?.key != definition?.key || self.documentID != documentID {
            lastLineValues = [:]
            for well in wells.values { well.dismissColorControls() }
        }
        self.documentID = documentID
        self.definition = definition
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
        for (property, field) in fields {
            field.stringValue = isOverridden(property) ? Self.numberText(number(property, fallback: property == .characterSize ? 14 : 0)) : ""
            field.isEnabled = isOverridden(property)
            field.textColor = .labelColor
            setHelp(field, property)
            synchronizeStepper(property, value: Double(number(property, fallback: property == .characterSize ? 14 : 0)), enabled: field.isEnabled)
        }
        refreshThemeColors(theme)
        for (property, popup) in directions { popup.selectItem(at: isOverridden(property) ? min(2, Int(unsigned(property))) : -1); popup.isEnabled = isOverridden(property); setHelp(popup, property) }
        for (property, button) in overrideButtons {
            button.isEnabled = canEdit(property) && !isBaseParagraph
            button.state = isBaseParagraph || definition?.properties[property]?.isDeclared == true ? .on : .off
        }
        alignment.selectedSegment = isOverridden(.paragraphAlignment) ? (alignmentValues.firstIndex(of: unsigned(.paragraphAlignment)) ?? 0) : -1
        alignment.isEnabled = isOverridden(.paragraphAlignment)
        lineKind.isEnabled = isOverridden(.paragraphLineSpacing)
        if case let .lineSpacing(value)? = definition?.properties[.paragraphLineSpacing]?.effective {
            lineKind.selectItem(withTag: Int(value.kind))
            if value.kind != UInt32(VIEM_STYLE_LINE_SPACING_NORMAL) { lastLineValues[value.kind] = value.value }
            lineValue.stringValue = value.kind == UInt32(VIEM_STYLE_LINE_SPACING_NORMAL) ? "" : Self.numberText(value.value)
        } else { lineKind.selectItem(at: 0); lineValue.stringValue = "" }
        if !isOverridden(.paragraphLineSpacing) { lineKind.select(nil); lineValue.stringValue = "" }
        lineValue.isEnabled = isOverridden(.paragraphLineSpacing) && lineKind.indexOfSelectedItem > 0
        synchronizeStepper(.paragraphLineSpacing, value: Double(Float(lineValue.stringValue) ?? 0), enabled: lineValue.isEnabled)
    }

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
        face.removeAllItems()
        for member in fontFaces { face.addItem(withTitle: member.styleName) }
        // A popup selects its first item automatically when populated. An
        // unresolved request must not appear to select that unrelated face.
        face.select(nil)
        if isOverridden(.characterWeight), let current = currentFontFace, let index = fontFaces.firstIndex(of: current) {
            face.selectItem(at: index)
        }
        face.isEnabled = isOverridden(.characterWeight) && !fontFaces.isEmpty
        featureButton.isEnabled = isOverridden(.characterOpenTypeFeatures) && !EVFontCatalog.features(for: chosen).isEmpty
    }

    private var currentFontFace: EVFontFace? {
        let chosen = stringList(.characterFontFamilies).first ?? "Helvetica"
        let weight = number(.characterWeight, fallback: 400)
        let italic = unsigned(.characterSlant) != 0
        let matching = fontFaces.filter { Float($0.weight) == weight && $0.italic == italic }
        return matching.first { $0.postScriptName == chosen } ?? matching.first
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
        let unit = EVStyleControlLabel(labelWithString: "pt")
        unit.onClick = { [weak self, weak unit] in self?.activateFromLabel(property, label: unit) }
        unit.textColor = .secondaryLabelColor
        unit.font = .systemFont(ofSize: 11)
        unit.setContentCompressionResistancePriority(.required, for: .horizontal)
        var items: [NSView] = symbol.map { [icon($0)] } ?? []
        items += [field, stepper(property, title: title), unit]
        let controls = overrideGroup(property, control: row(items, spacing: 3))
        return showsLabel ? labeled(title, control: controls, property: property) : controls
    }
    private func stepper(_ property: EVStyleProperty, title: String) -> EVStyleStepper {
        let control = EVStyleStepper()
        control.controlSize = .small
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
        let multiplier = property == .paragraphLineSpacing && lineKind.selectedItem?.tag == Int(VIEM_STYLE_LINE_SPACING_MULTIPLIER)
        let positive = property == .characterSize || multiplier
        control.increment = multiplier || property == .characterLetterSpacing ? 0.1 : 1
        control.minValue = positive ? min(max(value, Double(Float.leastNormalMagnitude)), sourceFormat == .rtf && property == .characterSize ? 0.5 : 0.1)
            : property == .paragraphLineSpacing ? 0 : -Double(Float.greatestFiniteMagnitude)
        control.maxValue = Double(Float.greatestFiniteMagnitude)
        control.doubleValue = value
        control.isEnabled = enabled
        setHelp(control, property)
    }

    @objc private func stepperChanged(_ sender: EVStyleStepper) {
        guard !updating, editable, let property = EVStyleProperty(rawValue: UInt32(sender.tag)) else { return }
        let field = property == .paragraphLineSpacing ? lineValue : fields[property]
        guard let field else { return }
        field.stringValue = Self.numberText(Float(sender.doubleValue))
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

    private func overrideGroup(_ property: EVStyleProperty, control: NSView) -> NSView {
        let checkbox = NSButton(checkboxWithTitle: "", target: self, action: #selector(overrideChanged(_:)))
        checkbox.controlSize = .small
        checkbox.tag = Int(property.rawValue)
        checkbox.toolTip = "Override inherited"
        checkbox.setAccessibilityLabel("Override \(property.displayName.lowercased())")
        overrideButtons[property] = checkbox
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
        return row([checkbox, wrapper], spacing: 3)
    }

    private func canEdit(_ property: EVStyleProperty) -> Bool {
        editable && (!EVStyleProperty.paragraphProperties.contains(property) || definition?.kind == .paragraph)
    }

    private func isOverridden(_ property: EVStyleProperty) -> Bool {
        canEdit(property) && (isBaseParagraph || definition?.properties[property]?.isDeclared == true)
    }

    private func enableOverride(_ property: EVStyleProperty) {
        guard canEdit(property), !isOverridden(property) else { return }
        var value = definition?.properties[property]?.effective
        if property == .characterBackground, value == nil {
            value = .color(EVStyleColor(red: 0, green: 0, blue: 0, alpha: 0))
        }
        if property == .characterForeground, definition?.properties[property]?.usesThemeDefault == true {
            let color = theme.foreground
            value = .color(EVStyleColor(red: Float(color.red), green: Float(color.green), blue: Float(color.blue), alpha: Float(color.alpha)))
        }
        if let value { send([.setDeclaration(property, value)]) }
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

    private func color(_ property: EVStyleProperty, title: String) -> NSView {
        let well = EVStyleColorWell(frame: .zero)
        well.target = self
        well.action = #selector(colorChanged(_:))
        well.tag = Int(property.rawValue)
        well.setAccessibilityLabel(property == .characterForeground ? "Text color" : "Background color")
        well.widthAnchor.constraint(equalToConstant: 31).isActive = true
        well.heightAnchor.constraint(equalToConstant: 27).isActive = true
        wells[property] = well
        return labeled(title, control: overrideGroup(property, control: well), property: property)
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
    private func unsigned(_ property: EVStyleProperty) -> UInt32 {
        switch definition?.properties[property]?.effective { case let .fontSlant(value)?, let .writingDirection(value)?, let .paragraphAlignment(value)?: value; default: 0 }
    }
    private func boolean(_ property: EVStyleProperty) -> Bool { if case let .boolean(value)? = definition?.properties[property]?.effective { return value }; return false }
    private func string(_ property: EVStyleProperty) -> String { if case let .string(value)? = definition?.properties[property]?.effective { return value }; return "" }
    private func stringList(_ property: EVStyleProperty) -> [String] { if case let .stringList(value)? = definition?.properties[property]?.effective { return value }; return [] }
    private static func numberText(_ value: Float) -> String { String(format: "%g", value) }
    private func send(_ mutations: [EVStyleMutation]) { guard !updating, editable else { return }; onMutations?(mutations) }

    @objc private func familyChanged(_ sender: Any?) {
        changeFamily(to: family.currentEditor()?.string ?? family.stringValue)
    }
    func comboBoxSelectionDidChange(_ notification: Notification) {
        guard notification.object as? NSComboBox === family, !updating, !publishingFontChange,
              let value = family.objectValueOfSelectedItem as? String else { return }
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
        if let member = EVFontCatalog.faceForFamilyChange(to: value, currentFace: currentFontFace) {
            chooseFace(member)
        } else {
            publishFontMutations([.setDeclaration(.characterFontFamilies, .stringList(replacingPrimaryFamily(with: value)))])
        }
    }
    @objc private func faceChanged(_ sender: Any?) {
        guard !updating, !publishingFontChange, fontFaces.indices.contains(face.indexOfSelectedItem) else { return }
        chooseFace(fontFaces[face.indexOfSelectedItem])
    }
    private func chooseFace(_ member: EVFontFace) {
        publishFontMutations([.setDeclaration(.characterFontFamilies, .stringList(replacingPrimaryFamily(with: member.postScriptName))), .setDeclaration(.characterWeight, .unsigned(UInt32(member.weight))), .setDeclaration(.characterSlant, .fontSlant(member.italic ? 1 : 0))])
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
            .normalizedForNativePicker(format: sourceFormat)
        send([.setDeclaration(property, .color(color))])
    }
    @objc private func directionChanged(_ sender: NSPopUpButton) { guard let property = EVStyleProperty(rawValue: UInt32(sender.tag)) else { return }; send([.setDeclaration(property, .writingDirection(UInt32(sender.indexOfSelectedItem)))]) }
    @objc private func alignmentChanged(_ sender: NSSegmentedControl) {
        guard alignmentValues.indices.contains(sender.selectedSegment) else { return }
        send([.setDeclaration(.paragraphAlignment, .paragraphAlignment(alignmentValues[sender.selectedSegment]))])
    }
    @objc private func lineSpacingChanged(_ sender: Any?) {
        guard let item = lineKind.selectedItem else { return }
        let kind = UInt32(item.tag)
        let normal = kind == UInt32(VIEM_STYLE_LINE_SPACING_NORMAL)
        var value = Float(lineValue.stringValue)
        if sender is NSPopUpButton, !normal {
            value = lastLineValues[kind] ?? (kind == UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER) ? 1 : number(.characterSize, fallback: 14) * 1.2)
            lineValue.stringValue = Self.numberText(value!)
        }
        let valid = value.map { $0.isFinite && (kind == UInt32(VIEM_STYLE_LINE_SPACING_MULTIPLIER) ? $0 > 0 : $0 >= 0) } == true
        guard normal || valid else {
            lineValue.textColor = .systemRed
            steppers[.paragraphLineSpacing]?.isEnabled = false
            hasInvalidDraft = true
            return
        }
        if !normal, let value { lastLineValues[kind] = value }
        hasInvalidDraft = false
        lineValue.textColor = .labelColor
        send([.setDeclaration(.paragraphLineSpacing, .lineSpacing(EVLineSpacing(kind: kind, value: normal ? 0 : value!)))])
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
        // AppKit may end a temporary field editor while forwarding the first
        // native click. The wrapper owns that transition's gesture lifetime.
        if !inheritedGestureActive { onEditEnded?() }
    }
    func controlTextDidChange(_ notification: Notification) {
        guard let field = notification.object as? NSTextField, !(field is NSComboBox), let property = EVStyleProperty(rawValue: UInt32(field.tag)), !updating else { return }
        if property == .paragraphLineSpacing { lineSpacingChanged(field); return }
        guard let value = Float(field.stringValue), value.isFinite, property != .characterSize || value > 0 else { field.textColor = .systemRed; steppers[property]?.isEnabled = false; hasInvalidDraft = true; return }
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
