import AppKit
import CViemCore
import CoreText
import ViemAppShell
import ViemCoreTextProvider
import XCTest
@testable import ViemEditor

@MainActor
final class EVDefaultStyleIntegrationTests: XCTestCase {
  func testMarkdownThemeRoundTripRestoresRenderedFontAndSurvivesResize() throws {
    var root = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { root.deleteLastPathComponent() }
    let fontDirectory = root.appendingPathComponent("assets/fonts/recursive")
    XCTAssertTrue(EVFontCatalog.registerBundledFonts(in: fontDirectory).isEmpty)
    defer {
      CTFontManagerUnregisterFontsForURL(fontDirectory.appendingPathComponent("Recursive_VF_1.085.ttf") as CFURL, .process, nil)
      EVFontCatalog.invalidate()
    }
    let config = try configuration()
    try config.selectTheme(named: "Paper")
    try config.createTheme(named: "Recursive")
    let styles = try EVThemeStyleSession(configuration: config, format: .markdown)
    try styles.edit(key: .baseParagraph, expected: styles.snapshot().identity,
      mutation: .setDeclaration(.characterFontFamilies, .stringList(["Recursive"])))
    try styles.edit(key: .baseParagraph, expected: styles.snapshot().identity,
      mutation: .setDeclaration(.characterSize, .float(16)))
    let specimen = "# Heading\n\nWriting with [a link](https://example.com) and ~~struck text~~.\n\n<!-- Comment -->\n\n[reference][unknown]\n\n"
    // Keep most of the document outside the viewport to exercise regional
    // reflow while the automatically styled content remains visible.
    let source = Data(String(repeating: specimen, count: 256).utf8)
    try config.selectTheme(named: "Paper")
    let backend = EVCoreDocumentBackend(configuration: config)
    try backend.read(source: source, typeName: EVDocument.markdownType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let session = try XCTUnwrap(surface.session)
    _ = try session.resize(width: 700, height: 180)
    surface.refreshPresentation()
    func renderedFont() throws -> String {
      let cluster = try XCTUnwrap(surface.layoutSnapshot?.clusters.first { $0.text_start == 0 })
      let font = try XCTUnwrap(session.provider.renderRegistry.resolvedFont(
        identifier: cluster.render_run.identifier, metricsGeneration: cluster.render_run.metrics_generation))
      return CTFontCopyFamilyName(font) as String
    }
    let originalFont = try renderedFont()
    let original = try backend.recoverySnapshot()
    for name in ["Recursive", "Paper", "Recursive", "Paper"] {
      try config.selectTheme(named: name)
      XCTAssertNil(backend.configurationWarning)
      XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?
        .properties[.characterFontFamilies]?.effective,
        .stringList([name == "Paper" ? "system-ui" : "Recursive"]))
      surface.refreshPresentation()
      XCTAssertEqual(try renderedFont(), name == "Paper" ? originalFont : "Recursive")
      for width: CGFloat in [420, 900, 700] {
        _ = try session.resize(width: width, height: 180)
        surface.refreshPresentation()
        XCTAssertNil(surface.commandOutput, surface.statusBarState.message)
        XCTAssertEqual(try renderedFont(), name == "Paper" ? originalFont : "Recursive")
      }
      XCTAssertEqual(try backend.recoverySnapshot(), original)
      XCTAssertFalse(surface.canUndo)
      XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
    }
  }

