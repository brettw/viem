import AppKit
import EvimAppShell
import XCTest

@testable import EvimEditor

final class EVStyleMenuBridgeTests: XCTestCase {
    @MainActor
    func testHeadingMenuWritesFormatMarkersAndRejectsStaleActions() throws {
        for (type, source, expected) in [
            (EVDocument.markdownSourceType, "Paragraph", "## Paragraph"),
            (EVDocument.markdownType, "Paragraph", "## Paragraph"),
            (EVDocument.htmlType, "<p data-keep='yes'>Paragraph</p>", "<h2 data-keep='yes'>Paragraph</h2>"),
        ] {
            let backend = EVCoreDocumentBackend()
            try backend.read(source: Data(source.utf8), typeName: type)
            let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
            surface.loadViewIfNeeded()
            let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
            XCTAssertTrue(try XCTUnwrap(catalogue.entries.first { $0.stableID == "Heading2" }).presentation.isEnabled)
            let action = EVStyleMenuAction(kind: .assign, role: .paragraph, stableID: "Heading2",
                documentID: catalogue.documentID, documentRevision: catalogue.documentRevision,
                styleSheetRevision: catalogue.styleSheetRevision)
            surface.perform(styleMenuAction: action, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(expected.utf8))
            XCTAssertEqual(surface.statusBarState.message, "")
            let stale = EVStyleMenuAction(kind: .assign, role: .paragraph, stableID: "Paragraph",
                documentID: catalogue.documentID, documentRevision: catalogue.documentRevision,
                styleSheetRevision: catalogue.styleSheetRevision)
            surface.perform(styleMenuAction: stale, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(expected.utf8))
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try backend.serializedSource(typeName: type), Data(source.utf8))
        }
    }

    @MainActor
    func testLiveCoreCatalogueExposesEveryRoleAndDisablesUnsupportedPlainTextAssignment() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("plain text".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()

        let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        let coreSnapshot = try backend.styleSheetSnapshot()

        XCTAssertEqual(catalogue.documentID, coreSnapshot.identity.documentID)
        XCTAssertEqual(catalogue.documentRevision, coreSnapshot.identity.documentRevision)
        XCTAssertEqual(catalogue.styleSheetRevision, coreSnapshot.identity.styleSheetRevision)
        XCTAssertEqual(catalogue.entries.count, coreSnapshot.definitions.count)
        XCTAssertEqual(catalogue.entries.filter { $0.role == .character }.map(\.stableID), [
            "Character",
        ])
        XCTAssertEqual(Set(catalogue.entries.filter { $0.role == .paragraph }.map(\.stableID)), Set([
            "Paragraph", "Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6",
        ] + (1...16).map { "List\($0)" }))
        XCTAssertEqual(catalogue.entries.filter { $0.role == .document }.map(\.stableID), [
            "Document",
        ])
        XCTAssertTrue(catalogue.entries.allSatisfy { !$0.presentation.isEnabled })
        XCTAssertTrue(catalogue.entries.allSatisfy { $0.presentation.state == .off })
        XCTAssertTrue(catalogue.canEditStyles)
    }

    @MainActor
    func testEditActionRetargetsSingletonToExactStableStyleIdentity() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("# Heading".utf8), typeName: "public.markdown")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        let coordinator = EVStyleEditorCoordinator.shared
        coordinator.close()
        defer { coordinator.close() }

        surface.perform(
            styleMenuAction: EVStyleMenuAction(
                kind: .edit,
                role: .paragraph,
                stableID: "Heading2",
                documentID: catalogue.documentID,
                documentRevision: catalogue.documentRevision,
                styleSheetRevision: catalogue.styleSheetRevision
            ),
            sender: nil
        )

        XCTAssertEqual(
            coordinator.inspection?.selectedStyleKey,
            EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading2"))
        )
        XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, catalogue.documentID)
    }

    @MainActor
    func testAssignmentPayloadCannotBypassDisabledAdapterCapability() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("plain text".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let before = try backend.nativeSaveSnapshot(typeName: "public.plain-text")
        let catalogue = try XCTUnwrap(surface.currentStyleMenuCatalogue())
        let coordinator = EVStyleEditorCoordinator.shared
        coordinator.close()

        surface.perform(
            styleMenuAction: EVStyleMenuAction(
                kind: .assign,
                role: .paragraph,
                stableID: "Heading1",
                documentID: catalogue.documentID,
                documentRevision: catalogue.documentRevision,
                styleSheetRevision: catalogue.styleSheetRevision
            ),
            sender: nil
        )

        XCTAssertEqual(try backend.nativeSaveSnapshot(typeName: "public.plain-text"), before)
        XCTAssertNil(coordinator.styleWindow)
    }
}
