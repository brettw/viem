import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVHostEffectsIntegrationTests: XCTestCase {
    private final class Pasteboard: EVPasteboardAccess {
        var text: String?
        var generation: UInt64 = 1
        var isWritable = true
        var acceptsWrites = true
        var writes: [String] = []

        var viemGeneration: UInt64 { generation }
        var viemIsWritable: Bool { isWritable }
        func viemString() -> String? { text }
        func viemCanReadString() -> Bool { text != nil }
        func viemClearContents() -> Int {
            text = nil
            generation &+= 1
            return Int(generation)
        }
        func viemSetString(_ string: String) -> Bool {
            guard acceptsWrites else { return false }
            text = string
            writes.append(string)
            generation &+= 1
            return true
        }
    }

    private final class DocumentHost: EVDocumentHostEffectHandling {
        var requests: [[EVDocumentHostRequest]] = []
        var result: Result<String?, Error> = .success(nil)

        func perform(
            documentHostRequests: [EVDocumentHostRequest],
            completion: @escaping @MainActor (Result<String?, Error>) -> Void
        ) {
            requests.append(documentHostRequests)
            completion(result)
        }
    }

    private final class CommandTurnHost: EVCommandTurnHost {
        var snapshots: [EVClipboardTurnSnapshot] = []
        var batches: [EVHostEffectBatch] = []

        func clipboardSnapshotsForCommandTurn() -> [EVClipboardTurnSnapshot] { snapshots }
        func applyHostEffectBatch(_ batch: EVHostEffectBatch) throws { batches.append(batch) }
    }

    func testClipboardAndPrimarySnapshotsStayDistinctWhileSharingNativeStorage() throws {
        let (_, surface, _) = try makeSurface("abc")
        let pasteboard = Pasteboard()
        pasteboard.text = "shared"
        pasteboard.generation = 73
        surface.pasteboard = pasteboard

        let snapshots = surface.clipboardSnapshotsForCommandTurn()

        XCTAssertEqual(snapshots.count, 2)
        XCTAssertEqual(snapshots.map(\.target), [
            UInt32(VIEM_CLIPBOARD_TARGET_CLIPBOARD),
            UInt32(VIEM_CLIPBOARD_TARGET_PRIMARY),
        ])
        XCTAssertEqual(snapshots.map(\.plainText), ["shared", "shared"])
        XCTAssertEqual(snapshots.map(\.generation), [73, 73])
        XCTAssertTrue(snapshots.allSatisfy(\.isWritable))

        pasteboard.text = nil
        XCTAssertEqual(
            surface.clipboardSnapshotsForCommandTurn().map(\.generation),
            [0, 0],
            "a target without HAS_READ must not leak a nonzero generation"
        )
    }

    func testDefaultPasteboardAdapterSurvivesSequentialSurfaceAutoreleasePools() throws {
        for _ in 0 ..< 4 {
            try autoreleasepool {
                let (_, surface, session) = try makeSurface("abc")
                try sendKey("l", through: session)
                surface.refreshPresentation()
                XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, 1)
            }
        }
    }

    func testSystemClipboardYankAndPutUseCoreRegisterSemantics() throws {
        do {
            let (_, surface, session) = try makeSurface("abc")
            let pasteboard = Pasteboard()
            surface.pasteboard = pasteboard
            try sendKeys("vl\"+y", through: session)

            XCTAssertEqual(pasteboard.text, "ab")
            XCTAssertEqual(pasteboard.writes, ["ab"])
            XCTAssertEqual(surface.formattedText, "abc")
            XCTAssertEqual(session.hostEffectAccessCounters.copyCalls, 1)
            XCTAssertEqual(session.hostEffectAccessCounters.releaseCalls, 1)
        }

        do {
            let (_, surface, session) = try makeSurface("abc")
            let pasteboard = Pasteboard()
            pasteboard.text = "ZZ"
            pasteboard.generation = 8
            surface.pasteboard = pasteboard
            try sendKeys("\"+p", through: session)
            surface.refreshPresentation()

            XCTAssertEqual(surface.formattedText, "aZZbc")
            XCTAssertTrue(pasteboard.writes.isEmpty)
        }
    }

    func testPrimaryRegisterEffectRetainsTargetShapeAndExactPostTurnIdentity() throws {
        let (_, surface, session) = try makeSurface("abc")
        let host = CommandTurnHost()
        host.snapshots = [
            EVClipboardTurnSnapshot(
                target: UInt32(VIEM_CLIPBOARD_TARGET_CLIPBOARD),
                generation: 0,
                plainText: nil,
                isWritable: true
            ),
            EVClipboardTurnSnapshot(
                target: UInt32(VIEM_CLIPBOARD_TARGET_PRIMARY),
                generation: 0,
                plainText: nil,
                isWritable: true
            ),
        ]
        session.commandTurnHost = host

        try sendKeys("vl\"*y", through: session)

        let batch = try XCTUnwrap(host.batches.last)
        let write = try XCTUnwrap(batch.clipboardWrites.first)
        XCTAssertEqual(write.target, UInt32(VIEM_CLIPBOARD_TARGET_PRIMARY))
        XCTAssertEqual(write.registerKind, UInt32(VIEM_REGISTER_KIND_CHARACTER))
        XCTAssertEqual(write.plainText, "ab")
        XCTAssertEqual(write.documentID, surface.documentState.document_id)
        XCTAssertEqual(write.documentRevision, surface.documentState.document_revision)
    }

    func testNativePasteDispatchesClipboardRegisterIntentionsInNormalAndInsertModes() throws {
        do {
            let (_, surface, _) = try makeSurface("abc")
            let pasteboard = Pasteboard()
            pasteboard.text = "ZZ"
            surface.pasteboard = pasteboard

            surface.perform(menuCommand: .paste, sender: nil)

            XCTAssertEqual(surface.formattedText, "aZZbc")
        }

        do {
            let (_, surface, session) = try makeSurface("abc")
            let pasteboard = Pasteboard()
            pasteboard.text = "ZZ"
            surface.pasteboard = pasteboard
            try sendKey("i", through: session)
            surface.refreshPresentation()

            surface.perform(menuCommand: .paste, sender: nil)

            XCTAssertEqual(surface.formattedText, "ZZabc")
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
        }
    }

    func testClipboardWriteFailureStillReleasesOwnedBatch() throws {
        let (_, surface, session) = try makeSurface("abc")
        let pasteboard = Pasteboard()
        pasteboard.acceptsWrites = false
        surface.pasteboard = pasteboard

        try sendKeys("vl\"+", through: session)
        XCTAssertThrowsError(try sendKey("y", through: session)) { error in
            XCTAssertEqual(error as? EVCoreFrontendError, .pasteboardWriteFailed)
        }

        let counters = session.hostEffectAccessCounters
        XCTAssertEqual(counters.copyCalls, 1)
        XCTAssertEqual(counters.releaseCalls, 1)
        XCTAssertNotEqual(counters.lastReleasedHandle, 0)
        var info = ViemEffectBatchInfoV1()
        info.struct_size = UInt32(MemoryLayout<ViemEffectBatchInfoV1>.size)
        XCTAssertEqual(
            viem_effect_batch_info(counters.lastReleasedHandle, &info),
            UInt32(VIEM_STATUS_INVALID_HANDLE)
        )
    }

    func testTypedOptionsPrintAndSubstitutionResultsReachMessageSurface() throws {
        let (_, surface, session) = try makeSurface("one\ntwo")

        try sendEx("set ff?", through: session)
        XCTAssertEqual(try commandOutput(from: surface), "fileformat=unix")

        try sendEx("%s/o/O/gp", through: session)
        surface.refreshPresentation()
        XCTAssertEqual(surface.formattedText, "One\ntwO")
        XCTAssertEqual(try commandOutput(from: surface), "One\ntwO\n2 substitutions")
    }

    func testResolvedMarksRegistersAndJumpsRenderWithoutFrontendProjectionReads() throws {
        let (backend, surface, session) = try makeSurface("α one\nβ two\nγ three")
        try sendKeys("ma\"ayyG", through: session)
        backend.resetFormattedAccessCounters()

        try sendEx("marks a", through: session)
        XCTAssertTrue(try commandOutput(from: surface).contains("a  1  0  α one"))

        try sendEx("registers a", through: session)
        XCTAssertTrue(try commandOutput(from: surface).contains("\"a"))
        XCTAssertTrue(try commandOutput(from: surface).contains("α one^J"))

        try sendEx("jumps", through: session)
        XCTAssertTrue(try commandOutput(from: surface).contains("γ three"))
        XCTAssertTrue(try commandOutput(from: surface).contains(">"))
        XCTAssertEqual(
            backend.formattedAccessCounters.fullRangeReadCalls,
            0,
            "resolved Ex payloads must not make the frontend flatten the projection"
        )
    }

    func testFileRequestsPreserveKindFlagsPathRangeAndExactIdentity() throws {
        let (_, surface, session) = try makeSurface("body")
        let host = DocumentHost()
        surface.documentHostEffectHandler = host

        try sendEx("write", through: session)
        let write = try XCTUnwrap(host.requests.last?.first)
        XCTAssertEqual(write.kind, .write)
        XCTAssertNil(write.path)
        XCTAssertNil(write.hardLineRange)
        XCTAssertEqual(write.documentID, surface.documentState.document_id)
        XCTAssertEqual(write.documentRevision, surface.documentState.document_revision)

        try sendEx("saveas! nested/file.md", through: session)
        let saveAs = try XCTUnwrap(host.requests.last?.first)
        XCTAssertEqual(saveAs.kind, .saveAs)
        XCTAssertEqual(saveAs.path, "nested/file.md")
        XCTAssertTrue(saveAs.force)
    }

    func testHostFailureIsRenderedAndWriteQuitRemainsOneTypedRequest() throws {
        let (_, surface, session) = try makeSurface("body")
        let host = DocumentHost()
        host.result = .failure(EVDocumentHostError.saveCancelledOrFailed)
        surface.documentHostEffectHandler = host

        try sendEx("wq", through: session)

        XCTAssertEqual(host.requests.last?.map(\.kind), [.writeQuit])
        XCTAssertEqual(
            try commandOutput(from: surface),
            EVDocumentHostError.saveCancelledOrFailed.localizedDescription
        )
    }

    func testStaleAndInvalidBatchesAreRejectedBeforePasteboardMutation() throws {
        let (_, surface, session) = try makeSurface("abc")
        let pasteboard = Pasteboard()
        pasteboard.text = "original"
        surface.pasteboard = pasteboard
        let oldID = surface.documentState.document_id
        let oldRevision = surface.documentState.document_revision

        try sendKeys("ix", through: session)
        let stale = EVHostEffectBatch(
            documentID: oldID,
            documentRevision: oldRevision,
            flags: 0,
            navigationUTF8Offset: nil,
            navigationRestoresHistory: false,
            substitutionCount: 0,
            clipboardWrites: [],
            exEffects: []
        )
        XCTAssertThrowsError(try surface.applyHostEffectBatch(stale)) { error in
            XCTAssertEqual(error as? EVCoreFrontendError, .staleHostEffect)
        }

        let state = try surface.backend.documentState()
        let invalid = EVHostEffectBatch(
            documentID: state.document_id,
            documentRevision: state.document_revision,
            flags: 0,
            navigationUTF8Offset: nil,
            navigationRestoresHistory: false,
            substitutionCount: 0,
            clipboardWrites: [
                EVClipboardWriteEffect(
                    target: 999,
                    registerKind: UInt32(VIEM_REGISTER_KIND_CHARACTER),
                    documentID: state.document_id,
                    documentRevision: state.document_revision,
                    plainText: "bad",
                    hardBreaks: []
                ),
            ],
            exEffects: []
        )
        XCTAssertThrowsError(try surface.applyHostEffectBatch(invalid)) { error in
            XCTAssertEqual(error as? EVCoreFrontendError, .invalidHostEffect)
        }
        XCTAssertEqual(pasteboard.text, "original")
        XCTAssertTrue(pasteboard.writes.isEmpty)
    }

    private func commandOutput(from surface: EVEditorSurfaceController, file: StaticString = #filePath, line: UInt = #line) throws -> String {
        let output = try XCTUnwrap(surface.commandOutput, file: file, line: line)
        let bar = surface.editorView.commandOutputBar
        XCTAssertFalse(bar.isHidden, file: file, line: line)
        XCTAssertFalse(bar.textView.isEditable, file: file, line: line)
        XCTAssertTrue(bar.textView.isSelectable, file: file, line: line)
        XCTAssertEqual(bar.textView.string, output, file: file, line: line)
        XCTAssertEqual(surface.statusBarState.message, "", "output is not duplicated in the status bar", file: file, line: line)
        let close = try XCTUnwrap(bar.subviews.compactMap { $0 as? NSButton }.first, file: file, line: line)
        XCTAssertEqual(close.accessibilityLabel(), "Close command output", file: file, line: line)
        XCTAssertFalse(close.isHidden, file: file, line: line)
        XCTAssertNotNil(close.image, file: file, line: line)
        return output
    }

    private func makeSurface(
        _ text: String
    ) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(text.utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 520, height: 260)
        surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }

    private func sendEx(_ command: String, through session: EVCoreViewSession) throws {
        try sendKey(":", through: session)
        _ = try session.sendText(command)
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_ENTER))
    }

    private func sendKeys(_ keys: String, through session: EVCoreViewSession) throws {
        for key in keys { try sendKey(key, through: session) }
    }

    private func sendKey(_ key: Character, through session: EVCoreViewSession) throws {
        let scalar = try XCTUnwrap(key.unicodeScalars.first)
        _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: scalar.value)
    }
}
