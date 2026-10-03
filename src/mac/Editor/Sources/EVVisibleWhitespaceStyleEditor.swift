import AppKit
import CViemCore
import ViemAppShell
import ViemCoreTextProvider

/// History belongs to the application setting, independently of document undo.
@MainActor
final class EVVisibleWhitespaceStyleSession {
    let configuration: EVConfigurationStore
    let undoManager = UndoManager()
    var onChange: (() -> Void)?
    private(set) var style: EVVisibleWhitespaceStyle
    private(set) var lastError: String?
    private var groupStart: EVVisibleWhitespaceStyle?
    private var writing = false
    private var observer: NSObjectProtocol?

    init(configuration: EVConfigurationStore) {
        self.configuration = configuration
        style = configuration.whitespacePresentation.visibleWhitespace.style
        lastError = configuration.lastError
        undoManager.groupsByEvent = false
        observer = NotificationCenter.default.addObserver(forName: .viemConfigurationDidChange, object: nil, queue: .main) { [weak self] notification in
            MainActor.assumeIsolated {
                guard let self, !self.writing, let source = notification.object as? EVConfigurationStore,
                      source.directory.standardizedFileURL == self.configuration.directory.standardizedFileURL else { return }
                self.refreshExternalChange()
            }
        }
    }

    deinit { if let observer { NotificationCenter.default.removeObserver(observer) } }

    func beginGroup() { if groupStart == nil { groupStart = style } }
    func endGroup() {
        guard let before = groupStart else { return }
        groupStart = nil
        if style != before { registerUndo(before) }
    }

    @discardableResult
    func apply(_ mutations: [EVStyleMutation]) -> Bool {
        do {
            try configuration.reloadFromDisk()
            guard configuration.whitespacePresentation.visibleWhitespace.style == style else {
                refreshExternalChange()
                throw Self.invalid("Visible whitespace changed in another settings owner. The controls were refreshed; retry the edit.")
            }
            var candidate = style
            for mutation in mutations { try candidate.apply(mutation) }
            return try save(candidate)
        } catch {
            lastError = error.localizedDescription
            onChange?()
            return false
        }
    }

    @discardableResult
    func restoreDefaults() -> Bool {
        endGroup()
        do { return try save(.defaultStyle) }
        catch { lastError = error.localizedDescription; onChange?(); return false }
    }

    private func save(_ candidate: EVVisibleWhitespaceStyle) throws -> Bool {
        guard candidate.isValid else { throw Self.invalid("Invalid Visible whitespace character style.") }
        guard candidate != style else { lastError = nil; return true }
        try configuration.reloadFromDisk()
        var options = configuration.whitespacePresentation
        options.visibleWhitespace.style = candidate
        let before = style
        writing = true
        defer { writing = false }
        try configuration.setWhitespacePresentation(options)
        style = candidate
        lastError = nil
        if groupStart == nil { registerUndo(before) }
        onChange?()
        return true
    }

    private func registerUndo(_ previous: EVVisibleWhitespaceStyle) {
        let ownsGroup = undoManager.groupingLevel == 0
        if ownsGroup { undoManager.beginUndoGrouping() }
        undoManager.registerUndo(withTarget: self) { session in
            do { _ = try session.save(previous) }
            catch { session.lastError = error.localizedDescription; session.onChange?() }
        }
        undoManager.setActionName("Visible Whitespace Style")
        if ownsGroup { undoManager.endUndoGrouping() }
    }

    private func refreshExternalChange() {
        do {
            try configuration.reloadFromDisk()
            let current = configuration.whitespacePresentation.visibleWhitespace.style
            if current != style {
                style = current
                groupStart = nil
                undoManager.removeAllActions()
            }
            lastError = nil
        } catch { lastError = error.localizedDescription }
        onChange?()
    }

