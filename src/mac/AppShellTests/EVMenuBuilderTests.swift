import AppKit
import XCTest
@testable import ViemAppShell

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
        @objc func showThemeSettings(_ sender: Any?) {}
        @objc func showHelpItem(_ sender: Any?) {}
    }

    private final class StyleProvider: EVStyleMenuProviding {
        var catalogue: EVStyleMenuCatalogue?

        init(catalogue: EVStyleMenuCatalogue?) {
            self.catalogue = catalogue
        }

        func currentStyleMenuCatalogue() -> EVStyleMenuCatalogue? { catalogue }
    }

    func testTopLevelMenuOrderMatchesSpecification() throws {
        let owner = Owner()
        let application = NSApplication.shared
        // AppKit binds the Window, Services, and Help menus to whichever menus
        // the installed main menu owns, so a menu that was never installed
        // keeps whatever an earlier test left behind. Install this one the way
        // the shell does, and put back what was there.
        let previousMain = application.mainMenu
        let previousWindows = application.windowsMenu
        let previousHelp = application.helpMenu
        let previousServices = application.servicesMenu
        defer {
            application.mainMenu = previousMain
            application.windowsMenu = previousWindows
            application.helpMenu = previousHelp
            application.servicesMenu = previousServices
        }
        let builder = EVMenuBuilder(owner: owner)
        let menu = builder.buildMainMenu(for: application)
        application.mainMenu = menu

        XCTAssertEqual(menu.items.map(\.title), [
            "Viem", "File", "Edit", "Style", "View", "Window", "Help",
        ])
        XCTAssertTrue(application.windowsMenu === menu.item(withTitle: "Window")?.submenu)
        XCTAssertTrue(application.helpMenu === menu.item(withTitle: "Help")?.submenu)
        // The builder attaches a Services submenu under the application menu
        // and offers it to AppKit. AppKit registers a Services menu once per
        // process and ignores every later one, so its identity cannot be
        // asserted from a test that is not the first to build a menu.
        let services = try XCTUnwrap(
            menu.item(withTitle: "Viem")?.submenu?.item(withTitle: "Services")?.submenu)
        XCTAssertEqual(services.title, "Services")
        XCTAssertEqual(application.servicesMenu?.title, "Services")
    }

    func testEveryStaticMenuGroupHasExactTitlesSeparatorsAndOrder() throws {
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner, recentDocumentURLs: { [] })
        let main = builder.buildMainMenu(for: NSApplication.shared)

        XCTAssertEqual(tokens(in: try submenu("Viem", of: main)), [
            "About Viem", "Settings…", "-", "Services", "-", "Hide Viem",
            "Hide Others", "Show All", "-", "Quit Viem",
        ])
        XCTAssertEqual(tokens(in: try submenu("File", of: main)), [
            "New", "Open…", "Open Recent", "-", "Close", "Save", "Save As…",
            "Export…", "Duplicate", "Revert To", "-",
            "Text Encoding", "Line Endings", "-",
            "Page Setup…", "Print…",
        ])
        XCTAssertEqual(tokens(in: try submenu("Edit", of: main)), [
            "Undo", "Redo", "-", "Cut", "Copy", "Copy Source", "Paste", "Paste and Match Style",
            "Delete", "-", "Select All", "Select", "-", "Find", "-",
            "Transformations", "-", "Start Dictation…", "Emoji & Symbols",
        ])
        XCTAssertNil(main.item(withTitle: "Format"))
        XCTAssertNil(try submenu("File", of: main).item(withTitle: "Convert to"))
        XCTAssertNil(try submenu("File", of: main).item(withTitle: "Reinterpret as"))
        XCTAssertEqual(tokens(in: try submenu("View", of: main)), [
            "Plain text", "Markdown", "Code", "-",
            "Show Status Bar", "-", "Word Wrap",
            "Flow Source Paragraphs", "Show Invisible Characters", "-", "Zoom In", "Zoom Out", "Actual Size",
            "-", "Enter Full Screen",
        ])
        XCTAssertEqual(tokens(in: try submenu("Window", of: main)), [
            "Minimize", "Zoom", "-", "New Window for Document", "-",
            "Bring All to Front", "-",
        ])
        XCTAssertEqual(tokens(in: try submenu("Help", of: main)), [
            "-", "Viem Help", "Vim Command Reference", "Keyboard Shortcuts",
            "Supported Vim Commands", "Document Format Compatibility",
            "Round-Trip and Source Preservation", "-", "Release Notes",
            "Report a Problem…",
        ])

        let file = try submenu("File", of: main)
        let edit = try submenu("Edit", of: main)
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

        XCTAssertEqual(tokens(in: try submenu("Character", of: submenu("Style", of: main))), [
            "Default Paragraph",
        ])
        XCTAssertEqual(tokens(in: try submenu("Paragraph", of: submenu("Style", of: main))), [
            "Bulleted List", "Numbered List", "Indent", "Unindent", "-",
            "Base Paragraph", "Heading 1", "Heading 2", "Heading 3", "Heading 4", "Heading 5", "Heading 6",
        ])
        XCTAssertEqual(tokens(in: try submenu("Style", of: main)), [
            "Theme", "-", "Paragraph", "Character", "-", "Edit Styles…",
            "-", "Reload style sheet",
        ])

    }

    func testAllAndOnlySpecifiedKeyEquivalentsAreInstalled() throws {
        let owner = Owner()
        let application = NSApplication.shared
        let previousMain = application.mainMenu
        let previousWindows = application.windowsMenu
        let previousHelp = application.helpMenu
        let previousServices = application.servicesMenu
        defer {
            application.mainMenu = previousMain
            application.windowsMenu = previousWindows
            application.helpMenu = previousHelp
            application.servicesMenu = previousServices
        }
        let builder = EVMenuBuilder(owner: owner)
        let main = builder.buildMainMenu(for: application)
        // AppKit resolves conflicting shortcuts when the menu is installed.
        // Unattached items can report equivalents that the app never displays.
        application.mainMenu = main
        let expected: [String: (String, NSEvent.ModifierFlags)] = [
            "Viem/Settings…": (",", [.command]),
            "Viem/Hide Viem": ("h", [.command]),
            "Viem/Hide Others": ("h", [.command, .option]),
            "Viem/Quit Viem": ("q", [.command]),
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
            "Edit/Copy Source": ("c", [.command, .shift]),
            "Edit/Paste": ("v", [.command]),
            "Edit/Paste and Match Style": ("v", [.command, .option, .shift]),
            "Edit/Select All": ("a", [.command]),
            "Edit/Find/Find…": ("f", [.command]),
            "Edit/Find/Find Next": ("g", [.command]),
            "Edit/Find/Find Previous": ("g", [.command, .shift]),
            "Edit/Find/Use Selection for Find": ("e", [.command]),
            "Edit/Find/Jump to Selection": ("j", [.command]),
            "Edit/Emoji & Symbols": (" ", [.command, .control]),
            "Style/Edit Styles…": (String(UnicodeScalar(NSF8FunctionKey)!), []),
            "Style/Paragraph/Base Paragraph": ("0", [.command]),
            "Style/Paragraph/Heading 1": ("1", [.command]),
            "Style/Paragraph/Heading 2": ("2", [.command]),
            "Style/Paragraph/Heading 3": ("3", [.command]),
            "Style/Paragraph/Heading 4": ("4", [.command]),
            "Style/Paragraph/Heading 5": ("5", [.command]),
            "Style/Paragraph/Heading 6": ("6", [.command]),
            "View/Zoom In": ("=", [.command]),
            "View/Zoom Out": ("-", [.command]),
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

    func testEveryMenuCommandAppearsExactlyOnceAndUsesItsExpectedRoute() {
        let owner = Owner()
        let main = EVMenuBuilder(owner: owner).buildMainMenu(for: NSApplication.shared)
        let tagged = allItems(in: main).compactMap { item -> (EVMenuCommand, NSMenuItem)? in
            EVMenuCommand(rawValue: item.tag).map { ($0, item) }
        }
        let grouped = Dictionary(grouping: tagged, by: \.0)

        let commandsWithoutMenuItems: Set<EVMenuCommand> = [
            .baseParagraphStyle, .removeList, .bold, .italic, .strikethrough,
        ]
        let menuCommands = Set(EVMenuCommand.allCases).subtracting(commandsWithoutMenuItems)
        XCTAssertEqual(Set(grouped.keys), menuCommands)
        for command in menuCommands {
            XCTAssertEqual(grouped[command]?.count, 1, "Unexpected menu count for \(command)")
        }

        let nativeDocumentCommands: Set<EVMenuCommand> = [
            .save, .saveAs, .duplicateDocument,
            .revertLastSaved, .browseVersions, .pageSetup, .printDocument,
        ]
        let styleCommands: Set<EVMenuCommand> = [
            .defaultParagraphStyle, .baseParagraphStyle,
        ]
        let coreSelector = #selector(EVEditorCommandRouting.performEditorMenuCommand(_:))
        let styleSelector = #selector(EVStyleMenuActionRouting.performEditorStyleMenuAction(_:))
        for (command, item) in tagged {
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
            "Viem/Settings…": #selector(EVApplicationCommandRouting.showSettings(_:)),
            "Style/Theme/Theme Settings…": #selector(EVApplicationCommandRouting.showThemeSettings(_:)),
            "File/New": #selector(EVApplicationCommandRouting.newDocument(_:)),
            "File/Open…": #selector(EVApplicationCommandRouting.openDocument(_:)),
        ]
        for (path, selector) in applicationRoutes {
            let menuItem = try item(at: path, in: main)
            XCTAssertEqual(menuItem.action, selector, path)
            XCTAssertTrue(menuItem.target === owner, path)
        }

        let responderRoutes: [String: Selector] = [
            "Viem/About Viem": #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
            "Viem/Hide Viem": #selector(NSApplication.hide(_:)),
            "Viem/Hide Others": #selector(NSApplication.hideOtherApplications(_:)),
            "Viem/Show All": #selector(NSApplication.unhideAllApplications(_:)),
            "Viem/Quit Viem": #selector(NSApplication.terminate(_:)),
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
            "Viem Help", "Vim Command Reference", "Keyboard Shortcuts",
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

    func testRecentDocumentsUseMinimalUniqueSuffixesForEveryDuplicateName() throws {
        let owner = Owner()
        let paths = [
            "/Projects/first/src/main.rs",
            "/Projects/notes.md",
            "/Projects/second/src/main.rs",
            "/Projects/third/tests/main.rs",
            "/Other/Project/README.md",
            "/Other/Project/readme.md",
        ]
        let urls = paths.map { URL(fileURLWithPath: $0) }
        let builder = EVMenuBuilder(owner: owner, recentDocumentURLs: { urls })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let recent = try submenu("Open Recent", of: submenu("File", of: main))
        builder.menuNeedsUpdate(recent)

        XCTAssertEqual(tokens(in: recent), [
            "first/src/main.rs", "notes.md", "second/src/main.rs", "tests/main.rs",
            "README.md", "readme.md", "-", "Clear Menu",
        ])
        for (index, url) in urls.enumerated() {
            let item = recent.items[index]
            XCTAssertEqual(item.representedObject as? URL, url, "Input recency order is preserved")
            XCTAssertEqual(item.toolTip, url.path)
            XCTAssertTrue(NSApplication.shared.sendAction(try XCTUnwrap(item.action), to: item.target, from: item))
        }
        XCTAssertEqual(owner.openedRecentURLs, urls, "Labels must never replace the full reopening URL")
    }

    func testRecentDocumentsDisambiguateUnequalDepthsWithoutDuplicateRootSeparators() throws {
        let owner = Owner()
        let urls = ["/report.txt", "/archive/report.txt", "/archive/archive/report.txt"]
            .map { URL(fileURLWithPath: $0) }
        let builder = EVMenuBuilder(owner: owner, recentDocumentURLs: { urls })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let recent = try submenu("Open Recent", of: submenu("File", of: main))
        builder.menuNeedsUpdate(recent)

        XCTAssertEqual(tokens(in: recent), [
            "/report.txt", "/archive/report.txt", "archive/archive/report.txt", "-", "Clear Menu",
        ])
        XCTAssertEqual(Set(recent.items.prefix(urls.count).map(\.title)).count, urls.count)
    }

    func testRecentDocumentsPreserveUnicodeSpacesAndExactFilenameExtensions() throws {
        let owner = Owner()
        var urls = [
            "/稿件/第一 稿/记录.v1.md",
            "/稿件/第二 稿/记录.v1.md",
            "/drafts/Café notes.TXT",
            "/drafts/Café notes.txt",
            "/drafts/extensionless",
        ].map { URL(fileURLWithPath: $0) }
        let builder = EVMenuBuilder(owner: owner, recentDocumentURLs: { urls })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let recent = try submenu("Open Recent", of: submenu("File", of: main))
        builder.menuNeedsUpdate(recent)
        XCTAssertEqual(tokens(in: recent), [
            "第一 稿/记录.v1.md", "第二 稿/记录.v1.md", "Café notes.TXT", "Café notes.txt",
            "extensionless", "-", "Clear Menu",
        ])

        urls.remove(at: 1)
        builder.menuNeedsUpdate(recent)
        XCTAssertEqual(recent.items.first?.title, "记录.v1.md", "Removed collisions shorten the remaining entry")
        XCTAssertEqual(recent.items.first?.representedObject as? URL, urls.first)
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

    func testNoBareViemTextKeyIsRegisteredAsAGlobalShortcut() {
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner)
        let menu = builder.buildMainMenu(for: NSApplication.shared)

        for item in allItems(in: menu) where !item.keyEquivalent.isEmpty {
            if [EVMenuCommand.editStyles]
                .contains(where: { $0.rawValue == item.tag }) {
                XCTAssertEqual(item.keyEquivalent, String(UnicodeScalar(NSF8FunctionKey)!))
                XCTAssertTrue(item.keyEquivalentModifierMask.isEmpty)
                continue
            }
            let modifiers = item.keyEquivalentModifierMask
            XCTAssertFalse(
                modifiers.intersection([.command, .control, .option]).isEmpty,
                "\(item.title) registers bare key \(item.keyEquivalent)"
            )
        }
    }

    func testTextEncodingSubmenuHasTheFourFormerStatusChoicesAndStableActions() throws {
        let owner = Owner()
        let builder = EVMenuBuilder(owner: owner)
        let menu = builder.buildMainMenu(for: NSApplication.shared)
        let file = try submenu("File", of: menu)
        XCTAssertNil(file.item(withTitle: "Text Encoding…"))
        let encodings = try submenu("Text Encoding", of: file)
        XCTAssertTrue(encodings.showsStateColumn)
        XCTAssertEqual(titles(in: encodings), ["UTF-8", "Latin-1", "UTF-16 LE", "UTF-16 BE"])
        XCTAssertEqual(encodings.items.map(\.tag), [EVMenuCommand.encodingUTF8, .encodingLatin1, .encodingUTF16LE, .encodingUTF16BE].map(\.rawValue))
        for item in encodings.items {
            XCTAssertEqual(item.action, #selector(EVEditorCommandRouting.performEditorMenuCommand(_:)))
            XCTAssertNil(item.target)
        }
        XCTAssertNil(try submenu("View", of: menu).item(withTitle: "Wrap at Word Boundaries"))
    }

    func testStyleMenusReserveOneNativeCheckmarkColumnForEveryRow() throws {
        let owner = Owner()
        let provider = StyleProvider(catalogue: styleCatalogue(documentRevision: 1, styleSheetRevision: 1, entries: [
            styleEntry(.paragraph, id: "Paragraph", name: "Base Paragraph", isBase: true, state: .on),
            styleEntry(.paragraph, id: "Heading1", name: "Heading 1"),
            styleEntry(.character, id: "", name: "Default Paragraph", isBase: true, state: .on),
            styleEntry(.character, id: "Accent", name: "Accent"),
        ]))
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { provider })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        for title in ["Paragraph", "Character"] {
            let menu = try submenu(title, of: submenu("Style", of: main))
            XCTAssertTrue(menu.showsStateColumn)
            builder.menuNeedsUpdate(menu)
            XCTAssertTrue(menu.showsStateColumn)
            XCTAssertEqual(menu.items.filter { $0.state == .on }.count, 1)
            for item in menu.items where !item.isSeparatorItem {
                XCTAssertNil(item.view, "Native rows share the state column")
                XCTAssertEqual(item.indentationLevel, 0)
            }
            XCTAssertEqual(menu.items.last?.state, .off)
            let actions = menu.items.compactMap(styleAction)
            XCTAssertTrue(actions.allSatisfy(\.marksCurrentStyle))
            XCTAssertNil(menu.item(withTitle: "Edit Styles…"))
        }
    }

    func testStyleMenusRefreshFromCurrentCatalogueWithBaseFirstAndStableIDs() throws {
        let owner = Owner()
        let provider = StyleProvider(catalogue: styleCatalogue(
            documentRevision: 7,
            styleSheetRevision: 11,
            entries: [
                styleEntry(.paragraph, id: "Heading2", name: "Heading", state: .mixed),
                styleEntry(.character, id: "", name: "Default Paragraph", isBase: true),
                styleEntry(.paragraph, id: "Paragraph", name: "Base Paragraph", isBase: true),
                styleEntry(.paragraph, id: "Heading1", name: "Heading", enabled: true, state: .on),
            ]
        ))
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { provider })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let paragraph = try submenu("Paragraph", of: submenu("Style", of: main))

        builder.menuNeedsUpdate(paragraph)

        XCTAssertEqual(tokens(in: paragraph), [
            "Bulleted List", "Numbered List", "Indent", "Unindent", "-",
            "Base Paragraph", "Heading", "Heading",
        ])
        XCTAssertTrue(paragraph.autoenablesItems)
        let styleItems = paragraph.items.filter { styleAction($0)?.kind == .assign }
        XCTAssertEqual(styleItems.compactMap(styleAction).map(\.stableID), [
            "Paragraph", "Heading2", "Heading1",
        ])
        XCTAssertEqual(styleItems.map(\.isEnabled), [false, false, true])
        XCTAssertEqual(styleItems.map(\.state), [.off, .mixed, .on])
        XCTAssertTrue(styleItems.allSatisfy {
            $0.action == #selector(EVEditorCommandRouting.performEditorMenuCommand(_:))
                && $0.target == nil
        })

        XCTAssertTrue(styleItems.compactMap(styleAction).allSatisfy {
            $0.documentID == 42 && $0.documentRevision == 7 && $0.styleSheetRevision == 11
        })
        XCTAssertNil(paragraph.item(withTitle: "Edit Styles…"))
    }

    func testCodeStyleMenuPreservesEditAndExplicitDefinitionActions() throws {
        let owner = Owner()
        let provider = StyleProvider(catalogue: styleCatalogue(documentRevision: 1, styleSheetRevision: 4, entries: [
            EVStyleMenuEntry(role: .character, stableID: "", displayName: "Default Paragraph",
                             isBase: true, presentation: .enabled, actionKind: .edit),
            EVStyleMenuEntry(role: .character, stableID: "keyword-id", displayName: "Keyword",
                             isBase: false, presentation: .enabled, actionKind: .edit),
            EVStyleMenuEntry(role: .character, stableID: "", displayName: "Define Custom…",
                             isBase: false, presentation: .enabled, actionKind: .defineSyntax, syntaxName: "Custom"),
        ]))
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { provider })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let menu = try submenu("Character", of: submenu("Style", of: main))
        builder.menuNeedsUpdate(menu)
        XCTAssertEqual(tokens(in: menu), ["Default Paragraph", "Keyword", "Define Custom…"])
        let actions = menu.items.compactMap(styleAction)
        XCTAssertEqual(actions.map(\.kind), [.edit, .edit, .defineSyntax])
        XCTAssertEqual(actions.map(\.marksCurrentStyle), [true, true, true])
        XCTAssertEqual(actions[2].syntaxName, "Custom")
        XCTAssertEqual(actions[2].stableID, "")
        for item in menu.items where !item.isSeparatorItem {
            XCTAssertEqual(item.action, #selector(EVStyleMenuActionRouting.performEditorStyleMenuAction(_:)))
            XCTAssertTrue(item.keyEquivalent.isEmpty)
        }
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
        let paragraph = try submenu("Paragraph", of: submenu("Style", of: main))
        builder.menuNeedsUpdate(paragraph)
        XCTAssertEqual(titles(in: paragraph), ["Bulleted List", "Numbered List", "Indent", "Unindent", "Base Paragraph", "Old Name"])

        provider.catalogue = styleCatalogue(
            documentRevision: 2,
            styleSheetRevision: 3,
            entries: [
                styleEntry(.paragraph, id: "Heading7", name: "New Name"),
                styleEntry(.paragraph, id: "Paragraph", name: "Renamed Base", isBase: true),
            ]
        )
        builder.menuNeedsUpdate(paragraph)

        XCTAssertEqual(titles(in: paragraph), ["Bulleted List", "Numbered List", "Indent", "Unindent", "Renamed Base", "New Name"])
        let actions = paragraph.items.compactMap(styleAction)
        XCTAssertEqual(actions.map(\.stableID), ["Paragraph", "Heading7"])
        XCTAssertTrue(actions.allSatisfy { $0.documentRevision == 2 })
        XCTAssertTrue(actions.allSatisfy { $0.styleSheetRevision == 3 })
    }

    func testStyleMenuDoesNotReplaceTrackedItems() throws {
        let owner = Owner()
        let provider = StyleProvider(catalogue: styleCatalogue(documentRevision: 1, styleSheetRevision: 1, entries: [styleEntry(.paragraph, id: "Paragraph", name: "Base Paragraph", isBase: true)]))
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { provider })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let paragraph = try submenu("Paragraph", of: submenu("Style", of: main))
        builder.menuWillOpen(paragraph)
        let original = try XCTUnwrap(paragraph.items.first)
        builder.menuNeedsUpdate(paragraph)
        XCTAssertTrue(paragraph.items.first === original)
        provider.catalogue = styleCatalogue(documentRevision: 2, styleSheetRevision: 2, entries: [styleEntry(.paragraph, id: "Paragraph", name: "Updated", isBase: true)])
        builder.menuNeedsUpdate(paragraph)
        XCTAssertTrue(paragraph.items.first === original)
        builder.menuDidClose(paragraph)
        builder.menuNeedsUpdate(paragraph)
        XCTAssertEqual(paragraph.items.first(where: { styleAction($0)?.kind == .assign })?.title, "Updated")
    }

    func testUnavailableStyleProviderLeavesHonestDisabledFallback() throws {
        let owner = Owner()
        let provider = StyleProvider(catalogue: nil)
        let builder = EVMenuBuilder(owner: owner, styleMenuProvider: { provider })
        let main = builder.buildMainMenu(for: NSApplication.shared)
        let character = try submenu("Character", of: submenu("Style", of: main))
        builder.menuNeedsUpdate(character)

        XCTAssertEqual(tokens(in: character), ["Default Paragraph"])
        XCTAssertTrue(character.items.filter { !$0.isSeparatorItem }.allSatisfy { !$0.isEnabled })
        XCTAssertEqual(character.items.compactMap(styleAction).map(\.stableID), [
            "",
        ])
        XCTAssertEqual(character.items.compactMap(styleAction).map(\.marksCurrentStyle), [true])
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
