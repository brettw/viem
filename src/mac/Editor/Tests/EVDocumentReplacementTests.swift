import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVDocumentReplacementTests: XCTestCase {
    func testFailedPreparationPreservesDirtySourceHistoryAndEveryLiveView() throws {
        let (backend, surfaces) = try makeDocumentWithTwoViews()
        let sessions = try surfaces.map { try XCTUnwrap($0.session) }
        _ = try sessions[0].sendText("i")
        _ = try sessions[0].sendText("unsaved ")
        _ = try sessions[0].sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        surfaces.forEach { $0.refreshPresentation() }
        let before = try backend.recoverySnapshot()
        let core = backend.core
        let persistence = backend.persistenceState
        let layout = try sessions[0].layoutExport()
        XCTAssertTrue(persistence.isDirty)
        var sourceNotifications = 0
        var persistenceNotifications = 0
        backend.sourceDidChange = { sourceNotifications += 1 }
        backend.persistenceStateDidChange = { _ in persistenceNotifications += 1 }

        let invalidRecovery = EVRecoverySnapshot(
            source: Data("replacement".utf8), format: .plainText,
            encoding: UInt32.max, fileFormat: UInt32(VIEM_FILE_FORMAT_UNIX),
            documentID: 100, documentRevision: 100
        )
        XCTAssertThrowsError(try backend.restoreRecovery(invalidRecovery)) { error in
            guard case let EVCoreFrontendError.core(operation, _) = error else {
                return XCTFail("Unexpected error: \(error)")
            }
            XCTAssertEqual(operation, "Open document")
        }

        XCTAssertEqual(backend.core, core)
        XCTAssertEqual(backend.persistenceState, persistence)
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertEqual(sourceNotifications, 0)
        XCTAssertEqual(persistenceNotifications, 0)
        for (surface, session) in zip(surfaces, sessions) {
            XCTAssertTrue(surface.session === session)
            XCTAssertNotEqual(session.viewID, 0)
            _ = try session.refreshState()
            surface.refreshPresentation()
            XCTAssertEqual(surface.formattedText, "unsaved original")
        }
        let retainedLayout = try sessions[0].layoutExport()
        XCTAssertEqual(retainedLayout.info.identity.document_id, layout.info.identity.document_id)
        XCTAssertEqual(retainedLayout.info.identity.document_revision, layout.info.identity.document_revision)
        XCTAssertEqual(retainedLayout.info.identity.layout_revision, layout.info.identity.layout_revision)
        _ = try sessions[0].undo()
        XCTAssertEqual(try backend.formattedText(), "original")
        _ = try sessions[0].redo()
        XCTAssertEqual(try backend.recoverySnapshot().source, before.source)
    }

    func testOptionalDetectionFailureStillTransfersCoreAndReattachesEveryLiveView() throws {
        let (backend, surfaces) = try makeDocumentWithTwoViews()
        let sessions = try surfaces.map { try XCTUnwrap($0.session) }
        _ = try sessions[0].sendText("i")
        _ = try sessions[0].sendText("unsaved ")
        _ = try sessions[0].setWrap(false)
        _ = try sessions[0].setScale(1.75)
        _ = try sessions[1].setWrap(true)
        _ = try sessions[1].setScale(0.85)
        let viewports = try sessions.map { try $0.viewportState() }
        let oldCore = backend.core
        let oldDocument = backend.currentDocumentState.document_id
        var sourceNotifications = 0
        var persistenceNotifications = 0
        backend.sourceDidChange = { sourceNotifications += 1 }
        backend.persistenceStateDidChange = { _ in persistenceNotifications += 1 }

        try backend.read(source: Data("# New".utf8), typeName: EVDocument.markdownType,
                         filename: "invalid\0filename.md", allowAutomaticCode: false)

        XCTAssertNotEqual(backend.core, oldCore)
        XCTAssertNotEqual(backend.currentDocumentState.document_id, oldDocument)
        XCTAssertEqual(backend.sourceFormat, .markdown)
        XCTAssertTrue(backend.configurationWarning?.contains("language detection") == true)
        XCTAssertFalse(backend.persistenceState.isDirty)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("# New".utf8))
        XCTAssertEqual(sourceNotifications, 0)
        XCTAssertEqual(persistenceNotifications, 1)
        var oldRevision: UInt64 = 0
        XCTAssertNotEqual(viem_core_revision(oldCore, &oldRevision), UInt32(VIEM_STATUS_OK))
        for (index, surface) in surfaces.enumerated() {
            let oldSession = sessions[index]
            XCTAssertEqual(oldSession.viewID, 0)
            let session = try XCTUnwrap(surface.session)
            XCTAssertFalse(session === oldSession)
            XCTAssertTrue(session.document === backend)
            _ = try session.refreshState()
            let viewport = try session.viewportState()
            XCTAssertEqual(viewport.flags & UInt32(VIEM_VIEWPORT_STATE_WRAP),
                           viewports[index].flags & UInt32(VIEM_VIEWPORT_STATE_WRAP))
            XCTAssertEqual(viewport.scale, viewports[index].scale)
            XCTAssertEqual(surface.formattedText, "New")
            XCTAssertEqual(try session.layoutExport().info.identity.document_id,
                           backend.currentDocumentState.document_id)
        }
    }

    func testFailedSecondViewPreparationKeepsLiveViewsAndReleasesTheStagedCore() throws {
        let (backend, surfaces) = try makeDocumentWithTwoViews()
        let sessions = try surfaces.map { try XCTUnwrap($0.session) }
        _ = try sessions[0].sendText("i")
        _ = try sessions[0].sendText("unsaved ")
        _ = try sessions[0].sendKey(kind: UInt32(VIEM_KEY_ESCAPE))
        let before = try backend.recoverySnapshot()
        let oldCore = backend.core
        var sourceNotifications = 0
        var persistenceNotifications = 0
        backend.sourceDidChange = { sourceNotifications += 1 }
        backend.persistenceStateDidChange = { _ in persistenceNotifications += 1 }
        var stagedCore: ViemCoreHandle = 0
        weak var stagedSession: EVCoreViewSession?
        surfaces[0].makeCoreViewSession = { document, size in
            stagedCore = document.core
            let session = try EVCoreViewSession(document: document, width: size.width, height: size.height)
            stagedSession = session
            return session
        }
        surfaces[1].makeCoreViewSession = { document, _ in
            // Exercise the real C view-creation validation after another
            // replacement view has already been prepared successfully.
            try EVCoreViewSession(document: document, width: .infinity, height: 100)
        }

        XCTAssertThrowsError(try backend.read(source: Data("replacement".utf8),
                                             typeName: EVDocument.plainTextType)) { error in
            guard case let EVCoreFrontendError.core(operation, _) = error else {
                return XCTFail("Unexpected error: \(error)")
            }
            XCTAssertEqual(operation, "Attach editor view")
        }
        XCTAssertEqual(backend.core, oldCore)
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertEqual(sourceNotifications, 0)
        XCTAssertEqual(persistenceNotifications, 0)
        XCTAssertNotEqual(stagedCore, 0)
        XCTAssertNil(stagedSession)
        var revision: UInt64 = 0
        XCTAssertNotEqual(viem_core_revision(stagedCore, &revision), UInt32(VIEM_STATUS_OK))
        for (surface, session) in zip(surfaces, sessions) {
            XCTAssertTrue(surface.session === session)
            _ = try session.refreshState()
            surface.refreshPresentation()
            XCTAssertEqual(surface.formattedText, "unsaved original")
        }
        _ = try sessions[0].undo()
        XCTAssertEqual(try backend.formattedText(), "original")
        _ = try sessions[0].redo()
        XCTAssertEqual(try backend.recoverySnapshot().source, before.source)
    }

    func testReplacementPreservesEachSourceViewsFlowAndLineMode() throws {
        let (backend, surfaces) = try makeDocumentWithTwoViews()
        try backend.read(source: Data("first\nsecond".utf8), typeName: EVDocument.markdownSourceType)
        let sessions = try surfaces.map { try XCTUnwrap($0.session) }
        try sessions[0].setParagraphFlow(true)
        try sessions[0].setLineMode(.physicalSource)
        try sessions[1].setParagraphFlow(false)
        try sessions[1].setLineMode(.visual)

        try backend.read(source: Data("first\nchanged".utf8), typeName: EVDocument.markdownSourceType)

        let first = try XCTUnwrap(surfaces[0].session)
        let second = try XCTUnwrap(surfaces[1].session)
        XCTAssertTrue(try first.paragraphFlow())
        XCTAssertEqual(try first.lineMode(), .physicalSource)
        XCTAssertFalse(try second.paragraphFlow())
        XCTAssertEqual(try second.lineMode(), .visual)
    }

    private func makeDocumentWithTwoViews() throws -> (EVCoreDocumentBackend, [EVEditorSurfaceController]) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-document-replacement-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data("original".utf8), typeName: EVDocument.plainTextType,
                         filename: "original.txt", allowAutomaticCode: false)
        let surfaces = try (0..<2).map { _ in
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            return surface
        }
        return (backend, surfaces)
    }
}
