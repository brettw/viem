import AppKit
import CViemCore
import Foundation

enum EVStyleNamespace: UInt32, Hashable {
    case block = 1
    case character = 2
}

struct EVStyleID: Hashable, RawRepresentable, CustomStringConvertible {
    let rawValue: String

    init(rawValue: String) {
        self.rawValue = rawValue
    }

    var description: String { rawValue }

    static let baseParagraph = EVStyleID(rawValue: "Paragraph")
    static let defaultParagraph = EVStyleID(rawValue: "")
}

struct EVStyleKey: Hashable, CustomStringConvertible {
    let namespace: EVStyleNamespace
    let id: EVStyleID

    var description: String { "\(namespace):\(id.rawValue)" }

    static let baseParagraph = EVStyleKey(namespace: .block, id: .baseParagraph)
    static let defaultParagraph = EVStyleKey(namespace: .character, id: .defaultParagraph)
}

enum EVStyleKind: Int, CaseIterable {
    case paragraph
    case character
    case quote
    case codeBlock
    case list
    case listItem

    var isContainer: Bool { self != .paragraph && self != .character }

    var displayName: String {
        switch self {
        case .paragraph: "Paragraph"
        case .character: "Character"
        case .quote, .codeBlock, .list, .listItem: "Container"
        }
    }

}

struct EVStyleSheetIdentity: Equatable {
    let documentID: UInt64
    let documentRevision: UInt64
    let styleSheetRevision: UInt64

    init(documentID: UInt64, documentRevision: UInt64, styleSheetRevision: UInt64) {
        self.documentID = documentID
        self.documentRevision = documentRevision
        self.styleSheetRevision = styleSheetRevision
    }

    init(_ value: ViemStyleSheetIdentityV1) {
        documentID = value.document_id
        documentRevision = value.document_revision
        styleSheetRevision = value.style_sheet_revision
    }

    var abiValue: ViemStyleSheetIdentityV1 {
        var value = ViemStyleSheetIdentityV1()
        value.struct_size = UInt32(MemoryLayout<ViemStyleSheetIdentityV1>.size)
        value.document_id = documentID
        value.document_revision = documentRevision
        value.style_sheet_revision = styleSheetRevision
        return value
    }
}

/// Exact, core-owned capability for one live style-field or gesture edit.
/// Keeping the complete ABI value prevents a token from being replayed against
/// another view or document even if the numeric token is copied accidentally.
struct EVStyleEditGroup {
    fileprivate let abiValue: ViemStyleEditGroupV1

    var token: UInt64 { abiValue.token }
}

struct EVStyleDefinitionFlags: OptionSet, Equatable {
    let rawValue: UInt32

    static let hasParent = Self(rawValue: UInt32(VIEM_STYLE_DEFINITION_HAS_PARENT))
    static let hasNextStyle = Self(rawValue: UInt32(VIEM_STYLE_DEFINITION_HAS_NEXT_STYLE))
    static let baseParagraph = Self(rawValue: UInt32(VIEM_STYLE_DEFINITION_BASE_PARAGRAPH))
    static let internalSyntax = Self(rawValue: UInt32(VIEM_STYLE_DEFINITION_INTERNAL))
    static let internalList = Self(rawValue: UInt32(VIEM_STYLE_DEFINITION_INTERNAL_LIST))
    static let implicit = Self(rawValue: UInt32(VIEM_STYLE_DEFINITION_IMPLICIT))

    var isBase: Bool {
        contains(.baseParagraph)
    }
}

struct EVStyleCapabilities: OptionSet, Equatable {
    let rawValue: UInt32

    static let declarations = Self(rawValue: UInt32(VIEM_STYLE_CAPABILITY_EDIT_DECLARATIONS))
    static let parent = Self(rawValue: UInt32(VIEM_STYLE_CAPABILITY_EDIT_PARENT))
    static let nextStyle = Self(rawValue: UInt32(VIEM_STYLE_CAPABILITY_EDIT_NEXT_STYLE))
    static let displayName = Self(rawValue: UInt32(VIEM_STYLE_CAPABILITY_EDIT_DISPLAY_NAME))
    static let assign = Self(rawValue: UInt32(VIEM_STYLE_CAPABILITY_ASSIGN))
    static let delete = Self(rawValue: UInt32(VIEM_STYLE_CAPABILITY_DELETE))
}

enum EVStyleOrigin: UInt32, Equatable {
    case generatedConfiguration = 2
    case syntheticReadOnly = 3

    var displayName: String {
        switch self {
        case .generatedConfiguration: "generated configuration"
        case .syntheticReadOnly: "synthetic read-only"
        }
    }
}

