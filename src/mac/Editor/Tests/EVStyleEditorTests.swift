import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

final class EVStyleEditorTests: XCTestCase {

    @MainActor
    func testCoordinatorReusesOneModelessWindowAndRetargetsByStableIdentity() throws {
        let firstBackend = EVCoreDocumentBackend()
        try firstBackend.read(source: Data("first".utf8), typeName: "public.plain-text")
        let firstSurface = try XCTUnwrap(firstBackend.makeEditorSurface() as? EVEditorSurfaceController)
        firstSurface.loadViewIfNeeded()

        let secondBackend = EVCoreDocumentBackend()
        try secondBackend.read(source: Data("# second".utf8), typeName: "public.markdown")
        let secondSurface = try XCTUnwrap(secondBackend.makeEditorSurface() as? EVEditorSurfaceController)
        secondSurface.loadViewIfNeeded()

        let coordinator = EVStyleEditorCoordinator()
        coordinator.show(document: firstSurface, sender: nil)
        defer { coordinator.close() }
        let originalWindow = try XCTUnwrap(coordinator.styleWindow)

        XCTAssertTrue(originalWindow is NSPanel)
        XCTAssertEqual(originalWindow.title, "Theme styles — Plain Text — \(firstBackend.configuration.currentThemeName ?? "Default")")
        XCTAssertTrue(originalWindow.styleMask.contains(.titled))
        XCTAssertTrue(originalWindow.styleMask.contains(.utilityWindow))
        XCTAssertTrue(originalWindow.styleMask.contains(.resizable))
        XCTAssertTrue(originalWindow.canBecomeMain)
        XCTAssertNil(originalWindow.sheetParent)
        XCTAssertEqual(originalWindow.level, .normal)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, EVStyleKey.baseParagraph)
        XCTAssertNotEqual(coordinator.inspection?.targetCoreDocumentID, try firstBackend.documentState().document_id)

        coordinator.show(document: secondSurface, sender: nil)

