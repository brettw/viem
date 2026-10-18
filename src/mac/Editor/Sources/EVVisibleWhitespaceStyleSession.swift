import AppKit
import CViemCore
import ViemAppShell
import ViemCoreTextProvider

/// History belongs to the application setting, independently of document undo.
@MainActor
final class EVVisibleWhitespaceStyleSession {
    private static var nextRevision: UInt64 = 0
    let configuration: EVConfigurationStore
    let undoManager: UndoManager
    var onChange: (() -> Void)?
    private(set) var style: EVVisibleWhitespaceStyle
    private(set) var revision: UInt64
    private(set) var lastError: String?
    private var groupStart: EVVisibleWhitespaceStyle?
    private var writing = false
    private var observer: NSObjectProtocol?

    init(configuration: EVConfigurationStore, undoManager: UndoManager? = nil) {
        self.configuration = configuration
        self.undoManager = undoManager ?? UndoManager()
        revision = Self.allocateRevision()
        style = configuration.whitespacePresentation.visibleWhitespace.style
        lastError = configuration.lastError
        self.undoManager.groupsByEvent = false
        observer = NotificationCenter.default.addObserver(forName: .viemConfigurationDidChange, object: nil, queue: .main) { [weak self] notification in
            MainActor.assumeIsolated {
                guard let self, !self.writing, let source = notification.object as? EVConfigurationStore,
                      source.directory.standardizedFileURL == self.configuration.directory.standardizedFileURL else { return }
                self.refreshExternalChange()
            }
        }
    }

    deinit { if let observer { NotificationCenter.default.removeObserver(observer) } }

    private static func allocateRevision() -> UInt64 {
        precondition(nextRevision < UInt64.max)
        nextRevision += 1
        return nextRevision
    }

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
        revision = Self.allocateRevision()
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
        let previous = style
        let previousError = lastError
        do {
            try configuration.reloadFromDisk()
            let current = configuration.whitespacePresentation.visibleWhitespace.style
            if current != style {
                style = current
                revision = Self.allocateRevision()
                groupStart = nil
                undoManager.removeAllActions()
            }
            lastError = nil
        } catch { lastError = error.localizedDescription }
        if style != previous || lastError != previousError { onChange?() }
    }

    private static func invalid(_ message: String) -> NSError {
        NSError(domain: "Viem.VisibleWhitespaceStyle", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
    }
}

extension EVVisibleWhitespaceStyle {
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
            case .characterSuperscript: superscript = nil
            case .characterSubscript: `subscript` = nil
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
        case let (.characterSuperscript, .boolean(v)): superscript = v; if v { `subscript` = false }
        case let (.characterSubscript, .boolean(v)): `subscript` = v; if v { superscript = false }
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

    func definition(in snapshot: EVStyleSheetSnapshot) -> EVStyleDefinition {
        let key = EVStyleKey.visibleWhitespace
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
        declared[.characterSuperscript] = superscript.map(EVStyleValue.boolean)
        declared[.characterSubscript] = `subscript`.map(EVStyleValue.boolean)
        declared[.characterLanguage] = language.map(EVStyleValue.string)
        declared[.characterDirection] = direction.map { .writingDirection($0 == .natural ? 0 : $0 == .leftToRight ? 1 : 2) }
        declared[.characterOpenTypeFeatures] = openTypeFeatures.map { .openTypeFeatures($0.sorted { $0.key < $1.key }.map { EVOpenTypeFeature(tag: $0.key, setting: $0.value) }) }
        declared[.characterLetterSpacing] = letterSpacing.map(EVStyleValue.float)
        let properties = Dictionary(uniqueKeysWithValues: EVStyleProperty.characterProperties.map { property in
            let inherited = snapshot.definition(for: .baseParagraph)?.properties[property]
            return (property, EVResolvedStyleProperty(property: property, declared: declared[property], effective: declared[property] ?? inherited?.effective,
                contributorKind: declared[property] == nil ? inherited?.contributorKind ?? UInt32(VIEM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY) : UInt32(VIEM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY),
                contributor: declared[property] == nil ? inherited?.contributor : key, dependencies: []))
        })
        return EVStyleDefinition(key: key, name: "Visible whitespace", kind: .character, origin: .generatedConfiguration,
            flags: [.internalSyntax], capabilities: [.declarations], parentID: nil, nextStyleID: nil, properties: properties)
    }
}
