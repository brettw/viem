import AppKit
import XCTest
@testable import ViemEditor

@MainActor final class EVCompletionPopupLayoutTests: XCTestCase {
    private func fixture() throws -> (EVCompletionPopup, NSWindow, NSRect, NSScrollView, NSTableView) {
        let screen = try XCTUnwrap(NSScreen.main?.visibleFrame)
        guard screen.width > 660, screen.height > 450 else {
            throw XCTSkip("Placement fixture needs room for an unclamped native popup")
        }
        let owner = NSWindow(contentRect: screen.insetBy(dx: 30, dy: 30),
                             styleMask: [.titled], backing: .buffered, defer: false)
        owner.isReleasedWhenClosed = false
        addTeardownBlock { owner.close() }
        let popup = EVCompletionPopup()
        addTeardownBlock { popup.hide() }
        let scroll = try XCTUnwrap(popup.window.contentView?.subviews.compactMap { $0 as? NSScrollView }.first)
        let table = try XCTUnwrap(scroll.documentView as? NSTableView)
        let anchor = NSRect(x: screen.midX, y: screen.maxY - 100, width: 1, height: 20)
        return (popup, owner, anchor, scroll, table)
    }

    private func drawingRect(of row: Int, table: NSTableView, window: NSWindow) throws -> (NSTextField, NSRect) {
        let cell = try XCTUnwrap(table.view(atColumn: 0, row: row, makeIfNecessary: true) as? NSTableCellView)
        cell.layoutSubtreeIfNeeded()
        let text = try XCTUnwrap(cell.textField)
        let drawing = try XCTUnwrap(text.cell).drawingRect(forBounds: text.bounds)
        return (text, window.convertToScreen(text.convert(drawing, to: nil)))
    }

    func testNativeTextLeadingEdgeMatchesWordAnchorAndDoesNotFollowSelection() throws {
        let (popup, owner, anchor, scroll, table) = try fixture()
        scroll.scrollerStyle = .legacy
        popup.show(items: [], selectedIndex: nil, searching: true, truncated: false,
                   at: anchor, in: owner, rightToLeft: false)
        let searchingX = popup.window.frame.minX
        let items = (0 ..< 20).map { "word\($0)" }
        popup.show(items: items, selectedIndex: 0, searching: true, truncated: false,
                   at: anchor, in: owner, rightToLeft: false)
        let firstFrame = popup.window.frame
        let (_, firstText) = try drawingRect(of: 0, table: table, window: popup.window)
        XCTAssertEqual(firstText.minX, anchor.minX, accuracy: 0.5)
        XCTAssertEqual(searchingX, firstFrame.minX, accuracy: 0.5)
        popup.show(items: items, selectedIndex: 17, searching: false, truncated: false,
                   at: anchor, in: owner, rightToLeft: false)
        let (_, selectedText) = try drawingRect(of: 17, table: table, window: popup.window)
        XCTAssertEqual(selectedText.minX, anchor.minX, accuracy: 0.5)
        XCTAssertEqual(popup.window.frame.minX, firstFrame.minX, accuracy: 0.5)
        XCTAssertEqual(popup.window.frame.maxY, firstFrame.maxY, accuracy: 0.5)
        XCTAssertEqual(popup.nativeSelectedRow, 17)
        let shifted = anchor.offsetBy(dx: 30, dy: -45)
        popup.show(items: items, selectedIndex: 17, searching: false, truncated: false,
                   at: shifted, in: owner, rightToLeft: false)
        let (_, shiftedText) = try drawingRect(of: 17, table: table, window: popup.window)
        XCTAssertEqual(shiftedText.minX, shifted.minX, accuracy: 0.5)
        XCTAssertEqual(popup.window.frame.maxY, firstFrame.maxY - 45, accuracy: 0.5)
    }