    private static func invalid(_ message: String) -> NSError {
        NSError(domain: "Viem.VisibleWhitespaceStyle", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
    }
}

private extension EVVisibleWhitespaceStyle {
    mutating func apply(_ mutation: EVStyleMutation) throws {
        let property: EVStyleProperty
        let value: EVStyleValue?
        switch mutation {
        case let .setDeclaration(p, v): property = p; value = v
        case let .clearDeclaration(p): property = p; value = nil
        default: throw EVStyleBridgeError.core(status: UInt32(VIEM_STATUS_INVALID_STYLE_VALUE))
        }
        guard EVStyleProperty.characterProperties.contains(property) else {
            throw EVStyleBridgeError.core(status: UInt32(VIEM_STATUS_INVALID_STYLE_VALUE))
        }
        if value == nil {
            switch property {
            case .characterFontFamilies: fontFamilies = nil; fontFace = nil; fontAxes = nil; weight = nil
            case .characterFontFace: fontFace = nil
            case .characterFontAxes: fontAxes = nil
            case .characterSize: size = nil
            case .characterWeight: weight = nil
            case .characterBold: bold = nil
            case .characterSlant: slant = nil
            case .characterForeground: foreground = nil
            case .characterBackground: background = nil
            case .characterUnderline: underline = nil
            case .characterStrikethrough: strikethrough = nil
            case .characterLanguage: language = nil
            case .characterDirection: direction = nil
            case .characterOpenTypeFeatures: openTypeFeatures = nil
            case .characterLetterSpacing: letterSpacing = nil
            default: break
            }
            return
        }
        switch (property, value!) {
        case let (.characterFontFamilies, .stringList(v)): fontFamilies = v
        case let (.characterFontFace, .string(v)): fontFace = v
        case let (.characterFontAxes, .string(v)): fontAxes = EVFontVariations.decode(v)
        case let (.characterSize, .float(v)): size = v
        case let (.characterWeight, .unsigned(v)) where v <= UInt16.max: weight = UInt16(v)
        case let (.characterBold, .boolean(v)): bold = v
        case let (.characterSlant, .fontSlant(v)) where v <= 2: slant = [.upright, .italic, .oblique][Int(v)]
        case let (.characterForeground, .color(v)): foreground = EVThemeColor(Double(v.red), Double(v.green), Double(v.blue), Double(v.alpha))
        case let (.characterBackground, .color(v)): background = EVThemeColor(Double(v.red), Double(v.green), Double(v.blue), Double(v.alpha))
        case let (.characterUnderline, .boolean(v)): underline = v
        case let (.characterStrikethrough, .boolean(v)): strikethrough = v
        case let (.characterLanguage, .string(v)): language = v
        case let (.characterDirection, .writingDirection(v)) where v <= 2: direction = [.natural, .leftToRight, .rightToLeft][Int(v)]
        case let (.characterOpenTypeFeatures, .openTypeFeatures(v)):
            var features: [String: UInt32] = [:]
            for item in v { features[item.tag] = item.setting }
            openTypeFeatures = features
        case let (.characterLetterSpacing, .float(v)): letterSpacing = v
        default: throw EVStyleBridgeError.core(status: UInt32(VIEM_STATUS_INVALID_STYLE_VALUE))
        }
    }