        XCTAssertTrue(coordinator.styleWindow === originalWindow)
        XCTAssertEqual(originalWindow.title, "Theme styles — Markdown — \(secondBackend.configuration.currentThemeName ?? "Default")")
        XCTAssertNil(coordinator.inspection?.targetDocumentIdentity)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1")))
        XCTAssertNotEqual(coordinator.inspection?.targetCoreDocumentID, try secondBackend.documentState().document_id)
    }

    @MainActor
    func testStyleMutationIsCoreOwnedSourcePreservingMultiViewAndUndoable() throws {
        let configuration = try styleConfiguration()
        let backend = EVCoreDocumentBackend(configuration: configuration)
        let source = Data("# Heading\nbody".utf8)
        try backend.read(source: source, typeName: "public.markdown")
        let first = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let second = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        first.loadViewIfNeeded()
        second.loadViewIfNeeded()
        let session = try XCTUnwrap(first.session)
        let editor = EVStyleEditorViewController()
        editor.retarget(
            document: first,
            styleKey: EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        )
        let secondRefreshBefore = second.presentationRefreshCount

        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(31)))
        let edited = try backend.styleSheetSnapshot()
        let heading = try XCTUnwrap(edited.definition(
            namespace: .block,
            id: EVStyleID(rawValue: "Heading1")
        ))
        XCTAssertEqual(heading.properties[.characterSize]?.declared, .float(31))
        XCTAssertEqual(try backend.serializedSource(typeName: "public.markdown"), source)
        XCTAssertGreaterThan(second.presentationRefreshCount, secondRefreshBefore)

        _ = try session.undo()

        let undone = try backend.styleSheetSnapshot()
        XCTAssertEqual(
            undone.definition(namespace: .block, id: EVStyleID(rawValue: "Heading1"))?
                .properties[.characterSize]?.declared,
            .float(24)
        )

        _ = try session.redo()
        XCTAssertEqual(
            try backend.styleSheetSnapshot()
                .definition(namespace: .block, id: EVStyleID(rawValue: "Heading1"))?
                .properties[.characterSize]?.declared,
            .float(31)
        )
    }

    @MainActor
    func testContinuousStyleFieldEditsCoalesceIntoOneCoreUndoUnit() throws {
        let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        let (backend, surface, editor) = try makeEditor(source: "# Heading", style: heading)
        let session = try XCTUnwrap(surface.session)
        defer { withExtendedLifetime(surface) {} }

        editor.beginContinuousStyleEditForTesting()
        XCTAssertTrue(editor.hasActiveStyleEditGroupForTesting)
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(28)))
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(31)))
        editor.endContinuousStyleEditForTesting()
        XCTAssertFalse(editor.hasActiveStyleEditGroupForTesting)
        XCTAssertEqual(
            try backend.styleSheetSnapshot().definition(for: heading)?
                .properties[.characterSize]?.declared,
            .float(31)
        )

        _ = try session.undo()
        XCTAssertEqual(
            try backend.styleSheetSnapshot().definition(for: heading)?
                .properties[.characterSize]?.declared,
            .float(24),
            "one Undo must reverse the complete live field edit"
        )

        _ = try session.redo()
        XCTAssertEqual(
            try backend.styleSheetSnapshot().definition(for: heading)?
                .properties[.characterSize]?.declared,
            .float(31)
        )
    }

    @MainActor
    func testStaleCallbackIsRejectedAndRefreshesCommittedValues() throws {
        let (backend, surface, editor) = try makeEditor(
            source: "# Heading",
            style: EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        )
        defer { withExtendedLifetime(surface) {} }
        let stale = try XCTUnwrap(editor.inspection.styleSheetRevision).flatMapIdentity(
            documentID: try XCTUnwrap(editor.inspection.targetCoreDocumentID),
            documentRevision: try backend.documentState().document_revision
        )
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(30)))

        XCTAssertFalse(editor.submitStaleMutationForTesting(
            expected: stale,
            mutation: .setDeclaration(.characterSize, .float(99))
        ))
        XCTAssertTrue(editor.inspection.diagnostic.contains("changed before"))
        XCTAssertEqual(
            try backend.styleSheetSnapshot()
                .definition(namespace: .block, id: EVStyleID(rawValue: "Heading1"))?
                .properties[.characterSize]?.declared,
            .float(30)
        )
    }

    @MainActor
    private func descendants(of view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants(of: $0) }
    }

    @MainActor
    private func makeEditor(
        source: String,
        style: EVStyleKey
    ) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVStyleEditorViewController) {
        let configuration = try styleConfiguration()
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data(source.utf8), typeName: "public.markdown")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(document: surface, styleKey: style)
        return (backend, surface, editor)
    }

    @MainActor
    private func styleConfiguration() throws -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-style-editor-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        try EVStyleTestFixtures.configure(configuration, declarations: [
            (.baseParagraph, .characterSize, .float(14)),
            (.baseParagraph, .characterFontFamilies, .stringList(["system-ui"])),
            (.baseParagraph, .characterFontAxes, .string("{}")),
            (.baseParagraph, .characterWeight, .unsigned(400)),
            (.baseParagraph, .paragraphFirstLineIndent, .float(0)),
            (.baseParagraph, .blockMarginTop, .float(0)),
            (.baseParagraph, .blockMarginBottom, .float(0)),
            (.baseParagraph, .paragraphLineSpacing, .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_NORMAL), value: 0))),
            (.baseParagraph, .paragraphAlignment, .paragraphAlignment(UInt32(VIEM_STYLE_PARAGRAPH_ALIGNMENT_START))),
            (EVStyleTestFixtures.block("Heading1"), .characterSize, .float(24)),
            (EVStyleTestFixtures.block("Heading1"), .characterFontFamilies, nil),
        ])
        return configuration
    }
}

private extension UInt64 {
    func flatMapIdentity(documentID: UInt64, documentRevision: UInt64) -> EVStyleSheetIdentity {
        EVStyleSheetIdentity(
            documentID: documentID,
            documentRevision: documentRevision,
            styleSheetRevision: self
        )
    }
}
