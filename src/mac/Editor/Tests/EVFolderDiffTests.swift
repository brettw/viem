import AppKit
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

/// "Diff <directory>" and "Diff Folder…": the files `git diff -- <folder>`
/// lists, shown in diff mode in a reused pristine window or a new one.
@MainActor
final class EVFolderDiffTests: XCTestCase {
    private var windows: [EVDocumentWindowController] = []
    private var documents: [EVDocument] = []

    override func setUp() {
        super.setUp()
        // The test host registers no document types, so open files directly.
        EVDocumentWindowController.folderDiffDocumentOpener = { [weak self] url, completion in
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
    }

    override func tearDown() {
        EVDocumentWindowController.folderDiffDidOpen = nil
        EVDocumentWindowController.folderDiffReport = nil
        EVDocumentWindowController.folderDiffDocumentOpener = nil
        for window in windows { window.diffMode?.disable(); window.close() }
        for document in documents { document.close() }
        windows = []
        documents = []
        super.tearDown()
    }

    private func run(_ arguments: [String], in directory: URL) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["git"] + arguments
        process.currentDirectoryURL = directory
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw XCTSkip("git is unavailable: git \(arguments)") }
    }

    /// A repository whose `sub` folder has one changed file and whose root
    /// has another; `clean` has no changes.
    private func repository() throws -> URL {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("viem-folder-diff-\(UUID().uuidString)").resolvingSymlinksInPath()
        try FileManager.default.createDirectory(at: root.appendingPathComponent("sub"), withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: root.appendingPathComponent("clean"), withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        for (path, text) in [("a.txt", "a\n"), ("sub/b.txt", "b\n"), ("clean/c.txt", "c\n")] {
            try Data(text.utf8).write(to: root.appendingPathComponent(path))
        }
        try run(["init", "-q"], in: root)
        try run(["-c", "user.name=Viem", "-c", "user.email=viem@example.invalid", "add", "."], in: root)
        try run(["-c", "user.name=Viem", "-c", "user.email=viem@example.invalid", "commit", "-q", "-m", "base"], in: root)
        try Data("a changed\n".utf8).write(to: root.appendingPathComponent("a.txt"))
        try Data("b changed\n".utf8).write(to: root.appendingPathComponent("sub/b.txt"))
        return root
    }

    private func window(text: String?) throws -> EVDocumentWindowController {
        EVFrontendRegistry.install { EVCoreDocumentBackend() }
        let document = EVDocument(editorBackend: EVCoreDocumentBackend())
        if let text {
            try document.read(from: Data(text.utf8), ofType: EVDocument.plainTextType)
            document.fileType = EVDocument.plainTextType
        }
        // Window controllers don't retain their document.
        documents.append(document)
        document.makeWindowControllers()
        let window = try XCTUnwrap(document.windowControllers.first as? EVDocumentWindowController)
        window.showWindow(nil)
        windows.append(window)
        return window
    }

    private func diff(_ folder: URL, from window: EVDocumentWindowController) -> EVDocumentWindowController? {
        let opened = expectation(description: "folder diff opened")
        var target: EVDocumentWindowController?
        EVDocumentWindowController.folderDiffDidOpen = { controller in
            target = controller
            opened.fulfill()
        }
        EVDocumentWindowController.folderDiffReport = { message in
            XCTFail("reported instead of opening: \(message)")
            opened.fulfill()
        }
        EVDocumentWindowController.diffFolder(folder, from: window)
        wait(for: [opened], timeout: 20)
        if let target, !windows.contains(where: { $0 === target }) { windows.append(target) }
        return target
    }

    func testAPristineWindowShowsTheFirstChangedFileUnderTheFolder() throws {
        let root = try repository()
        let placeholder = try window(text: nil)
        let target = try XCTUnwrap(diff(root.appendingPathComponent("sub"), from: placeholder))
        XCTAssertTrue(target === placeholder, "the pristine untitled window is reused")
        XCTAssertEqual(target.activeDocument?.fileURL?.resolvingSymlinksInPath(),
                       root.appendingPathComponent("sub/b.txt"))
        let mode = try XCTUnwrap(target.diffMode)
        XCTAssertTrue(mode.isEnabled)
        XCTAssertEqual(mode.base, .unstaged)
        XCTAssertEqual(mode.files.map { $0.url.resolvingSymlinksInPath() }, [root.appendingPathComponent("sub/b.txt")],
                       "only files under the folder are listed")
        XCTAssertEqual(mode.scope, root.appendingPathComponent("sub"))
    }

    func testAWindowWithContentStaysAndTheDiffOpensInANewWindow() throws {
        let root = try repository()
        let busy = try window(text: "keep me\n")
        let target = try XCTUnwrap(diff(root, from: busy))
        XCTAssertFalse(target === busy)
        XCTAssertEqual(target.diffMode?.files.count, 2, "the repository root lists every changed file")
        XCTAssertEqual(busy.activeDocument?.fileURL, nil)
        XCTAssertFalse(busy.diffMode?.isEnabled ?? false)
    }

    func testANewDiffWindowStartsFromAndRecordsTheDiffFrame() throws {
        let root = try repository()
        let remembered = NSRect(x: 120, y: 140, width: 1000, height: 560)
        var saved: [NSRect] = []
        EVDocumentWindowController.folderDiffPlacement = EVDocumentWindowPlacement(
            loadFrame: { remembered }, saveFrame: { saved.append($0) })
        defer { EVDocumentWindowController.folderDiffPlacement = nil }
        let busy = try window(text: "keep me\n")
        let target = try XCTUnwrap(diff(root, from: busy))
        let window = try XCTUnwrap(target.window)
        XCTAssertEqual(window.frame.height, remembered.height, accuracy: 1)
        XCTAssertGreaterThanOrEqual(window.frame.width, remembered.width - 1)
        saved.removeAll()
        window.setFrame(NSRect(x: 130, y: 150, width: 1100, height: 600), display: false)
        XCTAssertEqual(saved.last?.height, 600, "a diff window's resize is a diff frame")
    }

    func testAReusedPlaceholderTakesTheDiffWindowSize() throws {
        EVFrontendRegistry.install { EVCoreDocumentBackend() }
        let document = EVDocument(editorBackend: EVCoreDocumentBackend())
        documents.append(document)
        let remembered = NSRect(x: 0, y: 0, width: 1000, height: 520)
        let controller = EVDocumentWindowController(
            document: document, editorSurface: document.editorBackend.makeEditorSurface(),
            placement: EVDocumentWindowPlacement(loadFrame: { NSRect(x: 200, y: 200, width: 600, height: 400) }),
            diffPlacement: EVDocumentWindowPlacement(loadFrame: { remembered }))
        document.addWindowController(controller)
        controller.showWindow(nil)
        windows.append(controller)
        let window = try XCTUnwrap(controller.window)
        let top = window.frame.maxY
        XCTAssertEqual(window.frame.height, 400, accuracy: 1, "a regular window starts from the regular frame")
        controller.adoptDiffPlacement()
        XCTAssertEqual(window.frame.height, remembered.height, accuracy: 1)
        XCTAssertGreaterThanOrEqual(window.frame.width, remembered.width - 1)
        XCTAssertEqual(window.frame.maxY, top, accuracy: 1, "the top edge stays put")
    }

    func testFoldersWithoutChangesOrARepositoryReportInsteadOfOpening() throws {
        let root = try repository()
        let placeholder = try window(text: nil)
        var reports: [String] = []
        let reported = expectation(description: "reported")
        reported.expectedFulfillmentCount = 2
        EVDocumentWindowController.folderDiffReport = { message in
            reports.append(message)
            reported.fulfill()
        }
        EVDocumentWindowController.folderDiffDidOpen = { _ in XCTFail("no window opens") }
        EVDocumentWindowController.diffFolder(root.appendingPathComponent("clean"), from: placeholder)
        let outside = FileManager.default.temporaryDirectory.appendingPathComponent("viem-no-repo-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: outside, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: outside) }
        EVDocumentWindowController.diffFolder(outside, from: placeholder)
        wait(for: [reported], timeout: 20)
        XCTAssertTrue(reports.contains { $0.contains("no changes") })
        XCTAssertEqual(reports.count, 2)
        XCTAssertFalse(placeholder.diffMode?.isEnabled ?? false)
    }

    func testTheFilePopupListsSameNamedFilesSeparatelyAndFollowsTheShownFile() throws {
        let root = try repository()
        for folder in ["one", "two"] {
            let directory = root.appendingPathComponent(folder)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try Data("same\n".utf8).write(to: directory.appendingPathComponent("same.txt"))
        }
        try run(["-c", "user.name=Viem", "-c", "user.email=viem@example.invalid", "add", "."], in: root)
        try run(["-c", "user.name=Viem", "-c", "user.email=viem@example.invalid", "commit", "-q", "-m", "same"], in: root)
        for folder in ["one", "two"] {
            try Data("changed\n".utf8).write(to: root.appendingPathComponent(folder).appendingPathComponent("same.txt"))
        }
        let placeholder = try window(text: nil)
        let target = try XCTUnwrap(diff(root, from: placeholder))
        let mode = try XCTUnwrap(target.diffMode)
        let popup = mode.toolbar.filePopup
        XCTAssertEqual(popup.numberOfItems, mode.files.count, "one item per listed file")
        XCTAssertEqual(Set(popup.itemTitles).count, popup.numberOfItems, "every title is distinct")
        XCTAssertTrue(popup.itemTitles.contains("same.txt — one"))
        XCTAssertTrue(popup.itemTitles.contains("same.txt — two"))
        for _ in 0..<mode.files.count {
            let presented = expectation(description: "presented")
            mode.didPresent = { presented.fulfill() }
            mode.moveFile(by: 1)
            wait(for: [presented], timeout: 20)
            let shown = try XCTUnwrap(target.activeDocument?.fileURL?.resolvingSymlinksInPath())
            let selected = popup.indexOfSelectedItem
            XCTAssertEqual(mode.files[selected].url.resolvingSymlinksInPath(), shown,
                           "the popup names the shown file: \(popup.titleOfSelectedItem ?? "")")
        }
    }

    func testTheMenuNamesTheDirectoryClippedFromTheLeft() {
        XCTAssertEqual(EVDiffFolder.menuTitle(forDirectory: "/Users/me/project"), "Diff /Users/me/project")
        let long = "/Users/someone/projects/" + String(repeating: "deep/", count: 10) + "viem"
        let title = EVDiffFolder.menuTitle(forDirectory: long)
        XCTAssertTrue(title.hasPrefix("Diff …"))
        XCTAssertTrue(title.hasSuffix("deep/viem"))
        XCTAssertEqual(title.count, "Diff ".count + EVDiffFolder.menuPathLimit)
    }
}