    var definition: EVStyleDefinition {
        let key = EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Visible whitespace"))
        var declared: [EVStyleProperty: EVStyleValue] = [:]
        declared[.characterFontFamilies] = fontFamilies.map(EVStyleValue.stringList)
        declared[.characterFontFace] = fontFace.map(EVStyleValue.string)
        declared[.characterFontAxes] = fontAxes.map { .string(EVFontVariations.encode($0)) }
        declared[.characterSize] = size.map(EVStyleValue.float)
        declared[.characterWeight] = weight.map { .unsigned(UInt32($0)) }
        declared[.characterBold] = bold.map(EVStyleValue.boolean)
        declared[.characterSlant] = slant.map { .fontSlant($0 == .upright ? 0 : $0 == .italic ? 1 : 2) }
        declared[.characterForeground] = foreground.map { .color(EVStyleColor(red: Float($0.red), green: Float($0.green), blue: Float($0.blue), alpha: Float($0.alpha))) }
        declared[.characterBackground] = background.map { .color(EVStyleColor(red: Float($0.red), green: Float($0.green), blue: Float($0.blue), alpha: Float($0.alpha))) }
        declared[.characterUnderline] = underline.map(EVStyleValue.boolean)
        declared[.characterStrikethrough] = strikethrough.map(EVStyleValue.boolean)
        declared[.characterLanguage] = language.map(EVStyleValue.string)
        declared[.characterDirection] = direction.map { .writingDirection($0 == .natural ? 0 : $0 == .leftToRight ? 1 : 2) }
        declared[.characterOpenTypeFeatures] = openTypeFeatures.map { .openTypeFeatures($0.sorted { $0.key < $1.key }.map { EVOpenTypeFeature(tag: $0.key, setting: $0.value) }) }
        declared[.characterLetterSpacing] = letterSpacing.map(EVStyleValue.float)
        let properties = Dictionary(uniqueKeysWithValues: EVStyleProperty.characterProperties.map { property in
            (property, EVResolvedStyleProperty(property: property, declared: declared[property], effective: declared[property],
                contributorKind: UInt32(VIEM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY), contributor: declared[property] == nil ? nil : key, dependencies: []))
        })
        return EVStyleDefinition(key: key, name: "Visible whitespace", kind: .character, origin: .generatedConfiguration,
            flags: [], capabilities: [.declarations], parentID: nil, nextStyleID: nil, properties: properties)
    }
}

private final class EVVisibleWhitespaceStylePanel: NSPanel {
    var settingsUndoManager: UndoManager?
    override var undoManager: UndoManager? { settingsUndoManager ?? super.undoManager }
    override var canBecomeMain: Bool { true }
}

@MainActor
final class EVVisibleWhitespaceStyleEditor: NSWindowController, NSWindowDelegate, NSTextFieldDelegate {
    private static var sharedController: EVVisibleWhitespaceStyleEditor?
    let session: EVVisibleWhitespaceStyleSession
    private let controls = EVCompactStyleControls()
    private let preview = EVCoreTextStylePreviewView()
    private let diagnostic = NSTextField(wrappingLabelWithString: "")
    private let language = NSTextField(string: "")
    private let languageOverride = NSButton(checkboxWithTitle: "Language", target: nil, action: nil)
    private let weight = NSTextField(string: "")
    private let slant = NSPopUpButton()
    private var themeObserver: NSObjectProtocol?

    static func show(configuration: EVConfigurationStore) {
        if sharedController?.session.configuration.directory.standardizedFileURL != configuration.directory.standardizedFileURL {
            sharedController?.close()
            sharedController = EVVisibleWhitespaceStyleEditor(configuration: configuration)
        }
        sharedController?.showWindow(nil)
        sharedController?.window?.makeKeyAndOrderFront(nil)
    }

    init(configuration: EVConfigurationStore) {
        session = EVVisibleWhitespaceStyleSession(configuration: configuration)
        let panel = EVVisibleWhitespaceStylePanel(contentRect: NSRect(x: 0, y: 0, width: 720, height: 470),
            styleMask: [.titled, .closable, .resizable, .utilityWindow], backing: .buffered, defer: false)
        panel.title = "Visible whitespace"
        panel.setAccessibilityLabel("Visible whitespace style editor")
        panel.settingsUndoManager = session.undoManager
        panel.isFloatingPanel = false
        panel.level = .normal
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.contentMinSize = NSSize(width: 700, height: 440)
        super.init(window: panel)
        panel.delegate = self
        panel.center()
        build()
        controls.onMutations = { [weak self] in _ = self?.session.apply($0) }
        controls.onEditBegan = { [weak self] in self?.session.beginGroup() }
        controls.onEditEnded = { [weak self] in self?.session.endGroup() }
        session.onChange = { [weak self] in self?.refresh() }
        themeObserver = NotificationCenter.default.addObserver(forName: .viemThemeDidChange, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
        refresh()
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
    deinit { if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) } }

    func windowWillClose(_ notification: Notification) { session.endGroup() }

