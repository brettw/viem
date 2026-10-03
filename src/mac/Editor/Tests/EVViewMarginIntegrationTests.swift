import AppKit
import CViemCore
@testable import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVViewMarginIntegrationTests: XCTestCase {
    private func markdownDemo() throws -> Data {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try Data(contentsOf: root.appendingPathComponent("docs/markdown_demo.md"))
    }

    private func saveMarkdownSize(_ size: Float, configuration: EVConfigurationStore) throws {
        var json = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(configuration.styleDefaults(named: "markdown"))) as? [String: Any])
        var blocks = try XCTUnwrap(json["block_styles"] as? [[String: Any]])
        let index = try XCTUnwrap(blocks.firstIndex { $0["id"] as? String == "Paragraph" })
        var properties = blocks[index]["character"] as? [String: Any] ?? [:]
        properties["size"] = size
        blocks[index]["character"] = properties
        json["block_styles"] = blocks
        try configuration.saveStyleDefaults(JSONSerialization.data(withJSONObject: json), named: "markdown")
        try EVStyleTestFixtures.configure(configuration, declarations: [
            (EVStyleTestFixtures.block("Block quote"), .characterSize, nil),
            (EVStyleTestFixtures.block("Code Block"), .characterSize, nil),
        ])
    }

    func testMarkdownDemoReadBeforeNativeViewAttachmentAppliesMargins() throws {
        let source = try markdownDemo()
        let savedDefaults: [String?] = [nil, "valid", "{invalid", #"{"version":99}"#]
        for useDocumentURL in [true, false] {
            for defaults in savedDefaults {
                let directory = FileManager.default.temporaryDirectory
                    .appendingPathComponent("viem-markdown-startup-\(UUID().uuidString)")
                try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
                addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
                let initial = EVConfigurationStore(directory: directory)
                let settings = try XCTUnwrap(initial.selectedThemeURL)
                if defaults == nil || defaults == "valid" { try saveMarkdownSize(21, configuration: initial) }
                else if let defaults { try Data(defaults.utf8).write(to: settings) }
                let saved = try Data(contentsOf: settings)
                let configuration = EVConfigurationStore(directory: directory)
                let backend = EVCoreDocumentBackend(configuration: configuration)
                let document = EVDocument(editorBackend: backend)
                defer { document.close() }
                if useDocumentURL {
                    let url = directory.appendingPathComponent("markdown_demo.md")
                    try source.write(to: url)
                    try document.read(from: url, ofType: EVDocument.plainTextType)
                    XCTAssertEqual(backend.sourceFormat, .markdownSource)
                } else {
                    try backend.read(source: source, typeName: EVDocument.markdownType)
                    XCTAssertEqual(backend.sourceFormat, .markdown)
                }
                let surface = EVEditorSurfaceController(backend: backend,
                    viewPreferences: EVViewPreferences(configuration: configuration))
                surface.loadViewIfNeeded()
                if defaults != nil && defaults != "valid" {
                    let warning = try XCTUnwrap(backend.configurationWarning)
                    XCTAssertTrue(warning.contains(settings.deletingPathExtension().lastPathComponent), warning)
                    XCTAssertEqual(surface.commandOutput, warning)
                    XCTAssertTrue(warning.contains("Default"), warning)
                } else {
                    XCTAssertNil(backend.configurationWarning)
                    XCTAssertNil(surface.commandOutput)
                }
                let session = try XCTUnwrap(surface.session)
                let styles = try backend.styleSheetSnapshot()
                let quote = try XCTUnwrap(styles.definition(for: EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Block quote"))))
                let code = try XCTUnwrap(styles.definition(for: EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Code Block"))))
                XCTAssertEqual(quote.kind, .quote)
                XCTAssertEqual(code.kind, .codeBlock)
                if defaults == nil || defaults == "valid" {
                    XCTAssertEqual(styles.definition(for: .baseParagraph)?.properties[.characterSize]?.effective, .float(21))
                    XCTAssertEqual(quote.properties[.characterSize]?.effective, .float(21))
                    XCTAssertEqual(code.properties[.characterSize]?.effective, .float(21))
                } else {
                    // Failure must use the complete built-in stylesheet. Its
                    // particular sizes belong to the preset, not this test.
                    let fallbackDirectory = directory.appendingPathComponent("fallback")
                    let fallback = EVConfigurationStore(directory: fallbackDirectory)
                    try fallback.selectTheme(named: nil)
                    let fallbackBackend = EVCoreDocumentBackend(configuration: fallback)
                    try fallbackBackend.read(source: source, typeName: backend.sourceFormat == .markdownSource
                        ? EVDocument.markdownSourceType : EVDocument.markdownType)
                    XCTAssertEqual(styles.definitions, try fallbackBackend.styleSheetSnapshot().definitions)
                }
                XCTAssertNil(quote.nextStyleID)
                XCTAssertNil(code.nextStyleID)
                surface.view.frame = NSRect(x: 0, y: 0, width: 920, height: 655)
                surface.viewDidLayout()
                XCTAssertNotNil(surface.layoutSnapshot)
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
                XCTAssertFalse(backend.persistenceState.isDirty)
                surface.performInput { _ = try session.sendText("iX") }
                XCTAssertNil(surface.commandOutput)
                XCTAssertNotEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
                surface.performInput { _ = try session.undo() }
                XCTAssertNil(surface.commandOutput)
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
                XCTAssertEqual(try Data(contentsOf: settings), saved)
            }
        }
    }

    func testReplacingAnAttachedDocumentAppliesCurrentThemeStylesAndKeepsSource() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-replacement-style-warning-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        XCTAssertNil(surface.commandOutput)
        let settings = try XCTUnwrap(configuration.selectedThemeURL)
        try saveMarkdownSize(21, configuration: configuration)
        let defaults = try Data(contentsOf: settings)
        let source = try markdownDemo()
        try backend.read(source: source, typeName: EVDocument.markdownSourceType,
            filename: "markdown_demo.md", allowAutomaticCode: false)
        XCTAssertNil(backend.configurationWarning)
        XCTAssertNil(surface.commandOutput)
        XCTAssertNotNil(surface.session)
        XCTAssertNotNil(surface.layoutSnapshot)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.effective, .float(21))
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
        XCTAssertEqual(try Data(contentsOf: settings), defaults)
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    func testUntitledStartupAppliesViewMarginsBeforeTheFirstPresentationAndAcceptsTyping() throws {
        for margins in [EVViewMargins(), EVViewMargins(top: 23, left: 31, bottom: 17, right: 29)] {
            let directory = FileManager.default.temporaryDirectory
                .appendingPathComponent("viem-untitled-view-margins-\(UUID().uuidString)")
            addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
            let configuration = EVConfigurationStore(directory: directory)
            try configuration.setViewMargins(margins)
            let preferences = EVViewPreferences(configuration: configuration)
            // Untitled startup uses the initial empty core, without reading a
            // source file or priming layout before the native view attaches.
            let backend = EVCoreDocumentBackend(configuration: configuration)
            XCTAssertEqual(try backend.formattedText(), "")
            let surface = EVEditorSurfaceController(backend: backend, viewPreferences: preferences)
            surface.loadViewIfNeeded()
            XCTAssertNil(surface.commandOutput, "Untitled startup: \(surface.commandOutput ?? "")")
            let session = try XCTUnwrap(surface.session, "View-margin setup must complete before installing the initial session")
            surface.view.frame = NSRect(x: 0, y: 0, width: 920, height: 655)
            surface.viewDidLayout()
            let snapshot = try XCTUnwrap(surface.layoutSnapshot)
            XCTAssertEqual(snapshot.info.content_insets.top, Float(margins.top))
            XCTAssertEqual(snapshot.info.content_insets.left, Float(margins.left))
            XCTAssertEqual(snapshot.info.content_insets.bottom, Float(margins.bottom))
            XCTAssertEqual(snapshot.info.content_insets.right, Float(margins.right))
            XCTAssertEqual(snapshot.rows.count, 1)
            XCTAssertFalse(backend.persistenceState.isDirty)
            _ = try session.sendText("iFirst words")
            surface.refreshPresentation()
            XCTAssertEqual(try backend.formattedText(), "First words")
            XCTAssertNil(surface.commandOutput)
        }
    }

    func testHiddenLegacyHorizontalScrollbarDoesNotClipTheBottomTextBand() throws {
        let (_, surface, _) = try fixture(style: .legacy)
        let view = surface.editorView
        let session = try XCTUnwrap(surface.session)
        let height = view.bounds.height
        let scrollbarWidth = NSScroller.scrollerWidth(for: .regular, scrollerStyle: .legacy)
        XCTAssertEqual(view.layoutViewportSize.height, height)
        XCTAssertEqual(try XCTUnwrap(surface.layoutSnapshot).info.viewport_height, Float(height))
        XCTAssertFalse(view.documentScrollbars.horizontalAvailable)

        // Position an ordinary row in the strip that used to be reserved for
        // a hidden horizontal scrollbar, while more document remains below.
        surface.requestVerticalViewport(top: 300)
        let before = try XCTUnwrap(surface.layoutSnapshot)
        let target = try XCTUnwrap(before.rows.last { $0.baseline > Float(height) })
        surface.requestVerticalViewport(top: CGFloat(target.baseline) - height + 4)
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let row = try XCTUnwrap(snapshot.rows.first { $0.text_start == target.text_start })
        XCTAssertEqual(CGFloat(row.baseline) - view.viewportOrigin.y, height - 4, accuracy: 0.1)
        XCTAssertGreaterThan(snapshot.info.total_height, Float(view.viewportOrigin.y + height))
        let point = CGPoint(x: CGFloat(snapshot.info.content_insets.left) + 2, y: height - 8)
        let hit = try session.hitTest(view.layoutPoint(fromViewPoint: point), in: snapshot.info)
        XCTAssertGreaterThanOrEqual(hit.text_offset, row.text_start)
        XCTAssertLessThanOrEqual(hit.text_offset, row.text_end)

        let width = Int(view.bounds.width)
        let bitmap = try XCTUnwrap(CGContext(data: nil, width: width, height: Int(height),
            bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        bitmap.translateBy(x: 0, y: height)
        bitmap.scaleBy(x: 1, y: -1)
        bitmap.clip(to: CGRect(x: 0, y: height - scrollbarWidth, width: CGFloat(width), height: scrollbarWidth))
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: bitmap, flipped: true)
        view.draw(view.bounds)
        NSGraphicsContext.restoreGraphicsState()
        let image = NSBitmapImageRep(cgImage: try XCTUnwrap(bitmap.makeImage()))
        var inkPixels = 0
        for y in 0..<image.pixelsHigh {
            let background = try XCTUnwrap(image.colorAt(x: width - 50, y: y)?.usingColorSpace(.sRGB))
            guard background.alphaComponent > 0.9 else { continue }
            for x in 20..<200 {
                let color = try XCTUnwrap(image.colorAt(x: x, y: y)?.usingColorSpace(.sRGB))
                if abs(color.redComponent - background.redComponent)
                    + abs(color.greenComponent - background.greenComponent)
                    + abs(color.blueComponent - background.blueComponent) > 0.2 { inkPixels += 1 }
            }
        }
        XCTAssertGreaterThan(inkPixels, 5, "Text must paint through the former scrollbar strip to the status line")
    }

    func testTypingAndNewlinesKeepTheWholeRowAboveTheBottomMarginInBothScrollbarStyles() throws {
        for style in [NSScroller.Style.legacy, .overlay] {
            let (backend, surface, preferences) = try fixture(style: style)
            let session = try XCTUnwrap(surface.session)
            _ = try session.sendText("GA")
            for _ in 0..<3 {
                _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
                _ = try session.sendText("more text")
                surface.refreshPresentation()
                let snapshot = try XCTUnwrap(surface.layoutSnapshot)
                let offset = surface.viewPresentation.cursor_utf8_offset
                let ranges = snapshot.rows.map { ($0.text_start, $0.text_end) }
                let row = try XCTUnwrap(snapshot.rows.last { $0.text_start <= offset && offset <= $0.text_end },
                    "Caret \(offset) must be covered by rows \(ranges) at viewport \(surface.viewportState.top)")
                let bottom = max(row.y + row.ascent + row.descent,
                    snapshot.clusters.filter { $0.row_index == row.row_index }
                        .map { $0.ink_bounds.y + $0.ink_bounds.height }.max() ?? 0)
                XCTAssertLessThanOrEqual(bottom - surface.viewportState.top,
                    Float(surface.editorView.bounds.height - CGFloat(preferences.margins.bottom)) + 0.1)
                XCTAssertGreaterThanOrEqual(snapshot.info.total_height - bottom, Float(preferences.margins.bottom) - 0.1)
            }
            XCTAssertTrue(try backend.formattedText().hasSuffix("more text\nmore text\nmore text"))
        }
    }

    func testTypingOnAnAlreadyVisibleRowNeverMovesTheDisplayedRows() throws {
        for type in [EVDocument.plainTextType, EVDocument.codeType] {
            for atLineEnd in [false, true] {
                let (backend, surface, _) = try fixture(style: .legacy, typeName: type)
                if type == EVDocument.codeType {
                    let styles = try EVCodeStyleSession(configuration: backend.configuration)
                    try styles.edit(key: .baseParagraph, expected: styles.snapshot().identity,
                        mutation: .setDeclaration(.characterSize, .float(32)))
                    surface.refreshPresentation()
                }
                let view = surface.editorView
                let noReplacement = NSRange(location: NSNotFound, length: 0)
                view.insertText("40G", replacementRange: noReplacement)
                view.insertText(atLineEnd ? "A" : "lli", replacementRange: noReplacement)
                let rowBeforeScroll = try activeRow(surface)
                surface.requestVerticalViewport(top: CGFloat(rowBeforeScroll.baseline) - 90)
                let before = try activeRow(surface)
                let viewport = surface.viewportState.top
                let baseline = before.baseline - viewport
                let displayedRows = try XCTUnwrap(surface.layoutSnapshot).rows.filter {
                    $0.y >= viewport && $0.y + $0.ascent + $0.descent <= viewport + 160
                }.map { ($0.hard_line_index, $0.baseline - viewport) }
                XCTAssertEqual(baseline, 90, accuracy: 0.1)
                XCTAssertGreaterThan(before.y - viewport, 28)
                XCTAssertLessThan(before.y + before.ascent + before.descent - viewport, 132)
                for character in "xyz" {
                    view.insertText(String(character), replacementRange: noReplacement)
                    let after = try activeRow(surface)
                    if abs(after.baseline - before.baseline) < 0.01 {
                        XCTAssertEqual(surface.viewportState.top, viewport, accuracy: 0.1,
                            "Unchanged document coordinates must retain the numeric viewport origin")
                    }
                    XCTAssertEqual(after.baseline - surface.viewportState.top, baseline, accuracy: 0.1,
                        "\(type), lineEnd=\(atLineEnd): the same row must stay at its original screen position")
                    let rows = try XCTUnwrap(surface.layoutSnapshot).rows
                    for (line, screenBaseline) in displayedRows {
                        let row = try XCTUnwrap(rows.first { $0.hard_line_index == line })
                        XCTAssertEqual(row.baseline - surface.viewportState.top, screenBaseline, accuracy: 0.1,
                            "Typing must also leave the other displayed rows stationary")
                    }
                }
            }
        }
    }

    private func activeRow(_ surface: EVEditorSurfaceController) throws -> ViemVisualRowV1 {
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let offset = surface.viewPresentation.cursor_utf8_offset
        return try XCTUnwrap(snapshot.rows.last { $0.text_start <= offset && offset <= $0.text_end })
    }

    private func fixture(style: NSScroller.Style, typeName: String? = nil) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVViewPreferences) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-bottom-viewport-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory)
        let preferences = EVViewPreferences(configuration: configuration)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(Array(repeating: "HHHHMMMM", count: 100).joined(separator: "\n").utf8),
            typeName: typeName ?? EVDocument.plainTextType)
        let surface = EVEditorSurfaceController(backend: backend, viewPreferences: preferences)
        surface.view = EVEditorView(surface: surface, scrollbarStyleProvider: { style })
        surface.editorView.editingPreferences = EVEditingPreferences(configuration: configuration)
        surface.view.frame = NSRect(x: 0, y: 0, width: 420, height: 160)
        surface.viewDidLayout()
        return (backend, surface, preferences)
    }

    func testViewMarginNotificationUpdatesOpenViewsWithoutChangingTheirDocuments() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-view-margins-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory)
        let preferences = EVViewPreferences(configuration: configuration)
        let original = preferences.margins
        var backends: [EVCoreDocumentBackend] = []
        var surfaces: [EVEditorSurfaceController] = []
        for _ in 0..<2 {
            let backend = EVCoreDocumentBackend(configuration: configuration)
            try backend.read(source: Data("A paragraph with words to wrap in this view.".utf8), typeName: EVDocument.markdownType)
            let surface = EVEditorSurfaceController(backend: backend, viewPreferences: preferences)
            surface.loadViewIfNeeded()
            surface.view.frame = NSRect(x: 0, y: 0, width: 500, height: 300)
            surface.viewDidLayout()
            backends.append(backend)
            surfaces.append(surface)
        }
        let before = try backends.map { try $0.recoverySnapshot() }
        let margins = EVViewMargins(top: original.top == 41 ? 42 : 41, left: 51, bottom: 31, right: 61)
        XCTAssertTrue(preferences.setMargins(margins))
        for (index, surface) in surfaces.enumerated() {
            let insets = try XCTUnwrap(surface.layoutSnapshot).info.content_insets
            XCTAssertEqual(insets.top, Float(margins.top))
            XCTAssertEqual(insets.left, Float(margins.left))
            XCTAssertEqual(insets.bottom, Float(margins.bottom))
            XCTAssertEqual(insets.right, Float(margins.right))
            XCTAssertEqual(try backends[index].recoverySnapshot(), before[index])
            XCTAssertFalse(surface.canUndo)
            XCTAssertFalse(backends[index].persistenceState.isDirty)
        }
    }
}
