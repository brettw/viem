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

  func testReloadPreservesEachViewPositionAndClampsAfterTruncation() throws {
    let backend = EVCoreDocumentBackend()
    let source = Data(String(repeating: "a fairly long line of content\n", count: 120).utf8)
    try backend.read(source: source, typeName: EVDocument.plainTextType)
    let first = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    let second = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    first.loadViewIfNeeded(); second.loadViewIfNeeded()
    let sessions = try [XCTUnwrap(first.session), XCTUnwrap(second.session)]
    for (index, session) in sessions.enumerated() {
      _ = try session.resize(width: 350, height: 120)
      _ = try session.sendText(index == 0 ? "jlll" : "jjlllll")
      _ = try session.setViewportOrigin(left: 0, top: CGFloat(310 + index * 220))
    }
    func captured(_ surface: EVEditorSurfaceController) throws -> ViemViewRestorationV1 {
      var state = ViemViewRestorationV1()
      let session = try XCTUnwrap(surface.session)
      XCTAssertEqual(viem_core_view_capture_restoration(backend.core, session.viewID, &state), UInt32(VIEM_STATUS_OK))
      return state
    }
    let before = try [captured(first), captured(second)]
    try backend.read(source: source, typeName: EVDocument.plainTextType)
    for (surface, expected) in zip([first, second], before) {
      let after = try captured(surface)
      XCTAssertEqual(after.cursor_line, expected.cursor_line)
      XCTAssertEqual(after.cursor_column, expected.cursor_column)
      XCTAssertEqual(after.viewport_line, expected.viewport_line)
      XCTAssertEqual(after.row_fraction, expected.row_fraction, accuracy: 0.001)
    }
    try backend.read(source: Data(), typeName: EVDocument.plainTextType)
    for surface in [first, second] {
      let after = try captured(surface)
      XCTAssertEqual(after.cursor_line, 0); XCTAssertEqual(after.cursor_column, 0)
      XCTAssertEqual(after.viewport_line, 0)
    }
  }

  func testReadOnlyIsBufferPolicyWithoutChangingSourceOrRevision() throws {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data("text".utf8), typeName: EVDocument.plainTextType)
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
    XCTAssertEqual(surface.statusBarState.commandOutput, output)
    surface.dismissCommandOutput()
    XCTAssertNil(surface.commandOutput)
    XCTAssertNil(surface.statusBarState.commandOutput)
    XCTAssertEqual(surface.statusBarState.mode, "NORMAL")
    XCTAssertEqual(
      try backend.serializedSource(typeName: EVDocument.plainTextType), Data("original".utf8))
  }
}
