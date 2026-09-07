import AppKit

@MainActor
@objc public protocol EVApplicationCommandRouting: AnyObject {
    func newDocument(_ sender: Any?)
    func openDocument(_ sender: Any?)
    func openRecentDocument(_ sender: Any?)
    func clearRecentDocuments(_ sender: Any?)
    func showSettings(_ sender: Any?)
    func showHelpItem(_ sender: Any?)
}

@MainActor
public final class EVMenuBuilder: NSObject, NSMenuDelegate {
    private weak var owner: (any EVApplicationCommandRouting)?
    private weak var recentDocumentsMenu: NSMenu?
    private let recentDocumentURLs: @MainActor () -> [URL]
    private let styleMenuProvider: @MainActor () -> (any EVStyleMenuProviding)?
    private var styleMenuRoles: [ObjectIdentifier: EVStyleMenuRole] = [:]
    private var styleMenuCatalogues: [ObjectIdentifier: EVStyleMenuCatalogue] = [:]
    private var openTypeMenu: NSMenu?
    private var trackingMenus = Set<ObjectIdentifier>()

    public init(
        owner: any EVApplicationCommandRouting,
        recentDocumentURLs: @escaping @MainActor () -> [URL] = {
            NSDocumentController.shared.recentDocumentURLs
        },
        styleMenuProvider: (@MainActor () -> (any EVStyleMenuProviding)?)? = nil
    ) {
        self.owner = owner
        self.recentDocumentURLs = recentDocumentURLs
        self.styleMenuProvider = styleMenuProvider ?? {
            EVMenuBuilder.focusedStyleMenuProvider()
        }
        super.init()
    }

    public func buildMainMenu(for application: NSApplication) -> NSMenu {
        let mainMenu = NSMenu(title: "Main Menu")
        mainMenu.addItem(topLevelItem("eVim", submenu: makeApplicationMenu(for: application)))
        mainMenu.addItem(topLevelItem("File", submenu: makeFileMenu()))
        mainMenu.addItem(topLevelItem("Edit", submenu: makeEditMenu()))
        mainMenu.addItem(topLevelItem("Format", submenu: makeFormatMenu()))
        mainMenu.addItem(makeStyleMenu(title: "Paragraph", role: .paragraph,
            baseTitle: "Base Paragraph", baseCommand: .baseParagraphStyle, editCommand: .editParagraphStyles))
        mainMenu.addItem(makeStyleMenu(title: "Character", role: .character,
            baseTitle: "Base Character", baseCommand: .baseCharacterStyle, editCommand: .editCharacterStyles))
        mainMenu.addItem(topLevelItem("View", submenu: makeViewMenu()))

        let windowMenu = makeWindowMenu(for: application)
        mainMenu.addItem(topLevelItem("Window", submenu: windowMenu))
        application.windowsMenu = windowMenu

        let helpMenu = makeHelpMenu()
        mainMenu.addItem(topLevelItem("Help", submenu: helpMenu))
        application.helpMenu = helpMenu

        return mainMenu
    }

    public func menuWillOpen(_ menu: NSMenu) {
        menuNeedsUpdate(menu)
        trackingMenus.insert(ObjectIdentifier(menu))
    }

    public func menuDidClose(_ menu: NSMenu) {
        trackingMenus.remove(ObjectIdentifier(menu))
    }

    public func menuNeedsUpdate(_ menu: NSMenu) {
        // AppKit may request validation again while a mouse is held down.
        // Keep the tracked NSMenuItem objects and their geometry stable.
        guard !trackingMenus.contains(ObjectIdentifier(menu)) else { return }
        if menu === openTypeMenu {
            if let provider = styleMenuProvider() as? any EVOpenTypeMenuProviding {
                provider.populateOpenTypeFeatureMenu(menu)
            } else { menu.removeAllItems() }
            return
        }
        if let role = styleMenuRoles[ObjectIdentifier(menu)] {
            rebuildStyleMenu(menu, role: role)
            return
        }

        guard menu === recentDocumentsMenu else { return }

        menu.removeAllItems()
        let urls = recentDocumentURLs()
        for url in urls {
            let item = NSMenuItem(
                title: FileManager.default.displayName(atPath: url.path),
                action: #selector(EVApplicationCommandRouting.openRecentDocument(_:)),
                keyEquivalent: ""
            )
            item.representedObject = url
            item.toolTip = url.path
            item.target = owner
            menu.addItem(item)
        }

        if !urls.isEmpty {
            menu.addItem(.separator())
        }
        let clear = applicationItem(
            "Clear Menu",
            action: #selector(EVApplicationCommandRouting.clearRecentDocuments(_:))
        )
        clear.isEnabled = !urls.isEmpty
        menu.addItem(clear)
    }