  private func configuration() throws -> EVConfigurationStore {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-style-defaults-\(UUID().uuidString)")
    let suite = "viem-style-defaults-\(UUID().uuidString)"
    let legacy = try XCTUnwrap(UserDefaults(suiteName: suite))
    addTeardownBlock { try? FileManager.default.removeItem(at: directory); legacy.removePersistentDomain(forName: suite) }
    return EVConfigurationStore(directory: directory, legacyDefaults: legacy)
  }
  func testThemeStyleEditsUpdateExistingAndFutureDocumentsWithoutChangingSourceOrUndo() throws {
    for (format, type, source) in [(EVSourceFormat.plainText, EVDocument.plainTextType, "Text"),
      (.markdown, EVDocument.markdownType, "# Heading\n\nText"),
    ] {
      let config = try configuration()
      let backends = [EVCoreDocumentBackend(configuration: config), EVCoreDocumentBackend(configuration: config)]
      let surfaces = try backends.map { backend -> EVEditorSurfaceController in
        try backend.read(source: Data(source.utf8), typeName: type)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        return surface
      }
      let originals = try backends.map { try $0.recoverySnapshot() }
      let session = try EVThemeStyleSession(configuration: config, format: format)
      try session.edit(key: .baseParagraph, expected: session.snapshot().identity,
        mutation: .setDeclaration(.characterSize, .float(27)))
      for (index, backend) in backends.enumerated() {
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.effective, .float(27))
        XCTAssertEqual(try backend.recoverySnapshot(), originals[index])
        XCTAssertFalse(surfaces[index].canUndo)
      }
      let reopened = EVCoreDocumentBackend(configuration: config)
      try reopened.read(source: Data(source.utf8), typeName: type)
      XCTAssertEqual(try reopened.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.effective, .float(27))
      XCTAssertFalse(reopened.persistenceState.isDirty)
      XCTAssertEqual(try reopened.serializedSource(typeName: type), Data(source.utf8))
      session.undoManager.undo()
      XCTAssertNotEqual(try reopened.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.effective, .float(27))
      session.undoManager.redo()
      XCTAssertEqual(try reopened.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.effective, .float(27))
    }
  }

  func testDefaultThemeStyleEditsRemainInMemoryAndAreClonedByNewTheme() throws {
    let config = try configuration()
    try config.selectTheme(named: nil)
    let initialConfig = try Data(contentsOf: config.directory.appendingPathComponent("config.json"))
    let session = try EVThemeStyleSession(configuration: config, format: .markdown)
    try session.edit(key: .baseParagraph, expected: session.snapshot().identity,
      mutation: .setDeclaration(.characterSize, .float(29)))
    XCTAssertNil(config.selectedThemeURL)
    XCTAssertEqual(try Data(contentsOf: config.directory.appendingPathComponent("config.json")), initialConfig)
    try config.createTheme(named: "My writing")
    let reopened = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: config.directory))
    try reopened.read(source: Data("Text".utf8), typeName: EVDocument.markdownType)
    XCTAssertEqual(try reopened.styleSheetSnapshot().definition(for: .baseParagraph)?.properties[.characterSize]?.effective, .float(29))
  }

  func testMalformedSelectedThemeWarnsWithoutBlockingOrRewritingSource() throws {
    let config = try configuration()
    let file = try XCTUnwrap(config.selectedThemeURL)
    let invalid = Data(#"{"version":99}"#.utf8)
    try invalid.write(to: file)
    let reloaded = EVConfigurationStore(directory: config.directory)
    let backend = EVCoreDocumentBackend(configuration: reloaded)
    let source = Data("Text".utf8)
    try backend.read(source: source, typeName: EVDocument.markdownType)
    XCTAssertNotNil(backend.configurationWarning)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), source)
    XCTAssertEqual(try Data(contentsOf: file), invalid)
    XCTAssertFalse(backend.persistenceState.isDirty)
  }
  func testDefaultImportRejectsAnAlreadyOpenedViewWithoutMutation() throws {
    let config = try configuration()
    let backend = EVCoreDocumentBackend(configuration: config)
    try backend.read(source: Data("Text".utf8), typeName: EVDocument.plainTextType)
    let surface = backend.makeEditorSurface()
    let bytes = Data(#"{"version":1}"#.utf8)
    let revision = try backend.documentState().document_revision
    let status = bytes.withUnsafeBytes { raw in
      viem_core_initialize_style_defaults(backend.core, revision, raw.bindMemory(to: UInt8.self).baseAddress, UInt64(raw.count), nil, nil)
    }
    XCTAssertEqual(status, UInt32(VIEM_STATUS_INVALID_ARGUMENT))
    XCTAssertEqual(try backend.documentState().document_revision, revision)
    XCTAssertEqual(try backend.formattedText(), "Text")
    withExtendedLifetime(surface) {}
  }
  func testExportRejectsStaleSnapshotsAndNeverWritesPartialJSON() throws {
    let backend = EVCoreDocumentBackend(configuration: try configuration())
    let revision = try backend.documentState().document_revision
    var required: UInt64 = 0
    XCTAssertEqual(viem_core_export_style_defaults(backend.core, revision, nil, 0, &required), UInt32(VIEM_STATUS_BUFFER_TOO_SMALL))
    XCTAssertGreaterThan(required, 0)
    var bytes = [UInt8](repeating: 0xCC, count: Int(required))
    let status = bytes.withUnsafeMutableBufferPointer {
      viem_core_export_style_defaults(backend.core, revision, $0.baseAddress, required - 1, &required)
    }
    XCTAssertEqual(status, UInt32(VIEM_STATUS_BUFFER_TOO_SMALL))
    XCTAssertTrue(bytes.allSatisfy { $0 == 0xCC })
    XCTAssertEqual(viem_core_export_style_defaults(backend.core, revision + 1, nil, 0, &required), UInt32(VIEM_STATUS_STALE_REVISION))
    XCTAssertEqual(required, 0)
  }

}
