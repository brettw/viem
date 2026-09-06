import AppKit
import CEvimCore
import EvimAppShell
import EvimCoreTextProvider

/// A compact, declaration-aware typography palette. Default always removes the
/// declaration; inherited values are displayed without being written back.
@MainActor
final class EVCompactStyleControls: NSObject, NSTextFieldDelegate, NSComboBoxDelegate {
    let characterView = NSStackView()
    let paragraphView = NSStackView()
    var onMutations: (([EVStyleMutation]) -> Void)?
    var onEditBegan: (() -> Void)?
    var onEditEnded: (() -> Void)?
    private(set) var hasInvalidDraft = false
    private var definition: EVStyleDefinition?
    private var theme = EVTheme.paper
    private var sourceFormat = EVSourceFormat.plainText
    private let alignmentValues: [UInt32] = [UInt32(EVIM_STYLE_PARAGRAPH_ALIGNMENT_START), UInt32(EVIM_STYLE_PARAGRAPH_ALIGNMENT_CENTER), UInt32(EVIM_STYLE_PARAGRAPH_ALIGNMENT_END)]
    private var updating = false
    private var editable = false
    private let family = NSComboBox()
    private let face = NSPopUpButton()
    private let fallbackButton = NSButton()
    private(set) var fallbackPopover: NSPopover?
    private let featureButton = NSButton()
    private var fields: [EVStyleProperty: NSTextField] = [:]
    private var buttons: [EVStyleProperty: NSButton] = [:]
    private var wells: [EVStyleProperty: NSColorWell] = [:]
    private var directions: [EVStyleProperty: NSPopUpButton] = [:]
    private var resetButtons: [EVStyleProperty: NSButton] = [:]
    private let alignment = NSSegmentedControl()
    private let lineKind = NSPopUpButton()
    private let lineValue = NSTextField()
    private var fontFaces: [EVFontFace] = []

