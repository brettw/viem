import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVDefaultStyleIntegrationTests: XCTestCase {
  private func configuration() throws -> EVConfigurationStore {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-style-defaults-\(UUID().uuidString)")
    let suite = "viem-style-defaults-\(UUID().uuidString)"
    let legacy = try XCTUnwrap(UserDefaults(suiteName: suite))
    addTeardownBlock { try? FileManager.default.removeItem(at: directory); legacy.removePersistentDomain(forName: suite) }
    return EVConfigurationStore(directory: directory, legacyDefaults: legacy)
  }
  func testSaveDefaultMenuReloadsSparseStyleWithoutDirtyingNewDocument() throws {
    for (type, source, name) in [(EVDocument.plainTextType,"Text","text"),
      (EVDocument.markdownType,"Text","markdown"),
      (EVDocument.rtfType,#"{\rtf1 Text}"#,"rtf")] {
      let config = try configuration()
      let original = EVCoreDocumentBackend(configuration: config)
      try original.read(source: Data(source.utf8), typeName: type)
      let surface = try XCTUnwrap(original.makeEditorSurface() as? EVEditorSurfaceController)
      surface.loadViewIfNeeded()
      let snapshot = try original.styleSheetSnapshot()
      try surface.session?.editStyle(key: .baseParagraph, expected: snapshot.identity,
        mutation: .setDeclaration(.characterSize, .float(27)))
      XCTAssertEqual(surface.presentation(for: .saveDefaultStyle).title, "Save as default \(name) style")
      let before = try original.documentState()
      let bytes = try original.serializedSource(typeName: type)
      surface.perform(menuCommand: .saveDefaultStyle, sender: nil)
      XCTAssertTrue(FileManager.default.fileExists(atPath: config.directory.appendingPathComponent("\(name)_style.json").path))
      XCTAssertEqual(try original.documentState().document_revision, before.document_revision)
      XCTAssertEqual(try original.serializedSource(typeName: type), bytes)
      let reopened = EVCoreDocumentBackend(configuration: config)
      try reopened.read(source: Data(source.utf8), typeName: type)
      XCTAssertNil(reopened.configurationWarning)
      XCTAssertEqual(try reopened.documentState().document_revision, 0)
      XCTAssertFalse(reopened.persistenceState.isDirty)
      XCTAssertEqual(try reopened.serializedSource(typeName: type), Data(source.utf8))
      let style = try XCTUnwrap(try reopened.styleSheetSnapshot().definition(for: .baseParagraph))
      XCTAssertEqual(style.properties[.characterSize]?.declared, .float(27), "Saved defaults are ordinary visible declarations")
      XCTAssertEqual(style.properties[.characterSize]?.effective, .float(27))
    }
  }
  func testMalformedDefaultFileWarnsWithoutBlockingOrRewritingSource() throws {
    let config = try configuration()
    try FileManager.default.createDirectory(at: config.directory, withIntermediateDirectories: true)
    let file = config.directory.appendingPathComponent("markdown_style.json")
    let invalid = Data(#"{"version":99}"#.utf8); try invalid.write(to: file)
    let backend = EVCoreDocumentBackend(configuration: config)
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
