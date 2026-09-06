import AppKit
import XCTest
@testable import EvimAppShell

@MainActor
final class EVMenuBuilderTests: XCTestCase {
    private final class Owner: NSObject, EVApplicationCommandRouting {
        var newDocumentCount = 0
        var openedRecentURLs: [URL] = []
        var clearRecentDocumentsCount = 0

        @objc func newDocument(_ sender: Any?) { newDocumentCount += 1 }
        @objc func openDocument(_ sender: Any?) {}
        @objc func openRecentDocument(_ sender: Any?) {
            if let url = (sender as? NSMenuItem)?.representedObject as? URL {
                openedRecentURLs.append(url)
            }
        }
        @objc func clearRecentDocuments(_ sender: Any?) { clearRecentDocumentsCount += 1 }
        @objc func showSettings(_ sender: Any?) {}
        @objc func showHelpItem(_ sender: Any?) {}
    }

    private final class StyleProvider: EVStyleMenuProviding {
        var catalogue: EVStyleMenuCatalogue?

        init(catalogue: EVStyleMenuCatalogue?) {
            self.catalogue = catalogue
        }

        func currentStyleMenuCatalogue() -> EVStyleMenuCatalogue? { catalogue }
    }

    func testTopLevelMenuOrderMatchesSpecification() {
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner)
        let menu = builder.buildMainMenu(for: NSApplication.shared)

