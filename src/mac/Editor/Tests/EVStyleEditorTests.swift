import AppKit
import CEvimCore
import XCTest

@testable import EvimEditor

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
        coordinator.show(document: firstSurface, preferredStyle: .paragraph, sender: nil)
        defer { coordinator.close() }
        let originalWindow = try XCTUnwrap(coordinator.styleWindow)

        XCTAssertFalse(originalWindow is NSPanel)
        XCTAssertTrue(originalWindow.styleMask.contains(.titled))
        XCTAssertTrue(originalWindow.styleMask.contains(.resizable))
        XCTAssertNil(originalWindow.sheetParent)
        XCTAssertEqual(originalWindow.level, .normal)
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, EVStyleKey.baseParagraph)
        XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, try firstBackend.documentState().document_id)

        coordinator.show(document: secondSurface, preferredStyle: .character, sender: nil)

        XCTAssertTrue(coordinator.styleWindow === originalWindow)
        XCTAssertEqual(coordinator.inspection?.targetDocumentIdentity, ObjectIdentifier(secondBackend))
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, EVStyleKey.baseCharacter)
        XCTAssertEqual(coordinator.inspection?.targetCoreDocumentID, try secondBackend.documentState().document_id)
    }

    @MainActor
    func testLiveCatalogueContainsEveryDefinitionAndEveryEditableProperty() throws {
        let (backend, surface, editor) = try makeEditor(source: "# Heading", style: EVStyleKey.baseParagraph)
        _ = backend
        defer { withExtendedLifetime(surface) {} }

        XCTAssertEqual(editor.inspection.styleCount, 9)
        XCTAssertEqual(editor.inspection.selectedKind, .paragraph)
        XCTAssertEqual(editor.inspection.characterPropertyCount, 13)
        XCTAssertEqual(editor.inspection.paragraphPropertyCount, 8)
        XCTAssertTrue(editor.inspection.paragraphTabEnabled)
        XCTAssertTrue(editor.inspection.mutationsEnabled)
        XCTAssertTrue(editor.inspection.nameEditable)
        XCTAssertTrue(editor.inspection.summary.contains("contributions"))
        XCTAssertTrue(editor.inspection.summary.contains("Following paragraph style"))

        editor.selectStyle(EVStyleKey.baseDocument)
        XCTAssertEqual(editor.inspection.selectedStyleKey, EVStyleKey.baseDocument)
        XCTAssertEqual(editor.inspection.characterPropertyCount, 13)
        XCTAssertEqual(editor.inspection.paragraphPropertyCount, 0)
    }

    @MainActor
    func testStyleMutationIsCoreOwnedSourcePreservingMultiViewAndUndoable() throws {
        let backend = EVCoreDocumentBackend()
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
        XCTAssertTrue(editor.inspection.summary.contains("Font size: 31 pt"))

        _ = try session.undo()

        let undone = try backend.styleSheetSnapshot()
        XCTAssertEqual(
            undone.definition(namespace: .block, id: EVStyleID(rawValue: "Heading1"))?
                .properties[.characterSize]?.declared,
            .float(24)
        )
        XCTAssertTrue(editor.inspection.summary.contains("Font size: 24 pt"))

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
    func testExplicitNormalValueAndUseInheritedRemainDistinct() throws {
        let (backend, surface, editor) = try makeEditor(
            source: "# Heading",
            style: EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        )
        defer { withExtendedLifetime(surface) {} }
        XCTAssertFalse(editor.inspection.declaredProperties.contains(.characterUnderline))
        XCTAssertTrue(editor.inspection.summary.contains("Underline: Off — Inherited · Base Document"))

        XCTAssertTrue(editor.setPropertyForTesting(.characterUnderline, value: .boolean(false)))
        XCTAssertTrue(editor.inspection.declaredProperties.contains(.characterUnderline))
        XCTAssertEqual(
            try backend.styleSheetSnapshot()
                .definition(namespace: .block, id: EVStyleID(rawValue: "Heading1"))?
                .properties[.characterUnderline]?.declared,
            .boolean(false)
        )

        XCTAssertTrue(editor.useInheritedForTesting(.characterUnderline))
        XCTAssertFalse(editor.inspection.declaredProperties.contains(.characterUnderline))
        XCTAssertTrue(editor.inspection.summary.contains("Underline: Off — Inherited · Base Document"))
    }

    @MainActor
    func testTabsAndBaseParentRestrictionsFollowCoreRolesAndCapabilities() throws {
        let (_, surface, editor) = try makeEditor(source: "text", style: EVStyleKey.baseParagraph)
        defer { withExtendedLifetime(surface) {} }

        XCTAssertTrue(editor.inspection.paragraphTabEnabled)
        XCTAssertEqual(editor.inspection.parentValue, "Base Document")
        XCTAssertTrue(editor.inspection.parentChoices.isEmpty)
        editor.selectTab(.paragraph)
        XCTAssertEqual(editor.inspection.selectedTab, .paragraph)

        editor.selectStyle(EVStyleKey.baseDocument)
        XCTAssertFalse(editor.inspection.paragraphTabEnabled)
        XCTAssertEqual(editor.inspection.selectedTab, .character)
        XCTAssertEqual(editor.inspection.parentValue, "None")
        XCTAssertTrue(editor.inspection.parentChoices.isEmpty)

        editor.selectStyle(EVStyleKey.baseCharacter)
        XCTAssertFalse(editor.inspection.paragraphTabEnabled)
        XCTAssertEqual(editor.inspection.parentValue, "None")
        XCTAssertTrue(editor.inspection.parentChoices.isEmpty)

        let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        editor.selectStyle(heading)
        XCTAssertTrue(editor.inspection.paragraphTabEnabled)
        XCTAssertFalse(editor.inspection.parentChoices.contains(heading))
        XCTAssertTrue(editor.inspection.parentChoices.contains(EVStyleKey.baseParagraph))
        XCTAssertFalse(editor.inspection.parentChoices.contains(EVStyleKey.baseDocument))
    }

    @MainActor
    func testDisplayNameAndFollowingStyleCommitWithoutChangingStableSelection() throws {
        let (backend, surface, editor) = try makeEditor(
            source: "# Heading",
            style: EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        )
        defer { withExtendedLifetime(surface) {} }
        XCTAssertTrue(editor.renameForTesting("Chapter Heading"))
        XCTAssertEqual(editor.inspection.selectedStyleID, EVStyleID(rawValue: "Heading1"))
        XCTAssertTrue(editor.inspection.summary.contains("Style: Chapter Heading"))
        XCTAssertEqual(
            try backend.styleSheetSnapshot()
                .definition(namespace: .block, id: EVStyleID(rawValue: "Heading1"))?.name,
            "Chapter Heading"
        )

        XCTAssertTrue(editor.setFollowingStyleForTesting(nil))
        XCTAssertNil(
            try backend.styleSheetSnapshot()
                .definition(namespace: .block, id: EVStyleID(rawValue: "Heading1"))?.nextStyleID
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
        XCTAssertTrue(editor.inspection.summary.contains("Font size: 30 pt"))
    }

    @MainActor
    func testInvalidNameDraftDoesNotCommit() throws {
        let (backend, surface, editor) = try makeEditor(
            source: "# Heading",
            style: EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        )
        defer { withExtendedLifetime(surface) {} }
        let before = try backend.documentState().document_revision

        editor.enterNameDraftForTesting("   ")

        XCTAssertTrue(editor.inspection.hasInvalidDraft)
        XCTAssertEqual(try backend.documentState().document_revision, before)
        XCTAssertEqual(
            try backend.styleSheetSnapshot()
                .definition(namespace: .block, id: EVStyleID(rawValue: "Heading1"))?.name,
            "Heading 1"
        )
    }

    @MainActor
    func testTargetWindowClosureDisablesEditorThroughLifecycleObserver() throws {
        let backend = EVCoreDocumentBackend()
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let documentWindow = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 480),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        documentWindow.contentViewController = surface
        let coordinator = EVStyleEditorCoordinator()
        coordinator.show(document: surface, preferredStyle: .paragraph, sender: nil)
        defer { coordinator.close() }
        XCTAssertTrue(coordinator.inspection?.hasDocument == true)

        NotificationCenter.default.post(name: NSWindow.willCloseNotification, object: documentWindow)

        XCTAssertFalse(coordinator.inspection?.hasDocument ?? true)
        XCTAssertNil(coordinator.inspection?.selectedStyleKey)
        XCTAssertTrue(coordinator.inspection?.summary.contains("No document") == true)
        XCTAssertNotNil(coordinator.styleWindow)
    }

    @MainActor
    func testClosingTargetRetargetsBaseParagraphInCurrentOtherDocument() throws {
        let firstBackend = EVCoreDocumentBackend()
        let secondBackend = EVCoreDocumentBackend()
        let first = try XCTUnwrap(firstBackend.makeEditorSurface() as? EVEditorSurfaceController)
        let second = try XCTUnwrap(secondBackend.makeEditorSurface() as? EVEditorSurfaceController)
        first.loadViewIfNeeded()
        second.loadViewIfNeeded()
        let coordinator = EVStyleEditorCoordinator { closing in
            closing === first ? second : nil
        }
        coordinator.show(document: first, preferredStyle: .character, sender: nil)
        defer { coordinator.close() }
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseCharacter)

        coordinator.documentDidClose(first)

        XCTAssertEqual(coordinator.inspection?.targetDocumentIdentity, ObjectIdentifier(secondBackend))
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, .baseParagraph)
        XCTAssertEqual(coordinator.inspection?.selectedKind, .paragraph)
    }

    @MainActor
    func testClosingOneViewRetargetsToAnotherViewOfSameDocument() throws {
        let backend = EVCoreDocumentBackend()
        let first = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        let second = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        first.loadViewIfNeeded()
        second.loadViewIfNeeded()
        let firstWindow = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 500, height: 400),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        let secondWindow = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 500, height: 400),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        firstWindow.contentViewController = first
        secondWindow.contentViewController = second
        let coordinator = EVStyleEditorCoordinator()
        coordinator.show(document: first, preferredStyle: .paragraph, sender: nil)
        defer { coordinator.close() }

        NotificationCenter.default.post(name: NSWindow.willCloseNotification, object: firstWindow)

        XCTAssertTrue(coordinator.inspection?.hasDocument == true)
        XCTAssertEqual(coordinator.inspection?.targetDocumentIdentity, ObjectIdentifier(backend))
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, EVStyleKey.baseParagraph)
    }

    @MainActor
    func testModelessWindowDoesNotPreventTargetDocumentEditing() throws {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data("world".utf8), typeName: "public.plain-text")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let session = try XCTUnwrap(surface.session)
        let coordinator = EVStyleEditorCoordinator()
        coordinator.show(document: surface, preferredStyle: .paragraph, sender: nil)
        defer { coordinator.close() }

        _ = try session.sendText("i")
        _ = try session.sendText("Hello ")

        XCTAssertEqual(try backend.formattedText(), "Hello world")
        XCTAssertEqual(coordinator.inspection?.selectedStyleKey, EVStyleKey.baseParagraph)
    }

    @MainActor
    func testCoreTextPreviewReceivesExactCommittedEffectiveValuesIncludingInheritance() throws {
        let headingKey = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))
        let (backend, surface, editor) = try makeEditor(source: "# Heading", style: headingKey)
        defer { withExtendedLifetime(surface) {} }
        let definition = try XCTUnwrap(try backend.styleSheetSnapshot().definition(for: headingKey))
        let expectedValues = definition.properties.reduce(into: [EVStyleProperty: EVStyleValue]()) {
            values, entry in
            if let effective = entry.value.effective { values[entry.key] = effective }
        }

        XCTAssertEqual(editor.inspection.preview.effectiveValues, expectedValues)
        XCTAssertNil(definition.properties[.characterFontFamilies]?.declared)
        XCTAssertEqual(editor.inspection.preview.requestedFontFamilies, ["SF Pro"])
        XCTAssertEqual(editor.inspection.preview.kind, .paragraph)
        XCTAssertTrue(editor.inspection.preview.accessibilityText.contains("Previous paragraph"))
        XCTAssertTrue(editor.inspection.preview.accessibilityText.contains("Following paragraph"))
    }

    @MainActor
    func testCoreTextPreviewUsesSFProFourteenPointDocumentDefault() throws {
        let (_, surface, editor) = try makeEditor(source: "text", style: EVStyleKey.baseParagraph)
        defer { withExtendedLifetime(surface) {} }
        let preview = editor.previewInspectionForTesting(
            layoutSize: CGSize(width: 560, height: 300)
        )

        XCTAssertEqual(preview.requestedFontFamilies, ["SF Pro"])
        XCTAssertEqual(preview.requestedFontSize, 14)
        XCTAssertEqual(preview.resolvedFontFamily, "SF Pro")
        XCTAssertEqual(preview.resolvedFontSize, 14, accuracy: 0.001)
        XCTAssertEqual(
            preview.canvasBackground,
            EVStyleColor(red: 1, green: 1, blue: 1, alpha: 1),
            "the preview uses the document canvas instead of the window's dark appearance"
        )
        XCTAssertGreaterThanOrEqual(preview.currentStyleLines.count, 2)
        XCTAssertGreaterThanOrEqual(preview.lines.filter { !$0.isCurrentStyle }.count, 2)
    }

    @MainActor
    func testCharacterPropertyScrollerStartsWithControlsVisible() throws {
        let (_, surface, editor) = try makeEditor(source: "text", style: EVStyleKey.baseParagraph)
        defer { withExtendedLifetime(surface) {} }
        editor.view.frame = NSRect(x: 0, y: 0, width: 760, height: 820)
        editor.view.layoutSubtreeIfNeeded()

        XCTAssertGreaterThan(
            editor.visiblePropertyRowCountForTesting,
            0,
            "a newly opened style editor must not initialize its property document view offscreen"
        )
    }

    @MainActor
    func testDarkAppearancePropagatesToEditorAndKeepsPreviewCanvasLight() throws {
        let (_, surface, editor) = try makeEditor(source: "text", style: EVStyleKey.baseParagraph)
        defer { withExtendedLifetime(surface) {} }
        let panel = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 760, height: 820),
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: false
        )
        panel.appearance = try XCTUnwrap(NSAppearance(named: .darkAqua))
        panel.contentViewController = editor
        panel.contentView?.layoutSubtreeIfNeeded()
        editor.view.layoutSubtreeIfNeeded()
        panel.displayIfNeeded()
        editor.view.displayIfNeeded()

        XCTAssertEqual(
            editor.view.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]),
            .darkAqua
        )
        XCTAssertEqual(
            editor.inspection.preview.canvasBackground,
            EVStyleColor(red: 1, green: 1, blue: 1, alpha: 1),
            "the document preview remains a white canvas independent of window appearance"
        )
    }

    @MainActor
    func testParagraphPreviewCoreTextGeometryRespondsToResolvedLayoutProperties() throws {
        let (_, surface, editor) = try makeEditor(source: "text", style: EVStyleKey.baseParagraph)
        defer { withExtendedLifetime(surface) {} }
        let layoutSize = CGSize(width: 560, height: 320)
        let initial = editor.previewInspectionForTesting(layoutSize: layoutSize)
        let initialCurrent = initial.currentStyleLines
        let initialContext = initial.lines.filter { !$0.isCurrentStyle }
        XCTAssertGreaterThanOrEqual(initialCurrent.count, 2)
        XCTAssertGreaterThanOrEqual(initialContext.count, 2)

        XCTAssertTrue(editor.setPropertyForTesting(.paragraphFirstLineIndent, value: .float(36)))
        let indented = editor.previewInspectionForTesting(layoutSize: layoutSize)
        XCTAssertNotEqual(
            try XCTUnwrap(indented.currentStyleLines.first).origin.x,
            try XCTUnwrap(initialCurrent.first).origin.x,
            accuracy: 0.01
        )

        XCTAssertTrue(editor.setPropertyForTesting(.paragraphSpacingBefore, value: .float(24)))
        let spacedBefore = editor.previewInspectionForTesting(layoutSize: layoutSize)
        XCTAssertNotEqual(
            try XCTUnwrap(spacedBefore.currentStyleLines.first).origin.y,
            try XCTUnwrap(indented.currentStyleLines.first).origin.y,
            accuracy: 0.01
        )

        XCTAssertTrue(editor.setPropertyForTesting(
            .paragraphLineSpacing,
            value: .lineSpacing(EVLineSpacing(
                kind: UInt32(EVIM_STYLE_LINE_SPACING_EXACT),
                value: 34
            ))
        ))
        let exactLineSpacing = editor.previewInspectionForTesting(layoutSize: layoutSize)
        let priorFirstLine = try XCTUnwrap(spacedBefore.currentStyleLines.first)
        let priorSecondLine = try XCTUnwrap(spacedBefore.currentStyleLines.dropFirst().first)
        let exactFirstLine = try XCTUnwrap(exactLineSpacing.currentStyleLines.first)
        let exactSecondLine = try XCTUnwrap(exactLineSpacing.currentStyleLines.dropFirst().first)
        let priorLineGap = abs(priorFirstLine.origin.y - priorSecondLine.origin.y)
        let exactLineGap = abs(exactFirstLine.origin.y - exactSecondLine.origin.y)
        XCTAssertNotEqual(exactLineGap, priorLineGap, accuracy: 0.01)
        XCTAssertEqual(exactLineGap, 34, accuracy: 0.5)

        XCTAssertTrue(editor.setPropertyForTesting(.paragraphSpacingAfter, value: .float(22)))
        let spacedAfter = editor.previewInspectionForTesting(layoutSize: layoutSize)
        let beforeFollowing = try XCTUnwrap(exactLineSpacing.lines.last(where: { !$0.isCurrentStyle }))
        let afterFollowing = try XCTUnwrap(spacedAfter.lines.last(where: { !$0.isCurrentStyle }))
        XCTAssertNotEqual(afterFollowing.origin.y, beforeFollowing.origin.y, accuracy: 0.01)

        XCTAssertTrue(editor.setPropertyForTesting(
            .paragraphAlignment,
            value: .paragraphAlignment(UInt32(EVIM_STYLE_PARAGRAPH_ALIGNMENT_CENTER))
        ))
        let centered = editor.previewInspectionForTesting(layoutSize: layoutSize)
        XCTAssertNotEqual(
            try XCTUnwrap(centered.currentStyleLines.first).origin.x,
            try XCTUnwrap(spacedAfter.currentStyleLines.first).origin.x,
            accuracy: 0.01
        )
    }

    @MainActor
    private func makeEditor(
        source: String,
        style: EVStyleKey
    ) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVStyleEditorViewController) {
        let backend = EVCoreDocumentBackend()
        try backend.read(source: Data(source.utf8), typeName: "public.markdown")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let editor = EVStyleEditorViewController()
        editor.retarget(document: surface, styleKey: style)
        return (backend, surface, editor)
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
