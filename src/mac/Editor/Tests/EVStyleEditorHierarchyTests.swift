import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVStyleEditorHierarchyTests: XCTestCase {
    private let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))

    func testParentButtonsTraverseFixedBaseStylesWithoutChangingDocumentOrHistory() throws {
        let (backend, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let before = try backend.recoverySnapshot()
        let identity = try backend.styleSheetSnapshot().identity
        let parent = try button("Edit based on style", in: editor)

        XCTAssertTrue(parent.isEnabled)
        editor.selectTab(.paragraph)
        parent.performClick(nil)
        XCTAssertEqual(editor.inspection.selectedStyleKey, .baseParagraph)
        XCTAssertTrue(editor.inspection.parentChoices.isEmpty,
                      "A fixed parent can be inspected even though it cannot be changed")
        XCTAssertFalse(parent.isEnabled, "Base Paragraph is the style root")
        parent.performClick(nil)
        XCTAssertEqual(editor.inspection.selectedStyleKey, .baseParagraph)

        editor.selectStyle(EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Code")))
        XCTAssertEqual(editor.inspection.parentValue, "Default Paragraph")
        XCTAssertTrue(editor.inspection.parentChoices.contains(.defaultParagraph))
        XCTAssertFalse(parent.isEnabled, "The contextual paragraph default is not an editable definition")
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertEqual(try backend.styleSheetSnapshot().identity, identity)
        XCTAssertFalse(surface.canUndo)
    }

    func testNextParagraphButtonNavigatesOnlyARealDifferentParagraph() throws {
        let (backend, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let target = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading2"))
        XCTAssertTrue(editor.setFollowingStyleForTesting(target), editor.inspection.diagnostic)
        let before = try backend.recoverySnapshot()
        let identity = try backend.styleSheetSnapshot().identity
        let next = try button("Edit next paragraph style", in: editor)

        XCTAssertTrue(next.isEnabled)
        next.performClick(nil)
        XCTAssertEqual(editor.inspection.selectedStyleKey, target)
        XCTAssertEqual(try backend.recoverySnapshot(), before)
        XCTAssertEqual(try backend.styleSheetSnapshot().identity, identity)

        editor.selectStyle(heading)
        XCTAssertTrue(editor.setFollowingStyleForTesting(nil))
        XCTAssertFalse(next.isEnabled, "Same Style has no other style to navigate to")
        editor.selectStyle(EVStyleKey(namespace: .character, id: EVStyleID(rawValue: "Code")))
        XCTAssertFalse(next.isEnabled)
        editor.disableForClosedDocument()
        XCTAssertFalse(next.isEnabled)
        XCTAssertFalse(try button("Edit based on style", in: editor).isEnabled)
    }

    func testNavigatingAfterLiveFieldEditsKeepsCommittedValuesAndClosesOneUndoGroup() throws {
        let (backend, surface, editor) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let session = try XCTUnwrap(surface.session)
        editor.beginContinuousStyleEditForTesting()
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(26)))
        XCTAssertTrue(editor.setPropertyForTesting(.characterSize, value: .float(29)))
        try button("Edit based on style", in: editor).performClick(nil)

        XCTAssertEqual(editor.inspection.selectedStyleKey, .baseParagraph)
        XCTAssertFalse(editor.hasActiveStyleEditGroupForTesting)
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: heading)?
            .properties[.characterSize]?.declared, .float(29))
        _ = try session.undo()
        XCTAssertEqual(try backend.styleSheetSnapshot().definition(for: heading)?
            .properties[.characterSize]?.declared, .float(24))
        XCTAssertFalse(surface.canUndo, "Navigation must not introduce another undo operation")
    }

    func testGlobalCodeHierarchyNavigationDoesNotPersistOrAddSettingsUndo() throws {
        let configuration = isolatedConfiguration()
        let session = try EVCodeStyleSession(configuration: configuration)
        let snapshot = try session.snapshot()
        let comment = try XCTUnwrap(snapshot.definitions.first { $0.name == "Comment.documentation" })
        let parentKey = try XCTUnwrap(comment.parentKey)
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(settingsSession: session)
        editor.selectStyle(comment.key)
        let persisted = try configuration.codeStyleSheet()

        let parent = try button("Edit based on style", in: editor)
        XCTAssertTrue(parent.isEnabled)
        parent.performClick(nil)
        XCTAssertEqual(editor.inspection.selectedStyleKey, parentKey)
        XCTAssertEqual(try session.snapshot().identity, snapshot.identity)
        XCTAssertEqual(try configuration.codeStyleSheet(), persisted)
        XCTAssertFalse(session.undoManager.canUndo)
        XCTAssertFalse(try button("Edit next paragraph style", in: editor).isEnabled)
    }

    func testHeaderLabelsAlignWithNativeControlsAndNavigationButtonsStayBesidePopups() throws {
        let (_, surface, _) = try makeEditor()
        defer { withExtendedLifetime(surface) {} }
        let coordinator = EVStyleEditorCoordinator()
        coordinator.show(document: surface, sender: nil)
        defer { coordinator.close() }
        let window = try XCTUnwrap(coordinator.styleWindow)
        let editor = try XCTUnwrap(window.contentViewController as? EVStyleEditorViewController)
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            window.appearance = try XCTUnwrap(NSAppearance(named: appearance))
            for (width, height) in [(CGFloat(700), CGFloat(545)), (1000, 820)] {
                window.setContentSize(NSSize(width: width, height: height))
                editor.view.layoutSubtreeIfNeeded()
                let allViews = descendants(of: editor.view)
                for (title, accessibilityLabel) in [
                    ("Document type", "Document type"),
                    ("Style", "Style"),
                    ("Style type", "Style type"),
                    ("Based on", "Based on style"),
                    ("Next paragraph", "Following paragraph style"),
                ] {
                    let label = try XCTUnwrap(allViews.compactMap { $0 as? NSTextField }
                        .first { $0.stringValue == title && !$0.isEditable })
                    let control = try XCTUnwrap(allViews.first {
                        $0 !== label && $0.accessibilityLabel() == accessibilityLabel
                    })
                    let labelRect = alignmentRect(of: label, in: editor.view)
                    let controlRect = alignmentRect(of: control, in: editor.view)
                    XCTAssertEqual(labelRect.midY, controlRect.midY, accuracy: 1,
                                   "\(title) must align with its native control at width \(width)")
                }
                for (popupLabel, buttonLabel) in [
                    ("Based on style", "Edit based on style"),
                    ("Following paragraph style", "Edit next paragraph style"),
                ] {
                    let popup = try XCTUnwrap(allViews.first { $0.accessibilityLabel() == popupLabel })
                    let button = try button(buttonLabel, in: editor)
                    let popupRect = alignmentRect(of: popup, in: editor.view)
                    let buttonRect = alignmentRect(of: button, in: editor.view)
                    XCTAssertGreaterThanOrEqual(buttonRect.minX, popupRect.maxX)
                    XCTAssertLessThanOrEqual(buttonRect.minX - popupRect.maxX, 10)
                    XCTAssertEqual(buttonRect.midY, popupRect.midY, accuracy: 1)
                    let otherButton = try self.button("Edit based on style", in: editor)
                    XCTAssertEqual(buttonRect.maxX, alignmentRect(of: otherButton, in: editor.view).maxX, accuracy: 1,
                                   "Relationship controls should share a right edge")
                    XCTAssertNotNil(button.image)
                }
                let rowCenters = try ["Style type", "Based on", "Next paragraph"].map { title in
                    let label = try XCTUnwrap(allViews.compactMap { $0 as? NSTextField }
                        .first { $0.stringValue == title })
                    return alignmentRect(of: label, in: editor.view).midY
                }
                XCTAssertEqual(abs(rowCenters[0] - rowCenters[1]), abs(rowCenters[1] - rowCenters[2]), accuracy: 1,
                               "The text-only Style type row must use the same spacing as the popup rows")
                if width == 700,
                   let directory = ProcessInfo.processInfo.environment["VIEM_STYLE_EDITOR_SCREENSHOT_DIR"] {
                    window.displayIfNeeded()
                    editor.view.displayIfNeeded()
                    let bitmap = try XCTUnwrap(editor.view.bitmapImageRepForCachingDisplay(in: editor.view.bounds))
                    editor.view.cacheDisplay(in: editor.view.bounds, to: bitmap)
                    let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                    let url = URL(fileURLWithPath: directory, isDirectory: true)
                        .appendingPathComponent("style-editor-\(appearance.rawValue).png")
                    try png.write(to: url)
                }
            }
        }
    }

    private func makeEditor() throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVStyleEditorViewController) {
        let configuration = isolatedConfiguration()
        try EVStyleTestFixtures.configure(configuration, declarations: [
            (.baseParagraph, .characterFontFamilies, .stringList(["system-ui"])),
            (.baseParagraph, .characterFontAxes, .string("{}")),
            (heading, .characterSize, .float(24)),
        ])
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data("# Heading\n\nText".utf8), typeName: "public.markdown")
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(document: surface, styleKey: heading)
        return (backend, surface, editor)
    }

    private func isolatedConfiguration() -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-style-hierarchy-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVConfigurationStore(directory: directory, legacyDefaults: nil)
    }

    private func button(_ label: String, in editor: EVStyleEditorViewController) throws -> NSButton {
        try XCTUnwrap(descendants(of: editor.view).compactMap { $0 as? NSButton }
            .first { $0.accessibilityLabel() == label })
    }

    private func descendants(of view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants(of: $0) }
    }

    private func alignmentRect(of view: NSView, in root: NSView) -> NSRect {
        root.convert(view.alignmentRect(forFrame: view.frame), from: view.superview)
    }
}
