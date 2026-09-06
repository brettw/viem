import AppKit
import EvimAppShell
import XCTest

@testable import EvimEditor

final class EVStyleMenuBridgeTests: XCTestCase {
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
        XCTAssertEqual(Set(catalogue.entries.filter { $0.role == .paragraph }.map(\.stableID)), [
            "Paragraph", "Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6",
        ])
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
