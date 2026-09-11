import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVDocumentFormatMenuTests: XCTestCase {
    private let markdown = "# Heading\n\n**bold**"
    private let html = "<h1>Heading</h1><p><strong>bold</strong></p>"
    private let commands: [EVMenuCommand] = [
        .convertToText, .convertToMarkdown, .convertToHTML,
        .reinterpretAsText, .reinterpretAsCode, .reinterpretAsMarkdown, .reinterpretAsHTML,
    ]

    func testCurrentFormatFamilyIsDisabledInBothMenusIncludingSourceViews() throws {
        for format in EVSourceFormat.allCases {
            let source = format == .rtf ? "{\\rtf1 Heading}" : "Heading"
            let (backend, surface) = try makeSurface(source, format: format)
            let before = try backend.documentState()
            let bytes = try serializedSource(backend)
            for command in commands {
                let change = try XCTUnwrap(command.formatChange)
                let enabled = format == .code || change.format == .code ? format != change.format : !format.hasSameSerialization(as: change.format)
                XCTAssertEqual(surface.presentation(for: command).isEnabled, enabled, "\(format): \(command)")
                if !enabled {
                    surface.perform(menuCommand: command, sender: nil)
                    XCTAssertEqual(backend.sourceFormat, format)
                    XCTAssertEqual(try backend.documentState().document_revision, before.document_revision)
                    XCTAssertEqual(try serializedSource(backend), bytes)
                    XCTAssertFalse(surface.canUndo)
                }
            }
        }
    }

    func testFileFormatActionsRemainAvailableForReadOnlyBuffers() throws {
        let (backend, surface) = try makeSurface(markdown, format: .plainText)
        try backend.setReadOnly(true)
        for command in [EVMenuCommand.convertToMarkdown, .reinterpretAsMarkdown] {
            XCTAssertTrue(surface.presentation(for: command).isEnabled)
            surface.perform(menuCommand: command, sender: nil)
            XCTAssertEqual(backend.sourceFormat, .markdown)
            XCTAssertTrue(backend.persistenceState.isReadOnly)
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(backend.sourceFormat, .plainText)
        }
    }

    func testAllSixFileActionsUseExplicitSemanticsAndUndoRedoSourceAndFormatTogether() throws {
        let fixtures: [(EVMenuCommand, EVSourceFormat, String, String)] = [
            (.convertToText, .html, html, "Heading\n\nbold"),
            (.convertToMarkdown, .html, html, "Heading\nbold"),
            (.convertToHTML, .markdown, markdown, "Heading\nbold"),
            (.reinterpretAsText, .markdown, markdown, markdown),
            (.reinterpretAsMarkdown, .plainText, markdown, "Heading\nbold"),
            (.reinterpretAsHTML, .plainText, html, "Heading\nbold"),
            (.reinterpretAsCode, .markdown, markdown, markdown),
        ]
        for (command, originalFormat, source, expectedText) in fixtures {
            let (backend, surface) = try makeSurface(source, format: originalFormat)
            let originalText = surface.formattedText
            let change = try XCTUnwrap(command.formatChange)
            XCTAssertTrue(surface.presentation(for: command).isEnabled)

            surface.perform(menuCommand: command, sender: nil)

            XCTAssertEqual(backend.sourceFormat, change.format, "\(command)")
            XCTAssertEqual(surface.formattedText, expectedText, "\(command)")
            XCTAssertTrue(surface.canUndo)
            XCTAssertFalse(surface.presentation(for: command).isEnabled)
            let changedSource = try serializedSource(backend)
            if change.operation == .reinterpret {
                XCTAssertEqual(changedSource, Data(source.utf8))
            } else {
                XCTAssertNotEqual(changedSource, Data(source.utf8))
            }

            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(backend.sourceFormat, originalFormat)
            XCTAssertEqual(surface.formattedText, originalText)
            XCTAssertEqual(try serializedSource(backend), Data(source.utf8))
            XCTAssertFalse(surface.canUndo, "One format action must be one undo unit")
            XCTAssertTrue(surface.canRedo)

            surface.perform(menuCommand: .redo, sender: nil)
            XCTAssertEqual(backend.sourceFormat, change.format)
            XCTAssertEqual(surface.formattedText, expectedText)
            XCTAssertEqual(try serializedSource(backend), changedSource)
        }
    }

    func testConversionFromSourceViewsUsesRichContentAndSelectsWYSIWYGTarget() throws {
        for (format, source, command, expectedFormat) in [
            (EVSourceFormat.markdownSource, markdown, EVMenuCommand.convertToHTML, EVSourceFormat.html),
            (.htmlSource, html, .convertToMarkdown, .markdown),
        ] {
            let (backend, surface) = try makeSurface(source, format: format)
            surface.perform(menuCommand: command, sender: nil)
            XCTAssertEqual(backend.sourceFormat, expectedFormat)
            XCTAssertEqual(surface.formattedText, "Heading\nbold")
            XCTAssertEqual(try XCTUnwrap(surface.session).selectedNamedStyles().paragraph?.rawValue, "Heading1")
        }
    }

    private func makeSurface(_ source: String, format: EVSourceFormat) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: typeName(format))
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 520, height: 260)
        surface.viewDidLayout()
        return (backend, surface)
    }

    private func serializedSource(_ backend: EVCoreDocumentBackend) throws -> Data {
        try backend.serializedSource(typeName: typeName(backend.sourceFormat))
    }

    private func typeName(_ format: EVSourceFormat) -> String {
        switch format {
        case .plainText: EVDocument.plainTextType
        case .markdown: EVDocument.markdownType
        case .markdownSource: EVDocument.markdownSourceType
        case .html: EVDocument.htmlType
        case .htmlSource: EVDocument.htmlSourceType
        case .rtf: EVDocument.rtfType
        case .code: EVDocument.codeType
        }
    }
}