enum EVStyleProperty: UInt32, CaseIterable, Hashable {
    case canvasBackground = 1
    case canvasPaddingTop = 2
    case canvasPaddingRight = 3
    case canvasPaddingBottom = 4
    case canvasPaddingLeft = 5
    case blockMarginTop = 6
    case blockMarginBottom = 7
    case paragraphLineSpacing = 8
    case paragraphFirstLineIndent = 9
    case paragraphLeadingIndent = 10
    case paragraphTrailingIndent = 11
    case paragraphAlignment = 12
    case paragraphBaseDirection = 13
    case characterFontFamilies = 14
    case characterSize = 15
    case characterWeight = 16
    case characterSlant = 17
    case characterForeground = 18
    case characterBackground = 19
    case characterUnderline = 20
    case characterStrikethrough = 21
    case characterLanguage = 22
    case characterDirection = 23
    case characterOpenTypeFeatures = 24
    case characterFontAxes = 43
    case characterLetterSpacing = 25
    case characterBold = 27
    case blockMarginRight = 28
    case blockMarginLeft = 29
    case blockPaddingTop = 30
    case blockPaddingRight = 31
    case blockPaddingBottom = 32
    case blockPaddingLeft = 33
    case blockBorderTopWidth = 34
    case blockBorderTopColor = 35
    case blockBorderRightWidth = 36
    case blockBorderRightColor = 37
    case blockBorderBottomWidth = 38
    case blockBorderBottomColor = 39
    case blockBorderLeftWidth = 40
    case blockBorderLeftColor = 41
    case blockBackground = 42

    static let characterProperties: [Self] = [
        .characterFontFamilies, .characterSize, .characterWeight, .characterSlant, .characterBold,
        .characterForeground, .characterBackground, .characterUnderline,
        .characterStrikethrough, .characterLanguage, .characterDirection,
        .characterOpenTypeFeatures, .characterFontAxes, .characterLetterSpacing,
    ]

    static let paragraphProperties: [Self] = [
        .paragraphFirstLineIndent,
        .paragraphLeadingIndent, .paragraphTrailingIndent, .paragraphLineSpacing,
        .paragraphAlignment, .paragraphBaseDirection,
    ]

    static let blockProperties: [Self] = [
        .blockBackground, .blockMarginTop, .blockMarginRight, .blockMarginBottom, .blockMarginLeft,
        .blockPaddingTop, .blockPaddingRight, .blockPaddingBottom, .blockPaddingLeft,
        .blockBorderTopWidth, .blockBorderTopColor, .blockBorderRightWidth, .blockBorderRightColor,
        .blockBorderBottomWidth, .blockBorderBottomColor, .blockBorderLeftWidth, .blockBorderLeftColor,
    ]
    static let nonnegativeBlockProperties: Set<Self> = [
        .blockPaddingTop, .blockPaddingRight, .blockPaddingBottom, .blockPaddingLeft,
        .blockBorderTopWidth, .blockBorderRightWidth, .blockBorderBottomWidth, .blockBorderLeftWidth,
    ]
    static let blockBorderColors: Set<Self> = [
        .blockBorderTopColor, .blockBorderRightColor, .blockBorderBottomColor, .blockBorderLeftColor,
    ]

    var displayName: String {
        switch self {
        case .canvasBackground: "Canvas background"
        case .canvasPaddingTop: "Canvas padding top"
        case .canvasPaddingRight: "Canvas padding right"
        case .canvasPaddingBottom: "Canvas padding bottom"
        case .canvasPaddingLeft: "Canvas padding left"
        case .blockMarginTop: "Margin top"
        case .blockMarginBottom: "Margin bottom"
        case .paragraphLineSpacing: "Line spacing"
        case .paragraphFirstLineIndent: "First-line indent"
        case .paragraphLeadingIndent: "Start indent"
        case .paragraphTrailingIndent: "End indent"
        case .paragraphAlignment: "Alignment"
        case .paragraphBaseDirection: "Base direction"
        case .characterFontFamilies: "Font families"
        case .characterSize: "Font size"
        case .characterFontAxes: "Font axes"
        case .characterWeight: "Weight"
        case .characterBold: "Bold"
        case .characterSlant: "Slant"
        case .characterForeground: "Foreground"
        case .characterBackground: "Background"
        case .characterUnderline: "Underline"
        case .characterStrikethrough: "Strikethrough"
        case .characterLanguage: "Language"
        case .characterDirection: "Writing direction"
        case .characterOpenTypeFeatures: "OpenType features"
        case .characterLetterSpacing: "Letter spacing"
        case .blockMarginRight: "Margin right"
        case .blockMarginLeft: "Margin left"
        case .blockPaddingTop: "Padding top"
        case .blockPaddingRight: "Padding right"
        case .blockPaddingBottom: "Padding bottom"
        case .blockPaddingLeft: "Padding left"
        case .blockBorderTopWidth: "Border top weight"
        case .blockBorderTopColor: "Border top color"
        case .blockBorderRightWidth: "Border right weight"
        case .blockBorderRightColor: "Border right color"
        case .blockBorderBottomWidth: "Border bottom weight"
        case .blockBorderBottomColor: "Border bottom color"
        case .blockBorderLeftWidth: "Border left weight"
        case .blockBorderLeftColor: "Border left color"
        case .blockBackground: "Block background"
        }
    }
}

struct EVStyleColor: Equatable {
    let red: Float
    let green: Float
    let blue: Float
    let alpha: Float

    var appKitColor: NSColor {
        // Use the same portable RGB space when displaying and reading colors;
        // calibrated/device RGB conversion would change their source values.
        NSColor(
            srgbRed: CGFloat(red),
            green: CGFloat(green),
            blue: CGFloat(blue),
            alpha: CGFloat(alpha)
        )
    }
}

struct EVOpenTypeFeature: Equatable {
    let tag: String
    let setting: UInt32
}

struct EVLineSpacing: Equatable {
    let kind: UInt32
    let value: Float
}