    private func build() {
        let explanation = NSTextField(wrappingLabelWithString: "This character style applies to visible whitespace in Text, Code, and source views. Unchecked properties inherit the surrounding text. The preview uses a sample text style.")
        explanation.textColor = .secondaryLabelColor
        languageOverride.target = self
        languageOverride.action = #selector(changeLanguageOverride(_:))
        languageOverride.setAccessibilityLabel("Override marker language")
        language.placeholderString = "Language tag"
        language.setAccessibilityLabel("Marker language")
        language.delegate = self
        language.target = self
        language.action = #selector(changeLanguage(_:))
        language.widthAnchor.constraint(equalToConstant: 110).isActive = true
        weight.placeholderString = "Inherited"
        weight.setAccessibilityLabel("Marker weight")
        weight.toolTip = "Weight from 1 to 1000; leave blank to inherit"
        weight.delegate = self
        weight.target = self
        weight.action = #selector(changeWeight(_:))
        weight.widthAnchor.constraint(equalToConstant: 85).isActive = true
        slant.addItems(withTitles: ["Inherited", "Upright", "Italic", "Oblique"])
        slant.setAccessibilityLabel("Marker slant")
        slant.target = self
        slant.action = #selector(changeSlant(_:))
        let additional = NSStackView(views: [languageOverride, language, NSTextField(labelWithString: "Weight"), weight, NSTextField(labelWithString: "Slant"), slant])
        additional.spacing = 10
        let restore = NSButton(title: "Restore Defaults", target: self, action: #selector(restoreDefaults))
        diagnostic.textColor = .systemRed
        diagnostic.font = .systemFont(ofSize: 12)
        let stack = NSStackView(views: [explanation, controls.characterView, additional, diagnostic, preview, restore])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 18
        stack.edgeInsets = NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
        stack.translatesAutoresizingMaskIntoConstraints = false
        let content = NSView()
        content.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: content.leadingAnchor), stack.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            stack.topAnchor.constraint(equalTo: content.topAnchor), stack.bottomAnchor.constraint(lessThanOrEqualTo: content.bottomAnchor),
            controls.characterView.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -40),
            explanation.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -40),
            preview.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -40), preview.heightAnchor.constraint(equalToConstant: 120),
        ])
        window?.contentView = content
    }

    private func refresh() {
        let style = session.style
        let definition = style.definition
        controls.configure(definition, theme: EVThemeStore.shared.theme, allowsPercentageSize: false)
        languageOverride.state = style.language == nil ? .off : .on
        language.isEnabled = style.language != nil
        language.stringValue = style.language ?? ""
        weight.stringValue = style.weight.map(String.init) ?? ""
        slant.selectItem(at: style.slant.map { $0 == .upright ? 1 : $0 == .italic ? 2 : 3 } ?? 0)
        diagnostic.stringValue = session.lastError ?? ""
        diagnostic.isHidden = diagnostic.stringValue.isEmpty
        var values = definition.properties.compactMapValues(\.effective)
        if values[.characterForeground] == nil {
            let color = EVThemeStore.shared.theme.foreground
            values[.characterForeground] = .color(EVStyleColor(red: Float(color.red), green: Float(color.green), blue: Float(color.blue), alpha: Float(color.alpha)))
        }
        preview.apply(kind: .character, effectiveValues: values)
    }

    @objc private func changeLanguageOverride(_ sender: NSButton) {
        session.endGroup()
        session.apply([sender.state == .on ? .setDeclaration(.characterLanguage, .string("en")) : .clearDeclaration(.characterLanguage)])
    }
    @objc private func changeLanguage(_ sender: NSTextField) {
        session.apply([.setDeclaration(.characterLanguage, .string(sender.stringValue))])
    }
    @objc private func changeWeight(_ sender: NSTextField) {
        let value = sender.stringValue.trimmingCharacters(in: .whitespaces)
        if value.isEmpty { session.apply([.clearDeclaration(.characterWeight)]) }
        else if let number = UInt32(value), (1...1000).contains(number) { session.apply([.setDeclaration(.characterWeight, .unsigned(number))]) }
        else { refresh(); diagnostic.stringValue = "Weight must be a whole number from 1 to 1000."; diagnostic.isHidden = false }
    }
    @objc private func changeSlant(_ sender: NSPopUpButton) {
        session.endGroup()
        session.apply([sender.indexOfSelectedItem == 0 ? .clearDeclaration(.characterSlant) : .setDeclaration(.characterSlant, .fontSlant(UInt32(sender.indexOfSelectedItem - 1)))])
    }
    @objc private func restoreDefaults() { session.restoreDefaults() }
    func controlTextDidBeginEditing(_ notification: Notification) { session.beginGroup() }
    func controlTextDidEndEditing(_ notification: Notification) {
        if notification.object as AnyObject? === language { changeLanguage(language) }
        if notification.object as AnyObject? === weight { changeWeight(weight) }
        session.endGroup()
    }
}