    func testRightToLeftPopupMirrorsTextEdgeWritingDirectionAndScrollbar() throws {
        let (popup, owner, anchor, scroll, table) = try fixture()
        scroll.scrollerStyle = .legacy
        let items = (0 ..< 20).map { "كلمة\($0)" }
        popup.show(items: items, selectedIndex: 0, searching: true, truncated: false,
                   at: anchor, in: owner, rightToLeft: true)
        let firstFrame = popup.window.frame
        let (text, drawing) = try drawingRect(of: 0, table: table, window: popup.window)
        XCTAssertEqual(drawing.maxX, anchor.maxX, accuracy: 0.5)
        XCTAssertEqual(text.alignment, .right)
        XCTAssertEqual(text.baseWritingDirection, .rightToLeft)
        XCTAssertEqual(table.userInterfaceLayoutDirection, .rightToLeft)
        let scroller = try XCTUnwrap(scroll.verticalScroller)
        XCTAssertFalse(scroller.isHidden)
        XCTAssertLessThanOrEqual(scroller.frame.maxX, scroll.contentView.frame.minX)
        popup.show(items: items, selectedIndex: 19, searching: false, truncated: false,
                   at: anchor, in: owner, rightToLeft: true)
        let (_, selectedDrawing) = try drawingRect(of: 19, table: table, window: popup.window)
        XCTAssertEqual(selectedDrawing.maxX, anchor.maxX, accuracy: 0.5)
        XCTAssertEqual(popup.window.frame.minX, firstFrame.minX, accuracy: 0.5)
        XCTAssertEqual(popup.window.frame.maxY, firstFrame.maxY, accuracy: 0.5)
        popup.show(items: ["word"], selectedIndex: 0, searching: false, truncated: false,
                   at: anchor, in: owner, rightToLeft: false)
        let (ltrText, ltrDrawing) = try drawingRect(of: 0, table: table, window: popup.window)
        XCTAssertEqual(ltrDrawing.minX, anchor.minX, accuracy: 0.5)
        XCTAssertEqual(ltrText.alignment, .left)
        XCTAssertEqual(ltrText.baseWritingDirection, .leftToRight)
    }

    func testGrowingResultsKeepTheSameSideOfAnUnchangedWordAnchor() {
        let screen = NSRect(x: 100, y: 200, width: 800, height: 600)
        let anchor = NSRect(x: 500, y: 300, width: 1, height: 20)
        let small = EVCompletionPopup.frame(for: NSSize(width: 320, height: 30), maximumHeight: 222,
                                           anchor: anchor, textInset: 8, rightToLeft: false, visibleFrame: screen)
        let expanded = EVCompletionPopup.frame(for: NSSize(width: 320, height: 222), maximumHeight: 222,
                                              anchor: anchor, textInset: 8, rightToLeft: false, visibleFrame: screen)
        XCTAssertEqual(small.minY, anchor.maxY + 3)
        XCTAssertEqual(expanded.minY, small.minY)
        XCTAssertEqual(expanded.minX, small.minX)
    }

    func testFrameClampsMirroredTextAlignmentOnlyAtScreenBoundary() {
        let screen = NSRect(x: 100, y: 200, width: 800, height: 600)
        let size = NSSize(width: 250, height: 200)
        let anchor = NSRect(x: 500, y: 600, width: 1, height: 20)
        let ltr = EVCompletionPopup.frame(for: size, maximumHeight: 222, anchor: anchor, textInset: 12,
                                         rightToLeft: false, visibleFrame: screen)
        let rtl = EVCompletionPopup.frame(for: size, maximumHeight: 222, anchor: anchor, textInset: 12,
                                         rightToLeft: true, visibleFrame: screen)
        XCTAssertEqual(ltr.minX + 12, anchor.minX)
        XCTAssertEqual(rtl.maxX - 12, anchor.maxX)
        XCTAssertEqual(ltr.maxY, rtl.maxY)
        let edge = EVCompletionPopup.frame(for: size, maximumHeight: 222, anchor: NSRect(x: 105, y: 205, width: 1, height: 20),
                                          textInset: 12, rightToLeft: true, visibleFrame: screen)
        XCTAssertEqual(edge.minX, screen.minX)
        XCTAssertTrue(screen.contains(edge))
    }
}