    private func makeApplicationMenu(for application: NSApplication) -> NSMenu {
        let menu = NSMenu(title: "eVim")
        menu.addItem(responderItem(
            "About eVim",
            action: #selector(NSApplication.orderFrontStandardAboutPanel(_:))
        ))
        menu.addItem(applicationItem(
            "Settings…",
            action: #selector(EVApplicationCommandRouting.showSettings(_:)),
            key: ","
        ))
        menu.addItem(.separator())

        let services = NSMenu(title: "Services")
        let servicesItem = NSMenuItem(title: "Services", action: nil, keyEquivalent: "")
        servicesItem.submenu = services
        menu.addItem(servicesItem)
        application.servicesMenu = services

        menu.addItem(.separator())
        menu.addItem(responderItem("Hide eVim", action: #selector(NSApplication.hide(_:)), key: "h"))
        menu.addItem(responderItem(
            "Hide Others",
            action: #selector(NSApplication.hideOtherApplications(_:)),
            key: "h",
            modifiers: [.command, .option]
        ))
        menu.addItem(responderItem("Show All", action: #selector(NSApplication.unhideAllApplications(_:))))
        menu.addItem(.separator())
        menu.addItem(responderItem("Quit eVim", action: #selector(NSApplication.terminate(_:)), key: "q"))
        return menu
    }

    private func makeFileMenu() -> NSMenu {
        let menu = NSMenu(title: "File")
        menu.addItem(applicationItem(
            "New",
            action: #selector(EVApplicationCommandRouting.newDocument(_:)),
            key: "n"
        ))
        menu.addItem(applicationItem(
            "Open…",
            action: #selector(EVApplicationCommandRouting.openDocument(_:)),
            key: "o"
        ))

        let recentMenu = NSMenu(title: "Open Recent")
        recentMenu.delegate = self
        recentDocumentsMenu = recentMenu
        let recentItem = NSMenuItem(title: "Open Recent", action: nil, keyEquivalent: "")
        recentItem.submenu = recentMenu
        menu.addItem(recentItem)

        menu.addItem(.separator())
        menu.addItem(responderItem("Close", action: #selector(NSWindow.performClose(_:)), key: "w"))
        menu.addItem(documentItem(
            "Save",
            action: #selector(NSDocument.save(_:)),
            command: .save,
            key: "s"
        ))
        menu.addItem(documentItem(
            "Save As…",
            action: #selector(NSDocument.saveAs(_:)),
            command: .saveAs,
            key: "s",
            modifiers: [.command, .shift]
        ))
        menu.addItem(documentItem(
            "Duplicate",
            action: #selector(NSDocument.duplicate(_:)),
            command: .duplicateDocument
        ))
        menu.addItem(documentItem(
            "Rename…",
            action: #selector(NSDocument.rename(_:)),
            command: .renameDocument
        ))
        menu.addItem(documentItem(
            "Move To…",
            action: #selector(NSDocument.move(_:)),
            command: .moveDocument
        ))

        let revertMenu = NSMenu(title: "Revert To")
        revertMenu.addItem(documentItem(
            "Last Saved Version",
            action: #selector(NSDocument.revertToSaved(_:)),
            command: .revertLastSaved
        ))
        revertMenu.addItem(documentItem(
            "Browse All Versions…",
            action: #selector(NSDocument.browseVersions(_:)),
            command: .browseVersions
        ))
        let revertItem = NSMenuItem(title: "Revert To", action: nil, keyEquivalent: "")
        revertItem.submenu = revertMenu
        menu.addItem(revertItem)

        menu.addItem(.separator())
        menu.addItem(coreItem("Document Format…", command: .documentFormat))
        menu.addItem(coreItem("Text Encoding…", command: .textEncoding))

        let endings = NSMenu(title: "Line Endings")
        endings.addItem(coreItem("Unix (LF)", command: .lineEndingUnix))
        endings.addItem(coreItem("Windows (CRLF)", command: .lineEndingWindows))
        endings.addItem(coreItem("Classic Mac (CR)", command: .lineEndingClassicMac))
        let endingsItem = NSMenuItem(title: "Line Endings", action: nil, keyEquivalent: "")
        endingsItem.submenu = endings
        menu.addItem(endingsItem)

        menu.addItem(.separator())
        menu.addItem(documentItem(
            "Page Setup…",
            action: #selector(NSDocument.runPageLayout(_:)),
            command: .pageSetup
        ))
        menu.addItem(documentItem(
            "Print…",
            action: #selector(NSDocument.printDocument(_:)),
            command: .printDocument,
            key: "p"
        ))
        return menu
    }

    private func makeEditMenu() -> NSMenu {
        let menu = NSMenu(title: "Edit")
        menu.addItem(coreItem("Undo", command: .undo, key: "z"))
        menu.addItem(coreItem("Redo", command: .redo, key: "z", modifiers: [.command, .shift]))
        menu.addItem(.separator())
        menu.addItem(coreItem("Cut", command: .cut, key: "x"))
        menu.addItem(coreItem("Copy", command: .copy, key: "c"))
        menu.addItem(coreItem("Paste", command: .paste, key: "v"))
        menu.addItem(coreItem(
            "Paste and Match Style",
            command: .pasteAndMatchStyle,
            key: "v",
            modifiers: [.command, .option, .shift]
        ))
        menu.addItem(coreItem("Delete", command: .delete))
        menu.addItem(.separator())
        menu.addItem(coreItem("Select All", command: .selectAll, key: "a"))

        let select = NSMenu(title: "Select")
        select.addItem(coreItem("Word", command: .selectWord))
        select.addItem(coreItem("Sentence", command: .selectSentence))
        select.addItem(coreItem("Paragraph", command: .selectParagraph))
        select.addItem(coreItem("Hard Line", command: .selectHardLine))
        select.addItem(coreItem("Visual Row", command: .selectVisualRow))
        menu.addItem(submenuItem("Select", submenu: select))

        menu.addItem(.separator())
        let find = NSMenu(title: "Find")
        find.addItem(coreItem("Find…", command: .find, key: "f"))
        find.addItem(coreItem("Find and Replace…", command: .findAndReplace))
        find.addItem(coreItem("Find Next", command: .findNext, key: "g"))
        find.addItem(coreItem(
            "Find Previous",
            command: .findPrevious,
            key: "g",
            modifiers: [.command, .shift]
        ))
        find.addItem(coreItem("Use Selection for Find", command: .useSelectionForFind, key: "e"))
        find.addItem(coreItem("Jump to Selection", command: .jumpToSelection, key: "j"))
        menu.addItem(submenuItem("Find", submenu: find))

        menu.addItem(.separator())
        let transformations = NSMenu(title: "Transformations")
        transformations.addItem(coreItem("Make Uppercase", command: .makeUppercase))
        transformations.addItem(coreItem("Make Lowercase", command: .makeLowercase))
        transformations.addItem(coreItem("Toggle Case", command: .toggleCase))
        menu.addItem(submenuItem("Transformations", submenu: transformations))

        menu.addItem(.separator())
        menu.addItem(responderItem("Start Dictation…", action: Selector(("startDictation:"))))
        menu.addItem(responderItem(
            "Emoji & Symbols",
            action: #selector(NSApplication.orderFrontCharacterPalette(_:)),
            key: " ",
            modifiers: [.command, .control]
        ))
        return menu
    }

    private func makeFormatMenu() -> NSMenu {
        let menu = NSMenu(title: "Format")

        let font = NSMenu(title: "Font")
        font.addItem(coreItem("Show Fonts", command: .showFonts, key: "t"))
        font.addItem(.separator())
        font.addItem(coreItem("Bold", command: .bold, key: "b"))
        font.addItem(coreItem("Italic", command: .italic, key: "i"))
        font.addItem(coreItem("Underline", command: .underline, key: "u"))
        font.addItem(coreItem("Strikethrough", command: .strikethrough))
        font.addItem(.separator())
        font.addItem(coreItem("Bigger", command: .bigger))
        font.addItem(coreItem("Smaller", command: .smaller))
        font.addItem(.separator())

        let ligatures = NSMenu(title: "Ligatures")
        ligatures.addItem(coreItem("Use Default Ligatures", command: .defaultLigatures))
        ligatures.addItem(coreItem("Use All Ligatures", command: .allLigatures))
        ligatures.addItem(coreItem("Use No Ligatures", command: .noLigatures))
        font.addItem(submenuItem("Ligatures", submenu: ligatures))

        let kerning = NSMenu(title: "Kerning")
        kerning.addItem(coreItem("Use Default Kerning", command: .defaultKerning))
        kerning.addItem(coreItem("Use No Kerning", command: .noKerning))
        font.addItem(submenuItem("Kerning", submenu: kerning))

        let baseline = NSMenu(title: "Baseline")
        baseline.addItem(coreItem("Superscript", command: .superscript))
        baseline.addItem(coreItem("Subscript", command: .subscriptBaseline))
        baseline.addItem(coreItem("Raise", command: .raiseBaseline))
        baseline.addItem(coreItem("Lower", command: .lowerBaseline))
        font.addItem(submenuItem("Baseline", submenu: baseline))
        let features = NSMenu(title: "OpenType Features")
        features.delegate = self
        openTypeMenu = features
        let featuresItem = submenuItem("OpenType Features", submenu: features)
        featuresItem.tag = EVMenuCommand.openTypeFeatures.rawValue
        font.addItem(featuresItem)
        for item in font.items { font.removeItem(item); menu.addItem(item) }

        let color = NSMenu(title: "Color")
        color.addItem(coreItem("Show Colors", command: .showColors))
        color.addItem(coreItem("Text Color…", command: .textColor))
        color.addItem(coreItem("Highlight Color…", command: .highlightColor))
        menu.addItem(.separator())
        for item in color.items { color.removeItem(item); menu.addItem(item) }
        menu.addItem(.separator())

        let styles = NSMenu(title: "Style")
        styles.addItem(coreItem("Edit document style…", command: .editDocumentStyles))
        styles.addItem(coreItem("Save as default text style", command: .saveDefaultStyle))
        menu.addItem(submenuItem("Style", submenu: styles))
        menu.addItem(.separator())

        let paragraph = NSMenu(title: "Paragraph")
        let alignment = NSMenu(title: "Alignment")
        alignment.addItem(coreItem("Start", command: .alignStart))
        alignment.addItem(coreItem("Center", command: .alignCenter))
        alignment.addItem(coreItem("End", command: .alignEnd))
        paragraph.addItem(submenuItem("Alignment", submenu: alignment))

        let direction = NSMenu(title: "Writing Direction")
        direction.addItem(coreItem("Automatic", command: .directionAutomatic))
        direction.addItem(coreItem("Left to Right", command: .directionLeftToRight))
        direction.addItem(coreItem("Right to Left", command: .directionRightToLeft))
        paragraph.addItem(submenuItem("Writing Direction", submenu: direction))
        paragraph.addItem(coreItem("Increase Indent", command: .increaseIndent))
        paragraph.addItem(coreItem("Decrease Indent", command: .decreaseIndent))
        paragraph.addItem(coreItem("Paragraph Spacing…", command: .paragraphSpacing))

        let lineSpacing = NSMenu(title: "Line Spacing")
        lineSpacing.addItem(coreItem("Normal", command: .lineSpacingNormal))
        lineSpacing.addItem(coreItem("Single", command: .lineSpacingSingle))
        lineSpacing.addItem(coreItem("1.5 Lines", command: .lineSpacingOneAndHalf))
        lineSpacing.addItem(coreItem("Double", command: .lineSpacingDouble))
        lineSpacing.addItem(coreItem("Custom…", command: .lineSpacingCustom))
        paragraph.addItem(submenuItem("Line Spacing", submenu: lineSpacing))
        let lists = NSMenu(title: "List")
        lists.addItem(coreItem("Bulleted List", command: .bulletedList))
        lists.addItem(coreItem("Numbered List", command: .numberedList))
        lists.addItem(coreItem("Remove List", command: .removeList))
        paragraph.addItem(submenuItem("List", submenu: lists))
        menu.addItem(submenuItem("Paragraph", submenu: paragraph))

        menu.addItem(.separator())
        menu.addItem(coreItem("Copy Style", command: .copyStyle))
        menu.addItem(coreItem("Paste Style", command: .pasteStyle))
        menu.addItem(coreItem(
            "Clear Direct Character Formatting",
            command: .clearDirectCharacterFormatting
        ))
        menu.addItem(coreItem(
            "Clear Direct Paragraph Formatting",
            command: .clearDirectParagraphFormatting
        ))
        menu.addItem(coreItem("Clear All Direct Formatting", command: .clearAllDirectFormatting))
        return menu
    }

    private func makeViewMenu() -> NSMenu {
        let menu = NSMenu(title: "View")
        menu.addItem(responderItem(
            "Show Status Bar",
            action: #selector(EVDocumentContentViewController.toggleStatusBar(_:))
        ))
        menu.addItem(.separator())
        menu.addItem(coreItem("Word Wrap", command: .wordWrap))
        menu.addItem(coreItem("Wrap at Word Boundaries", command: .wrapAtWordBoundaries))
        menu.addItem(coreItem("Flow Source Paragraphs", command: .flowParagraphs))
        menu.addItem(coreItem("Show Invisible Characters", command: .showInvisibleCharacters))
        menu.addItem(.separator())
        menu.addItem(coreItem("Zoom In", command: .zoomIn))
        menu.addItem(coreItem("Zoom Out", command: .zoomOut))
        menu.addItem(coreItem("Actual Size", command: .actualSize))
        menu.addItem(.separator())
        menu.addItem(responderItem(
            "Enter Full Screen",
            action: #selector(NSWindow.toggleFullScreen(_:)),
            key: "f",
            modifiers: [.command, .control]
        ))
        return menu
    }

    private func makeWindowMenu(for application: NSApplication) -> NSMenu {
        let menu = NSMenu(title: "Window")
        menu.addItem(responderItem("Minimize", action: #selector(NSWindow.performMiniaturize(_:)), key: "m"))
        menu.addItem(responderItem("Zoom", action: #selector(NSWindow.performZoom(_:))))
        menu.addItem(.separator())
        menu.addItem(coreItem("New Window for Document", command: .newWindowForDocument))
        menu.addItem(.separator())
        let front = responderItem("Bring All to Front", action: #selector(NSApplication.arrangeInFront(_:)))
        front.target = application
        menu.addItem(front)
        menu.addItem(.separator())
        return menu
    }

    private func makeHelpMenu() -> NSMenu {
        let menu = NSMenu(title: "Help")
        menu.addItem(.separator())
        for title in [
            "eVim Help",
            "Vim Command Reference",
            "Keyboard Shortcuts",
            "Supported Vim Commands",
            "Document Format Compatibility",
            "Round-Trip and Source Preservation",
        ] {
            let item = applicationItem(title, action: #selector(EVApplicationCommandRouting.showHelpItem(_:)))
            item.representedObject = title
            menu.addItem(item)
        }
        menu.addItem(.separator())
        for title in ["Release Notes", "Report a Problem…"] {
            let item = applicationItem(title, action: #selector(EVApplicationCommandRouting.showHelpItem(_:)))
            item.representedObject = title
            menu.addItem(item)
        }
        return menu
    }

    private func makeStyleMenu(
        title: String,
        role: EVStyleMenuRole,
        baseTitle: String,
        baseCommand: EVMenuCommand,
        editCommand: EVMenuCommand
    ) -> NSMenuItem {
        let submenu = NSMenu(title: title)
        submenu.delegate = self
        submenu.autoenablesItems = true
        styleMenuRoles[ObjectIdentifier(submenu)] = role
        populateUnavailableStyleMenu(
            submenu,
            role: role,
            baseTitle: baseTitle,
            baseCommand: baseCommand,
            editCommand: editCommand
        )
        return submenuItem(title, submenu: submenu)
    }

    private func rebuildStyleMenu(_ menu: NSMenu, role: EVStyleMenuRole) {
        let specification = styleMenuSpecification(for: role)
        guard let provider = styleMenuProvider(),
              let catalogue = provider.currentStyleMenuCatalogue()
        else {
            populateUnavailableStyleMenu(
                menu,
                role: role,
                baseTitle: specification.baseTitle,
                baseCommand: specification.selectionCommand,
                editCommand: specification.editCommand
            )
            return
        }

        let menuID = ObjectIdentifier(menu)
        guard styleMenuCatalogues[menuID] != catalogue else { return }
        styleMenuCatalogues[menuID] = catalogue
        let matching = catalogue.entries.filter { $0.role == role }
        let baseEntries = matching.filter(\.isBase)
        let namedEntries = matching.filter { !$0.isBase }
        let entries: [EVStyleMenuEntry]
        if baseEntries.isEmpty {
            entries = [unavailableBaseEntry(role: role, title: specification.baseTitle)]
                + namedEntries
        } else {
            entries = baseEntries + namedEntries
        }

        menu.removeAllItems()
        for entry in entries {
            menu.addItem(styleActionItem(
                title: entry.displayName,
                command: specification.selectionCommand,
                payload: EVStyleMenuAction(
                    kind: .assign,
                    role: role,
                    stableID: entry.stableID,
                    documentID: catalogue.documentID,
                    documentRevision: catalogue.documentRevision,
                    styleSheetRevision: catalogue.styleSheetRevision
                ),
                presentation: entry.presentation
            ))
        }
        menu.addItem(.separator())

        let requestedBaseID = entries.first(where: \.isBase)?.stableID
            ?? specification.baseStableID
        menu.addItem(styleActionItem(
            title: "Edit Styles…",
            command: specification.editCommand,
            payload: EVStyleMenuAction(
                kind: .edit,
                role: role,
                stableID: requestedBaseID,
                documentID: catalogue.documentID,
                documentRevision: catalogue.documentRevision,
                styleSheetRevision: catalogue.styleSheetRevision
            ),
            presentation: EVMenuItemPresentation(isEnabled: catalogue.canEditStyles)
        ))
    }

    private func populateUnavailableStyleMenu(
        _ menu: NSMenu,
        role: EVStyleMenuRole,
        baseTitle: String,
        baseCommand: EVMenuCommand,
        editCommand: EVMenuCommand
    ) {
        let specification = styleMenuSpecification(for: role)
        styleMenuCatalogues.removeValue(forKey: ObjectIdentifier(menu))
        menu.removeAllItems()
        menu.addItem(styleActionItem(
            title: baseTitle,
            command: baseCommand,
            payload: EVStyleMenuAction(
                kind: .assign,
                role: role,
                stableID: specification.baseStableID,
                documentID: 0,
                documentRevision: 0,
                styleSheetRevision: 0
            ),
            presentation: .disabled
        ))
        if role == .paragraph {
            for level in 1...6 {
                menu.addItem(coreItem("Heading \(level)", command: EVMenuCommand(rawValue: EVMenuCommand.heading0.rawValue + level)!, key: String(level)))
            }
        }
        menu.addItem(.separator())
        menu.addItem(styleActionItem(
            title: "Edit Styles…",
            command: editCommand,
            payload: EVStyleMenuAction(
                kind: .edit,
                role: role,
                stableID: specification.baseStableID,
                documentID: 0,
                documentRevision: 0,
                styleSheetRevision: 0
            ),
            presentation: .disabled
        ))
    }

    private func styleActionItem(
        title: String,
        command: EVMenuCommand,
        payload: EVStyleMenuAction,
        presentation: EVMenuItemPresentation
    ) -> NSMenuItem {
        let item = NSMenuItem(
            title: title,
            action: #selector(EVStyleMenuActionRouting.performEditorStyleMenuAction(_:)),
            keyEquivalent: ""
        )
        if payload.kind == .assign, payload.role == .paragraph,
           let level = payload.stableID == "Paragraph" ? 0 : Int(payload.stableID.hasPrefix("Heading") ? String(payload.stableID.dropFirst(7)) : ""),
           (0...6).contains(level) {
            item.action = #selector(EVEditorCommandRouting.performEditorMenuCommand(_:))
            item.tag = EVMenuCommand.heading0.rawValue + level
            item.keyEquivalent = String(level)
            item.keyEquivalentModifierMask = [.command]
        } else { item.tag = command.rawValue }
        item.target = nil
        item.representedObject = payload
        item.isEnabled = presentation.isEnabled
        item.state = presentation.state
        return item
    }

    private func unavailableBaseEntry(
        role: EVStyleMenuRole,
        title: String
    ) -> EVStyleMenuEntry {
        EVStyleMenuEntry(
            role: role,
            stableID: styleMenuSpecification(for: role).baseStableID,
            displayName: title,
            isBase: true,
            presentation: .disabled
        )
    }

    private func styleMenuSpecification(
        for role: EVStyleMenuRole
    ) -> (baseTitle: String, baseStableID: String, selectionCommand: EVMenuCommand, editCommand: EVMenuCommand) {
        switch role {
        case .character:
            ("Base Character", "Character", .baseCharacterStyle, .editCharacterStyles)
        case .paragraph:
            ("Base Paragraph", "Paragraph", .baseParagraphStyle, .editParagraphStyles)
        case .document:
            ("Base Document", "Document", .baseDocumentStyle, .editDocumentStyles)
        }
    }

    private static func focusedStyleMenuProvider() -> (any EVStyleMenuProviding)? {
        let windows = [NSApplication.shared.keyWindow, NSApplication.shared.mainWindow]
            .compactMap { $0 } + NSApplication.shared.orderedWindows
        for window in windows {
            if let provider = (window.windowController as? EVDocumentWindowController)?
                .editorSurface as? any EVStyleMenuProviding
            {
                return provider
            }
        }
        return nil
    }

    private func topLevelItem(_ title: String, submenu: NSMenu) -> NSMenuItem {
        submenuItem(title, submenu: submenu)
    }

    private func submenuItem(_ title: String, submenu: NSMenu) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.submenu = submenu
        return item
    }

    private func coreItem(
        _ title: String,
        command: EVMenuCommand,
        key: String = "",
        modifiers: NSEvent.ModifierFlags = [.command]
    ) -> NSMenuItem {
        let item = NSMenuItem(
            title: title,
            action: command.nativeEditAction ?? #selector(EVEditorCommandRouting.performEditorMenuCommand(_:)),
            keyEquivalent: key
        )
        item.tag = command.rawValue
        item.target = nil
        if !key.isEmpty {
            item.keyEquivalentModifierMask = modifiers
        }
        return item
    }

    private func applicationItem(
        _ title: String,
        action: Selector,
        key: String = "",
        modifiers: NSEvent.ModifierFlags = [.command]
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.target = owner
        if !key.isEmpty {
            item.keyEquivalentModifierMask = modifiers
        }
        return item
    }

    /// Native document operations stay on AppKit's responder chain so
    /// NSDocument owns panels, file coordination, and window behavior. The
    /// stable command tag still identifies the semantic operation for tests,
    /// validation, and future telemetry without introducing a parallel save or
    /// document lifecycle in the editor surface.
    private func documentItem(
        _ title: String,
        action: Selector,
        command: EVMenuCommand,
        key: String = "",
        modifiers: NSEvent.ModifierFlags = [.command]
    ) -> NSMenuItem {
        let item = responderItem(title, action: action, key: key, modifiers: modifiers)
        item.tag = command.rawValue
        return item
    }

    private func responderItem(
        _ title: String,
        action: Selector,
        key: String = "",
        modifiers: NSEvent.ModifierFlags = [.command]
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.target = nil
        if !key.isEmpty {
            item.keyEquivalentModifierMask = modifiers
        }
        return item
    }
}