enum EVStyleValue: Equatable {
    case float(Float)
    case percentage(UInt32)
    case unsigned(UInt32)
    case boolean(Bool)
    case color(EVStyleColor)
    case string(String)
    case stringList([String])
    case fontSlant(UInt32)
    case writingDirection(UInt32)
    case openTypeFeatures([EVOpenTypeFeature])
    case lineSpacing(EVLineSpacing)
    case paragraphAlignment(UInt32)
}

struct EVResolvedStyleProperty: Equatable {
    let property: EVStyleProperty
    let declared: EVStyleValue?
    let effective: EVStyleValue?
    let contributorKind: UInt32
    let contributor: EVStyleKey?
    let dependencies: [EVStyleKey]

    var isDeclared: Bool { declared != nil }

    var usesThemeDefault: Bool {
        [.characterForeground, .canvasBackground].contains(property)
            && declared == nil && contributor == nil
            && contributorKind == UInt32(VIEM_STYLE_CONTRIBUTOR_ENGINE_EMERGENCY)
    }
}

struct EVStyleDefinition: Equatable {
    let key: EVStyleKey
    let name: String
    let kind: EVStyleKind
    let origin: EVStyleOrigin
    let flags: EVStyleDefinitionFlags
    let capabilities: EVStyleCapabilities
    let parentID: EVStyleID?
    let nextStyleID: EVStyleID?
    let properties: [EVStyleProperty: EVResolvedStyleProperty]

    var parentKey: EVStyleKey? {
        parentID.map { EVStyleKey(namespace: key.namespace, id: $0) }
    }
}

struct EVStyleSheetSnapshot: Equatable {
    let identity: EVStyleSheetIdentity
    let definitions: [EVStyleDefinition]

    func definition(for key: EVStyleKey) -> EVStyleDefinition? {
        definitions.first { $0.key == key }
    }

    func definition(namespace: EVStyleNamespace, id: EVStyleID) -> EVStyleDefinition? {
        definition(for: EVStyleKey(namespace: namespace, id: id))
    }

    var fallbackDefinition: EVStyleDefinition? {
        definition(for: .baseParagraph) ?? definitions.first
    }

    func legalParents(for selected: EVStyleDefinition) -> [EVStyleDefinition] {
        guard selected.capabilities.contains(.parent) else { return [] }
        return definitions.filter { candidate in
            guard candidate.key.namespace == selected.key.namespace,
                  candidate.key != selected.key,
                  (rolesCanInherit(child: selected.kind, parent: candidate.kind)
                    || (selected.kind.isContainer && candidate.flags.contains(.baseParagraph))),
                  !isDescendant(candidate.key, of: selected.key)
            else { return false }
            return true
        }
    }

    func compatibleFollowingStyles(for selected: EVStyleDefinition) -> [EVStyleDefinition] {
        guard selected.kind == .paragraph else { return [] }
        return definitions.filter { $0.kind == .paragraph }
    }

    private func rolesCanInherit(child: EVStyleKind, parent: EVStyleKind) -> Bool {
        child == parent
    }

    private func isDescendant(_ possibleDescendant: EVStyleKey, of ancestor: EVStyleKey) -> Bool {
        var current = definition(for: possibleDescendant)?.parentKey
        var visited = Set<EVStyleKey>()
        while let key = current, visited.insert(key).inserted {
            if key == ancestor { return true }
            current = definition(for: key)?.parentKey
        }
        return false
    }
}

enum EVStyleMutation {
    case setDeclaration(EVStyleProperty, EVStyleValue)
    case clearDeclaration(EVStyleProperty)
    case setParent(EVStyleID)
    case clearParent
    case setNextStyle(EVStyleID)
    case clearNextStyle
    case setDisplayName(String)
}

enum EVStyleBridgeError: LocalizedError, Equatable {
    case core(status: UInt32)
    case malformedSnapshot(String)
    case noEditingView

    var errorDescription: String? {
        switch self {
        case let .core(status): Self.message(for: status)
        case let .malformedSnapshot(reason): "The core returned a malformed style sheet: \(reason)."
        case .noEditingView: "The target document no longer has an active editor view."
        }
    }

    var isStale: Bool {
        if case let .core(status) = self {
            return status == UInt32(VIEM_STATUS_STALE_REVISION)
                || status == UInt32(VIEM_STATUS_UNKNOWN_STYLE)
        }
        return false
    }

    var isEndedStyleEditGroup: Bool {
        guard case let .core(status) = self else { return false }
        return status == UInt32(VIEM_STATUS_INVALID_STYLE_EDIT_GROUP)
            || status == UInt32(VIEM_STATUS_STYLE_EDIT_GROUP_WRONG_OWNER)
    }

    private static func message(for status: UInt32) -> String {
        switch status {
        case UInt32(VIEM_STATUS_STALE_REVISION):
            "The style sheet changed before this edit could be applied. The fields were refreshed."
        case UInt32(VIEM_STATUS_UNKNOWN_STYLE):
            "The selected style no longer exists. Base Paragraph was selected."
        case UInt32(VIEM_STATUS_STYLE_READ_ONLY):
            "This style is owned by the source adapter and is read-only."
        case UInt32(VIEM_STATUS_INVALID_STYLE_VALUE):
            "The style value is invalid."
        case UInt32(VIEM_STATUS_STYLE_INHERITANCE_CYCLE):
            "That parent would create an inheritance cycle."
        case UInt32(VIEM_STATUS_INCOMPATIBLE_STYLE_ROLE):
            "That style cannot be used for this role."
        case UInt32(VIEM_STATUS_INVALID_STYLE_RELATIONSHIP):
            "That style relationship is not valid."
        case UInt32(VIEM_STATUS_STYLE_EDIT_GROUP_ACTIVE):
            "Another live style edit is already in progress."
        case UInt32(VIEM_STATUS_INVALID_STYLE_EDIT_GROUP):
            "That live style edit has already ended. The fields were refreshed."
        case UInt32(VIEM_STATUS_STYLE_EDIT_GROUP_WRONG_OWNER):
            "That live style edit belongs to another document view."
        default:
            "The style edit was rejected (Viem core status \(status))."
        }
    }
}

