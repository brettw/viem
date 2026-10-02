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
}
