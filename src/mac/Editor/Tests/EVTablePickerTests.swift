import AppKit
import XCTest
@testable import ViemEditor

@MainActor
final class EVTablePickerTests: XCTestCase {
    func testHoverNeverGrowsAndDraggingClampsEachDimensionIndependently() {
        var state = EVTablePickerState()
        state.point(column: 2, row: 3, growing: false)
        XCTAssertEqual(state.selectedColumns, 3); XCTAssertEqual(state.selectedRows, 4)
        state.point(column: 18, row: 30, growing: false)
        XCTAssertFalse(state.hasSelection); XCTAssertEqual(state.columns, 10); XCTAssertEqual(state.rows, 10)
        state.point(column: 200, row: 14, growing: true)
        XCTAssertEqual(state.selectedColumns, 20); XCTAssertEqual(state.selectedRows, 15)
        state.point(column: 2, row: 200, growing: true)
        XCTAssertEqual(state.selectedColumns, 3); XCTAssertEqual(state.selectedRows, 50)
        state.point(column: -1, row: 200, growing: true)
        XCTAssertFalse(state.hasSelection)
        state.point(column: 200, row: -1, growing: true)
        XCTAssertFalse(state.hasSelection)
        state.point(column: 0, row: 0, growing: false)
        XCTAssertEqual(state.selectedColumns, 1); XCTAssertEqual(state.selectedRows, 1)
        XCTAssertEqual(state.columns, 20); XCTAssertEqual(state.rows, 50)
    }

    func testKeyboardCanReachEveryPickerSizeAndRetainsExpandedExtent() {
        var state = EVTablePickerState(); state.move(columns: 0, rows: 0)
        for _ in 0..<100 { state.move(columns: 1, rows: 1) }
        XCTAssertEqual(state.selectedColumns, 20); XCTAssertEqual(state.selectedRows, 50)
        for _ in 0..<100 { state.move(columns: -1, rows: -1) }
        XCTAssertEqual(state.selectedColumns, 1); XCTAssertEqual(state.selectedRows, 1)
        XCTAssertEqual(state.columns, 20); XCTAssertEqual(state.rows, 50)
    }