extension Notification.Name {
    static let viemCoreDocumentDidChange = Notification.Name("EVCoreDocumentDidChange")
}

@MainActor
extension EVCoreDocumentBackend {
    func styleSheetSnapshot() throws -> EVStyleSheetSnapshot {
        let identity = try EVCoreStyleBridge.styleIdentity(core: core)
        if let cachedStyleSheet, cachedStyleSheet.identity == identity { return cachedStyleSheet }
        // A concurrent frontend turn can land between the size and copy calls.
        // Retrying obtains one exact immutable export; the copy itself never
        // substitutes a newer revision.
        for _ in 0..<3 {
            do {
                let fresh = try EVCoreStyleBridge.copyStyleSheet(core: core)
                cachedStyleSheet = fresh
                return fresh
            } catch let error as EVStyleBridgeError where error.isStale {
                continue
            }
        }
        throw EVStyleBridgeError.core(status: UInt32(VIEM_STATUS_STALE_REVISION))
    }
}

@MainActor
extension EVCoreViewSession {
    func beginStyleEditGroup(expected: EVStyleSheetIdentity) throws -> EVStyleEditGroup {
        guard viewID != 0 else { throw EVStyleBridgeError.noEditingView }
        return try EVCoreStyleBridge.beginGroup(
            core: document.core,
            view: viewID,
            expected: expected
        )
    }

    @discardableResult
    func editStyle(
        key: EVStyleKey,
        expected: EVStyleSheetIdentity,
        mutation: EVStyleMutation
    ) throws -> ViemCoreOutcomeV1 {
        guard viewID != 0 else { throw EVStyleBridgeError.noEditingView }
        let outcome = try EVCoreStyleBridge.apply(
            core: document.core,
            view: viewID,
            key: key,
            expected: expected,
            mutation: mutation
        )
        finishStyleEdit(outcome)
        return outcome
    }

    @discardableResult
    func editStyle(
        key: EVStyleKey,
        expected: EVStyleSheetIdentity,
        mutation: EVStyleMutation,
        in group: EVStyleEditGroup
    ) throws -> ViemCoreOutcomeV1 {
        guard viewID != 0 else { throw EVStyleBridgeError.noEditingView }
        let outcome = try EVCoreStyleBridge.apply(
            core: document.core,
            view: viewID,
            key: key,
            expected: expected,
            mutation: mutation,
            group: group
        )
        finishStyleEdit(outcome)
        return outcome
    }

    func endStyleEditGroup(_ group: EVStyleEditGroup) throws {
        guard viewID != 0 else { throw EVStyleBridgeError.noEditingView }
        try EVCoreStyleBridge.endGroup(core: document.core, view: viewID, group: group)
    }
}

enum EVCoreStyleBridge {
    static func styleIdentity(core: ViemCoreHandle) throws -> EVStyleSheetIdentity {
        var identity = ViemStyleSheetIdentityV1()
        identity.struct_size = UInt32(MemoryLayout<ViemStyleSheetIdentityV1>.size)
        try check(viem_core_style_sheet_identity(core, &identity))
        return EVStyleSheetIdentity(identity)
    }
    static func beginGroup(
        core: ViemCoreHandle,
        view: ViemViewId,
        expected: EVStyleSheetIdentity
    ) throws -> EVStyleEditGroup {
        var expectedValue = expected.abiValue
        var group = ViemStyleEditGroupV1()
        group.struct_size = UInt32(MemoryLayout<ViemStyleEditGroupV1>.size)
        try check(viem_core_view_begin_style_edit_group(core, view, &expectedValue, &group))
        return EVStyleEditGroup(abiValue: group)
    }

    static func endGroup(
        core: ViemCoreHandle,
        view: ViemViewId,
        group: EVStyleEditGroup
    ) throws {
        var value = group.abiValue
        try check(viem_core_view_end_style_edit_group(core, view, &value))
    }

    static func copyStyleSheet(core: ViemCoreHandle?) throws -> EVStyleSheetSnapshot {
        var info = ViemStyleSheetInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemStyleSheetInfoV1>.size)
        if let core { try check(viem_core_style_sheet_info(core, &info)) }
        else { try check(viem_code_style_sheet_info(&info)) }

        guard info.definition_count <= UInt64(Int.max),
              info.property_count <= UInt64(Int.max),
              info.value_item_count <= UInt64(Int.max),
              info.dependency_count <= UInt64(Int.max),
              info.string_bytes <= UInt64(Int.max)
        else { throw EVStyleBridgeError.malformedSnapshot("array length overflow") }

