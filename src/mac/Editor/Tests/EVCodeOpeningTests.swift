import AppKit
import CViemCore
import UniformTypeIdentifiers
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVCodeOpeningTests: XCTestCase {
    private func backend(bundleResourceURL: URL? = Bundle.main.resourceURL) -> EVCoreDocumentBackend {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-open-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory,
            legacyDefaults: nil, bundleResourceURL: bundleResourceURL))
    }

    private func assertRustKeywordIsHighlighted(
        backend: EVCoreDocumentBackend,
        surface: EVEditorSurfaceController,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws {
        // Exercise the same worker publication and paint refresh as the timer.
        // The fixture starts with `fn`; a Code format flag alone is insufficient.
        for _ in 0..<200 {
            backend.pollSyntax()
            if surface.layoutPaint?.runs.contains(where: {
                $0.text_start == 0 && $0.text_end >= 2
                    && $0.paint.flags & UInt32(VIEM_TEXT_PAINT_DEFAULT_FOREGROUND) == 0
            }) == true { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Rust's fn keyword did not acquire syntax paint: \(EVCodePreferences.shared.loadDiagnostics)",
                file: file, line: line)
    }

    func testProtocolReadPreservesFilenameForAutomaticCodeAndRustPaint() async throws {
        let concrete = backend()
        let backend: any EVDocumentBackend = concrete
        let source = Data("fn main() {\n    let answer = 42;\n}\n".utf8)
        try backend.read(source: source, typeName: EVDocument.plainTextType,
                         origin: .file(path: "main.rs", automaticCode: true))
        XCTAssertEqual(backend.sourceFormat, .code)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        try await assertRustKeywordIsHighlighted(backend: concrete, surface: surface)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), source)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(surface.canUndo)
    }

    func testDocumentURLReadDetectsRustAndPublishesSyntaxPaint() async throws {
        let backend = backend()
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-rust-document-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("main.rs")
        let source = Data("fn main() {\n    let answer = 42;\n}\n".utf8)
        try source.write(to: url)
        let document = EVDocument(editorBackend: backend)
        defer { document.close() }
        try document.read(from: url, ofType: EVDocument.plainTextType)
        XCTAssertEqual(backend.sourceFormat, .code)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        try await assertRustKeywordIsHighlighted(backend: backend, surface: surface)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), source)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(document.isDocumentEdited)
        XCTAssertNil(document.recoveryFailure)
    }

    func testMarkdownDefaultsToPreviewAndHTMLOpensAsCode() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-source-default-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }

        XCTAssertEqual(
            EVDocument.defaultOpeningType(
                for: directory.appendingPathComponent("typed-markdown"),
                nativeType: EVDocument.markdownType),
            EVDocument.markdownType)
        XCTAssertEqual(
            EVDocument.defaultOpeningType(
                for: directory.appendingPathComponent("typed-html"),
                nativeType: EVDocument.htmlType),
            EVDocument.codeType)

        for (filename, nativeType, expected, source) in [
            ("notes.md", EVDocument.markdownType, EVSourceFormat.markdown,
             Data("# Heading\n\n**body**\n".utf8)),
            ("page.html", EVDocument.htmlType, EVSourceFormat.code,
             Data("<h1>Heading</h1>\n<p>body</p>\n".utf8)),
            ("page.HTM", "public.data", EVSourceFormat.code,
             Data("<p>HTML source &amp; text</p>\r\n".utf8)),
            ("page.xhtml", "public.xml", EVSourceFormat.code,
             Data("<html xmlns='http://www.w3.org/1999/xhtml'><p>Text</p></html>".utf8)),
        ] {
            let backend = backend()
            let url = directory.appendingPathComponent(filename)
            try source.write(to: url)
            let document = EVDocument(editorBackend: backend)
            document.recordRecentDocument = { _ in }
            defer { document.close() }

            try document.read(from: url, ofType: nativeType)

            XCTAssertEqual(backend.sourceFormat, expected, filename)
            XCTAssertEqual(try document.data(ofType: EVDocument.typeName(for: expected)), source, filename)
            XCTAssertFalse(backend.persistenceState.isDirty, filename)
        }
    }

    func testNativeHTMLDataAndLegacyRecoveryOpenAsCode() throws {
        let backend = backend()
        let document = EVDocument(editorBackend: backend)
        defer { document.close() }
        let source = Data("<h1>Literal</h1>\r\n".utf8)
        for type in [EVDocument.htmlType] {
            try document.read(from: source, ofType: type)
            document.fileType = type
            XCTAssertEqual(backend.sourceFormat, .code)
            XCTAssertEqual(document.fileType, EVDocument.plainTextType)
            XCTAssertEqual(document.writableTypes(for: .saveAsOperation), [EVDocument.plainTextType])
            XCTAssertEqual(try document.data(ofType: EVDocument.plainTextType), source)
            XCTAssertFalse(backend.persistenceState.isDirty)
        }
        for legacy in ["html", "htmlSource"] {
            let format = try JSONDecoder().decode(EVSourceFormat.self, from: Data("\"\(legacy)\"".utf8))
            try backend.restoreRecovery(EVRecoverySnapshot(source: source, format: format,
                encoding: UInt32(VIEM_ENCODING_UTF8), fileFormat: UInt32(VIEM_FILE_FORMAT_DOS),
                documentID: 1, documentRevision: 1))
            XCTAssertEqual(backend.sourceFormat, .code)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), source)
            XCTAssertTrue(backend.persistenceState.isRecovered)
        }
    }

    func testUnknownDocumentURLTypesOpenAsTextWithoutChangingSourceBytes() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-unknown-document-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let dynamicType = try XCTUnwrap(UTType(filenameExtension: "viem-unknown-\(UUID().uuidString)")).identifier
        XCTAssertTrue(dynamicType.hasPrefix("dyn."))
        let fixtures: [(String, String, Data)] = [
            (".unknownrc", "public.data", Data("set number\r\nset expandtab\r\n".utf8)),
            ("notes", "public.data", Data("extensionless text\n".utf8)),
            ("notes.viem-unknown", dynamicType, Data("unknown extension\r\n".utf8)),
            ("image.png", "public.png", Data([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0xff])),
        ]
        for (filename, typeName, source) in fixtures {
            let backend = backend()
            let url = directory.appendingPathComponent(filename)
            try source.write(to: url)
            let document = EVDocument(editorBackend: backend)
            document.recordRecentDocument = { _ in }
            defer { document.close() }

            try document.read(from: url, ofType: typeName)

            XCTAssertEqual(backend.sourceFormat, .plainText, filename)
            XCTAssertEqual(document.fileType, EVDocument.plainTextType, filename)
            XCTAssertEqual(try document.data(ofType: EVDocument.plainTextType), source, filename)
            XCTAssertEqual(try Data(contentsOf: url), source, filename)
            XCTAssertFalse(backend.persistenceState.isDirty, filename)
            XCTAssertFalse(document.isDocumentEdited, filename)
            XCTAssertNil(document.recoveryFailure, filename)
        }
    }

    func testGenericDataDocumentTypeStillDetectsCodeFromFilename() throws {
        let backend = backend()
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-data-code-document-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("main.rs")
        let source = Data("fn main() {}\r\n".utf8)
        try source.write(to: url)
        let document = EVDocument(editorBackend: backend)
        document.recordRecentDocument = { _ in }
        defer { document.close() }

        try document.read(from: url, ofType: "public.data")

        XCTAssertEqual(backend.sourceFormat, .code)
        XCTAssertEqual(document.fileType, EVDocument.plainTextType)
        XCTAssertEqual(try document.data(ofType: EVDocument.plainTextType), source)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertFalse(document.isDocumentEdited)
        XCTAssertNil(document.recoveryFailure)
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
            try backend.read(source: bytes, typeName: EVDocument.plainTextType, origin: .file(path: filename, automaticCode: true))
            XCTAssertEqual(backend.sourceFormat, .code, filename)
            XCTAssertEqual(try backend.formattedText(), text, filename)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), bytes, filename)
            XCTAssertFalse(backend.persistenceState.isDirty, filename)
        }
        try backend.read(source: Data("ordinary notes".utf8), typeName: EVDocument.plainTextType, origin: .file(path: "notes.txt", automaticCode: true))
        XCTAssertEqual(backend.sourceFormat, .plainText)
    }

    func testExplicitFormatsAndExistingRichOpeningDefaultsWinOverAutomaticCodeSelection() throws {
        let backend = backend()
        let text = "# Heading\n<!-- vim: ft=rust -->"
        for (type, automatic, expected) in [
            (EVDocument.plainTextType, false, EVSourceFormat.plainText),
            (EVDocument.markdownType, true, .markdown),
            (EVDocument.htmlType, true, .code),
            (EVDocument.codeType, false, .code),
        ] {
            try backend.read(source: Data(text.utf8), typeName: type, origin: .file(path: "file.rs", automaticCode: automatic))
            XCTAssertEqual(backend.sourceFormat, expected)
        }
        try backend.readUntitled(source: Data("unclassified".utf8), typeName: EVDocument.codeType)
        XCTAssertEqual(backend.sourceFormat, .code)
        let recovery = try backend.recoverySnapshot()
        try backend.readUntitled(source: Data(), typeName: EVDocument.plainTextType)
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
            try backend.read(source: bytes, typeName: EVDocument.plainTextType, origin: .file(path: filename, automaticCode: true))
            XCTAssertEqual(backend.sourceFormat, .code)
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), bytes)
            XCTAssertFalse(backend.persistenceState.isDirty)
        }
        try backend.configuration.setCodeFilenameAssociations([])
        try backend.read(source: Data("plain".utf8), typeName: EVDocument.plainTextType, origin: .file(path: "example.custom", automaticCode: true))
        XCTAssertEqual(backend.sourceFormat, .plainText)
    }

    func testDiagnosticsRefreshWithoutAChangedPresentation() async throws {
        let backend = backend(bundleResourceURL: URL(fileURLWithPath: "/missing/viem-diagnostic-only"))
        try backend.readUntitled(source: Data("unrecognized language".utf8), typeName: EVDocument.codeType)
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

    func testCodeDisablesDocumentFormattingAndRetainsGlobalStyleEditing() throws {
        let backend = backend()
        let text = "# Heading\n<b>literal &amp;</b>\n"
        try backend.readUntitled(source: Data(text.utf8), typeName: EVDocument.codeType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let before = try backend.recoverySnapshot()
        let globalStyleCommands: Set<EVMenuCommand> = [.editStyles, .reloadStyleSheet]
        for command in EVMenuCommand.allCases where (300..<400).contains(command.rawValue) {
            if globalStyleCommands.contains(command) {
                XCTAssertTrue(surface.presentation(for: command).isEnabled, "\(command)")
                continue
            }
            XCTAssertFalse(surface.presentation(for: command).isEnabled, "\(command)")
            surface.perform(menuCommand: command, sender: nil)
        }
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        let styles = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        XCTAssertTrue(styles.canEditStyles)
        XCTAssertTrue(styles.entries.allSatisfy { $0.actionKind != .assign })
        XCTAssertFalse(surface.canUndo)
    }

    private func mode(_ backend: EVCoreDocumentBackend) throws -> EVDocumentModeState {
        try EVDocumentModeJSON.read { viem_core_copy_document_mode_json(backend.core, $0, $1, $2) }
    }

    /// Every new-file path builds its backend from the registry, as in the app.
    private func installRegistryBackends() throws -> URL {
        EVEditorComposition.install()
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-new-file-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let configuration = EVConfigurationStore(directory: directory.appendingPathComponent("profile"),
            legacyDefaults: nil, bundleResourceURL: Bundle.main.resourceURL)
        EVFrontendRegistry.install { EVCoreDocumentBackend(configuration: configuration) }
        addTeardownBlock { @MainActor in
            EVFrontendRegistry.install { EVCoreDocumentBackend() }
            try? FileManager.default.removeItem(at: directory)
        }
        return directory
    }

    /// `viem <path>` from a terminal.
    private func launch(_ url: URL) throws -> EVDocument {
        let delegate = EVApplicationDelegate(launchArguments: EVLaunchArguments(filenames: [url.path]))
        delegate.recordRecentDocument = { _ in }
        XCTAssertTrue(delegate.openLaunchArguments())
        let document = try XCTUnwrap(EVDocumentIdentity.existingDocument(at: url))
        addTeardownBlock { @MainActor in document.close() }
        return document
    }

    /// A window showing `url` read from disk, with an optional mode chosen in it.
    private func openWindow(_ url: URL, choosing choice: EVDocumentModeChoice?) throws
        -> (EVDocument, EVCoreDocumentBackend, EVDocumentWindowController) {
        let backend = try XCTUnwrap(EVFrontendRegistry.makeDocumentBackend() as? EVCoreDocumentBackend)
        let document = EVDocument(editorBackend: backend)
        document.recordRecentDocument = { _ in }
        try document.read(from: url, ofType: EVDocument.plainTextType)
        document.fileURL = url
        let window = EVDocumentWindowController(document: document, editorSurface: backend.makeEditorSurface())
        document.addWindowController(window)
        addTeardownBlock { @MainActor in withExtendedLifetime(document) { window.close() } }
        if let choice {
            let session = try XCTUnwrap((window.editorSurface as? EVEditorSurfaceController)?.session)
            try session.setDocumentMode(choice, expected: try mode(backend))
        }
        return (document, backend, window)
    }

    /// A bare Ex request such as `:e`.
    private func perform(_ kind: EVDocumentHostRequest.Kind, in window: EVDocumentWindowController,
                         backend: EVCoreDocumentBackend) async throws {
        let state = backend.persistenceState
        let result = await withCheckedContinuation { continuation in
            window.perform(documentHostRequests: [.init(kind: kind, documentID: state.documentID,
                documentRevision: state.documentRevision)]) { continuation.resume(returning: $0) }
        }
        _ = try result.get()
    }

    private func file(_ name: String, in directory: URL, text: String) throws -> URL {
        let url = directory.appendingPathComponent("\(UUID().uuidString)/\(name)")
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data(text.utf8).write(to: url)
        return url
    }

    /// Both reload paths; the core tests cover the rest of the policy.
    func testReloadKeepsTheBuffersChoiceAndPicksUpADeclaredLanguage() async throws {
        let directory = try installRegistryBackends()
        let cases: [(String, EVDocumentModeChoice?, String, EVSourceFormat, String?, Bool)] = [
            // A chosen language, and a chosen format that reopening would undo.
            ("notes.txt", .code("python"), "x = 1\n", .code, "python", false),
            ("notes.md", .plainText, "x = 1\n", .plainText, nil, false),
            // Detected Text becomes Code once its file declares a language.
            ("script", nil, "#!/usr/bin/env python3\n", .code, nil, true),
        ]
        for (name, choice, reloaded, format, language, automatic) in cases {
            let url = try file(name, in: directory, text: "plain words\n")
            let (document, backend, window) = try openWindow(url, choosing: choice)
            for reload in [":e", "an external change"] {
                let label = "\(name) after \(reload)"
                let source = Data((reloaded + "# \(reload)\n").utf8)
                try source.write(to: url)
                if reload == ":e" {
                    try await perform(.edit, in: window, backend: backend)
                } else {
                    try document.installExternalFileSnapshot(EVExternalFileSnapshot.read(url), from: url)
                }
                let state = try mode(backend)
                XCTAssertEqual(backend.sourceFormat, format, label)
                XCTAssertEqual(state.language, language, label)
                XCTAssertEqual(state.automatic, automatic, label)
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.typeName(for: format)), source, label)
                XCTAssertFalse(backend.persistenceState.isDirty, label)
            }
        }
    }

    func testTypingInLaunchedMissingFileGetsSyntaxPaint() async throws {
        let directory = try installRegistryBackends()
        let document = try launch(directory.appendingPathComponent("main.rs"))
        let backend = try XCTUnwrap(document.editorBackend as? EVCoreDocumentBackend)
        let controller = try XCTUnwrap(document.windowControllers.first as? EVDocumentWindowController)
        let surface = try XCTUnwrap(controller.editorSurface as? EVEditorSurfaceController)
        let session = try XCTUnwrap(surface.session)
        _ = try session.sendTestKeys("ifn main() {}")
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        try await assertRustKeywordIsHighlighted(backend: backend, surface: surface)
    }
}
