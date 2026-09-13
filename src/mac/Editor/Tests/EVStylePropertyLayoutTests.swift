import AppKit
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVStylePropertyLayoutTests: XCTestCase {
    private let heading = EVStyleKey(namespace: .block, id: EVStyleID(rawValue: "Heading1"))

    func testColorCaptionsEnableInheritedColorsWithoutOpeningTheColorPanel() throws {
        let (backend, surface, editor, window) = try makeEditor()
        defer { window.orderOut(nil); withExtendedLifetime(surface) {} }
        let panel = NSColorPanel.shared
        panel.orderOut(nil)
        defer { panel.orderOut(nil) }
        let recorder = NativeActionRecorder()
        for (title, accessibilityName, property) in [
            ("Background Color", "Background color", EVStyleProperty.characterBackground),
            ("Text Color", "Text color", .characterForeground),
        ] {
            let well = try control(NSColorWell.self, label: accessibilityName, in: editor.view)
            let checkbox = try control(NSButton.self, label: "Override \(property.displayName.lowercased())", in: editor.view)
            let caption = try caption(title, in: editor.view)
            XCTAssertFalse(well.isEnabled)
            XCTAssertEqual(checkbox.state, .off)
            let before = try backend.styleSheetSnapshot()
            if property == .characterBackground {
                XCTAssertNil(before.definition(for: heading)?.properties[property]?.effective,
                    "The crash regression must begin with no inherited background value")
            }
            well.target = recorder
            well.action = #selector(NativeActionRecorder.record(_:))
            try clickCaption(caption, in: editor.view)
            XCTAssertTrue(well.isEnabled)
            XCTAssertEqual(checkbox.state, .on)
            XCTAssertFalse(well.isActive)
            XCTAssertFalse(panel.isVisible, "A caption only enables the property; it does not open its color chooser")
            XCTAssertEqual(recorder.count, 0)
            let changed = try backend.styleSheetSnapshot()
            XCTAssertTrue(changed.definition(for: heading)?.properties[property]?.isDeclared == true)
            if property == .characterBackground {
                XCTAssertEqual(changed.definition(for: heading)?.properties[property]?.declared,
                    .color(EVStyleColor(red: 0, green: 0, blue: 0, alpha: 0)))
            }
            try clickCaption(caption, in: editor.view)
            XCTAssertEqual(try backend.styleSheetSnapshot(), changed,
                "Clicking an already active caption must not toggle or reapply it")
            XCTAssertEqual(recorder.count, 0)
            XCTAssertFalse(panel.isVisible)
        }
    }

    func testOpenTypeDirectionAndNumericCaptionsOnlyEnableTheirProperties() throws {
        let (backend, surface, editor, window) = try makeEditor()
        defer { window.orderOut(nil); withExtendedLifetime(surface) {} }
        var menusOpened = 0
        let observer = NotificationCenter.default.addObserver(forName: NSMenu.didBeginTrackingNotification,
            object: nil, queue: .main) { _ in menusOpened += 1 }
        defer { NotificationCenter.default.removeObserver(observer) }
        let recorder = NativeActionRecorder()
        for (title, name, property) in [
            ("OpenType", "OpenType features", EVStyleProperty.characterOpenTypeFeatures),
            ("Direction", "Character direction", .characterDirection),
            ("Tracking", "Tracking", .characterLetterSpacing),
        ] {
            let native = try control(NSControl.self, label: name, in: editor.view)
            XCTAssertFalse(native.isEnabled)
            native.target = recorder
            native.action = #selector(NativeActionRecorder.record(_:))
            try clickCaption(caption(title, in: editor.view), in: editor.view)
            XCTAssertTrue(native.isEnabled)
            XCTAssertTrue(try backend.styleSheetSnapshot().definition(for: heading)?.properties[property]?.isDeclared == true)
            XCTAssertEqual(recorder.count, 0, "Caption clicks do not dispatch the underlying control action")
            XCTAssertEqual(menusOpened, 0)
            if let field = native as? NSTextField {
                XCTAssertNil(field.currentEditor(), "Clicking the caption does not focus the input")
                XCTAssertEqual(field.floatValue, 0, "Clicking Tracking must not perform a step")
            }
        }
        XCTAssertFalse(editor.hasActiveStyleEditGroupForTesting)
    }

    func testCaptionedAppearanceRowAlignsItsControlsAndPreservesOneEmSectionGaps() throws {
        let (_, surface, editor, window) = try makeEditor()
        defer { window.orderOut(nil); withExtendedLifetime(surface) {} }
        let em = NSFont.systemFontSize
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            window.appearance = try XCTUnwrap(NSAppearance(named: appearance))
            for (width, height) in [(CGFloat(700), CGFloat(620)), (1000, 820)] {
                window.setContentSize(NSSize(width: width, height: height))
                editor.view.layoutSubtreeIfNeeded()
                let bold = try control(NSButton.self, label: "Bold", in: editor.view)
                let baseline = alignmentRect(of: bold, in: editor.view).midY
                for name in ["Italic", "Underline", "Strikethrough", "Override foreground", "Override background", "Override opentype features"] {
                    let aligned = try control(NSButton.self, label: name, in: editor.view)
                    XCTAssertEqual(alignmentRect(of: aligned, in: editor.view).midY, baseline, accuracy: 1,
                        "\(name) should align with the emphasis row at width \(width)")
                }
                for (title, controlName) in [("Text Color", "Text color"), ("Background Color", "Background color"), ("OpenType", "OpenType features")] {
                    let label = try caption(title, in: editor.view)
                    let native = try control(NSControl.self, label: controlName, in: editor.view)
                    let labelRect = editor.view.convert(label.bounds, from: label)
                    let nativeRect = alignmentRect(of: native, in: editor.view)
                    XCTAssertGreaterThanOrEqual(labelRect.minY, nativeRect.maxY,
                        "\(title) belongs above its native control")
                    XCTAssertTrue(editor.view.bounds.contains(labelRect))
                    XCTAssertTrue(editor.view.bounds.contains(nativeRect))
                }
                for (preceding, following) in [
                    ("Bold", "Override slant"), ("Italic", "Override underline"),
                    ("Underline", "Override strikethrough"), ("Strikethrough", "Override foreground"),
                    ("Text color", "Override background"), ("Background color", "Override opentype features"),
                ] {
                    let prior = try control(NSControl.self, label: preceding, in: editor.view)
                    let next = try control(NSButton.self, label: following, in: editor.view)
                    XCTAssertGreaterThanOrEqual(alignmentRect(of: next, in: editor.view).minX - alignmentRect(of: prior, in: editor.view).maxX,
                        em - 1, "Leave at least one em before \(following)")
                }
            }
        }
    }

    func testMinimumSizeShowsCompleteCaptionsAndUnitsOnBothTabs() throws {
        let (_, surface, editor, window) = try makeEditor()
        defer { window.orderOut(nil); withExtendedLifetime(surface) {} }
        window.setContentSize(NSSize(width: 700, height: 620))
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            window.appearance = try XCTUnwrap(NSAppearance(named: appearance))
            for tab in [EVStyleEditorTab.character, .paragraph] {
                editor.selectTab(tab)
                editor.view.layoutSubtreeIfNeeded()
                let labels = descendants(of: editor.view).compactMap { $0 as? EVStyleControlLabel }
                    .filter { !$0.isHiddenOrHasHiddenAncestor && !$0.stringValue.isEmpty }
                XCTAssertFalse(labels.isEmpty)
                XCTAssertTrue(labels.contains { $0.stringValue == "pt" })
                for label in labels {
                    XCTAssertGreaterThanOrEqual(label.bounds.width, label.intrinsicContentSize.width - 0.5,
                        "The complete \(label.stringValue) caption must fit on the \(tab) tab")
                    XCTAssertTrue(editor.view.bounds.contains(editor.view.convert(label.bounds, from: label)),
                        "The \(label.stringValue) caption must remain inside the minimum-size dialog")
                }
                if let directory = ProcessInfo.processInfo.environment["VIEM_STYLE_EDITOR_SCREENSHOT_DIR"] {
                    for child in descendants(of: editor.view) { child.needsDisplay = true }
                    window.displayIfNeeded()
                    editor.view.displayIfNeeded()
                    let bitmap = try XCTUnwrap(editor.view.bitmapImageRepForCachingDisplay(in: editor.view.bounds))
                    editor.view.cacheDisplay(in: editor.view.bounds, to: bitmap)
                    let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                    let filename = "style-properties-\(tab)-\(appearance.rawValue).png"
                    try png.write(to: URL(fileURLWithPath: directory, isDirectory: true).appendingPathComponent(filename))
                }
            }
        }
    }

    private func makeEditor() throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVStyleEditorViewController, NSWindow) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-property-layout-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        let backend = EVCoreDocumentBackend(configuration: configuration)
        try backend.read(source: Data("# Heading\n\nBody".utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let editor = EVStyleEditorViewController()
        editor.themeStore = EVThemeStore(configuration: configuration)
        editor.retarget(document: surface, styleKey: heading)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 650),
            styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.contentViewController = editor
        window.makeKeyAndOrderFront(nil)
        editor.view.layoutSubtreeIfNeeded()
        return (backend, surface, editor, window)
    }

    private func descendants(of view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants(of: $0) }
    }

    private func control<T: NSView>(_ type: T.Type, label: String, in root: NSView) throws -> T {
        try XCTUnwrap(descendants(of: root).first { $0 is T && $0.accessibilityLabel() == label } as? T,
            "Missing \(label)")
    }

    private func caption(_ title: String, in root: NSView) throws -> EVStyleControlLabel {
        try XCTUnwrap(descendants(of: root).compactMap { $0 as? EVStyleControlLabel }
            .first { $0.stringValue == title && !$0.isHiddenOrHasHiddenAncestor }, "Missing caption \(title)")
    }

    private func clickCaption(_ label: EVStyleControlLabel, in root: NSView) throws {
        let window = try XCTUnwrap(root.window)
        let parent = try XCTUnwrap(root.superview)
        let location = label.convert(NSPoint(x: label.bounds.midX, y: label.bounds.midY), to: nil)
        let hit = try XCTUnwrap(root.hitTest(parent.convert(location, from: nil)))
        XCTAssertTrue(hit === label, "The visible caption receives the original native click")
        let event = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: location,
            modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber,
            context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        hit.mouseDown(with: event)
    }

    private func alignmentRect(of view: NSView, in root: NSView) -> NSRect {
        root.convert(view.alignmentRect(forFrame: view.frame), from: view.superview)
    }
}

@MainActor
private final class NativeActionRecorder: NSObject {
    var count = 0
    @objc func record(_ sender: Any?) { count += 1 }
}
