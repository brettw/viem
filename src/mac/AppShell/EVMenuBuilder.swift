import AppKit

@MainActor
@objc public protocol EVApplicationCommandRouting: AnyObject {
    func newDocument(_ sender: Any?)
    func openDocument(_ sender: Any?)
    func openRecentDocument(_ sender: Any?)
    func clearRecentDocuments(_ sender: Any?)
    func showSettings(_ sender: Any?)
    func showThemeSettings(_ sender: Any?)
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
    private let documentModeProvider: @MainActor () -> (any EVDocumentModeMenuProviding)?
    private var viewMenu: NSMenu?
    private var codeModeMenu: NSMenu?
    private var codeModeItem: NSMenuItem?
    private var obscureCodeModeMenu: NSMenu?
    private var obscureCodeModeItem: NSMenuItem?
    private var modeItems: [NSMenuItem] = []
    private var themeMenu: NSMenu?
    let themeActions: EVThemeActions
    private var trackingMenus = Set<ObjectIdentifier>()

    public init(
        owner: any EVApplicationCommandRouting,
        recentDocumentURLs: @escaping @MainActor () -> [URL] = {
            EVConfigurationStore.shared.recentDocumentURLs
        },
        styleMenuProvider: (@MainActor () -> (any EVStyleMenuProviding)?)? = nil,
        themeStore: EVThemeStore? = nil,
        documentModeProvider: (@MainActor () -> (any EVDocumentModeMenuProviding)?)? = nil
    ) {
        self.owner = owner
        self.themeActions = EVThemeActions(store: themeStore ?? .shared)
        self.recentDocumentURLs = recentDocumentURLs
        self.styleMenuProvider = styleMenuProvider ?? {
            EVMenuBuilder.focusedStyleMenuProvider()
        }
        self.documentModeProvider = documentModeProvider ?? {
            let windows = [NSApplication.shared.keyWindow, NSApplication.shared.mainWindow].compactMap { $0 }
                + NSApplication.shared.orderedWindows
            return windows.compactMap {
                ($0.windowController as? EVDocumentWindowController)?.editorSurface as? any EVDocumentModeMenuProviding
            }.first
        }
        super.init()
    }