        var definitions = Array(repeating: ViemStyleDefinitionV1(), count: Int(info.definition_count))
        var properties = Array(repeating: ViemStylePropertyV1(), count: Int(info.property_count))
        var items = Array(repeating: ViemStyleValueItemV1(), count: Int(info.value_item_count))
        var dependencies = Array(repeating: ViemStyleDependencyV1(), count: Int(info.dependency_count))
        var strings = Array(repeating: UInt8(0), count: Int(info.string_bytes))
        var copiedInfo = ViemStyleSheetInfoV1()
        copiedInfo.struct_size = UInt32(MemoryLayout<ViemStyleSheetInfoV1>.size)
        var expected = info.identity

        let status = definitions.withUnsafeMutableBufferPointer { definitionsBuffer in
            properties.withUnsafeMutableBufferPointer { propertiesBuffer in
                items.withUnsafeMutableBufferPointer { itemsBuffer in
                    dependencies.withUnsafeMutableBufferPointer { dependenciesBuffer in
                        strings.withUnsafeMutableBufferPointer { stringsBuffer in
                            if let core { return viem_core_copy_style_sheet(
                                core,
                                &expected,
                                definitionsBuffer.baseAddress,
                                UInt64(definitionsBuffer.count),
                                propertiesBuffer.baseAddress,
                                UInt64(propertiesBuffer.count),
                                itemsBuffer.baseAddress,
                                UInt64(itemsBuffer.count),
                                dependenciesBuffer.baseAddress,
                                UInt64(dependenciesBuffer.count),
                                stringsBuffer.baseAddress,
                                UInt64(stringsBuffer.count),
                                &copiedInfo
                            ) }
                            return viem_code_copy_style_sheet(
                                &expected,
                                definitionsBuffer.baseAddress, UInt64(definitionsBuffer.count),
                                propertiesBuffer.baseAddress, UInt64(propertiesBuffer.count),
                                itemsBuffer.baseAddress, UInt64(itemsBuffer.count),
                                dependenciesBuffer.baseAddress, UInt64(dependenciesBuffer.count),
                                stringsBuffer.baseAddress, UInt64(stringsBuffer.count),
                                &copiedInfo
                            )
                        }
                    }
                }
            }
        }
        try check(status)
        return try decode(
            info: copiedInfo,
            definitions: definitions,
            properties: properties,
            items: items,
            dependencies: dependencies,
            strings: strings
        )
    }

    static func apply(
        core: ViemCoreHandle,
        view: ViemViewId,
        key: EVStyleKey,
        expected: EVStyleSheetIdentity,
        mutation: EVStyleMutation,
        group: EVStyleEditGroup? = nil
    ) throws -> ViemCoreOutcomeV1 {
        let encoded = EncodedMutation(key: key, expected: expected, mutation: mutation)
        return try encoded.withRequest { request in
            var request = request
            var outcome = ViemCoreOutcomeV1()
            outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
            if let group {
                var groupValue = group.abiValue
                try check(viem_core_view_edit_style_in_group(
                    core,
                    view,
                    &groupValue,
                    &request,
                    &outcome
                ))
            } else {
                try check(viem_core_view_edit_style(core, view, &request, &outcome))
            }
            return outcome
        }
    }

    static func applyCodeStyle(key: EVStyleKey, expected: EVStyleSheetIdentity, mutation: EVStyleMutation) throws {
        let encoded = EncodedMutation(key: key, expected: expected, mutation: mutation)
        try encoded.withRequest { request in
            var request = request
            var info = ViemStyleSheetInfoV1()
            info.struct_size = UInt32(MemoryLayout<ViemStyleSheetInfoV1>.size)
            try check(viem_code_edit_style(&request, &info))
        }
    }

    static func createCodeStyle(key: EVStyleKey, name: String, expected: EVStyleSheetIdentity) throws {
        var request = ViemCreateStyleV1()
        request.struct_size = UInt32(MemoryLayout<ViemCreateStyleV1>.size)
        request.namespace = key.namespace.rawValue
        request.identity = expected.abiValue
        var info = ViemStyleSheetInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemStyleSheetInfoV1>.size)
        try Array(key.id.rawValue.utf8).withUnsafeBufferPointer { id in
            try Array(name.utf8).withUnsafeBufferPointer { name in
                request.style_id.data = id.baseAddress
                request.style_id.length = UInt64(id.count)
                request.display_name.data = name.baseAddress
                request.display_name.length = UInt64(name.count)
                try check(viem_code_create_style(&request, &info))
            }
        }
    }

    /// Generates the implicit definition for a syntax name, and its missing
    /// dotted ancestors. An existing definition is left unchanged.
    static func materializeCodeStyle(name: String) throws {
        var info = ViemStyleSheetInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemStyleSheetInfoV1>.size)
        try Array(name.utf8).withUnsafeBufferPointer { name in
            try check(viem_code_materialize_style(name.baseAddress, UInt64(name.count), &info))
        }
    }

    static func deleteCodeStyle(key: EVStyleKey, expected: EVStyleSheetIdentity) throws {
        var request = ViemDeleteStyleV1()
        request.struct_size = UInt32(MemoryLayout<ViemDeleteStyleV1>.size)
        request.namespace = key.namespace.rawValue
        request.identity = expected.abiValue
        var info = ViemStyleSheetInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemStyleSheetInfoV1>.size)
        try Array(key.id.rawValue.utf8).withUnsafeBufferPointer { id in
            request.style_id.data = id.baseAddress
            request.style_id.length = UInt64(id.count)
            try check(viem_code_delete_style(&request, &info))
        }
    }

    private static func check(_ status: UInt32) throws {
        guard status == UInt32(VIEM_STATUS_OK) else {
            throw EVStyleBridgeError.core(status: status)
        }
    }

    private static func decode(
        info: ViemStyleSheetInfoV1,
        definitions rawDefinitions: [ViemStyleDefinitionV1],
        properties rawProperties: [ViemStylePropertyV1],
        items: [ViemStyleValueItemV1],
        dependencies: [ViemStyleDependencyV1],
        strings: [UInt8]
    ) throws -> EVStyleSheetSnapshot {
        func text(_ reference: ViemStyleStringRefV1) throws -> String {
            guard reference.offset <= UInt64(Int.max), reference.length <= UInt64(Int.max) else {
                throw EVStyleBridgeError.malformedSnapshot("string range overflow")
            }
            let start = Int(reference.offset)
            let length = Int(reference.length)
            guard start >= 0, length >= 0, start <= strings.count, length <= strings.count - start else {
                throw EVStyleBridgeError.malformedSnapshot("string range outside arena")
            }
            guard let value = String(bytes: strings[start..<(start + length)], encoding: .utf8) else {
                throw EVStyleBridgeError.malformedSnapshot("invalid UTF-8 string")
            }
            return value
        }

        func range(first: UInt64, count: UInt64, limit: Int, label: String) throws -> Range<Int> {
            guard first <= UInt64(Int.max), count <= UInt64(Int.max) else {
                throw EVStyleBridgeError.malformedSnapshot("\(label) range overflow")
            }
            let start = Int(first)
            let length = Int(count)
            guard start >= 0, length >= 0, start <= limit, length <= limit - start else {
                throw EVStyleBridgeError.malformedSnapshot("\(label) range outside array")
            }
            return start..<(start + length)
        }

        func decodeValue(_ raw: ViemStyleValueV1) throws -> EVStyleValue? {
            switch raw.kind {
            case UInt32(VIEM_STYLE_VALUE_NONE): return nil
            case UInt32(VIEM_STYLE_VALUE_FLOAT): return .float(raw.number)
            case UInt32(VIEM_STYLE_VALUE_PERCENTAGE): return .percentage(raw.enum_value)
            case UInt32(VIEM_STYLE_VALUE_UNSIGNED): return .unsigned(raw.enum_value)
            case UInt32(VIEM_STYLE_VALUE_BOOLEAN): return .boolean(raw.enum_value != 0)
            case UInt32(VIEM_STYLE_VALUE_COLOR):
                return .color(EVStyleColor(
                    red: raw.color.red,
                    green: raw.color.green,
                    blue: raw.color.blue,
                    alpha: raw.color.alpha
                ))
            case UInt32(VIEM_STYLE_VALUE_STRING): return .string(try text(raw.string))
            case UInt32(VIEM_STYLE_VALUE_STRING_LIST):
                let itemRange = try range(
                    first: raw.first_item,
                    count: raw.item_count,
                    limit: items.count,
                    label: "value-item"
                )
                return .stringList(try itemRange.map { index in
                    guard items[index].kind == UInt32(VIEM_STYLE_VALUE_ITEM_STRING) else {
                        throw EVStyleBridgeError.malformedSnapshot("unexpected string-list item")
                    }
                    return try text(items[index].string)
                })
            case UInt32(VIEM_STYLE_VALUE_FONT_SLANT): return .fontSlant(raw.enum_value)
            case UInt32(VIEM_STYLE_VALUE_WRITING_DIRECTION): return .writingDirection(raw.enum_value)
            case UInt32(VIEM_STYLE_VALUE_OPEN_TYPE_FEATURES):
                let itemRange = try range(
                    first: raw.first_item,
                    count: raw.item_count,
                    limit: items.count,
                    label: "OpenType item"
                )
                return .openTypeFeatures(try itemRange.map { index in
                    guard items[index].kind == UInt32(VIEM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE) else {
                        throw EVStyleBridgeError.malformedSnapshot("unexpected OpenType item")
                    }
                    return EVOpenTypeFeature(
                        tag: try text(items[index].string),
                        setting: items[index].unsigned_value
                    )
                })
            case UInt32(VIEM_STYLE_VALUE_LINE_SPACING):
                return .lineSpacing(EVLineSpacing(kind: raw.enum_value, value: raw.number))
            case UInt32(VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT):
                return .paragraphAlignment(raw.enum_value)
            default:
                throw EVStyleBridgeError.malformedSnapshot("unknown value kind \(raw.kind)")
            }
        }

        var decodedDefinitions: [EVStyleDefinition] = []
        decodedDefinitions.reserveCapacity(rawDefinitions.count)
        for raw in rawDefinitions {
            guard let namespace = EVStyleNamespace(rawValue: raw.namespace_id) else {
                throw EVStyleBridgeError.malformedSnapshot("unknown namespace \(raw.namespace_id)")
            }
            let kind: EVStyleKind
            if namespace == .character {
                kind = .character
            } else if raw.role == UInt32(VIEM_STYLE_ROLE_PARAGRAPH) {
                kind = .paragraph
            } else {
                switch raw.role {
                case UInt32(VIEM_STYLE_ROLE_QUOTE): kind = .quote
                case UInt32(VIEM_STYLE_ROLE_CODE_BLOCK): kind = .codeBlock
                case UInt32(VIEM_STYLE_ROLE_LIST): kind = .list
                case UInt32(VIEM_STYLE_ROLE_LIST_ITEM): kind = .listItem
                default: throw EVStyleBridgeError.malformedSnapshot("unknown block role \(raw.role)")
                }
            }
            guard let origin = EVStyleOrigin(rawValue: raw.origin) else {
                throw EVStyleBridgeError.malformedSnapshot("unknown style origin \(raw.origin)")
            }
            let flags = EVStyleDefinitionFlags(rawValue: raw.flags)
            let propertyRange = try range(
                first: raw.first_property,
                count: raw.property_count,
                limit: rawProperties.count,
                label: "property"
            )
            var decodedProperties: [EVStyleProperty: EVResolvedStyleProperty] = [:]
            for propertyIndex in propertyRange {
                let rawProperty = rawProperties[propertyIndex]
                guard let property = EVStyleProperty(rawValue: rawProperty.property) else {
                    throw EVStyleBridgeError.malformedSnapshot("unknown property \(rawProperty.property)")
                }
                let dependencyRange = try range(
                    first: rawProperty.first_dependency,
                    count: rawProperty.dependency_count,
                    limit: dependencies.count,
                    label: "dependency"
                )
                let decodedDependencies = try dependencyRange.map { index -> EVStyleKey in
                    let dependency = dependencies[index]
                    guard let dependencyNamespace = EVStyleNamespace(rawValue: dependency.namespace_id) else {
                        throw EVStyleBridgeError.malformedSnapshot("unknown dependency namespace")
                    }
                    return EVStyleKey(
                        namespace: dependencyNamespace,
                        id: EVStyleID(rawValue: try text(dependency.style_id))
                    )
                }
                let contributor: EVStyleKey?
                if rawProperty.flags & UInt32(VIEM_STYLE_PROPERTY_CONTRIBUTOR_HAS_STYLE) != 0 {
                    guard let contributorNamespace = EVStyleNamespace(rawValue: rawProperty.contributor_namespace) else {
                        throw EVStyleBridgeError.malformedSnapshot("unknown contributor namespace")
                    }
                    contributor = EVStyleKey(
                        namespace: contributorNamespace,
                        id: EVStyleID(rawValue: try text(rawProperty.contributor_style_id))
                    )
                } else {
                    contributor = nil
                }
                let declared = rawProperty.flags & UInt32(VIEM_STYLE_PROPERTY_DECLARED) != 0
                    ? try decodeValue(rawProperty.declared)
                    : nil
                let effective = rawProperty.flags & UInt32(VIEM_STYLE_PROPERTY_EFFECTIVE_PRESENT) != 0
                    ? try decodeValue(rawProperty.effective)
                    : nil
                decodedProperties[property] = EVResolvedStyleProperty(
                    property: property,
                    declared: declared,
                    effective: effective,
                    contributorKind: rawProperty.contributor_kind,
                    contributor: contributor,
                    dependencies: decodedDependencies
                )
            }
            decodedDefinitions.append(EVStyleDefinition(
                key: EVStyleKey(namespace: namespace, id: EVStyleID(rawValue: try text(raw.stable_id))),
                name: try text(raw.display_name),
                kind: kind,
                origin: origin,
                flags: flags,
                capabilities: EVStyleCapabilities(rawValue: raw.capabilities),
                parentID: flags.contains(.hasParent) ? EVStyleID(rawValue: try text(raw.parent_id)) : nil,
                nextStyleID: flags.contains(.hasNextStyle) ? EVStyleID(rawValue: try text(raw.next_style_id)) : nil,
                properties: decodedProperties
            ))
        }
        return EVStyleSheetSnapshot(identity: EVStyleSheetIdentity(info.identity), definitions: decodedDefinitions)
    }

    struct EncodedMutation {
        let key: EVStyleKey
        let expected: EVStyleSheetIdentity
        let operation: UInt32
        let property: UInt32
        let value: EVStyleValue?
        let relationship: String?

        init(key: EVStyleKey, expected: EVStyleSheetIdentity, mutation: EVStyleMutation) {
            self.key = key
            self.expected = expected
            switch mutation {
            case let .setDeclaration(property, value):
                operation = UInt32(VIEM_STYLE_EDIT_SET_DECLARATION)
                self.property = property.rawValue
                self.value = value
                relationship = nil
            case let .clearDeclaration(property):
                operation = UInt32(VIEM_STYLE_EDIT_CLEAR_DECLARATION)
                self.property = property.rawValue
                value = nil
                relationship = nil
            case let .setParent(id):
                operation = UInt32(VIEM_STYLE_EDIT_SET_PARENT)
                property = 0
                value = nil
                relationship = id.rawValue
            case .clearParent:
                operation = UInt32(VIEM_STYLE_EDIT_CLEAR_PARENT)
                property = 0
                value = nil
                relationship = nil
            case let .setNextStyle(id):
                operation = UInt32(VIEM_STYLE_EDIT_SET_NEXT_STYLE)
                property = 0
                value = nil
                relationship = id.rawValue
            case .clearNextStyle:
                operation = UInt32(VIEM_STYLE_EDIT_CLEAR_NEXT_STYLE)
                property = 0
                value = nil
                relationship = nil
            case let .setDisplayName(name):
                operation = UInt32(VIEM_STYLE_EDIT_SET_DISPLAY_NAME)
                property = 0
                value = nil
                relationship = name
            }
        }

        func withRequest<Result>(_ body: (ViemStyleEditV1) throws -> Result) rethrows -> Result {
            var groups = [Array(key.id.rawValue.utf8)]
            var relationshipGroup: Int?
            var itemGroupIndices: [Int] = []
            var itemSpecs: [(kind: UInt32, unsigned: UInt32)] = []
            if let relationship {
                relationshipGroup = groups.count
                groups.append(Array(relationship.utf8))
            }
            if let value {
                switch value {
                case let .string(text):
                    relationshipGroup = groups.count
                    groups.append(Array(text.utf8))
                case let .stringList(values):
                    for text in values {
                        itemGroupIndices.append(groups.count)
                        itemSpecs.append((UInt32(VIEM_STYLE_VALUE_ITEM_STRING), 0))
                        groups.append(Array(text.utf8))
                    }
                case let .openTypeFeatures(values):
                    for feature in values {
                        itemGroupIndices.append(groups.count)
                        itemSpecs.append((UInt32(VIEM_STYLE_VALUE_ITEM_OPEN_TYPE_FEATURE), feature.setting))
                        groups.append(Array(feature.tag.utf8))
                    }
                default:
                    break
                }
            }
            var offsets: [Int] = []
            var arena: [UInt8] = []
            for group in groups {
                offsets.append(arena.count)
                arena.append(contentsOf: group)
            }

            return try arena.withUnsafeBufferPointer { arenaBuffer in
                func slice(for group: Int) -> ViemUtf8Slice {
                    var slice = ViemUtf8Slice()
                    let bytes = groups[group]
                    slice.data = bytes.isEmpty ? nil : arenaBuffer.baseAddress?.advanced(by: offsets[group])
                    slice.length = UInt64(bytes.count)
                    return slice
                }

                var editItems: [ViemStyleEditValueItemV1] = []
                for (index, spec) in itemSpecs.enumerated() {
                    var item = ViemStyleEditValueItemV1()
                    item.struct_size = UInt32(MemoryLayout<ViemStyleEditValueItemV1>.size)
                    item.kind = spec.kind
                    item.text = slice(for: itemGroupIndices[index])
                    item.unsigned_value = spec.unsigned
                    editItems.append(item)
                }
                return try editItems.withUnsafeBufferPointer { itemBuffer in
                    var abiValue = ViemStyleEditValueV1()
                    abiValue.struct_size = UInt32(MemoryLayout<ViemStyleEditValueV1>.size)
                    if let relationshipGroup {
                        abiValue.kind = UInt32(VIEM_STYLE_VALUE_STRING)
                        abiValue.text = slice(for: relationshipGroup)
                    } else if let value {
                        switch value {
                        case let .float(number):
                            abiValue.kind = UInt32(VIEM_STYLE_VALUE_FLOAT)
                            abiValue.number = number
                        case let .percentage(number):
                            abiValue.kind = UInt32(VIEM_STYLE_VALUE_PERCENTAGE)
                            abiValue.enum_value = number
                        case let .unsigned(number):
                            abiValue.kind = UInt32(VIEM_STYLE_VALUE_UNSIGNED)
                            abiValue.enum_value = number
                        case let .boolean(enabled):
                            abiValue.kind = UInt32(VIEM_STYLE_VALUE_BOOLEAN)
                            abiValue.enum_value = enabled ? 1 : 0
                        case let .color(color):
                            abiValue.kind = UInt32(VIEM_STYLE_VALUE_COLOR)
                            abiValue.color.red = color.red
                            abiValue.color.green = color.green
                            abiValue.color.blue = color.blue
                            abiValue.color.alpha = color.alpha
                        case .string:
                            abiValue.kind = UInt32(VIEM_STYLE_VALUE_STRING)
                        case .stringList:
                            abiValue.kind = UInt32(VIEM_STYLE_VALUE_STRING_LIST)
                            abiValue.items = itemBuffer.baseAddress
                            abiValue.item_count = UInt64(itemBuffer.count)
                        case let .fontSlant(slant):
                            abiValue.kind = UInt32(VIEM_STYLE_VALUE_FONT_SLANT)
                            abiValue.enum_value = slant
                        case let .writingDirection(direction):
                            abiValue.kind = UInt32(VIEM_STYLE_VALUE_WRITING_DIRECTION)
                            abiValue.enum_value = direction
                        case .openTypeFeatures:
                            abiValue.kind = UInt32(VIEM_STYLE_VALUE_OPEN_TYPE_FEATURES)
                            abiValue.items = itemBuffer.baseAddress
                            abiValue.item_count = UInt64(itemBuffer.count)
                        case let .lineSpacing(spacing):
                            abiValue.kind = UInt32(VIEM_STYLE_VALUE_LINE_SPACING)
                            abiValue.enum_value = spacing.kind
                            abiValue.number = spacing.value
                        case let .paragraphAlignment(alignment):
                            abiValue.kind = UInt32(VIEM_STYLE_VALUE_PARAGRAPH_ALIGNMENT)
                            abiValue.enum_value = alignment
                        }
                    } else {
                        abiValue.kind = UInt32(VIEM_STYLE_VALUE_NONE)
                    }

                    var request = ViemStyleEditV1()
                    request.struct_size = UInt32(MemoryLayout<ViemStyleEditV1>.size)
                    request.identity = expected.abiValue
                    request.namespace_id = key.namespace.rawValue
                    request.operation = operation
                    request.property = property
                    request.style_id = slice(for: 0)
                    request.value = abiValue
                    return try body(request)
                }
            }
        }
    }
}
