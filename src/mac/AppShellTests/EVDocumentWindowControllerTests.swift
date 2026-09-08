import AppKit
import XCTest

@testable import ViemAppShell

@MainActor
final class EVDocumentWindowControllerTests: XCTestCase {
  private final class EditorTestView: NSView {
    override var acceptsFirstResponder: Bool { true }
  }
  private final class Surface: EVEditorSurface {
    let viewController: NSViewController = {
      let controller = NSViewController()
      controller.view = EditorTestView(frame: NSRect(x: 0, y: 0, width: 920, height: 655))
      return controller
    }()
    var statusBarState = EVStatusBarState()
    var statusBarStateDidChange: ((EVStatusBarState) -> Void)?
    var performedCommands: [EVMenuCommand] = []
    var presentations: [EVMenuCommand: EVMenuItemPresentation] = [:]

    func perform(menuCommand: EVMenuCommand, sender _: Any?) {
      performedCommands.append(menuCommand)
    }

    func presentation(for command: EVMenuCommand) -> EVMenuItemPresentation {
      presentations[command] ?? .enabled
    }
  }

  private final class Backend: EVDocumentBackend {
    var sourceDidChange: (() -> Void)?
    var persistenceStateDidChange: ((EVDocumentPersistenceState) -> Void)?
    var persistenceState = EVDocumentPersistenceState()
    var sourceFormat: EVSourceFormat = .plainText
    var surfaces: [Surface] = []

    func makeEditorSurface() -> any EVEditorSurface {
      let surface = Surface()
      surfaces.append(surface)
      return surface
    }

    func read(source _: Data, typeName _: String) throws {}
    func serializedSource(typeName _: String) throws -> Data { Data() }
    func nativeSaveSnapshot(typeName _: String) throws -> EVDocumentSaveSnapshot {
      EVDocumentSaveSnapshot(data: Data(), documentID: 1, documentRevision: 1)
    }
    func acknowledgeNativeSave(_: EVDocumentSaveSnapshot) throws {}
  }

  private func preservingStatusBarDefault(_ body: () throws -> Void) rethrows {
    let configuration = EVConfigurationStore.shared
    let previous = configuration.showStatusBar
    try? configuration.setShowStatusBar(true)
    defer { try? configuration.setShowStatusBar(previous) }
    try body()
  }

  func testInitialShowUsesRequestedContentSizeAndFillsCanvasAboveStatusBar() throws {
    try preservingStatusBarDefault {
      let document = EVDocument()
      let surface = Surface()
      let controller = EVDocumentWindowController(document: document, editorSurface: surface)
      defer { controller.close() }

      XCTAssertFalse(controller.window?.isVisible ?? true)
      controller.showWindow(nil)

      let window = try XCTUnwrap(controller.window)
      let geometry = try XCTUnwrap(controller.currentGeometry)
      XCTAssertEqual(window.contentLayoutRect.width, 920, accuracy: 0.5)
      XCTAssertEqual(window.contentLayoutRect.height, 680, accuracy: 0.5)
      XCTAssertEqual(window.contentMinSize, NSSize(width: 480, height: 280))
      XCTAssertFalse(window.isMiniaturized)
      XCTAssertTrue(window.isVisible)

      XCTAssertTrue(geometry.statusBarIsVisible)
      XCTAssertEqual(geometry.content.width, 920, accuracy: 0.5)
      XCTAssertEqual(geometry.content.height, 680, accuracy: 0.5)
      XCTAssertEqual(geometry.editor.width, geometry.content.width, accuracy: 0.5)
      XCTAssertEqual(geometry.statusBar.width, geometry.content.width, accuracy: 0.5)
      XCTAssertEqual(geometry.statusBar.height, EVStatusBarView.preferredHeight, accuracy: 0.5)
      XCTAssertEqual(
        geometry.editor.height,
        geometry.content.height - EVStatusBarView.preferredHeight,
        accuracy: 0.5
      )
      XCTAssertEqual(geometry.statusBar.minY, geometry.content.minY, accuracy: 0.5)
      XCTAssertEqual(geometry.editor.minY, geometry.statusBar.maxY, accuracy: 0.5)
      XCTAssertEqual(geometry.editor.maxY, geometry.content.maxY, accuracy: 0.5)
    }
  }