    func testToolbarReleaseOpensWithoutInsertingThenCellReleaseAccepts() throws {
        let window = NSWindow(contentRect: NSRect(x: 100, y: 200, width: 600, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        let content = try XCTUnwrap(window.contentView)
        window.isReleasedWhenClosed = false
        let button = NSButton(frame: NSRect(x: 20, y: 420, width: 28, height: 26))
        let editor = NSView(frame: NSRect(x: 0, y: 0, width: 600, height: 400))
        content.addSubview(button); content.addSubview(editor); window.makeKeyAndOrderFront(nil)
        let picker = EVTablePickerController(); defer { picker.close(); window.close() }
        var accepted: [(Int, Int)] = []
        func event(_ type: NSEvent.EventType, _ screenPoint: NSPoint) throws -> NSEvent {
            try XCTUnwrap(NSEvent.mouseEvent(with: type, location: window.convertPoint(fromScreen: screenPoint), modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        }
        let press = window.convertPoint(toScreen: button.convert(NSPoint(x: 10, y: 10), to: nil))
        picker.open(from: button, editor: editor, event: try event(.leftMouseDown, press)) { accepted.append(($0, $1)) }
        _ = picker.handle(try event(.leftMouseUp, press))
        XCTAssertTrue(picker.isOpen); XCTAssertTrue(accepted.isEmpty)
        let grid = picker.scrollScreenRect
        let cell = NSPoint(x: grid.minX + 2 * 21 + 10, y: grid.maxY - 3 * 21 - 10)
        _ = picker.handle(try event(.mouseMoved, cell))
        XCTAssertEqual(picker.state.selectedColumns, 3); XCTAssertEqual(picker.state.selectedRows, 4)
        _ = picker.handle(try event(.leftMouseDown, cell))
        _ = picker.handle(try event(.leftMouseUp, cell))
        XCTAssertFalse(picker.isOpen); XCTAssertEqual(accepted.count, 1)
        XCTAssertEqual(accepted.first?.0, 3); XCTAssertEqual(accepted.first?.1, 4)
    }

    func testToolbarDragThresholdAndAboveLeftReleaseCancel() throws {
        let window = NSWindow(contentRect: NSRect(x: 100, y: 200, width: 600, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let button = NSButton(frame: NSRect(x: 20, y: 420, width: 28, height: 26))
        let editor = NSView(); window.contentView?.addSubview(button); window.contentView?.addSubview(editor); window.makeKeyAndOrderFront(nil)
        let picker = EVTablePickerController(); defer { picker.close(); window.close() }
        var accepts = 0
        func event(_ type: NSEvent.EventType, _ point: NSPoint) throws -> NSEvent {
            try XCTUnwrap(NSEvent.mouseEvent(with: type, location: window.convertPoint(fromScreen: point), modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
        }
        let press = window.convertPoint(toScreen: button.convert(NSPoint(x: 10, y: 10), to: nil))
        picker.open(from: button, editor: editor, event: try event(.leftMouseDown, press)) { _, _ in accepts += 1 }
        XCTAssertTrue(picker.isOpen, "Picker should be open immediately")
        let moved = try event(.leftMouseDragged, NSPoint(x: press.x + 1, y: press.y + 1))
        _ = picker.handle(moved)
        XCTAssertTrue(picker.isOpen, "Jitter should keep the picker open")
        _ = picker.handle(try event(.leftMouseUp, press))
        XCTAssertTrue(picker.isOpen); XCTAssertEqual(accepts, 0)
        let grid = picker.scrollScreenRect
        let cell = NSPoint(x: grid.minX + 10, y: grid.maxY - 10)
        _ = picker.handle(try event(.leftMouseDown, cell))
        _ = picker.handle(try event(.leftMouseDragged, NSPoint(x: grid.minX - 5, y: grid.minY - 400)))
        _ = picker.handle(try event(.leftMouseUp, NSPoint(x: grid.minX - 5, y: grid.minY - 400)))
        XCTAssertFalse(picker.isOpen); XCTAssertEqual(accepts, 0)
    }
    func testNativeMenuHostsTheSameInteractiveGridAndAcceptsPointerSize() async throws {
        try await chooseInNativeMenu(heldDrag: false)
    }

    func testNativeMenuHeldPressCanDragIntoGridAndReleaseToInsert() async throws {
        try await chooseInNativeMenu(heldDrag: true)
    }

    private func chooseInNativeMenu(heldDrag: Bool) async throws {
        let window = NSWindow(contentRect: NSRect(x: 100, y: 200, width: 600, height: 400),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let editor = try XCTUnwrap(window.contentView)
        window.makeKeyAndOrderFront(nil)
        let menu = NSMenu(title: "Table")
        menu.autoenablesItems = false
        let picker = EVTablePickerController()
        defer { picker.close(); window.close() }
        let accepted = expectation(description: "native menu commits chosen table dimensions")
        var dimensions: (Int, Int)?
        let content = picker.menuView(in: menu, editor: editor) { columns, rows in
            dimensions = (columns, rows)
            accepted.fulfill()
        }
        let item = NSMenuItem(title: "Table dimensions", action: nil, keyEquivalent: "")
        item.view = content; menu.addItem(item)
        picker.didOpen = { [weak picker] in
            guard let picker else { return }
            // AppKit's didBeginTracking notification precedes custom-view
            // attachment. Wait for the picker's actual attached lifecycle.
            RunLoop.main.perform(inModes: [.eventTracking]) {
                MainActor.assumeIsolated {
                    guard let host = content.window else { XCTFail("Native menu did not attach the picker"); menu.cancelTracking(); return }
                    XCTAssertTrue(picker.isOpen)
                    let grid = picker.scrollScreenRect
                    let point = NSPoint(x: grid.minX + 2 * 21 + 10, y: grid.maxY - 3 * 21 - 10)
                    let events: [NSEvent.EventType] = heldDrag
                        ? [.leftMouseDragged, .leftMouseUp]
                        : [.mouseMoved, .leftMouseDown, .leftMouseUp]
                    for type in events {
                        guard let event = NSEvent.mouseEvent(with: type, location: host.convertPoint(fromScreen: point),
                            modifierFlags: [], timestamp: 0, windowNumber: host.windowNumber, context: nil,
                            eventNumber: 1, clickCount: 1, pressure: 1) else { continue }
                        // Native menus track outside NSApplication's synthetic
                        // postEvent queue. Deliver the AppKit event to the real
                        // attached item view while its NSMenu is tracking.
                        switch type {
                        case .mouseMoved: content.mouseMoved(with: event)
                        case .leftMouseDown: content.mouseDown(with: event)
                        case .leftMouseDragged: content.mouseDragged(with: event)
                        case .leftMouseUp: content.mouseUp(with: event)
                        default: XCTFail("Unexpected picker test event")
                        }
                    }
                }
            }
        }
        let watchdog = Timer(timeInterval: 5, repeats: false) { _ in
            MainActor.assumeIsolated { XCTFail("Table menu did not finish tracking"); menu.cancelTracking() }
        }
        RunLoop.main.add(watchdog, forMode: .eventTracking)
        defer { watchdog.invalidate() }
        menu.popUp(positioning: nil, at: NSPoint(x: 30, y: 300), in: editor)
        await fulfillment(of: [accepted], timeout: 2)
        XCTAssertEqual(dimensions?.0, 3)
        XCTAssertEqual(dimensions?.1, 4)
        XCTAssertFalse(picker.isOpen)
    }

}
