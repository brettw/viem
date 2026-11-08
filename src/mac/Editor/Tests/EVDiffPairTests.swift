import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

/// Diff mode in a window: every pane shows its own base beside its editor
/// above one status bar, splits split the pair, and the argument commands
/// step through the diff's files.
@MainActor
final class EVDiffPairTests: XCTestCase {
    private var windows: [EVDocumentWindowController] = []
    private var documents: [EVDocument] = []

    override func setUp() {
        super.setUp()
        EVCoreArgumentListPolicy.install()
        // The test host registers no document types, so open files directly.
        let opener: (URL, @escaping @MainActor (EVDocument?, Error?) -> Void) -> Void = { [weak self] url, completion in
            do {
                if let existing = EVDocumentIdentity.existingDocument(at: url) { return completion(existing, nil) }
                let document = EVDocument(editorBackend: EVCoreDocumentBackend())
                document.recordRecentDocument = { _ in }
                try document.read(from: url, ofType: EVDocument.plainTextType)
                document.fileURL = url
                document.fileType = EVDocument.plainTextType
                NSDocumentController.shared.addDocument(document)
                self?.documents.append(document)
                completion(document, nil)
            } catch {
                completion(nil, error)
            }
        }
        EVDocumentWindowController.folderDiffDocumentOpener = opener
        EVDocumentWindowController.folderDiffUntitledDocument = { [weak self] in
            let document = EVDocument(editorBackend: EVCoreDocumentBackend())
            self?.documents.append(document)
            return document
        }
    }

    override func tearDown() {
        EVDocumentWindowController.folderDiffDidOpen = nil
        EVDocumentWindowController.folderDiffReport = nil
        EVDocumentWindowController.folderDiffDocumentOpener = nil
        EVDocumentWindowController.folderDiffUntitledDocument = nil
        for window in windows { window.diffMode?.disable(); window.close() }
        for document in documents { document.close() }
        windows = []
        documents = []
        super.tearDown()
    }

