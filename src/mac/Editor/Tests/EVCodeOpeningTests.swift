import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVCodeOpeningTests: XCTestCase {
    private func backend() -> EVCoreDocumentBackend {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-open-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
    }

    func testFilenameAndLoadMarkersSelectCodeBeforeViewsExistAndKeepSourceClean() throws {
        let backend = backend()
        for (filename, text) in [
            ("file.c", "int x;"), ("file.cpp", "int x;"), ("file.rs", "fn x() {}"),
            ("file.swift", "let x = 0"), ("file.m", "@interface X"), ("file.cs", "class X {}"),
            ("file.js", "const x = 0;"), ("file.jsx", "<X/>"), ("file.ts", "const x: number = 0;"),
            ("file.tsx", "<X/>"), ("file.py", "x = 0"), ("script", "#!/usr/bin/env python3\nx = 0"),
            ("notes", "# vim: set filetype=rust:\nfn x() {}"),
        ] {
            let bytes = Data(text.utf8)
            try backend.read(source: bytes, typeName: EVDocument.plainTextType, filename: filename, allowAutomaticCode: true)
            XCTAssertEqual(backend.sourceFormat, .code, filename)
            XCTAssertEqual(try backend.formattedText(), text, filename)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), bytes, filename)
            XCTAssertFalse(backend.persistenceState.isDirty, filename)
        }
        try backend.read(source: Data("ordinary notes".utf8), typeName: EVDocument.plainTextType, filename: "notes.txt", allowAutomaticCode: true)
        XCTAssertEqual(backend.sourceFormat, .plainText)
    }

    func testExplicitFormatsAndExistingRichOpeningDefaultsWinOverAutomaticCodeSelection() throws {
        let backend = backend()
        let text = "# Heading\n<!-- vim: ft=rust -->"
        for (type, automatic, expected) in [
            (EVDocument.plainTextType, false, EVSourceFormat.plainText),
            (EVDocument.markdownType, true, .markdown),
            (EVDocument.htmlType, true, .html),
            (EVDocument.codeType, false, .code),
        ] {
            try backend.read(source: Data(text.utf8), typeName: type, filename: "file.rs", allowAutomaticCode: automatic)
            XCTAssertEqual(backend.sourceFormat, expected)
        }
        try backend.read(source: Data("unclassified".utf8), typeName: EVDocument.codeType)
        XCTAssertEqual(backend.sourceFormat, .code)
        let recovery = try backend.recoverySnapshot()
        try backend.read(source: Data(), typeName: EVDocument.plainTextType)
        try backend.restoreRecovery(recovery)
        XCTAssertEqual(backend.sourceFormat, .code)
        XCTAssertEqual(try backend.formattedText(), "unclassified")
    }

    func testConfiguredFilenameAssociationsApplyBeforeFirstDetectionAndRemainLiteral() throws {
        let backend = backend()
        try backend.configuration.setCodeFilenameAssociations([
            EVCodeFilenameAssociation(pattern: "Buildfile", language: "rust"),
            EVCodeFilenameAssociation(pattern: "*.custom", language: "python"),
        ])
        for filename in ["Buildfile", "example.custom"] {
            let bytes = Data("# literal <b>text</b>".utf8)
            try backend.read(source: bytes, typeName: EVDocument.plainTextType, filename: filename, allowAutomaticCode: true)
            XCTAssertEqual(backend.sourceFormat, .code)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), bytes)
            XCTAssertFalse(backend.persistenceState.isDirty)
        }
        try backend.configuration.setCodeFilenameAssociations([])
        try backend.read(source: Data("plain".utf8), typeName: EVDocument.plainTextType, filename: "example.custom", allowAutomaticCode: true)
        XCTAssertEqual(backend.sourceFormat, .plainText)
    }

    func testDiagnosticsRefreshWithoutAChangedPresentation() async throws {
        let backend = backend()
        try backend.configuration.setVimSyntaxDirectory("/missing/viem-diagnostic-only")
        try backend.read(source: Data("unrecognized language".utf8), typeName: EVDocument.codeType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let language = Array("viem_diagnostic_only".utf8)
        XCTAssertEqual(language.withUnsafeBufferPointer {
            viem_core_set_code_language(backend.core, 2, $0.baseAddress, UInt64($0.count))
        }, UInt32(VIEM_STATUS_OK))
        var diagnostic = ""
        for _ in 0..<200 {
            var changed: UInt8 = 0
            XCTAssertEqual(viem_core_poll_syntax(backend.core, &changed), UInt32(VIEM_STATUS_OK))
            var required: UInt64 = 0
            _ = viem_core_copy_syntax_diagnostics(backend.core, nil, 0, &required)
            if required > 0 {
                var bytes = [UInt8](repeating: 0, count: Int(required))
                XCTAssertEqual(viem_core_copy_syntax_diagnostics(backend.core, &bytes, required, &required), UInt32(VIEM_STATUS_OK))
                diagnostic = String(decoding: bytes, as: UTF8.self)
                if changed == 0 { break }
            }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertFalse(diagnostic.isEmpty)
        var changed: UInt8 = 1
        XCTAssertEqual(viem_core_poll_syntax(backend.core, &changed), UInt32(VIEM_STATUS_OK))
        XCTAssertEqual(changed, 0)
        backend.pollSyntax(now: ProcessInfo.processInfo.systemUptime + 1)
        for line in diagnostic.split(separator: "\n") {
            XCTAssertTrue(EVCodePreferences.shared.loadDiagnostics.contains(String(line)))
        }
        XCTAssertFalse(backend.persistenceState.isDirty)
    }

    func testCodeDisablesDocumentFormattingWhileRetainingLiteralReinterpretationUndo() throws {
        let backend = backend()
        let text = "# Heading\n<b>literal &amp;</b>\n"
        try backend.read(source: Data(text.utf8), typeName: EVDocument.codeType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let before = try backend.recoverySnapshot()
        for command in EVMenuCommand.allCases where (300..<400).contains(command.rawValue) {
            XCTAssertFalse(surface.presentation(for: command).isEnabled, "\(command)")
            surface.perform(menuCommand: command, sender: nil)
        }
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertNil(surface.currentStyleMenuCatalogue())
        XCTAssertFalse(surface.canUndo)
        surface.perform(menuCommand: .reinterpretAsText, sender: nil)
        XCTAssertEqual(backend.sourceFormat, .plainText)
        XCTAssertEqual(try backend.formattedText(), text)
        surface.perform(menuCommand: .undo, sender: nil)
        XCTAssertEqual(backend.sourceFormat, .code)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data(text.utf8))
    }

    func testConvertingCodeKeepsItsLiteralVisibleContentAndUndoRestoresCode() throws {
        let backend = backend()
        let source = "# literal\n<b>tags &amp;</b>"
        for command in [EVMenuCommand.convertToText, .convertToMarkdown, .convertToHTML] {
            try backend.read(source: Data(source.utf8), typeName: EVDocument.codeType)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            surface.perform(menuCommand: command, sender: nil)
            XCTAssertEqual(backend.sourceFormat, command.formatChange?.format)
            XCTAssertEqual(try backend.formattedText(), source)
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(backend.sourceFormat, .code)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), Data(source.utf8))
        }
    }
}