  func testInitialShowOverridesPreShowGeometryWithoutRestorationOrTabbing() throws {
    let document = EVDocument()
    let controller = EVDocumentWindowController(document: document, editorSurface: Surface())
    defer { controller.close() }

    let window = try XCTUnwrap(controller.window)
    window.setContentSize(NSSize(width: 500, height: 900))
    controller.showWindow(nil)

    XCTAssertEqual(window.contentLayoutRect.width, 920, accuracy: 0.5)
    XCTAssertEqual(window.contentLayoutRect.height, 680, accuracy: 0.5)
    XCTAssertFalse(window.isRestorable)
    XCTAssertEqual(window.tabbingMode, .disallowed)
  }

  func testLaterShowsPreserveAUserResize() throws {
    let document = EVDocument()
    let controller = EVDocumentWindowController(document: document, editorSurface: Surface())
    defer { controller.close() }
    controller.showWindow(nil)

    let window = try XCTUnwrap(controller.window)
    window.setContentSize(NSSize(width: 700, height: 500))
    XCTAssertEqual(controller.editorSurface.viewController.view.frame.width, 700, accuracy: 0.5)
    XCTAssertEqual(controller.editorSurface.viewController.view.frame.height, 475, accuracy: 0.5)
    window.orderOut(nil)
    controller.showWindow(nil)

    XCTAssertEqual(window.contentLayoutRect.width, 700, accuracy: 0.5)
    XCTAssertEqual(window.contentLayoutRect.height, 500, accuracy: 0.5)
    let geometry = try XCTUnwrap(controller.currentGeometry)
    XCTAssertEqual(geometry.editor.width, 700, accuracy: 0.5)
    XCTAssertEqual(geometry.editor.height, 475, accuracy: 0.5)
  }

  func testStatusBarMenuValidationToggleAndLayoutStayWindowLocal() throws {
    try preservingStatusBarDefault {
      let document = EVDocument()
      let controller = EVDocumentWindowController(document: document, editorSurface: Surface())
      defer { controller.close() }
      controller.showWindow(nil)
      let contentController = controller.documentContentController
      let item = NSMenuItem(
        title: "Show Status Bar",
        action: #selector(contentController.toggleStatusBar(_:)),
        keyEquivalent: ""
      )

      XCTAssertTrue(contentController.validateMenuItem(item))
      XCTAssertEqual(item.state, .on)
      contentController.toggleStatusBar(item)

      var geometry = try XCTUnwrap(controller.currentGeometry)
      XCTAssertFalse(geometry.statusBarIsVisible)
      XCTAssertEqual(geometry.statusBar.height, 0, accuracy: 0.5)
      XCTAssertEqual(geometry.editor, geometry.content)
      XCTAssertEqual(EVConfigurationStore.shared.showStatusBar, false)
      XCTAssertTrue(contentController.validateMenuItem(item))
      XCTAssertEqual(item.state, .off)

      contentController.toggleStatusBar(item)
      geometry = try XCTUnwrap(controller.currentGeometry)
      XCTAssertTrue(geometry.statusBarIsVisible)
      XCTAssertEqual(geometry.statusBar.height, EVStatusBarView.preferredHeight, accuracy: 0.5)
      XCTAssertEqual(
        geometry.editor.height,
        geometry.content.height - EVStatusBarView.preferredHeight,
        accuracy: 0.5
      )
    }
  }