        XCTAssertEqual(menu.items.map(\.title), [
            "eVim", "File", "Edit", "Format", "Paragraph", "Character", "View", "Window", "Help",
        ])
        XCTAssertTrue(NSApplication.shared.windowsMenu === menu.item(withTitle: "Window")?.submenu)
        XCTAssertTrue(NSApplication.shared.helpMenu === menu.item(withTitle: "Help")?.submenu)
        XCTAssertTrue(
            NSApplication.shared.servicesMenu
                === menu.item(withTitle: "eVim")?.submenu?.item(withTitle: "Services")?.submenu
        )
    }

    func testRequiredDirectMenuHierarchy() throws {
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner)
        let menu = builder.buildMainMenu(for: NSApplication.shared)

        XCTAssertEqual(try titles(in: submenu("eVim", of: menu)), [
            "About eVim", "Settings…", "Services", "Hide eVim", "Hide Others",
            "Show All", "Quit eVim",
        ])
        XCTAssertEqual(try titles(in: submenu("File", of: menu)), [
            "New", "Open…", "Open Recent", "Close", "Save", "Save As…", "Duplicate",
            "Rename…", "Move To…", "Revert To", "Document Format…", "Text Encoding…",
            "Line Endings", "Page Setup…", "Print…",
        ])
        XCTAssertEqual(try titles(in: submenu("Edit", of: menu)), [
            "Undo", "Redo", "Cut", "Copy", "Paste", "Paste and Match Style", "Delete",
            "Select All", "Select", "Find", "Transformations", "Start Dictation…",
            "Emoji & Symbols",
        ])
        XCTAssertEqual(try titles(in: submenu("Format", of: menu)), [
            "Show Fonts", "Bold", "Italic", "Underline", "Strikethrough", "Bigger", "Smaller", "Ligatures", "Kerning", "Baseline", "OpenType Features",
            "Show Colors", "Text Color…", "Highlight Color…", "Document Style", "Paragraph", "Copy Style", "Paste Style", "Clear Direct Character Formatting",
            "Clear Direct Paragraph Formatting", "Clear All Direct Formatting",
        ])
        XCTAssertEqual(try titles(in: submenu("View", of: menu)), [
            "Show Status Bar", "Word Wrap", "Wrap at Word Boundaries",
            "Show Invisible Characters", "Zoom In", "Zoom Out", "Actual Size",
            "Enter Full Screen",
        ])
        XCTAssertEqual(try titles(in: submenu("Window", of: menu)), [
            "Minimize", "Zoom", "New Window for Document", "Bring All to Front",
        ])
        XCTAssertEqual(try titles(in: submenu("Help", of: menu)), [
            "eVim Help", "Vim Command Reference", "Keyboard Shortcuts",
            "Supported Vim Commands", "Document Format Compatibility",
            "Round-Trip and Source Preservation", "Release Notes", "Report a Problem…",
        ])
    }

    func testEveryStaticMenuGroupHasExactTitlesSeparatorsAndOrder() throws {
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner, recentDocumentURLs: { [] })
        let main = builder.buildMainMenu(for: NSApplication.shared)

        XCTAssertEqual(tokens(in: try submenu("eVim", of: main)), [
            "About eVim", "Settings…", "-", "Services", "-", "Hide eVim",
            "Hide Others", "Show All", "-", "Quit eVim",
        ])
        XCTAssertEqual(tokens(in: try submenu("File", of: main)), [
            "New", "Open…", "Open Recent", "-", "Close", "Save", "Save As…",
            "Duplicate", "Rename…", "Move To…", "Revert To", "-",
            "Document Format…", "Text Encoding…", "Line Endings", "-",
            "Page Setup…", "Print…",
        ])
        XCTAssertEqual(tokens(in: try submenu("Edit", of: main)), [
            "Undo", "Redo", "-", "Cut", "Copy", "Paste", "Paste and Match Style",
            "Delete", "-", "Select All", "Select", "-", "Find", "-",
            "Transformations", "-", "Start Dictation…", "Emoji & Symbols",
        ])
        XCTAssertEqual(tokens(in: try submenu("Format", of: main)), [
            "Show Fonts", "-", "Bold", "Italic", "Underline", "Strikethrough", "-", "Bigger", "Smaller", "-", "Ligatures", "Kerning", "Baseline", "OpenType Features", "-",
            "Show Colors", "Text Color…", "Highlight Color…", "-", "Document Style", "-", "Paragraph", "-", "Copy Style", "Paste Style",
            "Clear Direct Character Formatting", "Clear Direct Paragraph Formatting",
            "Clear All Direct Formatting",
        ])
        XCTAssertEqual(tokens(in: try submenu("View", of: main)), [
            "Show Status Bar", "-", "Word Wrap", "Wrap at Word Boundaries",
            "Show Invisible Characters", "-", "Zoom In", "Zoom Out", "Actual Size",
            "-", "Enter Full Screen",
        ])
        XCTAssertEqual(tokens(in: try submenu("Window", of: main)), [
            "Minimize", "Zoom", "-", "New Window for Document", "-",
            "Bring All to Front", "-",
        ])
        XCTAssertEqual(tokens(in: try submenu("Help", of: main)), [
            "-", "eVim Help", "Vim Command Reference", "Keyboard Shortcuts",
            "Supported Vim Commands", "Document Format Compatibility",
            "Round-Trip and Source Preservation", "-", "Release Notes",
            "Report a Problem…",
        ])

        let file = try submenu("File", of: main)
        let edit = try submenu("Edit", of: main)
        let format = try submenu("Format", of: main)
        XCTAssertEqual(tokens(in: try submenu("Revert To", of: file)), [
            "Last Saved Version", "Browse All Versions…",
        ])
        XCTAssertEqual(tokens(in: try submenu("Line Endings", of: file)), [
            "Unix (LF)", "Windows (CRLF)", "Classic Mac (CR)",
        ])
        XCTAssertEqual(tokens(in: try submenu("Select", of: edit)), [
            "Word", "Sentence", "Paragraph", "Hard Line", "Visual Row",
        ])
        XCTAssertEqual(tokens(in: try submenu("Find", of: edit)), [
            "Find…", "Find and Replace…", "Find Next", "Find Previous",
            "Use Selection for Find", "Jump to Selection",
        ])
        XCTAssertEqual(tokens(in: try submenu("Transformations", of: edit)), [
            "Make Uppercase", "Make Lowercase", "Toggle Case",
        ])

        let font = format
        XCTAssertEqual(Array(tokens(in: font).prefix(14)), [
            "Show Fonts", "-", "Bold", "Italic", "Underline", "Strikethrough", "-",
            "Bigger", "Smaller", "-", "Ligatures", "Kerning", "Baseline",
            "OpenType Features",
        ])
        XCTAssertEqual(tokens(in: try submenu("Ligatures", of: font)), [
            "Use Default Ligatures", "Use All Ligatures", "Use No Ligatures",
        ])
        XCTAssertEqual(tokens(in: try submenu("Kerning", of: font)), [
            "Use Default Kerning", "Use No Kerning",
        ])
        XCTAssertEqual(tokens(in: try submenu("Baseline", of: font)), [
            "Superscript", "Subscript", "Raise", "Lower",
        ])
        XCTAssertEqual(tokens(in: format).filter { ["Show Colors", "Text Color…", "Highlight Color…"].contains($0) }, [
            "Show Colors", "Text Color…", "Highlight Color…",
        ])
        XCTAssertEqual(tokens(in: try submenu("Character", of: main)), [
            "Base Character", "-", "Edit Styles…",
        ])
        XCTAssertEqual(tokens(in: try submenu("Paragraph", of: main)), [
            "Base Paragraph", "Heading 1", "Heading 2", "Heading 3", "Heading 4", "Heading 5", "Heading 6", "-", "Edit Styles…",
        ])
        XCTAssertEqual(tokens(in: try submenu("Document Style", of: format)), [
            "Base Document", "-", "Edit Styles…",
        ])

        let paragraph = try submenu("Paragraph", of: format)
        XCTAssertEqual(tokens(in: paragraph), [
            "Alignment", "Writing Direction", "Increase Indent", "Decrease Indent",
            "Paragraph Spacing…", "Line Spacing", "List",
        ])
        XCTAssertEqual(tokens(in: try submenu("Alignment", of: paragraph)), [
            "Start", "Center", "End",
        ])
        XCTAssertEqual(tokens(in: try submenu("Writing Direction", of: paragraph)), [
            "Automatic", "Left to Right", "Right to Left",
        ])
        XCTAssertEqual(tokens(in: try submenu("Line Spacing", of: paragraph)), [
            "Normal", "Single", "1.5 Lines", "Double", "Custom…",
        ])
    }

    func testAllAndOnlySpecifiedKeyEquivalentsAreInstalled() throws {
        let owner = Owner()
        let main = EVMenuBuilder(owner: owner).buildMainMenu(for: NSApplication.shared)
        let expected: [String: (String, NSEvent.ModifierFlags)] = [
            "eVim/Settings…": (",", [.command]),
            "eVim/Hide eVim": ("h", [.command]),
            "eVim/Hide Others": ("h", [.command, .option]),
            "eVim/Quit eVim": ("q", [.command]),
            "File/New": ("n", [.command]),
            "File/Open…": ("o", [.command]),
            "File/Close": ("w", [.command]),
            "File/Save": ("s", [.command]),
            "File/Save As…": ("s", [.command, .shift]),
            "File/Print…": ("p", [.command]),
            "Edit/Undo": ("z", [.command]),
            "Edit/Redo": ("z", [.command, .shift]),
            "Edit/Cut": ("x", [.command]),
            "Edit/Copy": ("c", [.command]),
            "Edit/Paste": ("v", [.command]),
            "Edit/Paste and Match Style": ("v", [.command, .option, .shift]),
            "Edit/Select All": ("a", [.command]),
            "Edit/Find/Find…": ("f", [.command]),
            "Edit/Find/Find Next": ("g", [.command]),
            "Edit/Find/Find Previous": ("g", [.command, .shift]),
            "Edit/Find/Use Selection for Find": ("e", [.command]),
            "Edit/Find/Jump to Selection": ("j", [.command]),
            "Edit/Emoji & Symbols": (" ", [.command, .control]),
            "Format/Show Fonts": ("t", [.command]),
            "Format/Bold": ("b", [.command]),
            "Format/Italic": ("i", [.command]),
            "Format/Underline": ("u", [.command]),
            "Paragraph/Base Paragraph": ("0", [.command]),
            "Paragraph/Heading 1": ("1", [.command]),
            "Paragraph/Heading 2": ("2", [.command]),
            "Paragraph/Heading 3": ("3", [.command]),
            "Paragraph/Heading 4": ("4", [.command]),
            "Paragraph/Heading 5": ("5", [.command]),
            "Paragraph/Heading 6": ("6", [.command]),
            "View/Enter Full Screen": ("f", [.command, .control]),
            "Window/Minimize": ("m", [.command]),
        ]

        let actual = keyedItems(in: main)
        XCTAssertEqual(Set(actual.keys), Set(expected.keys))
        for (path, (key, modifiers)) in expected {
            let menuItem = try XCTUnwrap(actual[path], "Missing menu item at \(path)")
            XCTAssertEqual(menuItem.keyEquivalent, key, path)
            XCTAssertEqual(
                menuItem.keyEquivalentModifierMask.intersection(.deviceIndependentFlagsMask),
                modifiers,
                path
            )
        }
    }

    func testEveryTypedCommandAppearsExactlyOnceAndUsesItsExpectedRoute() {
        let owner = Owner()
        let main = EVMenuBuilder(owner: owner).buildMainMenu(for: NSApplication.shared)
        let tagged = allItems(in: main).compactMap { item -> (EVMenuCommand, NSMenuItem)? in
            EVMenuCommand(rawValue: item.tag).map { ($0, item) }
        }
        let grouped = Dictionary(grouping: tagged, by: \.0)

        XCTAssertEqual(Set(grouped.keys), Set(EVMenuCommand.allCases.filter { $0 != .baseParagraphStyle }))
        for command in EVMenuCommand.allCases where command != .baseParagraphStyle {
            XCTAssertEqual(grouped[command]?.count, 1, "Unexpected menu count for \(command)")
        }

        let nativeDocumentCommands: Set<EVMenuCommand> = [
            .save, .saveAs, .duplicateDocument, .renameDocument, .moveDocument,
            .revertLastSaved, .browseVersions, .pageSetup, .printDocument,
        ]
        let styleCommands: Set<EVMenuCommand> = [
            .baseCharacterStyle, .editCharacterStyles,
            .baseParagraphStyle, .editParagraphStyles,
            .baseDocumentStyle, .editDocumentStyles,
        ]
        let coreSelector = #selector(EVEditorCommandRouting.performEditorMenuCommand(_:))
        let styleSelector = #selector(EVStyleMenuActionRouting.performEditorStyleMenuAction(_:))
        for (command, item) in tagged {
            if command == .openTypeFeatures {
                XCTAssertNotNil(item.submenu)
                continue
            }
            if nativeDocumentCommands.contains(command) {
                XCTAssertNotEqual(item.action, coreSelector, "\(command) must remain NSDocument-owned")
            } else if let nativeAction = command.nativeEditAction {
                XCTAssertEqual(item.action, nativeAction, "\(command) must reach native field editors")
            } else if styleCommands.contains(command) {
                XCTAssertEqual(item.action, styleSelector, "\(command) must use the typed style route")
            } else {
                XCTAssertEqual(item.action, coreSelector, "\(command) must use the editor route")
            }
            XCTAssertNil(item.target, "\(command) must use the responder chain")
        }
    }

    func testCommandAUsesNativeNumericFieldEditorInsteadOfDocumentCommands() throws {
        let application = NSApplication.shared
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner)
        let main = builder.buildMainMenu(for: application)
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 300, height: 100), styleMask: [.titled], backing: .buffered, defer: false)
        defer { window.orderOut(nil) }
        let field = NSTextField(string: "11")
        field.frame = NSRect(x: 20, y: 30, width: 100, height: 24)
        window.contentView?.addSubview(field)
        window.makeKeyAndOrderFront(nil)
        XCTAssertTrue(window.makeFirstResponder(field))
        let fieldEditor = try XCTUnwrap(field.currentEditor() as? NSTextView)
        fieldEditor.setSelectedRange(NSRange(location: 2, length: 0))
        let selectAll = try XCTUnwrap(allItems(in: main).first { $0.tag == EVMenuCommand.selectAll.rawValue })
        XCTAssertEqual(selectAll.keyEquivalent, "a")
        XCTAssertEqual(selectAll.keyEquivalentModifierMask, [.command])
        XCTAssertTrue(try XCTUnwrap(window.firstResponder).tryToPerform(try XCTUnwrap(selectAll.action), with: selectAll))
        XCTAssertEqual(fieldEditor.selectedRange(), NSRange(location: 0, length: 2))
        fieldEditor.insertText("18", replacementRange: fieldEditor.selectedRange())
        XCTAssertEqual(fieldEditor.string, "18")
        XCTAssertTrue(window.makeFirstResponder(nil))
        XCTAssertEqual(field.stringValue, "18")
        let expected: [EVMenuCommand: String] = [.undo: "undo:", .redo: "redo:", .cut: "cut:", .copy: "copy:", .paste: "paste:", .pasteAndMatchStyle: "pasteAsPlainText:", .delete: "delete:", .selectAll: "selectAll:"]
        for (command, selector) in expected {
            let item = try XCTUnwrap(allItems(in: main).first { $0.tag == command.rawValue })
            XCTAssertEqual(item.action, NSSelectorFromString(selector))
            XCTAssertNil(item.target)
        }
        withExtendedLifetime(builder) {}
    }

    func testNativeDocumentItemsUseNSDocumentSelectorsAndSemanticTags() throws {
        let owner = Owner()
        let main = EVMenuBuilder(owner: owner).buildMainMenu(for: NSApplication.shared)
        let expected: [String: (Selector, EVMenuCommand)] = [
            "File/Save": (#selector(NSDocument.save(_:)), .save),
            "File/Save As…": (#selector(NSDocument.saveAs(_:)), .saveAs),
            "File/Duplicate": (#selector(NSDocument.duplicate(_:)), .duplicateDocument),
            "File/Rename…": (#selector(NSDocument.rename(_:)), .renameDocument),
            "File/Move To…": (#selector(NSDocument.move(_:)), .moveDocument),
            "File/Revert To/Last Saved Version": (#selector(NSDocument.revertToSaved(_:)), .revertLastSaved),
            "File/Revert To/Browse All Versions…": (#selector(NSDocument.browseVersions(_:)), .browseVersions),
            "File/Page Setup…": (#selector(NSDocument.runPageLayout(_:)), .pageSetup),
            "File/Print…": (#selector(NSDocument.printDocument(_:)), .printDocument),
        ]

        for (path, (selector, command)) in expected {
            let menuItem = try item(at: path, in: main)
            XCTAssertEqual(menuItem.action, selector, path)
            XCTAssertEqual(menuItem.tag, command.rawValue, path)
            XCTAssertNil(menuItem.target, path)
        }
    }

    func testApplicationAndWindowLifecycleActionsUseExpectedTargets() throws {
        let owner = Owner()
        let main = EVMenuBuilder(owner: owner).buildMainMenu(for: NSApplication.shared)

        let newItem = try item(at: "File/New", in: main)
        XCTAssertEqual(newItem.action, #selector(EVApplicationCommandRouting.newDocument(_:)))
        XCTAssertTrue(newItem.target === owner)
        _ = NSApplication.shared.sendAction(newItem.action!, to: newItem.target, from: newItem)
        XCTAssertEqual(owner.newDocumentCount, 1)

        let newWindow = try item(at: "Window/New Window for Document", in: main)
        XCTAssertEqual(newWindow.action, #selector(EVEditorCommandRouting.performEditorMenuCommand(_:)))
        XCTAssertEqual(newWindow.tag, EVMenuCommand.newWindowForDocument.rawValue)
        XCTAssertNil(newWindow.target)

        let bringFront = try item(at: "Window/Bring All to Front", in: main)
        XCTAssertEqual(bringFront.action, #selector(NSApplication.arrangeInFront(_:)))
        XCTAssertTrue(bringFront.target === NSApplication.shared)
    }

    func testShellAndSystemOwnedItemsUseExpectedSelectors() throws {
        let owner = Owner()
        let main = EVMenuBuilder(owner: owner).buildMainMenu(for: NSApplication.shared)
        let applicationRoutes: [String: Selector] = [
            "eVim/Settings…": #selector(EVApplicationCommandRouting.showSettings(_:)),
            "File/New": #selector(EVApplicationCommandRouting.newDocument(_:)),
            "File/Open…": #selector(EVApplicationCommandRouting.openDocument(_:)),
        ]
        for (path, selector) in applicationRoutes {
            let menuItem = try item(at: path, in: main)
            XCTAssertEqual(menuItem.action, selector, path)
            XCTAssertTrue(menuItem.target === owner, path)
        }

        let responderRoutes: [String: Selector] = [
            "eVim/About eVim": #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
            "eVim/Hide eVim": #selector(NSApplication.hide(_:)),
            "eVim/Hide Others": #selector(NSApplication.hideOtherApplications(_:)),
            "eVim/Show All": #selector(NSApplication.unhideAllApplications(_:)),
            "eVim/Quit eVim": #selector(NSApplication.terminate(_:)),
            "File/Close": #selector(NSWindow.performClose(_:)),
            "Edit/Start Dictation…": Selector(("startDictation:")),
            "Edit/Emoji & Symbols": #selector(NSApplication.orderFrontCharacterPalette(_:)),
            "View/Show Status Bar": #selector(EVDocumentContentViewController.toggleStatusBar(_:)),
            "View/Enter Full Screen": #selector(NSWindow.toggleFullScreen(_:)),
            "Window/Minimize": #selector(NSWindow.performMiniaturize(_:)),
            "Window/Zoom": #selector(NSWindow.performZoom(_:)),
        ]
        for (path, selector) in responderRoutes {
            let menuItem = try item(at: path, in: main)
            XCTAssertEqual(menuItem.action, selector, path)
            XCTAssertNil(menuItem.target, path)
        }

        for title in [
            "eVim Help", "Vim Command Reference", "Keyboard Shortcuts",
            "Supported Vim Commands", "Document Format Compatibility",
            "Round-Trip and Source Preservation", "Release Notes", "Report a Problem…",
        ] {
            let menuItem = try item(at: "Help/\(title)", in: main)
            XCTAssertEqual(menuItem.action, #selector(EVApplicationCommandRouting.showHelpItem(_:)))
            XCTAssertTrue(menuItem.target === owner)
            XCTAssertEqual(menuItem.representedObject as? String, title)
        }
    }

    func testRecentDocumentsAreRebuiltWithURLsAndClearBehavior() throws {
        let owner = Owner()
        let urls = [
            URL(fileURLWithPath: "/tmp/First Draft.txt"),
            URL(fileURLWithPath: "/tmp/notes.md"),
        ]
        let builder = EVMenuBuilder(owner: owner, recentDocumentURLs: { urls })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let recent = try submenu("Open Recent", of: submenu("File", of: main))

        builder.menuNeedsUpdate(recent)

        XCTAssertEqual(tokens(in: recent), ["First Draft.txt", "notes.md", "-", "Clear Menu"])
        for (index, url) in urls.enumerated() {
            let menuItem = recent.items[index]
            XCTAssertEqual(menuItem.action, #selector(EVApplicationCommandRouting.openRecentDocument(_:)))
            XCTAssertTrue(menuItem.target === owner)
            XCTAssertEqual(menuItem.representedObject as? URL, url)
            XCTAssertEqual(menuItem.toolTip, url.path)
        }
        let clear = try XCTUnwrap(recent.items.last)
        XCTAssertTrue(clear.isEnabled)
        XCTAssertTrue(clear.target === owner)

        let first = recent.items[0]
        _ = NSApplication.shared.sendAction(first.action!, to: first.target, from: first)
        _ = NSApplication.shared.sendAction(clear.action!, to: clear.target, from: clear)
        XCTAssertEqual(owner.openedRecentURLs, [urls[0]])
        XCTAssertEqual(owner.clearRecentDocumentsCount, 1)
    }

    func testEmptyRecentDocumentsMenuContainsOnlyDisabledClearCommand() throws {
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner, recentDocumentURLs: { [] })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let recent = try submenu("Open Recent", of: submenu("File", of: main))

        builder.menuNeedsUpdate(recent)

        XCTAssertEqual(tokens(in: recent), ["Clear Menu"])
        XCTAssertFalse(try XCTUnwrap(recent.items.last).isEnabled)
    }

    func testNestedMenuHierarchyMatchesSpecification() throws {
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner)
        let menu = builder.buildMainMenu(for: NSApplication.shared)
        let file = try submenu("File", of: menu)
        let edit = try submenu("Edit", of: menu)
        let format = try submenu("Format", of: menu)

        XCTAssertEqual(try titles(in: submenu("Revert To", of: file)), [
            "Last Saved Version", "Browse All Versions…",
        ])
        XCTAssertEqual(try titles(in: submenu("Line Endings", of: file)), [
            "Unix (LF)", "Windows (CRLF)", "Classic Mac (CR)",
        ])
        XCTAssertEqual(try titles(in: submenu("Select", of: edit)), [
            "Word", "Sentence", "Paragraph", "Hard Line", "Visual Row",
        ])
        XCTAssertEqual(try titles(in: submenu("Find", of: edit)), [
            "Find…", "Find and Replace…", "Find Next", "Find Previous",
            "Use Selection for Find", "Jump to Selection",
        ])
        XCTAssertEqual(try titles(in: submenu("Transformations", of: edit)), [
            "Make Uppercase", "Make Lowercase", "Toggle Case",
        ])

        let font = format
        XCTAssertEqual(Array(titles(in: font).prefix(11)), [
            "Show Fonts", "Bold", "Italic", "Underline", "Strikethrough", "Bigger", "Smaller",
            "Ligatures", "Kerning", "Baseline", "OpenType Features",
        ])
        XCTAssertEqual(try titles(in: submenu("Ligatures", of: font)), [
            "Use Default Ligatures", "Use All Ligatures", "Use No Ligatures",
        ])
        XCTAssertEqual(try titles(in: submenu("Kerning", of: font)), [
            "Use Default Kerning", "Use No Kerning",
        ])
        XCTAssertEqual(try titles(in: submenu("Baseline", of: font)), [
            "Superscript", "Subscript", "Raise", "Lower",
        ])
        XCTAssertEqual(titles(in: format).filter { ["Show Colors", "Text Color…", "Highlight Color…"].contains($0) }, [
            "Show Colors", "Text Color…", "Highlight Color…",
        ])
        XCTAssertEqual(try titles(in: submenu("Character", of: menu)), [
            "Base Character", "Edit Styles…",
        ])
        XCTAssertEqual(try titles(in: submenu("Paragraph", of: menu)), [
            "Base Paragraph", "Heading 1", "Heading 2", "Heading 3", "Heading 4", "Heading 5", "Heading 6", "Edit Styles…",
        ])
        XCTAssertEqual(try titles(in: submenu("Document Style", of: format)), [
            "Base Document", "Edit Styles…",
        ])

        let paragraph = try submenu("Paragraph", of: format)
        XCTAssertEqual(titles(in: paragraph), [
            "Alignment", "Writing Direction", "Increase Indent", "Decrease Indent",
            "Paragraph Spacing…", "Line Spacing", "List",
        ])
        XCTAssertEqual(try titles(in: submenu("Alignment", of: paragraph)), [
            "Start", "Center", "End",
        ])
        XCTAssertEqual(try titles(in: submenu("Writing Direction", of: paragraph)), [
            "Automatic", "Left to Right", "Right to Left",
        ])
        XCTAssertEqual(try titles(in: submenu("Line Spacing", of: paragraph)), [
            "Normal", "Single", "1.5 Lines", "Double", "Custom…",
        ])
    }

    func testEveryCoreMenuItemHasAStableTypedCommandTag() {
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner)
        let menu = builder.buildMainMenu(for: NSApplication.shared)
        let coreSelector = #selector(EVEditorCommandRouting.performEditorMenuCommand(_:))

        for item in allItems(in: menu) where item.action == coreSelector {
            XCTAssertNotNil(EVMenuCommand(rawValue: item.tag), "Missing typed tag for \(item.title)")
            XCTAssertNil(item.target, "Core action \(item.title) must use the responder chain")
        }
    }

    func testNoBareVimKeyIsRegisteredAsAGlobalShortcut() {
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner)
        let menu = builder.buildMainMenu(for: NSApplication.shared)

        for item in allItems(in: menu) where !item.keyEquivalent.isEmpty {
            let modifiers = item.keyEquivalentModifierMask
            XCTAssertFalse(
                modifiers.intersection([.command, .control, .option]).isEmpty,
                "\(item.title) registers bare key \(item.keyEquivalent)"
            )
        }
    }

    func testLineEndingsAreTheSpecifiedRadioChoices() throws {
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner)
        let menu = builder.buildMainMenu(for: NSApplication.shared)
        let file = try submenu("File", of: menu)
        let endings = try XCTUnwrap(file.item(withTitle: "Line Endings")?.submenu)

        XCTAssertEqual(titles(in: endings), ["Unix (LF)", "Windows (CRLF)", "Classic Mac (CR)"])
    }

    func testStyleMenusRefreshFromCurrentCatalogueWithBaseFirstAndStableIDs() throws {
        let owner = Owner()
        let provider = StyleProvider(catalogue: styleCatalogue(
            documentRevision: 7,
            styleSheetRevision: 11,
            entries: [
                styleEntry(.paragraph, id: "Heading2", name: "Heading", state: .mixed),
                styleEntry(.character, id: "Character", name: "Base Character", isBase: true),
                styleEntry(.paragraph, id: "Paragraph", name: "Base Paragraph", isBase: true),
                styleEntry(.paragraph, id: "Heading1", name: "Heading", enabled: true, state: .on),
                styleEntry(.document, id: "Document", name: "Base Document", isBase: true),
            ]
        ))
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { provider })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let paragraph = try submenu("Paragraph", of: main)

        builder.menuNeedsUpdate(paragraph)

        XCTAssertEqual(tokens(in: paragraph), [
            "Base Paragraph", "Heading", "Heading", "-", "Edit Styles…",
        ])
        XCTAssertTrue(paragraph.autoenablesItems)
        let styleItems = paragraph.items.filter { !$0.isSeparatorItem }.dropLast()
        XCTAssertEqual(styleItems.compactMap(styleAction).map(\.stableID), [
            "Paragraph", "Heading2", "Heading1",
        ])
        XCTAssertEqual(styleItems.map(\.isEnabled), [false, false, true])
        XCTAssertEqual(styleItems.map(\.state), [.off, .mixed, .on])
        XCTAssertTrue(styleItems.allSatisfy {
            $0.action == #selector(EVEditorCommandRouting.performEditorMenuCommand(_:))
                && $0.target == nil
        })

        let editItem = try XCTUnwrap(paragraph.items.last)
        let editAction = try XCTUnwrap(styleAction(editItem))
        XCTAssertEqual(editAction.kind, .edit)
        XCTAssertEqual(editAction.role, .paragraph)
        XCTAssertEqual(editAction.stableID, "Paragraph")
        XCTAssertEqual(editAction.documentID, 42)
        XCTAssertEqual(editAction.documentRevision, 7)
        XCTAssertEqual(editAction.styleSheetRevision, 11)
        XCTAssertTrue(editItem.isEnabled)
    }

    func testStyleMenuRequeriesProviderRatherThanCachingDefinitionsOrIndexes() throws {
        let owner = Owner()
        let provider = StyleProvider(catalogue: styleCatalogue(
            documentRevision: 1,
            styleSheetRevision: 1,
            entries: [
                styleEntry(.paragraph, id: "Heading1", name: "Old Name"),
                styleEntry(.paragraph, id: "Paragraph", name: "Base Paragraph", isBase: true),
            ]
        ))
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { provider })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let paragraph = try submenu("Paragraph", of: main)
        builder.menuNeedsUpdate(paragraph)
        XCTAssertEqual(titles(in: paragraph), ["Base Paragraph", "Old Name", "Edit Styles…"])

        provider.catalogue = styleCatalogue(
            documentRevision: 2,
            styleSheetRevision: 3,
            entries: [
                styleEntry(.paragraph, id: "Heading7", name: "New Name"),
                styleEntry(.paragraph, id: "Paragraph", name: "Renamed Base", isBase: true),
            ]
        )
        builder.menuNeedsUpdate(paragraph)

        XCTAssertEqual(titles(in: paragraph), ["Renamed Base", "New Name", "Edit Styles…"])
        let actions = paragraph.items.compactMap(styleAction)
        XCTAssertEqual(actions.map(\.stableID), ["Paragraph", "Heading7", "Paragraph"])
        XCTAssertTrue(actions.allSatisfy { $0.documentRevision == 2 })
        XCTAssertTrue(actions.allSatisfy { $0.styleSheetRevision == 3 })
    }

    func testStyleMenuDoesNotReplaceTrackedItems() throws {
        let owner = Owner()
        let provider = StyleProvider(catalogue: styleCatalogue(documentRevision: 1, styleSheetRevision: 1, entries: [styleEntry(.paragraph, id: "Paragraph", name: "Base Paragraph", isBase: true)]))
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { provider })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let paragraph = try submenu("Paragraph", of: main)
        builder.menuWillOpen(paragraph)
        let original = try XCTUnwrap(paragraph.items.first)
        builder.menuNeedsUpdate(paragraph)
        XCTAssertTrue(paragraph.items.first === original)
        provider.catalogue = styleCatalogue(documentRevision: 2, styleSheetRevision: 2, entries: [styleEntry(.paragraph, id: "Paragraph", name: "Updated", isBase: true)])
        builder.menuNeedsUpdate(paragraph)
        XCTAssertTrue(paragraph.items.first === original)
        builder.menuDidClose(paragraph)
        builder.menuNeedsUpdate(paragraph)
        XCTAssertEqual(paragraph.items.first?.title, "Updated")
    }

    func testUnavailableStyleProviderLeavesHonestDisabledFallback() throws {
        let owner = Owner()
        let provider = StyleProvider(catalogue: nil)
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { provider })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let character = try submenu("Character", of: main)
        builder.menuNeedsUpdate(character)

        XCTAssertEqual(tokens(in: character), ["Base Character", "-", "Edit Styles…"])
        XCTAssertTrue(character.items.filter { !$0.isSeparatorItem }.allSatisfy { !$0.isEnabled })
        XCTAssertEqual(character.items.compactMap(styleAction).map(\.stableID), [
            "Character", "Character",
        ])
    }

    private func submenu(_ title: String, of menu: NSMenu) throws -> NSMenu {
        try XCTUnwrap(menu.item(withTitle: title)?.submenu)
    }

    private func titles(in menu: NSMenu) -> [String] {
        menu.items.filter { !$0.isSeparatorItem }.map(\.title)
    }

    private func tokens(in menu: NSMenu) -> [String] {
        menu.items.map { $0.isSeparatorItem ? "-" : $0.title }
    }

    private func item(at path: String, in menu: NSMenu) throws -> NSMenuItem {
        let components = path.split(separator: "/").map(String.init)
        var current = menu
        var result: NSMenuItem?
        for (index, component) in components.enumerated() {
            result = current.item(withTitle: component)
            guard let result else {
                XCTFail("Missing menu item at \(components.prefix(index + 1).joined(separator: "/"))")
                throw MenuTestError.missingItem
            }
            if index < components.count - 1 {
                guard let submenu = result.submenu else {
                    XCTFail("Missing submenu at \(component)")
                    throw MenuTestError.missingItem
                }
                current = submenu
            }
        }
        return try XCTUnwrap(result)
    }

    private func keyedItems(in menu: NSMenu, prefix: String = "") -> [String: NSMenuItem] {
        var result: [String: NSMenuItem] = [:]
        for menuItem in menu.items where !menuItem.isSeparatorItem {
            let path = prefix.isEmpty ? menuItem.title : "\(prefix)/\(menuItem.title)"
            if !menuItem.keyEquivalent.isEmpty {
                result[path] = menuItem
            }
            if let submenu = menuItem.submenu {
                result.merge(keyedItems(in: submenu, prefix: path)) { first, _ in first }
            }
        }
        return result
    }

    private func allItems(in menu: NSMenu) -> [NSMenuItem] {
        menu.items.flatMap { item in
            [item] + (item.submenu.map(allItems(in:)) ?? [])
        }
    }

    private func styleCatalogue(
        documentRevision: UInt64,
        styleSheetRevision: UInt64,
        entries: [EVStyleMenuEntry]
    ) -> EVStyleMenuCatalogue {
        EVStyleMenuCatalogue(
            documentID: 42,
            documentRevision: documentRevision,
            styleSheetRevision: styleSheetRevision,
            entries: entries,
            canEditStyles: true
        )
    }

    private func styleEntry(
        _ role: EVStyleMenuRole,
        id: String,
        name: String,
        isBase: Bool = false,
        enabled: Bool = false,
        state: NSControl.StateValue = .off
    ) -> EVStyleMenuEntry {
        EVStyleMenuEntry(
            role: role,
            stableID: id,
            displayName: name,
            isBase: isBase,
            presentation: EVMenuItemPresentation(isEnabled: enabled, state: state)
        )
    }

    private func styleAction(_ item: NSMenuItem) -> EVStyleMenuAction? {
        item.representedObject as? EVStyleMenuAction
    }

    private enum MenuTestError: Error {
        case missingItem
    }
}
