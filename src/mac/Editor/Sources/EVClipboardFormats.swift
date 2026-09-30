import AppKit
import CViemCore
import CoreText
import ViemCoreTextProvider
import ViemAppShell

/// The portable fragment is authoritative for Viem paste. RTF is a native
/// interchange representation generated from the core's resolved styles.
struct EVClipboardFragment: Decodable {
    let schemaVersion: UInt32
    let plainText: String
    let hardBreaks: [Int]
    let isRich: Bool
    let sourceText: String
    let characterRuns: [CharacterRun]
    let paragraphRuns: [ParagraphRun]

    struct Color: Decodable {
        let red, green, blue, alpha: Double
        var native: NSColor {
            NSColor(srgbRed: red, green: green, blue: blue, alpha: alpha)
        }
    }

    struct CharacterRun: Decodable {
        let start, end: Int
        let fontFamilies: [String]
        let size, weight, baseWeight: Double
        let bold: Bool
        let slant: String
        let foreground: Color
        let foregroundIsDefault: Bool
        let background: Color?
        let underline, strikethrough: Bool
        let language: String?
        let direction: String
        let openTypeFeatures: [String: UInt32]
        let letterSpacing: Double
    }

    struct ParagraphRun: Decodable {
        let start, end: Int
        let marginTop, marginBottom: Double
        let lineSpacing: LineSpacing
        let firstLineIndent, leadingIndent, trailingIndent: Double
        let alignment, baseDirection, resolvedDirection: String
    }

    enum LineSpacing: Decodable {
        case normal, multiplier(Double), atLeast(Double), exact(Double)
        init(from decoder: any Decoder) throws {
            let value = try decoder.singleValueContainer()
            if (try? value.decode(String.self)) == "Normal" { self = .normal; return }
            let fields = try value.decode([String: Double].self)
            if fields.count == 1, let amount = fields["Multiplier"] { self = .multiplier(amount) }
            else if fields.count == 1, let amount = fields["AtLeast"] { self = .atLeast(amount) }
            else if fields.count == 1, let amount = fields["Exact"] { self = .exact(amount) }
            else { throw EVCoreFrontendError.invalidHostEffect }
        }
    }

    static func decode(_ data: Data) throws -> Self {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let value = try decoder.decode(Self.self, from: data)
        guard value.schemaVersion == 1 else { throw EVCoreFrontendError.invalidHostEffect }
        return value
    }

    private func range(start: Int, end: Int) throws -> NSRange {
        guard start >= 0, end >= start,
              let lower = plainText.utf8.index(plainText.utf8.startIndex, offsetBy: start, limitedBy: plainText.utf8.endIndex),
              let upper = plainText.utf8.index(lower, offsetBy: end - start, limitedBy: plainText.utf8.endIndex),
              let first = String.Index(lower, within: plainText),
              let last = String.Index(upper, within: plainText)
        else { throw EVCoreFrontendError.invalidHostEffect }
        return NSRange(first..<last, in: plainText)
    }