    public func buildMainMenu(for application: NSApplication) -> NSMenu {
        let mainMenu = NSMenu(title: "Main Menu")
        mainMenu.addItem(topLevelItem("Viem", submenu: makeApplicationMenu(for: application)))
        mainMenu.addItem(topLevelItem("File", submenu: makeFileMenu()))
        mainMenu.addItem(topLevelItem("Edit", submenu: makeEditMenu()))
        mainMenu.addItem(topLevelItem("Style", submenu: makeStyleMenu()))
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
        if menu === viewMenu || menu === codeModeMenu || menu === obscureCodeModeMenu {
            // Descending into a flyout retains the state captured when its
            // parent opened, including the revision verified by its actions.
            guard ![viewMenu, codeModeMenu, obscureCodeModeMenu].compactMap({ $0 }).contains(where: {
                trackingMenus.contains(ObjectIdentifier($0))
            }) else { return }
            updateDocumentModes()
            return
        }
        if menu === themeMenu { rebuildThemeMenu(menu); return }
        if let role = styleMenuRoles[ObjectIdentifier(menu)] {
            rebuildStyleMenu(menu, role: role)
            return
        }

        guard menu === recentDocumentsMenu else { return }

        menu.removeAllItems()
        let urls = recentDocumentURLs()
        for (url, title) in zip(urls, Self.recentDocumentTitles(for: urls)) {
            let item = NSMenuItem(
                title: title,
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

    /// Keep independent filenames short. Only entries whose current suffixes
    /// collide grow by one parent component, including the filesystem root
    /// when one full path is the suffix of another. Input order stays intact.
    private static func recentDocumentTitles(for urls: [URL]) -> [String] {
        let paths = urls.map(\.pathComponents)
        var depths = Array(repeating: 1, count: urls.count)
        var titles = urls.map(\.lastPathComponent)
        while true {
            let counts = titles.reduce(into: [String: Int]()) { $0[$1, default: 0] += 1 }
            var extended = false
            for index in urls.indices where counts[titles[index], default: 0] > 1 {
                guard depths[index] < paths[index].count else { continue }
                depths[index] += 1
                titles[index] = NSString.path(withComponents: Array(paths[index].suffix(depths[index])))
                extended = true
            }
            // The recent-file store deduplicates files. Still terminate if a
            // caller supplies an identical path twice: no longer suffix exists.
            if !extended { return titles }
        }
    }

    private func makeApplicationMenu(for application: NSApplication) -> NSMenu {
        let menu = NSMenu(title: "Viem")
        menu.addItem(responderItem(
            "About Viem",
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
        menu.addItem(responderItem("Hide Viem", action: #selector(NSApplication.hide(_:)), key: "h"))
        menu.addItem(responderItem(
            "Hide Others",
            action: #selector(NSApplication.hideOtherApplications(_:)),
            key: "h",
            modifiers: [.command, .option]
        ))
        menu.addItem(responderItem("Show All", action: #selector(NSApplication.unhideAllApplications(_:))))
        menu.addItem(.separator())
        menu.addItem(responderItem("Quit Viem", action: #selector(NSApplication.terminate(_:)), key: "q"))
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
        menu.addItem(coreItem("Export…", command: .exportHTML))
        menu.addItem(documentItem(
            "Duplicate",
            action: #selector(NSDocument.duplicate(_:)),
            command: .duplicateDocument
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
        let encodings = NSMenu(title: "Text Encoding")
        encodings.showsStateColumn = true
        encodings.addItem(coreItem("UTF-8", command: .encodingUTF8))
        encodings.addItem(coreItem("Latin-1", command: .encodingLatin1))
        encodings.addItem(coreItem("UTF-16 LE", command: .encodingUTF16LE))
        encodings.addItem(coreItem("UTF-16 BE", command: .encodingUTF16BE))
        let encodingItem = NSMenuItem(title: "Text Encoding", action: nil, keyEquivalent: "")
        encodingItem.submenu = encodings
        menu.addItem(encodingItem)

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
        menu.addItem(coreItem("Copy Source", command: .copySource, key: "c", modifiers: [.command, .shift]))
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

    private func makeViewMenu() -> NSMenu {
        let menu = NSMenu(title: "View")
        modeItems.removeAll()
        viewMenu = menu
        menu.delegate = self
        menu.showsStateColumn = true
        menu.addItem(documentModeItem("Plain text", choice: .plainText))
        menu.addItem(documentModeItem("Markdown", choice: .markdown))
        let code = NSMenu(title: "Code")
        code.delegate = self
        code.showsStateColumn = true
        codeModeMenu = code
        code.addItem(documentModeItem("Auto (Plain Text)", choice: .automatic))
        let obscure = NSMenu(title: "Obscure languages")
        obscure.delegate = self
        obscure.showsStateColumn = true
        obscureCodeModeMenu = obscure
        for language in EVCodeLanguage.obscureLanguages {
            obscure.addItem(documentModeItem(language.name, choice: .code(language.id)))
        }
        let obscureItem = submenuItem("Obscure languages", submenu: obscure)
        obscureCodeModeItem = obscureItem
        code.addItem(obscureItem)
        code.addItem(.separator())
        for language in EVCodeLanguage.primaryLanguages {
            code.addItem(documentModeItem(language.name, choice: .code(language.id)))
        }
        let codeItem = submenuItem("Code", submenu: code)
        codeModeItem = codeItem
        menu.addItem(codeItem)
        menu.addItem(.separator())
        updateDocumentModes()
        menu.addItem(responderItem(
            "Show Status Bar",
            action: #selector(EVDocumentContentViewController.toggleStatusBar(_:))
        ))
        menu.addItem(.separator())
        menu.addItem(coreItem("Word Wrap", command: .wordWrap))
        menu.addItem(coreItem("Flow Source Paragraphs", command: .flowParagraphs))
        menu.addItem(coreItem("Show Invisible Characters", command: .showInvisibleCharacters))
        menu.addItem(.separator())
        menu.addItem(coreItem("Zoom In", command: .zoomIn, key: "=", modifiers: [.command]))
        menu.addItem(coreItem("Zoom Out", command: .zoomOut, key: "-", modifiers: [.command]))
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

    private func documentModeItem(_ title: String, choice: EVDocumentModeChoice) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: #selector(selectDocumentMode(_:)), keyEquivalent: "")
        item.target = self
        item.representedObject = EVDocumentModeMenuAction(choice)
        modeItems.append(item)
        return item
    }

    private func updateDocumentModes() {
        let state = documentModeProvider()?.currentDocumentMode()
        codeModeItem?.state = state?.format == "code" ? .on : .off
        codeModeItem?.isEnabled = state != nil
        obscureCodeModeItem?.isEnabled = state != nil
        obscureCodeModeItem?.state = .off
        for item in modeItems {
            guard let action = item.representedObject as? EVDocumentModeMenuAction else { continue }
            action.expected = state
            item.isEnabled = state != nil
            let selected: Bool
            switch action.choice {
            case .plainText: selected = state?.format == "plainText"
            case .markdown: selected = state?.format == "markdown"
            case .automatic:
                selected = state?.automatic == true
                item.title = "Auto (\(state?.detectedName ?? "Plain Text"))"
            case let .code(id): selected = state?.format == "code" && state?.automatic == false && state?.language == id
            }
            item.state = selected ? .on : .off
            if selected && item.menu === obscureCodeModeMenu { obscureCodeModeItem?.state = .on }
        }
    }

    @objc private func selectDocumentMode(_ item: NSMenuItem) {
        guard let action = item.representedObject as? EVDocumentModeMenuAction,
              let expected = action.expected else { return }
        documentModeProvider()?.selectDocumentMode(action.choice, expected: expected)
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
            "Viem Help",
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

    private func makeStyleMenu() -> NSMenu {
        let menu = NSMenu(title: "Style")
        let themes = NSMenu(title: "Theme")
        themes.delegate = self
        themes.autoenablesItems = false
        themeMenu = themes
        rebuildThemeMenu(themes)
        menu.addItem(topLevelItem("Theme", submenu: themes))
        menu.addItem(.separator())
        menu.addItem(makeNamedStyleMenu(title: "Paragraph", role: .paragraph,
            command: .paragraphStyles))
        menu.addItem(makeNamedStyleMenu(title: "Character", role: .character,
            command: .characterStyles))
        menu.addItem(.separator())
        menu.addItem(coreItem("Edit Styles…", command: .editStyles,
            key: String(UnicodeScalar(NSF8FunctionKey)!), modifiers: []))
        menu.addItem(.separator())
        menu.addItem(coreItem("Reload style sheet", command: .reloadStyleSheet))
        return menu
    }

    private func rebuildThemeMenu(_ menu: NSMenu) {
        try? themeActions.store.ensureCurrentThemeExists()
        menu.removeAllItems()
        for theme in themeActions.store.availableThemes {
            let item = NSMenuItem(title: theme.name, action: #selector(selectTheme(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = theme
            item.state = themeActions.store.currentThemeFileName == theme.fileName ? .on : .off
            menu.addItem(item)
        }
        menu.addItem(.separator())
        let fallback = NSMenuItem(title: "Default", action: #selector(selectTheme(_:)), keyEquivalent: "")
        fallback.target = self
        fallback.state = themeActions.store.currentThemeName == nil ? .on : .off
        menu.addItem(fallback)
        let create = NSMenuItem(title: "New Theme…", action: #selector(createTheme(_:)), keyEquivalent: "")
        create.target = self
        menu.addItem(create)
        menu.addItem(applicationItem(
            "Theme Settings…",
            action: #selector(EVApplicationCommandRouting.showThemeSettings(_:))
        ))
    }

    @objc private func selectTheme(_ item: NSMenuItem) {
        themeActions.select(item.representedObject as? EVThemeChoice)
    }

    @objc private func createTheme(_ sender: Any?) {
        themeActions.create(window: NSApplication.shared.keyWindow)
    }

    private func makeNamedStyleMenu(
        title: String,
        role: EVStyleMenuRole,
        command: EVMenuCommand
    ) -> NSMenuItem {
        let submenu = NSMenu(title: title)
        submenu.delegate = self
        // Keep checked and unchecked style titles aligned in the native gutter.
        submenu.showsStateColumn = true
        submenu.autoenablesItems = true
        styleMenuRoles[ObjectIdentifier(submenu)] = role
        populateUnavailableStyleMenu(submenu, role: role)
        // Give submenu parents a responder-chain action so their availability
        // is validated against the active document just like other commands.
        let item = coreItem(title, command: command)
        item.submenu = submenu
        return item
    }

    private func rebuildStyleMenu(_ menu: NSMenu, role: EVStyleMenuRole) {
        let specification = styleMenuSpecification(for: role)
        guard let provider = styleMenuProvider(),
              let catalogue = provider.currentStyleMenuCatalogue()
        else {
            populateUnavailableStyleMenu(menu, role: role)
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
        if role == .paragraph { addParagraphListCommands(to: menu) }
        for entry in entries {
            menu.addItem(styleActionItem(
                title: entry.displayName,
                command: specification.selectionCommand,
                payload: EVStyleMenuAction(
                    kind: entry.actionKind,
                    role: role,
                    stableID: entry.stableID,
                    documentID: catalogue.documentID,
                    documentRevision: catalogue.documentRevision,
                    styleSheetRevision: catalogue.styleSheetRevision,
                    syntaxName: entry.syntaxName
                ),
                presentation: entry.presentation
            ))
        }
    }

    private func populateUnavailableStyleMenu(
        _ menu: NSMenu,
        role: EVStyleMenuRole
    ) {
        let specification = styleMenuSpecification(for: role)
        styleMenuCatalogues.removeValue(forKey: ObjectIdentifier(menu))
        menu.removeAllItems()
        if role == .paragraph { addParagraphListCommands(to: menu) }
        menu.addItem(styleActionItem(
            title: specification.baseTitle,
            command: specification.selectionCommand,
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
    }

    private func addParagraphListCommands(to menu: NSMenu) {
        menu.addItem(coreItem("Bulleted List", command: .bulletedList))
        menu.addItem(coreItem("Numbered List", command: .numberedList))
        menu.addItem(coreItem("Indent", command: .increaseIndent))
        menu.addItem(coreItem("Unindent", command: .decreaseIndent))
        menu.addItem(.separator())
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
    ) -> (baseTitle: String, baseStableID: String, selectionCommand: EVMenuCommand) {
        switch role {
        case .character:
            ("Default Paragraph", "", .defaultParagraphStyle)
        case .paragraph:
            ("Base Paragraph", "Paragraph", .baseParagraphStyle)
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

extension EVMenuBuilder: NSMenuItemValidation {
    public func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if let action = menuItem.representedObject as? EVDocumentModeMenuAction { return action.expected != nil }
        return true
    }
}
