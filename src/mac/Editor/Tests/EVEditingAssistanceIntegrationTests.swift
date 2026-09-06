import AppKit
import CEvimCore
import EvimAppShell
import XCTest

@testable import EvimEditor

@MainActor
final class EVEditingAssistanceIntegrationTests: XCTestCase {
  func testLiveSmartQuotePreferenceOnlyAffectsSubsequentTypedInput() throws {
    let suite = "evim-assistance-native-\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    let configDirectory = FileManager.default.temporaryDirectory.appendingPathComponent("evim-config-test-\(UUID().uuidString)")
    addTeardownBlock { try? FileManager.default.removeItem(at: configDirectory) }
    let configuration = EVConfigurationStore(directory: configDirectory, legacyDefaults: defaults)
    defer { defaults.removePersistentDomain(forName: suite) }
    let preferences = EVEditingPreferences(configuration: configuration)
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data(), typeName: EVDocument.plainTextType)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let session = try XCTUnwrap(surface.session)
    let view = surface.editorView
    view.editingPreferences = preferences
    surface.performInput { _ = try session.sendText("i") }
    func type(_ value: String) {
      view.insertText(value, replacementRange: NSRange(location: NSNotFound, length: 0))
    }
    type("\"")
    XCTAssertEqual(try backend.formattedText(), "\"")
    let before = try backend.serializedSource(typeName: EVDocument.plainTextType)
    let revision = try backend.documentState().document_revision
    preferences.setSmartQuotes(true)
    XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.plainTextType), before)
    XCTAssertEqual(try backend.documentState().document_revision, revision)
    type(" ")
    type("\"")
    type("word")
    type("\"")
    XCTAssertEqual(try backend.formattedText(), "\" “word”")
    preferences.setSmartQuotes(false)
    type("\"")
    XCTAssertEqual(try backend.formattedText(), "\" “word”\"")
    surface.performInput { _ = try session.sendKey(kind: UInt32(EVIM_KEY_ESCAPE)) }
    surface.perform(menuCommand: .undo, sender: nil)
    XCTAssertEqual(try backend.formattedText(), "")
  }
}