    @MainActor func attributedText() throws -> NSAttributedString {
        let result = NSMutableAttributedString(string: plainText)
        // AppKit serializes LF as a paragraph break. Preserve a hard line
        // inside a paragraph as a Unicode line separator in RTF only; the
        // plain-text and private representations retain normalized LF.
        var paragraphIndex = 0
        for offset in hardBreaks {
            while paragraphIndex < paragraphRuns.count && paragraphRuns[paragraphIndex].end <= offset {
                paragraphIndex += 1
            }
            guard paragraphIndex < paragraphRuns.count else { break }
            if paragraphRuns[paragraphIndex].start <= offset {
                result.replaceCharacters(in: try range(start: offset, end: offset + 1), with: "\u{2028}")
            }
        }
        for run in paragraphRuns {
            let paragraph = NSMutableParagraphStyle()
            paragraph.paragraphSpacingBefore = run.marginTop
            paragraph.paragraphSpacing = run.marginBottom
            paragraph.firstLineHeadIndent = run.firstLineIndent
            paragraph.headIndent = run.leadingIndent
            paragraph.tailIndent = -run.trailingIndent
            // A partial selection retains the original paragraph's direction,
            // even if its first selected strong character points the other way.
            switch run.resolvedDirection {
            case "LeftToRight": paragraph.baseWritingDirection = .leftToRight
            case "RightToLeft": paragraph.baseWritingDirection = .rightToLeft
            default: paragraph.baseWritingDirection = .natural
            }
            switch run.alignment {
            case "Center": paragraph.alignment = .center
            case "End": paragraph.alignment = run.resolvedDirection == "RightToLeft" ? .left : .right
            default: paragraph.alignment = run.resolvedDirection == "RightToLeft" ? .right : .left
            }
            switch run.lineSpacing {
            case .normal: break
            case .multiplier(let amount): paragraph.lineHeightMultiple = amount
            case .atLeast(let amount): paragraph.minimumLineHeight = amount
            case .exact(let amount): paragraph.minimumLineHeight = amount; paragraph.maximumLineHeight = amount
            }
            result.addAttribute(.paragraphStyle, value: paragraph, range: try range(start: run.start, end: run.end))
        }
        for run in characterRuns {
            guard run.size.isFinite, run.size > 0, run.weight.isFinite else { throw EVCoreFrontendError.invalidHostEffect }
            let slant: UInt32 = run.slant == "Italic" ? UInt32(VIEM_FONT_SLANT_ITALIC)
                : run.slant == "Oblique" ? UInt32(VIEM_FONT_SLANT_OBLIQUE) : UInt32(VIEM_FONT_SLANT_UPRIGHT)
            let font = resolveFont(families: run.fontFamilies, size: run.size, cssWeight: run.weight,
                slant: slant, features: run.openTypeFeatures.sorted { $0.key < $1.key }.map { ($0.key, $0.value) }, relativeBold: run.bold)
            var attributes: [NSAttributedString.Key: Any] = [
                .font: font as NSFont,
                .foregroundColor: run.foregroundIsDefault ? EVThemeStore.shared.theme.foreground.color : run.foreground.native,
            ]
            attributes.merge(letterSpacingAttributes(CGFloat(run.letterSpacing))) { _, value in value }
            // AppKit's RTF writer drops tracking. Nonzero kern preserves its
            // spacing in RTF with kerning enabled; tracking takes precedence
            // when Core Text measures this attributed representation.
            if run.letterSpacing != 0 { attributes[.kern] = run.letterSpacing }
            if run.weight >= 500 && Double(EVFontCatalog.weight(of: font)) < run.weight { attributes[.strokeWidth] = -3.0 }
            if let background = run.background { attributes[.backgroundColor] = background.native }
            if run.underline { attributes[.underlineStyle] = NSUnderlineStyle.single.rawValue }
            if run.strikethrough { attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
            if let language = run.language { attributes[NSAttributedString.Key(kCTLanguageAttributeName as String)] = language }
            if run.direction != "Natural" {
                let direction: NSWritingDirection = run.direction == "RightToLeft" ? .rightToLeft : .leftToRight
                attributes[.writingDirection] = [direction.rawValue | NSWritingDirectionFormatType.override.rawValue]
            }
            result.addAttributes(attributes, range: try range(start: run.start, end: run.end))
        }
        return result
    }

    @MainActor func representations(json: Data) throws -> EVClipboardRepresentations {
        guard isRich else { return EVClipboardRepresentations(plainText: sourceText) }
        let attributed = try attributedText()
        let rtf = try attributed.data(from: NSRange(location: 0, length: attributed.length),
            documentAttributes: [.documentType: NSAttributedString.DocumentType.rtf])
        return EVClipboardRepresentations(plainText: plainText, richText: rtf, fragment: json)
    }
}

func readClipboardJSON(
    operation: String,
    _ read: (UnsafeMutablePointer<UInt8>?, UInt64, UnsafeMutablePointer<UInt64>) -> UInt32
) throws -> Data? {
    var required: UInt64 = 0
    let measured = read(nil, 0, &required)
    guard measured == UInt32(VIEM_STATUS_OK) || measured == UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) else {
        throw EVCoreFrontendError.core(operation: operation, status: measured)
    }
    guard required <= UInt64(Int.max) else { throw EVCoreFrontendError.invalidHostEffect }
    if required == 0 { return nil }
    var data = Data(count: Int(required))
    var written: UInt64 = 0
    let status = data.withUnsafeMutableBytes { bytes in
        read(bytes.bindMemory(to: UInt8.self).baseAddress, UInt64(bytes.count), &written)
    }
    guard status == UInt32(VIEM_STATUS_OK) else {
        throw EVCoreFrontendError.core(operation: operation, status: status)
    }
    guard written == required else { throw EVCoreFrontendError.invalidHostEffect }
    return data
}

extension EVCoreDocumentBackend {
    func clipboardFragment(in range: Range<Int>, snapshot: EVFormattedSnapshot) throws -> EVClipboardFragment {
        try EVClipboardFragment.decode(clipboardFragmentJSON(in: range, snapshot: snapshot))
    }

    func clipboardFragmentJSON(in range: Range<Int>, snapshot: EVFormattedSnapshot) throws -> Data {
        guard range.lowerBound >= 0, range.upperBound >= range.lowerBound else {
            throw EVCoreFrontendError.invalidHostEffect
        }
        var request = ViemFormattedUtf8RangeV1()
        request.struct_size = UInt32(MemoryLayout<ViemFormattedUtf8RangeV1>.size)
        request.identity = snapshot.info.identity
        request.utf8_start = UInt64(range.lowerBound)
        request.utf8_end = UInt64(range.upperBound)
        guard let json = try readClipboardJSON(operation: "Copy selection source", { bytes, capacity, required in
            viem_core_copy_clipboard_json(core, &request, bytes, capacity, required)
        }) else { throw EVCoreFrontendError.invalidHostEffect }
        return json
    }
}