  func testNewWindowCommandRoutesToDocumentWhileOtherCommandsStaySurfaceOwned() throws {
    let backend = Backend()
    let document = EVDocument(editorBackend: backend)
    document.makeWindowControllers()
    defer { document.close() }
    let first = try XCTUnwrap(document.windowControllers.first as? EVDocumentWindowController)
    let newWindowItem = NSMenuItem(
      title: "New Window for Document",
      action: #selector(first.documentContentController.performEditorMenuCommand(_:)),
      keyEquivalent: ""
    )
    newWindowItem.tag = EVMenuCommand.newWindowForDocument.rawValue

    XCTAssertTrue(first.documentContentController.validateMenuItem(newWindowItem))
    first.documentContentController.performEditorMenuCommand(newWindowItem)

    XCTAssertEqual(document.windowControllers.count, 2)
    XCTAssertEqual(backend.surfaces.count, 2)
    XCTAssertTrue(document.windowControllers.last?.window?.isVisible ?? false)
    XCTAssertTrue(backend.surfaces[0].performedCommands.isEmpty)

    let unsupported = NSMenuItem(
      title: "Bold",
      action: #selector(first.documentContentController.performEditorMenuCommand(_:)),
      keyEquivalent: ""
    )
    unsupported.tag = EVMenuCommand.bold.rawValue
    backend.surfaces[0].presentations[.bold] = EVMenuItemPresentation(
      isEnabled: false,
      state: .mixed,
      title: "Bold (Unavailable)"
    )

    XCTAssertFalse(first.documentContentController.validateMenuItem(unsupported))
    XCTAssertEqual(unsupported.state, .mixed)
    XCTAssertEqual(unsupported.title, "Bold (Unavailable)")
    first.documentContentController.performEditorMenuCommand(unsupported)
    XCTAssertEqual(backend.surfaces[0].performedCommands, [.bold])

    let undo = NSMenuItem(
      title: "Undo",
      action: #selector(first.documentContentController.performEditorMenuCommand(_:)),
      keyEquivalent: "z"
    )
    undo.tag = EVMenuCommand.undo.rawValue
    backend.surfaces[0].presentations[.undo] = EVMenuItemPresentation(
      isEnabled: true,
      title: "Undo Insert Text"
    )
    XCTAssertTrue(first.documentContentController.validateMenuItem(undo))
    XCTAssertEqual(undo.title, "Undo Insert Text")
  }

  func testStackedSplitSharesBackendAndClosingOnePaneKeepsTheDocument() throws {
    let backend = Backend()
    let document = EVDocument(editorBackend: backend)
    document.makeWindowControllers()
    defer { document.close() }
    let controller = try XCTUnwrap(document.windowControllers.first as? EVDocumentWindowController)
    controller.showWindow(nil)
    let initial = controller.editorSurface
    var result: Result<String?, Error>?
    let state = backend.persistenceState
    controller.perform(documentHostRequests: [
      EVDocumentHostRequest(
        kind: .split,
        documentID: state.documentID, documentRevision: state.documentRevision)
    ]) { result = $0 }
    _ = try XCTUnwrap(result).get()
    XCTAssertEqual(controller.paneCount, 2)
    XCTAssertEqual(backend.surfaces.count, 2)
    XCTAssertFalse(initial.viewController === controller.editorSurface.viewController)
    XCTAssertTrue(controller.activeDocument === document)
    let firstFrame = backend.surfaces[0].viewController.view.convert(
      backend.surfaces[0].viewController.view.bounds, to: nil)
    let secondFrame = backend.surfaces[1].viewController.view.convert(
      backend.surfaces[1].viewController.view.bounds, to: nil)
    XCTAssertEqual(firstFrame.width, secondFrame.width, accuracy: 1)
    XCTAssertFalse(firstFrame.intersects(secondFrame))
    XCTAssertLessThan(firstFrame.height, 400)
    XCTAssertGreaterThan(firstFrame.height, 100)
    controller.perform(documentHostRequests: [
      EVDocumentHostRequest(
        kind: .quit,
        documentID: state.documentID, documentRevision: state.documentRevision)
    ]) { result = $0 }
    _ = try XCTUnwrap(result).get()
    XCTAssertEqual(controller.paneCount, 1)
    XCTAssertTrue(controller.activeDocument === document)
    XCTAssertEqual(document.windowControllers.count, 1)
    XCTAssertTrue(controller.window?.isVisible == true)
  }