    override init() {
        super.init()
        family.addItems(withObjectValues: NSFontManager.shared.availableFontFamilies.sorted())
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
        face.widthAnchor.constraint(equalToConstant: 150).isActive = true
        fallbackButton.title = "…"
        fallbackButton.bezelStyle = .texturedRounded
        fallbackButton.target = self
        fallbackButton.action = #selector(showFallbacks(_:))
        fallbackButton.toolTip = "Edit ordered fallback fonts"
        fallbackButton.setAccessibilityLabel("Fallback fonts")
        fallbackButton.widthAnchor.constraint(equalToConstant: 28).isActive = true
        let familyRow = row([family, face, fallbackButton, reset(.characterFontFamilies), numeric(.characterSize, title: "Size", width: 66)])
        let emphasis = row([
            toggle(.characterBold, title: "B", font: .boldSystemFont(ofSize: 14)),
            toggle(.characterSlant, title: "I", font: NSFontManager.shared.convert(.systemFont(ofSize: 14), toHaveTrait: .italicFontMask)),
            toggle(.characterUnderline, title: "U"), toggle(.characterStrikethrough, title: "S̶"),
        ], spacing: 2)
        featureButton.image = EVStyleIcons.image(.features)
        featureButton.bezelStyle = .texturedRounded
        featureButton.target = self
        featureButton.action = #selector(showFeatures(_:))
        featureButton.toolTip = "OpenType features supported by this font"
        featureButton.setAccessibilityLabel("OpenType features")
        featureButton.widthAnchor.constraint(equalToConstant: 32).isActive = true
        featureButton.heightAnchor.constraint(equalToConstant: 27).isActive = true
        let appearance = row([emphasis, divider(), color(.characterForeground, title: "Text"), color(.characterBackground, title: "Highlight"), NSView(), featureButton])
        let metrics = row([
            numeric(.characterLetterSpacing, title: "Tracking", icon: .tracking),
            numeric(.characterBaselineShift, title: "Baseline", icon: .baseline),
            text(.characterLanguage, title: "Language", width: 94),
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
        let paraToolbar = row([alignment, reset(.paragraphAlignment), NSView(), direction(.paragraphBaseDirection, title: "Direction")])
        let indents = row([
            numeric(.paragraphLeadingIndent, title: "Start indent", icon: .leadingIndent),
            numeric(.paragraphTrailingIndent, title: "End indent", icon: .trailingIndent),
            numeric(.paragraphFirstLineIndent, title: "First line", icon: .firstIndent),
        ])
        lineKind.addItems(withTitles: ["Normal", "Multiple", "At least", "Exactly"])
        for (index, value) in [EVIM_STYLE_LINE_SPACING_NORMAL, EVIM_STYLE_LINE_SPACING_MULTIPLIER, EVIM_STYLE_LINE_SPACING_AT_LEAST, EVIM_STYLE_LINE_SPACING_EXACT].enumerated() {
            lineKind.item(at: index)?.tag = Int(value)
        }
        lineKind.target = self
        lineKind.action = #selector(lineSpacingChanged(_:))
        lineKind.setAccessibilityLabel("Line spacing kind")
        lineValue.delegate = self
        lineValue.tag = Int(EVStyleProperty.paragraphLineSpacing.rawValue)
        lineValue.setAccessibilityLabel("Line spacing value")
        lineValue.widthAnchor.constraint(equalToConstant: 48).isActive = true
        let line = labeled("Line spacing", control: row([icon(.lineSpacing), lineKind, lineValue, reset(.paragraphLineSpacing)], spacing: 3))
        let spacing = row([
            numeric(.paragraphSpacingBefore, title: "Space before", icon: .before),
            numeric(.paragraphSpacingAfter, title: "Space after", icon: .after), line,
        ])
        configure(paragraphView, rows: [paraToolbar, separator(), indents, spacing])
    }

    func configure(_ definition: EVStyleDefinition?, theme: EVTheme = .paper, sourceFormat: EVSourceFormat = .plainText) {
        fallbackPopover?.close()
        fallbackPopover = nil
        self.theme = theme
        self.sourceFormat = sourceFormat
        updating = true
        defer { updating = false }
        self.definition = definition
        editable = definition?.capabilities.contains(.declarations) == true
        hasInvalidDraft = false
        let chosen = stringList(.characterFontFamilies).first ?? "Helvetica"
        fontFaces = EVFontCatalog.faces(for: chosen)
        family.stringValue = EVFontCatalog.displayFamilyName(for: chosen)
        family.isEnabled = editable
        face.removeAllItems()
        for member in fontFaces { face.addItem(withTitle: member.styleName) }
        if let index = fontFaces.firstIndex(where: { $0.postScriptName == chosen }) { face.selectItem(at: index) }
        else if let index = fontFaces.firstIndex(where: { $0.weight == UInt16(number(.characterWeight, fallback: 400)) && $0.italic == (unsigned(.characterSlant) != 0) }) { face.selectItem(at: index) }
        face.isEnabled = editable && !fontFaces.isEmpty
        fallbackButton.isEnabled = editable
        let fallbackCount = max(0, stringList(.characterFontFamilies).count - 1)
        fallbackButton.toolTip = "Edit ordered fallback fonts (\(fallbackCount))"
        buttons[.characterBold]?.state = boolean(.characterBold) ? .on : .off
        buttons[.characterSlant]?.state = unsigned(.characterSlant) != 0 ? .on : .off
        for property in [EVStyleProperty.characterUnderline, .characterStrikethrough] { buttons[property]?.state = boolean(property) ? .on : .off }
        for (property, button) in buttons { button.isEnabled = editable; setHelp(button, property) }
        for (property, field) in fields {
            if property == .characterLanguage { field.stringValue = string(property) }
            else { field.stringValue = Self.numberText(number(property, fallback: property == .characterSize ? 14 : 0)) }
            field.isEnabled = editable
            field.textColor = .labelColor
            setHelp(field, property)
        }
        refreshThemeColors(theme)
        for (property, popup) in directions { popup.selectItem(at: min(2, Int(unsigned(property)))); popup.isEnabled = editable; setHelp(popup, property) }
        for (property, button) in resetButtons { button.isEnabled = editable && definition?.properties[property]?.isDeclared == true }
        alignment.selectedSegment = alignmentValues.firstIndex(of: unsigned(.paragraphAlignment)) ?? 0
        alignment.isEnabled = editable
        lineKind.isEnabled = editable
        lineValue.isEnabled = editable
        if case let .lineSpacing(value)? = definition?.properties[.paragraphLineSpacing]?.effective {
            lineKind.selectItem(withTag: Int(value.kind))
            lineValue.stringValue = value.kind == UInt32(EVIM_STYLE_LINE_SPACING_NORMAL) ? "" : Self.numberText(value.value)
        } else { lineKind.selectItem(at: 0); lineValue.stringValue = "" }
        lineValue.isEnabled = editable && lineKind.indexOfSelectedItem != 0
        featureButton.isEnabled = editable && !EVFontCatalog.features(for: chosen).isEmpty
    }

    func refreshThemeColors(_ theme: EVTheme) {
        self.theme = theme
        for (property, well) in wells {
            let resolved = definition?.properties[property]
            if property == .characterForeground && (resolved?.usesThemeDefault == true || resolved == nil) {
                well.color = theme.foreground.color
            } else if case let .color(value)? = resolved?.effective { well.color = value.appKitColor }
            else { well.color = property == .characterForeground ? theme.foreground.color : .clear }
            well.isEnabled = editable
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
    private func row(_ views: [NSView], spacing: CGFloat = 9) -> NSStackView {
        let stack = NSStackView(views: views)
        stack.orientation = .horizontal
        stack.alignment = .centerY
        stack.spacing = spacing
        return stack
    }
    private func labeled(_ title: String, control: NSView) -> NSStackView {
        let label = NSTextField(labelWithString: title)
        label.font = .systemFont(ofSize: 10, weight: .medium)
        label.textColor = .secondaryLabelColor
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
    private func numeric(_ property: EVStyleProperty, title: String, icon symbol: EVStyleIcons.Symbol? = nil, width: CGFloat = 65) -> NSView {
        let field = NSTextField()
        field.delegate = self
        field.tag = Int(property.rawValue)
        field.setAccessibilityLabel(title)
        field.alignment = .right
        field.widthAnchor.constraint(equalToConstant: width).isActive = true
        fields[property] = field
        let unit = NSTextField(labelWithString: "pt")
        unit.textColor = .secondaryLabelColor
        unit.font = .systemFont(ofSize: 11)
        var items: [NSView] = symbol.map { [icon($0)] } ?? []
        items += [field, unit, reset(property)]
        return labeled(title, control: row(items, spacing: 3))
    }
    private func text(_ property: EVStyleProperty, title: String, width: CGFloat) -> NSView {
        let field = NSTextField()
        field.delegate = self
        field.tag = Int(property.rawValue)
        field.setAccessibilityLabel(title)
        field.widthAnchor.constraint(equalToConstant: width).isActive = true
        fields[property] = field
        return labeled(title, control: row([field, reset(property)], spacing: 3))
    }
    private func toggle(_ property: EVStyleProperty, title: String, font: NSFont = .systemFont(ofSize: 14)) -> NSButton {
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
        let menu = NSMenu()
        let inherited = NSMenuItem(title: "Use Inherited \(property.displayName)", action: #selector(clearMenu(_:)), keyEquivalent: "")
        inherited.target = self
        inherited.tag = Int(property.rawValue)
        menu.addItem(inherited)
        button.menu = menu
        buttons[property] = button
        return button
    }
    private func reset(_ property: EVStyleProperty) -> NSButton {
        let button = NSButton(title: "↶", target: self, action: #selector(clear(_:)))
        button.bezelStyle = .inline
        button.font = .systemFont(ofSize: 12)
        button.tag = Int(property.rawValue)
        button.toolTip = "Use inherited \(property.displayName.lowercased())"
        button.setAccessibilityLabel("Default \(property.displayName.lowercased())")
        button.widthAnchor.constraint(greaterThanOrEqualToConstant: 18).isActive = true
        resetButtons[property] = button
        return button
    }
    private func color(_ property: EVStyleProperty, title: String) -> NSView {
        let well = NSColorWell(style: .minimal)
        well.target = self
        well.action = #selector(colorChanged(_:))
        well.tag = Int(property.rawValue)
        well.setAccessibilityLabel("\(title) color")
        well.widthAnchor.constraint(equalToConstant: 31).isActive = true
        well.heightAnchor.constraint(equalToConstant: 25).isActive = true
        wells[property] = well
        let button = reset(property)
        button.title = "Default"
        button.widthAnchor.constraint(equalToConstant: 48).isActive = true
        // The explicit word makes the inheritance behavior discoverable.
        let label = NSTextField(labelWithString: title)
        label.font = .systemFont(ofSize: 11)
        return row([label, well, button], spacing: 4)
    }
    private func direction(_ property: EVStyleProperty, title: String) -> NSView {
        let popup = NSPopUpButton()
        popup.addItems(withTitles: ["Automatic", "Left to right", "Right to left"])
        popup.target = self
        popup.action = #selector(directionChanged(_:))
        popup.tag = Int(property.rawValue)
        popup.setAccessibilityLabel("\(property == .characterDirection ? "Character" : "Paragraph") direction")
        directions[property] = popup
        return labeled(title, control: row([popup, reset(property)], spacing: 3))
    }
    private func divider() -> NSView { let view = NSBox(); view.boxType = .separator; view.widthAnchor.constraint(equalToConstant: 1).isActive = true; view.heightAnchor.constraint(equalToConstant: 22).isActive = true; return view }
    private func separator() -> NSView { let view = NSBox(); view.boxType = .separator; return view }
    private func setHelp(_ control: NSView, _ property: EVStyleProperty) { control.toolTip = definition?.properties[property]?.isDeclared == true ? "Declared in this style. Use Default to inherit." : "Inherited from the parent style." }
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
        guard !updating else { return }
        let value = family.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return }
        let members = EVFontCatalog.faces(for: value)
        let regular = members.min { abs(Int($0.weight) - 400) + ($0.italic ? 200 : 0) < abs(Int($1.weight) - 400) + ($1.italic ? 200 : 0) }
        if let regular { chooseFace(regular) } else { send([.setDeclaration(.characterFontFamilies, .stringList(replacingPrimaryFamily(with: value)))]) }
    }
    func comboBoxSelectionDidChange(_ notification: Notification) {
        if let value = family.objectValueOfSelectedItem as? String { family.stringValue = value }
        familyChanged(family)
    }
    @objc private func faceChanged(_ sender: Any?) { guard fontFaces.indices.contains(face.indexOfSelectedItem) else { return }; chooseFace(fontFaces[face.indexOfSelectedItem]) }
    private func chooseFace(_ member: EVFontFace) {
        send([.setDeclaration(.characterFontFamilies, .stringList(replacingPrimaryFamily(with: member.postScriptName))), .setDeclaration(.characterWeight, .unsigned(UInt32(member.weight))), .setDeclaration(.characterSlant, .fontSlant(member.italic ? 1 : 0))])
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
    @objc private func clearMenu(_ sender: NSMenuItem) { guard let property = EVStyleProperty(rawValue: UInt32(sender.tag)) else { return }; send([.clearDeclaration(property)]) }
    @objc private func clear(_ sender: NSButton) { guard let property = EVStyleProperty(rawValue: UInt32(sender.tag)) else { return }; send([.clearDeclaration(property)]) }
    @objc private func colorChanged(_ sender: NSColorWell) {
        guard let property = EVStyleProperty(rawValue: UInt32(sender.tag)), let rgb = sender.color.usingColorSpace(.deviceRGB) else { return }
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
        let normal = kind == UInt32(EVIM_STYLE_LINE_SPACING_NORMAL)
        var value = Float(lineValue.stringValue)
        if sender is NSPopUpButton, !normal, value.map({ $0.isFinite && $0 > 0 }) != true {
            value = kind == UInt32(EVIM_STYLE_LINE_SPACING_MULTIPLIER) ? 1 : number(.characterSize, fallback: 14) * 1.2
            lineValue.stringValue = Self.numberText(value!)
        }
        guard normal || value.map({ $0.isFinite && $0 > 0 }) == true else { lineValue.textColor = .systemRed; hasInvalidDraft = true; return }
        hasInvalidDraft = false
        lineValue.textColor = .labelColor
        send([.setDeclaration(.paragraphLineSpacing, .lineSpacing(EVLineSpacing(kind: kind, value: normal ? 0 : value!)))])
    }

    func controlTextDidBeginEditing(_ notification: Notification) { onEditBegan?() }
    func controlTextDidEndEditing(_ notification: Notification) {
        if notification.object as? NSComboBox === family { familyChanged(family) }
        onEditEnded?()
    }
    func controlTextDidChange(_ notification: Notification) {
        guard let field = notification.object as? NSTextField, !(field is NSComboBox), let property = EVStyleProperty(rawValue: UInt32(field.tag)), !updating else { return }
        if property == .characterLanguage { send([.setDeclaration(property, .string(field.stringValue))]); return }
        if property == .paragraphLineSpacing { lineSpacingChanged(field); return }
        guard let value = Float(field.stringValue), value.isFinite, property != .characterSize || value > 0 else { field.textColor = .systemRed; hasInvalidDraft = true; return }
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
    @objc private func resetFeatures(_ sender: Any?) { send([.clearDeclaration(.characterOpenTypeFeatures)]) }
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