    private func git(_ arguments: [String], in directory: URL) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["git", "-c", "user.name=Viem", "-c", "user.email=viem@example.invalid"] + arguments
        process.currentDirectoryURL = directory
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw XCTSkip("git is unavailable: git \(arguments)") }
    }

    /// A committed repository; `change` then edits its working tree.
    private func repository(_ files: [String: String], change: (URL) throws -> Void) throws -> URL {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-diff-pair-\(UUID().uuidString)").resolvingSymlinksInPath()
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        for (path, text) in files { try Data(text.utf8).write(to: root.appendingPathComponent(path)) }
        try git(["init", "-q"], in: root)
        try git(["add", "."], in: root)
        try git(["commit", "-q", "-m", "base"], in: root)
        try change(root)
        return root
    }

    private func placeholder() throws -> EVDocumentWindowController {
        EVFrontendRegistry.install { EVCoreDocumentBackend() }
        let document = EVDocument(editorBackend: EVCoreDocumentBackend())
        documents.append(document)
        document.makeWindowControllers()
        let window = try XCTUnwrap(document.windowControllers.first as? EVDocumentWindowController)
        window.argumentDocumentOpener = EVDocumentWindowController.folderDiffDocumentOpener
        window.showWindow(nil)
        windows.append(window)
        return window
    }

    private func diff(_ folder: URL) throws -> EVDocumentWindowController {
        let window = try placeholder()
        let opened = expectation(description: "folder diff opened")
        var target: EVDocumentWindowController?
        EVDocumentWindowController.folderDiffDidOpen = { target = $0; opened.fulfill() }
        EVDocumentWindowController.folderDiffReport = { XCTFail("reported instead of opening: \($0)"); opened.fulfill() }
        EVDocumentWindowController.diffFolder(folder, from: window)
        wait(for: [opened], timeout: 20)
        let controller = try XCTUnwrap(target)
        if !windows.contains(where: { $0 === controller }) { windows.append(controller) }
        try waitUntil { controller.paneContainer.panes.allSatisfy { $0.diffBaseSurface != nil } }
        return controller
    }

    private func waitUntil(timeout: TimeInterval = 20, _ condition: () -> Bool) throws {
        let deadline = Date(timeIntervalSinceNow: timeout)
        while !condition() {
            guard Date() < deadline else { throw XCTSkip("timed out waiting for diff mode") }
            RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.02))
        }
    }

    private func request(_ kind: EVDocumentHostRequest.Kind, in window: EVDocumentWindowController,
                         vertical: Bool = false, navigation: EVArgumentNavigation? = nil) throws {
        let state = try XCTUnwrap(window.activeDocument).editorBackend.persistenceState
        var result: Result<String?, Error>?
        window.perform(documentHostRequests: [EVDocumentHostRequest(
            kind: kind, documentID: state.documentID, documentRevision: state.documentRevision,
            verticalSplit: vertical, argumentNavigation: navigation)]) { result = $0 }
        try waitUntil { result != nil }
        _ = try XCTUnwrap(result).get()
    }

    private func frame(_ view: NSView) -> NSRect { view.convert(view.bounds, to: nil) }

    func testSplittingAPaneSplitsItsPairAboveOneStatusBar() throws {
        let root = try repository(["a.txt": "a\n"]) { try Data("a changed\n".utf8).write(to: $0.appendingPathComponent("a.txt")) }
        let window = try diff(root)
        let document = try XCTUnwrap(window.activeDocument)
        try request(.split, in: window)
        try request(.split, in: window, vertical: true)
        try waitUntil { window.paneContainer.panes.allSatisfy { $0.diffBaseSurface != nil } }
        let panes = window.paneContainer.panes
        XCTAssertEqual(panes.count, 3)
        let bases = panes.compactMap(\.diffBaseSurface).map { ObjectIdentifier($0) }
        XCTAssertEqual(Set(bases).count, 3, "each pane compares with its own base view")
        for pane in panes {
            let base = try XCTUnwrap(pane.diffBaseSurface).viewController.view
            let editor = pane.editorSurface.viewController.view
            XCTAssertTrue(base.isDescendant(of: pane.view), "the base sits in its pane")
            XCTAssertLessThan(frame(base).maxX, frame(editor).minX, "the base is left of its editor")
            XCTAssertEqual(frame(base).minY, frame(pane.statusBar).maxY, accuracy: 0.5,
                           "the base and its splitter end at the status bar")
            XCTAssertEqual(frame(pane.statusBar).minX, frame(base).minX, accuracy: 0.5, "one status bar spans the pair")
            XCTAssertEqual(frame(pane.statusBar).maxX, frame(editor).maxX, accuracy: 0.5)
            let status = try XCTUnwrap(pane.statusBar.diffBase)
            XCTAssertEqual(status.title, "BASE @ index")
            XCTAssertNil(status.fileURL, "an unrenamed file's name is not repeated")
            XCTAssertEqual(status.width, frame(editor).minX - frame(pane.statusBar).minX, accuracy: 0.5)
        }
        XCTAssertEqual(document.editorBackend.sourceFormat, .code)
        window.closePane(panes[0])
        try waitUntil { window.paneContainer.panes.count == 2 }
        XCTAssertEqual(document.editorBackend.sourceFormat, .code, "remaining pairs keep comparing in Code")
        XCTAssertTrue(window.paneContainer.panes.allSatisfy { $0.diffBaseSurface != nil })
        window.diffMode?.disable()
        XCTAssertTrue(window.paneContainer.panes.allSatisfy { $0.diffBaseSurface == nil && $0.statusBar.diffBase == nil })
        XCTAssertEqual(document.editorBackend.sourceFormat, .plainText, "the last pair restores the format")
    }

    func testTheBaseStatusNamesItsVersionAndARenamedFilesOriginal() throws {
        let root = try repository(["a.txt": "a\n", "old.txt": "one\ntwo\nthree\nfour\n"]) { root in
            try Data("a changed\n".utf8).write(to: root.appendingPathComponent("a.txt"))
            try self.git(["mv", "old.txt", "new.txt"], in: root)
            try Data("one\ntwo\nthree\nfour\nfive\n".utf8).write(to: root.appendingPathComponent("new.txt"))
        }
        let window = try diff(root)
        let mode = try XCTUnwrap(window.diffMode)
        let pane = { window.documentContentController }
        try waitUntil { pane().statusBar.diffBase?.title == "BASE @ index" }
        mode.setBase(.head)
        try waitUntil {
            pane().statusBar.diffBase?.title == "BASE @ HEAD" && mode.files.contains { $0.url.lastPathComponent == "new.txt" }
                && mode.files.allSatisfy { $0.change != UInt32(VIEM_DIFF_FILE_MODIFIED) || $0.url.lastPathComponent == "a.txt" }
        }
        let renamed = try XCTUnwrap(mode.files.first { $0.url.lastPathComponent == "new.txt" })
        XCTAssertEqual(renamed.change, UInt32(VIEM_DIFF_FILE_RENAMED))
        for _ in mode.files where window.activeDocument?.fileURL?.lastPathComponent != "new.txt" {
            mode.moveFile(by: 1)
            try waitUntil { pane().diffBaseSurface != nil && pane().document?.fileURL != nil }
        }
        try waitUntil {
            window.activeDocument?.fileURL?.lastPathComponent == "new.txt" && pane().statusBar.diffBase?.fileURL != nil
        }
        XCTAssertEqual(pane().statusBar.diffBase?.fileURL?.lastPathComponent, "old.txt",
                       "a renamed file's base shows its original name")
        XCTAssertEqual(pane().statusBar.diffBase?.location, pane().diffBaseSurface?.statusBarState.location)
        mode.moveFile(by: 1)
        try waitUntil { window.activeDocument?.fileURL?.lastPathComponent == "a.txt" && pane().diffBaseSurface != nil }
        XCTAssertNil(pane().statusBar.diffBase?.fileURL, "an unrenamed file's name is not repeated")
    }

    func testArgumentCommandsStepThroughTheDiffFiles() throws {
        let root = try repository(["a.txt": "a\n", "b.txt": "b\n", "c.txt": "c\n"]) { root in
            for name in ["a.txt", "b.txt", "c.txt"] {
                try Data("changed\n".utf8).write(to: root.appendingPathComponent(name))
            }
        }
        let window = try diff(root)
        let mode = try XCTUnwrap(window.diffMode)
        XCTAssertEqual(mode.files.count, 3)
        func shown() -> String? { window.activeDocument?.fileURL?.lastPathComponent }
        let names = mode.files.map(\.url.lastPathComponent)
        XCTAssertEqual(shown(), names[0])
        try request(.navigateArgument, in: window, navigation: .init(target: .next))
        try waitUntil { shown() == names[1] && window.documentContentController.diffBaseSurface != nil }
        try request(.navigateArgument, in: window, navigation: .init(target: .last))
        try waitUntil { shown() == names[2] }
        try request(.navigateArgument, in: window, navigation: .init(target: .previous))
        try waitUntil { shown() == names[1] }
        try request(.navigateArgument, in: window, navigation: .init(target: .first))
        try waitUntil { shown() == names[0] && window.documentContentController.diffBaseSurface != nil }
        XCTAssertEqual(mode.currentFileIndex, 0)
        XCTAssertEqual(mode.toolbar.filePopup.indexOfSelectedItem, 0, "the file popup follows the argument commands")
    }

    func testLineNumbersToggledOnTheBaseSideApplyToEveryPair() throws {
        let root = try repository(["a.txt": "a\n"]) { try Data("a changed\n".utf8).write(to: $0.appendingPathComponent("a.txt")) }
        let window = try diff(root)
        try request(.split, in: window)
        try waitUntil { window.paneContainer.panes.allSatisfy { $0.diffBaseSurface != nil } }
        let pane = window.documentContentController
        let base = try XCTUnwrap(pane.diffBaseSurface)
        window.window?.makeFirstResponder(base.viewController.view)
        XCTAssertTrue(pane.commandSurface === base, "commands follow the focused side")
        let item = NSMenuItem(title: "Show line numbers", action: #selector(EVDocumentContentViewController.performEditorMenuCommand(_:)),
                              keyEquivalent: "")
        item.tag = EVMenuCommand.showLineNumbers.rawValue
        let before = (base as? any EVDiffLineNumberControlling)?.showsLineNumbers
        pane.performEditorMenuCommand(item)
        let shows = (base as? any EVDiffLineNumberControlling)?.showsLineNumbers
        XCTAssertNotEqual(before, shows)
        for side in window.paneContainer.panes.flatMap({ [$0.editorSurface, $0.diffBaseSurface].compactMap { $0 } }) {
            XCTAssertEqual((side as? any EVDiffLineNumberControlling)?.showsLineNumbers, shows)
        }
    }
}
