import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVRecoveryCoreTests: XCTestCase {
  func testRecoveryRestoresPhysicalBytesAndForcedInterpretationAndRemainsDirty() throws {
    let backend = EVCoreDocumentBackend()
    let bytes = Data([0x23, 0x20, 0xe9, 0x0d, 0x62, 0x6f, 0x64, 0x79])
    let recovery = EVRecoverySnapshot(
      source: bytes, format: .markdownSource,
      encoding: UInt32(VIEM_ENCODING_LATIN1), fileFormat: UInt32(VIEM_FILE_FORMAT_MAC),
      documentID: 800, documentRevision: 200)
    try backend.restoreRecovery(recovery)
    XCTAssertEqual(try backend.formattedText(), "# é\nbody")
    XCTAssertEqual(backend.sourceFormat, .markdownSource)
    XCTAssertEqual(backend.currentDocumentState.encoding, UInt32(VIEM_ENCODING_LATIN1))
    XCTAssertEqual(backend.currentDocumentState.file_format, UInt32(VIEM_FILE_FORMAT_MAC))
    XCTAssertTrue(backend.persistenceState.isDirty)
    XCTAssertTrue(backend.persistenceState.isRecovered)
    let restored = try backend.recoverySnapshot()
    XCTAssertEqual(restored.source, bytes)
    XCTAssertEqual(restored.format, .markdownSource)
    XCTAssertEqual(restored.encoding, recovery.encoding)
    XCTAssertEqual(restored.fileFormat, recovery.fileFormat)
    XCTAssertNotEqual(restored.documentID, recovery.documentID)
    let saved = try backend.nativeSaveSnapshot(typeName: EVDocument.markdownType)
    try backend.acknowledgeNativeSave(saved)
    XCTAssertFalse(backend.persistenceState.isDirty)
    XCTAssertFalse(backend.persistenceState.isRecovered)
  }

  func testReadOnlyIsBufferPolicyWithoutChangingSourceOrRevision() throws {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data("{\\rtf1\\ansi text}".utf8), typeName: EVDocument.rtfType)
    let before = try backend.recoverySnapshot()
    try backend.setReadOnly(true)
    XCTAssertTrue(backend.persistenceState.isReadOnly)
    XCTAssertEqual(try backend.recoverySnapshot(), before)
    try backend.setReadOnly(false)
    XCTAssertFalse(backend.persistenceState.isReadOnly)
    XCTAssertEqual(try backend.recoverySnapshot(), before)
  }

  func testReadOnlyWritePublishesE45AndReturnsToNormalMode() throws {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data("original".utf8), typeName: EVDocument.plainTextType)
    try backend.setReadOnly(true)
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    let session = try XCTUnwrap(surface.session)
    surface.performInput { _ = try session.sendText(":w") }
    surface.performInput { _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER)) }
    let output = try XCTUnwrap(surface.commandOutput)
    XCTAssertTrue(output.contains("E45"))
    XCTAssertEqual(surface.statusBarState.message, "")
    let bar = surface.editorView.commandOutputBar
    XCTAssertFalse(bar.isHidden)
    XCTAssertFalse(bar.textView.isEditable)
    XCTAssertTrue(bar.textView.isSelectable)
    XCTAssertEqual(bar.textView.string, output)
    let close = try XCTUnwrap(bar.subviews.compactMap { $0 as? NSButton }.first)
    XCTAssertEqual(close.accessibilityLabel(), "Close command output")
    XCTAssertFalse(close.isHidden)
    XCTAssertNotNil(close.image)
    close.performClick(nil)
    XCTAssertNil(surface.commandOutput)
    XCTAssertTrue(bar.isHidden)
    XCTAssertEqual(surface.statusBarState.mode, "NORMAL")
    XCTAssertEqual(
      try backend.serializedSource(typeName: EVDocument.plainTextType), Data("original".utf8))
  }
}