  func testDocumentIdentityRecognizesSymbolicAndHardLinks() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let original = directory.appendingPathComponent("original.txt")
    let symbolic = directory.appendingPathComponent("symbolic.txt")
    let hard = directory.appendingPathComponent("hard.txt")
    let different = directory.appendingPathComponent("different.txt")
    try Data("same".utf8).write(to: original)
    try Data("same".utf8).write(to: different)
    try FileManager.default.createSymbolicLink(at: symbolic, withDestinationURL: original)
    try FileManager.default.linkItem(at: original, to: hard)
    XCTAssertTrue(EVDocumentIdentity.sameFile(original, symbolic))
    XCTAssertTrue(EVDocumentIdentity.sameFile(original, hard))
    XCTAssertFalse(EVDocumentIdentity.sameFile(original, different))
    let document = EVDocument(editorBackend: Backend())
    document.fileURL = original
    NSDocumentController.shared.addDocument(document)
    defer { document.close() }
    XCTAssertTrue(EVDocumentIdentity.existingDocument(at: hard) === document)
    XCTAssertTrue(EVDocumentIdentity.existingDocument(at: symbolic) === document)
  }

  func testSplitReusesAnotherOpenDocumentAndRebindsWhenTheOriginalPaneCloses() throws {
    let firstBackend = Backend()
    firstBackend.persistenceState.documentID = 10
    let secondBackend = Backend()
    secondBackend.persistenceState.documentID = 20
    let first = EVDocument(editorBackend: firstBackend)
    let second = EVDocument(editorBackend: secondBackend)
    let file = FileManager.default.temporaryDirectory.appendingPathComponent(
      "viem-pane-\(UUID().uuidString).txt")
    try Data().write(to: file)
    defer { try? FileManager.default.removeItem(at: file) }
    second.fileURL = file
    NSDocumentController.shared.addDocument(first)
    NSDocumentController.shared.addDocument(second)
    first.makeWindowControllers()
    defer {
      first.close()
      second.close()
    }
    let controller = try XCTUnwrap(first.windowControllers.first as? EVDocumentWindowController)
    controller.showWindow(nil)
    var result: Result<String?, Error>?
    controller.perform(documentHostRequests: [
      EVDocumentHostRequest(
        kind: .split,
        documentID: 10, documentRevision: 0, path: file.path)
    ]) { result = $0 }
    _ = try XCTUnwrap(result).get()
    XCTAssertEqual(controller.paneCount, 2)
    XCTAssertTrue(controller.activeDocument === second)
    XCTAssertTrue(EVDocumentWindowController.windowShowing(document: second) === controller.window)
    XCTAssertEqual(second.windowControllers.count, 0)
    XCTAssertEqual(secondBackend.surfaces.count, 1)
    XCTAssertTrue(
      controller.window?.makeFirstResponder(firstBackend.surfaces[0].viewController.view) == true)
    XCTAssertTrue(controller.activeDocument === first)
    controller.perform(documentHostRequests: [
      EVDocumentHostRequest(
        kind: .quit,
        documentID: 10, documentRevision: 0)
    ]) { result = $0 }
    _ = try XCTUnwrap(result).get()
    XCTAssertTrue(controller.activeDocument === second)
    XCTAssertEqual(controller.paneCount, 1)
    XCTAssertEqual(second.windowControllers.count, 1)
    XCTAssertTrue(controller.document === second)
    XCTAssertFalse(NSDocumentController.shared.documents.contains { $0 === first })
  }

  func testFileDropReplacesExactCleanPaneAndOpensRemainingFilesInNewWindows() throws {
    let backend = Backend()
    let original = EVDocument(editorBackend: backend)
    original.makeWindowControllers()
    let controller = try XCTUnwrap(original.windowControllers.first as? EVDocumentWindowController)
    controller.showWindow(nil)
    let target = controller.editorSurface
    var split: Result<String?, Error>?
    controller.perform(documentHostRequests: [.init(kind: .split, documentID: 0, documentRevision: 0)]) { split = $0 }
    _ = try XCTUnwrap(split).get()
    XCTAssertFalse(controller.editorSurface === target)
    let firstBackend = Backend()
    let first = EVDocument(editorBackend: firstBackend)
    let second = EVDocument(editorBackend: Backend())
    let urls = ["first", "second"].map { URL(fileURLWithPath: "/tmp/viem-drop-\(UUID().uuidString)-\($0).txt") }
    first.fileURL = urls[0]; second.fileURL = urls[1]
    for document in [original, first, second] { NSDocumentController.shared.addDocument(document) }
    defer { for document in [original, first, second] { document.close() } }
    let done = expectation(description: "files opened")
    controller.openDroppedFiles(urls, in: target) { result in
      if case .failure(let error) = result { XCTFail("\(error)") }
      done.fulfill()
    }
    wait(for: [done], timeout: 5)
    XCTAssertEqual(controller.paneCount, 2)
    XCTAssertTrue(controller.activeDocument === first)
    XCTAssertEqual(firstBackend.surfaces.count, 1)
    XCTAssertEqual(second.windowControllers.count, 1)
    XCTAssertTrue(second.windowControllers[0].window?.isVisible == true)
    XCTAssertEqual(original.windowControllers.count, 1, "The other pane retains the original document")
  }

  func testFileDropOnDirtyPaneOpensNewWindowWithTheExistingBackend() throws {
    let backend = Backend()
    backend.persistenceState.isDirty = true
    let original = EVDocument(editorBackend: backend)
    original.makeWindowControllers()
    let controller = try XCTUnwrap(original.windowControllers.first as? EVDocumentWindowController)
    let incomingBackend = Backend()
    let incoming = EVDocument(editorBackend: incomingBackend)
    let url = URL(fileURLWithPath: "/tmp/viem-drop-\(UUID().uuidString).txt")
    incoming.fileURL = url
    incoming.makeWindowControllers()
    NSDocumentController.shared.addDocument(incoming)
    defer { original.close(); incoming.close() }
    let done = expectation(description: "existing file opened in new window")
    controller.openDroppedFiles([url], in: controller.editorSurface) { result in
      if case .failure(let error) = result { XCTFail("\(error)") }
      done.fulfill()
    }
    wait(for: [done], timeout: 5)
    XCTAssertTrue(controller.activeDocument === original)
    XCTAssertTrue(backend.persistenceState.isDirty)
    XCTAssertEqual(incoming.windowControllers.count, 2)
    XCTAssertEqual(incomingBackend.surfaces.count, 2)
  }

  func testFileDropRechecksDirtyStateAfterAsynchronousOpenAndFailurePreservesPane() throws {
    let backend = Backend()
    let original = EVDocument(editorBackend: backend)
    original.makeWindowControllers()
    let controller = try XCTUnwrap(original.windowControllers.first as? EVDocumentWindowController)
    let incoming = EVDocument(editorBackend: Backend())
    let url = URL(fileURLWithPath: "/tmp/viem-drop-\(UUID().uuidString).txt")
    defer { original.close(); incoming.close() }
    var resume: (@MainActor (EVDocument?, Error?) -> Void)?
    let done = expectation(description: "delayed open")
    controller.openDroppedFiles([url], in: controller.editorSurface, using: { _, completion in resume = completion }) { result in
      if case .failure(let error) = result { XCTFail("\(error)") }
      done.fulfill()
    }
    backend.persistenceState.isDirty = true
    backend.persistenceStateDidChange?(backend.persistenceState)
    try XCTUnwrap(resume)(incoming, nil)
    wait(for: [done], timeout: 5)
    XCTAssertTrue(controller.activeDocument === original)
    XCTAssertEqual(incoming.windowControllers.count, 1)
    backend.persistenceState.isDirty = false
    backend.persistenceStateDidChange?(backend.persistenceState)
    let cleanAgain = expectation(description: "changed revision with a clean save point")
    controller.openDroppedFiles([url], in: controller.editorSurface, using: { _, completion in resume = completion }) { result in
      if case .failure(let error) = result { XCTFail("\(error)") }
      cleanAgain.fulfill()
    }
    backend.persistenceState.documentRevision += 1
    try XCTUnwrap(resume)(incoming, nil)
    wait(for: [cleanAgain], timeout: 5)
    XCTAssertTrue(controller.activeDocument === original, "Type then undo/save still changes the captured revision")
    XCTAssertEqual(incoming.windowControllers.count, 2)
    let failed = expectation(description: "failed open")
    controller.openDroppedFiles([url], in: controller.editorSurface, using: { _, completion in
      completion(nil, CocoaError(.fileReadNoSuchFile))
    }) { result in
      if case .success = result { XCTFail("Missing file must report an error") }
      failed.fulfill()
    }
    wait(for: [failed], timeout: 5)
    XCTAssertTrue(controller.activeDocument === original)
    XCTAssertFalse(backend.persistenceState.isDirty)
  }
}
