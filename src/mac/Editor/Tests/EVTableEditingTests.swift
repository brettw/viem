import AppKit
import CViemCore
import ViemAppShell
import XCTest
@testable import ViemEditor

@MainActor
final class EVTableEditingTests: XCTestCase {
    private final class Pasteboard: EVPasteboardAccess {
        var content: EVClipboardRepresentations?
        var viemGeneration: UInt64 = 1
        var viemIsWritable: Bool { true }
        func viemString() -> String? { content?.plainText }
        func viemCanReadString() -> Bool { content != nil }
        func viemClearContents() -> Int { content = nil; viemGeneration += 1; return Int(viemGeneration) }
        func viemSetString(_ string: String) -> Bool { content = EVClipboardRepresentations(plainText: string); viemGeneration += 1; return true }
        func viemData(forType type: NSPasteboard.PasteboardType) -> Data? {
            type == EVClipboardRepresentations.fragmentType ? content?.fragment : type == .rtf ? content?.richText : nil
        }
        func viemWrite(_ content: EVClipboardRepresentations) -> Bool { self.content = content; viemGeneration += 1; return true }
    }

    private func surface(_ text: String, source: Bool = false) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController, EVCoreViewSession) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-tables-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory))
        try backend.read(source: Data(text.utf8), typeName: source ? EVDocument.markdownSourceType : EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded(); surface.view.frame = NSRect(x: 0, y: 0, width: 700, height: 400); surface.viewDidLayout()
        return (backend, surface, try XCTUnwrap(surface.session))
    }

    func testInsertTableCountsBodyRowsAndRestoresExactBytesWithUndoInBothViews() throws {
        for source in [false, true] {
            let (backend, surface, session) = try surface("Before", source: source)
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: 6, length: 0))
            let context = try session.tableContext()
            XCTAssertNotEqual(context.flags & UInt32(VIEM_TABLE_CAN_INSERT), 0)
            _ = try session.insertTable(columns: 3, bodyRows: 4, expected: context.selection)
            surface.refreshPresentation()
            let after = try session.tableContext()
            XCTAssertEqual(after.columns, 3); XCTAssertEqual(after.rows, 5)
            XCTAssertEqual(after.row, 0); XCTAssertEqual(after.column, 0)
            XCTAssertEqual(surface.viewPresentation.mode, UInt32(VIEM_MODE_INSERT))
            let saved = try backend.serializedSource(typeName: EVDocument.markdownType)
            XCTAssertTrue(String(decoding: saved, as: UTF8.self).contains("| --- | --- | --- |"))
            _ = try session.undo()
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data("Before".utf8))
            _ = try session.redo()
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), saved)
        }
    }

    func testNativeStructuralActionsRetainAlignmentAndUndoExactSpelling() throws {
        let source = "| A | B |\n| :-- | --: |\n| one | two |"
        let (backend, surface, session) = try surface(source)
        let original = try session.tableContext()
        XCTAssertEqual(original.rows, 2); XCTAssertEqual(original.columns, 2)
        XCTAssertNoThrow(try session.tableAction(UInt32(VIEM_TABLE_INSERT_ROW_BELOW), expected: original))
        surface.refreshPresentation()
        XCTAssertEqual(try session.tableContext().rows, 3)
        _ = try session.undo()
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        let context = try session.tableContext()
        XCTAssertNoThrow(try session.tableAction(UInt32(VIEM_TABLE_SET_ALIGNMENT), alignment: UInt32(VIEM_TABLE_ALIGN_CENTER), expected: context))
        XCTAssertEqual(try session.tableContext().alignment, UInt32(VIEM_TABLE_ALIGN_CENTER))
        _ = try session.undo()
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
    }

    func testHoverDismissesWhenSelectionChangesAndContextActionsNameTheirTarget() throws {
        let (_, surface, _) = try surface("| Header | Other |\n| --- | --- |\n| body | value |")
        let editor = surface.editorView
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let header = try XCTUnwrap(snapshot.tableCells.first { $0.row == 0 && $0.column == 0 })
        let rect = editor.viewRect(header.rect), table = editor.viewRect(header.table_rect)
        let interaction = EVTableInteraction(editor: editor)
        let widget = try XCTUnwrap(editor.subviews.last as? NSStackView)
        interaction.moved(to: NSPoint(x: rect.midX, y: table.minY))
        XCTAssertFalse(widget.isHidden)
        XCTAssertLessThanOrEqual(widget.frame.minY, rect.maxY + 4, "A flipped widget stays near its cell, even for a long table")
        XCTAssertTrue(widget.arrangedSubviews.contains { $0.accessibilityLabel() == "Column 1 alignment" })
        editor.setAccessibilitySelectedTextRange(NSRange(location: Int(header.text_start), length: 1))
        interaction.invalidate()
        XCTAssertTrue(widget.isHidden, "Caret/selection changes retire the widget's exact expected selection")

        let key = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: 0, context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: 0))
        let menu = NSMenu()
        interaction.appendMenu(to: menu, event: key)
        XCTAssertFalse(menu.items.contains { $0.title.hasPrefix("Insert row above") })
        XCTAssertTrue(menu.items.contains { $0.title == "Delete header row" })
        XCTAssertTrue(menu.items.contains { $0.title == "Insert column left of column 1" })
        let align = try XCTUnwrap(menu.items.first { $0.title == "Column 1 alignment" }?.submenu)
        XCTAssertEqual(align.items.map(\.title), ["Left", "Center", "Right"])
    }

    func testRowHoverWorksOnBothSidesWithoutChangingSelectionAndKeepsItsTargetIntoWidget() throws {
        let text = "| Header | Other |\n| --- | --- |\n| first | one |\n| second | two |\n| third | three |"
        let (backend, surface, session) = try surface(text)
        let editor = surface.editorView
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let rows = snapshot.tableCells.filter { $0.column == 0 }.sorted { $0.row < $1.row }
        XCTAssertEqual(rows.count, 4)
        let selection = try session.listSelection()
        let presentation = surface.viewPresentation
        let interaction = EVTableInteraction(editor: editor)
        let widget = try XCTUnwrap(editor.subviews.last as? NSStackView)
        for right in [false, true] {
            interaction.hide()
            for cell in rows {
                let table = editor.viewRect(cell.table_rect), rect = editor.viewRect(cell.rect)
                let point = NSPoint(x: right ? table.maxX : table.minX, y: rect.midY)
                interaction.moved(to: point)
                XCTAssertFalse(widget.isHidden)
                let labels = widget.arrangedSubviews.compactMap { $0.accessibilityLabel() }
                XCTAssertEqual(labels, cell.row == 0
                    ? ["Delete header", "Insert row below row 1"]
                    : ["Insert row above row \(cell.row + 1)", "Delete row \(cell.row + 1)", "Insert row below row \(cell.row + 1)"])
                if right { XCTAssertGreaterThanOrEqual(widget.frame.minX, table.maxX, "Prefer the side that activated the widget when there is room") }
                // Cross the gap and enter a button without rebuilding or retargeting it.
                let button = try XCTUnwrap(widget.arrangedSubviews.last as? NSButton)
                let entryX = point.x < widget.frame.midX ? widget.frame.minX - 1 : widget.frame.maxX + 1
                interaction.moved(to: NSPoint(x: entryX, y: widget.frame.midY))
                interaction.moved(to: NSPoint(x: widget.frame.midX, y: widget.frame.midY))
                XCTAssertFalse(widget.isHidden)
                XCTAssertTrue(widget.arrangedSubviews.last === button)
                XCTAssertTrue(try session.listSelection().isSameSelection(as: selection))
                XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, presentation.cursor_utf8_offset)
                XCTAssertEqual(surface.viewPresentation.mode, presentation.mode)
            }
        }
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(text.utf8))

        interaction.hide()
        let target = try XCTUnwrap(rows.first { $0.row == 1 })
        let rect = editor.viewRect(target.rect), table = editor.viewRect(target.table_rect)
        interaction.moved(to: NSPoint(x: table.maxX + 20, y: rect.midY))
        let insert = try XCTUnwrap(widget.arrangedSubviews.compactMap { $0 as? NSButton }
            .first { $0.accessibilityLabel() == "Insert row below row 2" })
        insert.performClick(nil)
        XCTAssertEqual(try session.tableContext().rows, 5)
        XCTAssertEqual(try session.tableContext().row, 2, "An explicit right-side action affects the hovered row, even when the caret was elsewhere")
        _ = try session.undo()
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(text.utf8))
    }

    func testRowHoverEdgesCornersAndClippingUseActualTableBoundaries() throws {
        let (_, surface, _) = try surface("| A | B | C |\n| --- | --- | --- |\n| one | two | three |\n| four | five | six |")
        let editor = surface.editorView
        let snapshot = try XCTUnwrap(surface.layoutSnapshot)
        let first = try XCTUnwrap(snapshot.tableCells.first)
        let table = editor.viewRect(first.table_rect)
        let interaction = EVTableInteraction(editor: editor)
        let widget = try XCTUnwrap(editor.subviews.last as? NSStackView)
        func labels(at point: NSPoint) -> [String] {
            interaction.hide(); interaction.moved(to: point)
            return widget.isHidden ? [] : widget.arrangedSubviews.compactMap { $0.accessibilityLabel() }
        }
        XCTAssertTrue(labels(at: NSPoint(x: table.maxX, y: table.minY)).contains("Column 3 alignment"))
        XCTAssertTrue(labels(at: NSPoint(x: table.minX, y: table.minY)).contains("Column 1 alignment"))
        let secondColumn = try XCTUnwrap(snapshot.tableCells.first { $0.row == 0 && $0.column == 1 })
        XCTAssertTrue(labels(at: NSPoint(x: editor.viewRect(secondColumn.rect).minX, y: table.minY)).contains("Column 2 alignment"))
        let secondRow = try XCTUnwrap(snapshot.tableCells.first { $0.row == 1 })
        let secondRect = editor.viewRect(secondRow.rect)
        for x in [table.minX, table.maxX] {
            XCTAssertTrue(labels(at: NSPoint(x: x, y: secondRect.minY)).contains("Delete row 2"))
            XCTAssertTrue(labels(at: NSPoint(x: x, y: table.maxY)).contains("Delete row 3"))
        }
        XCTAssertTrue(labels(at: NSPoint(x: table.maxX + 20, y: secondRect.midY)).contains("Delete row 2"))
        XCTAssertTrue(labels(at: NSPoint(x: table.maxX + 21, y: secondRect.midY)).isEmpty)

        let bounds = editor.bounds
        defer { editor.bounds = bounds }
        editor.bounds.size.width = table.maxX + 25
        XCTAssertTrue(labels(at: NSPoint(x: table.maxX, y: secondRect.midY)).contains("Delete row 2"))
        XCTAssertGreaterThanOrEqual(widget.frame.minX, editor.bounds.minX)
        XCTAssertLessThanOrEqual(widget.frame.maxX, editor.bounds.maxX)
        editor.bounds.size.width = table.maxX - 10
        XCTAssertTrue(labels(at: NSPoint(x: editor.bounds.maxX - 1, y: secondRect.midY)).isEmpty,
            "A clipped right edge does not become a hotspot at the viewport boundary")
        XCTAssertTrue(labels(at: NSPoint(x: table.maxX, y: secondRect.midY)).isEmpty,
            "Offscreen table geometry cannot activate a widget")
    }

    func testRowHoverDoesNotShowWidgetsInSourceView() throws {
        let (_, surface, _) = try surface("| A | B |\n| --- | --- |\n| one | two |")
        let editor = surface.editorView
        let interaction = EVTableInteraction(editor: editor)
        let widget = try XCTUnwrap(editor.subviews.last as? NSStackView)
        let cells = try XCTUnwrap(surface.layoutSnapshot).tableCells
        XCTAssertFalse(cells.isEmpty)
        surface.formattingToolbar.formattedView.performClick(nil)
        for cell in cells {
            let table = editor.viewRect(cell.table_rect), rect = editor.viewRect(cell.rect)
            for x in [table.minX, table.maxX, table.maxX + 10] {
                interaction.moved(to: NSPoint(x: x, y: rect.midY))
                XCTAssertTrue(widget.isHidden)
            }
        }
    }

    func testTableGeometryAndCellSelectionRemainPortableAndDisableBlockControls() throws {
        let (_, surface, session) = try surface("| A | B |\n| --- | --- |\n| one | two |")
        let snapshot = try session.layoutExport()
        XCTAssertEqual(snapshot.tableCells.count, 4)
        let first = try XCTUnwrap(snapshot.tableCells.first)
        let second = try XCTUnwrap(snapshot.tableCells.first { $0.row == 0 && $0.column == 1 })
        XCTAssertEqual(first.rect.y, second.rect.y, accuracy: 0.01)
        XCTAssertGreaterThan(second.rect.x, first.rect.x)
        var selection = ViemTableSelectionV1(); selection.struct_size = UInt32(MemoryLayout<ViemTableSelectionV1>.size)
        selection.active = 1; selection.document_id = snapshot.info.identity.document_id; selection.document_revision = snapshot.info.identity.document_revision
        selection.table_id = first.table_id; selection.anchor_row = 0; selection.anchor_column = 0; selection.active_row = 1; selection.active_column = 1
        _ = try session.selectTableCells(selection)
        surface.refreshPresentation()
        XCTAssertEqual(try session.tableSelection().active, 1)
        XCTAssertFalse(surface.formattingToolbar.paragraphStyle.isEnabled)
        XCTAssertFalse(surface.formattingToolbar.commandButtons[.bulletedList]?.isEnabled ?? true)
        XCTAssertTrue(surface.formattingToolbar.codeBlock.isHidden)
        selection.anchor_column = 1
        _ = try session.selectTableCells(selection)
        surface.refreshPresentation()
        XCTAssertEqual(surface.editorView.accessibilitySelectedText(), "B\ntwo")
        XCTAssertEqual(surface.editorView.accessibilitySelectedTextRanges()?.count, 2)
        let current = try XCTUnwrap(surface.layoutSnapshot)
        let rectangles = surface.editorView.selectionRectsForDrawing(in: current)
        XCTAssertEqual(rectangles.count, 2)
        for (rect, cell) in zip(rectangles, current.tableCells.filter { $0.column == 1 }) {
            XCTAssertEqual(rect, surface.editorView.viewRect(cell.rect), "Selection damage includes padding and empty cell area")
        }

    }
    func testIdleTableWidthsRefineOnWorkerAndStopWithoutChangingSourceOrCaret() async throws {
        let source = "| A | B |\n| --- | --- |\n" + (0..<2000).map { "| row \($0) | \($0 == 1999 ? String(repeating: "wide ", count: 30) : "small") |" }.joined(separator: "\n")
        let (backend, surface, session) = try surface(source)
        let before = surface.viewPresentation
        for _ in 0..<100 {
            if session.tableWidthRefinement.started > 0 { break }
            await Task.yield()
        }
        // A small viewport change must cancel its captured visible request,
        // then resume discovery rather than treating stale work as a failure.
        _ = try session.setViewportOrigin(left: 0, top: 6)
        surface.refreshPresentation()
        let viewport = surface.viewportState
        for _ in 0..<750 {
            if session.tableWidthRefinement.isIdle && session.tableWidthRefinement.started > 0 { break }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTAssertTrue(session.tableWidthRefinement.isIdle)
        XCTAssertNil(session.tableWidthRefinement.lastError)
        XCTAssertGreaterThan(session.tableWidthRefinement.installed, 0, "started=\(session.tableWidthRefinement.started), discarded=\(session.tableWidthRefinement.discarded)")
        XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, before.cursor_utf8_offset)
        XCTAssertEqual(surface.viewportState.top, viewport.top, accuracy: 0.01)
        XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(source.utf8))
        let started = session.tableWidthRefinement.started
        surface.refreshPresentation()
        try await Task.sleep(nanoseconds: 50_000_000)
        XCTAssertEqual(session.tableWidthRefinement.started, started, "Idle does not poll or repeat completed discovery")
        session.detach()
    }

    func testNativeCutPasteRoundTripsStyledRectangleWithBreaksAndEmptyCells() throws {
        let text = "# Table interaction test\n\n| Feature | Example | Status |\n| :--- | :---: | ---: |\n| Bold | **bold**<br>Hello, world! | ready |\n| Italic | *italic* | test |\n| Empty | | |\n\nAfter the table.\n"
        for insertMode in [false, true] {
            let (backend, surface, session) = try surface(text)
            let clipboard = Pasteboard(); surface.pasteboard = clipboard
            if insertMode { _ = try session.sendKey(kind: UInt32(VIEM_KEY_CHARACTER), codepoint: 105) }
            let before = try backend.formattedText()
            let snapshot = try session.layoutExport()
            let cell = try XCTUnwrap(snapshot.tableCells.first { $0.row == 1 && $0.column == 1 })
            var selected = ViemTableSelectionV1(); selected.struct_size = UInt32(MemoryLayout<ViemTableSelectionV1>.size)
            selected.active = 1; selected.document_id = snapshot.info.identity.document_id; selected.document_revision = snapshot.info.identity.document_revision
            selected.table_id = cell.table_id; selected.anchor_row = 1; selected.anchor_column = 1; selected.active_row = 3; selected.active_column = 2
            _ = try session.selectTableCells(selected); surface.refreshPresentation()
            surface.editorView.cutDocumentSelection(nil)
            XCTAssertEqual(surface.statusBarState.message, "", "cut, insert=\(insertMode)")
            XCTAssertNotNil(clipboard.content?.fragment)
            let payload = try XCTUnwrap(clipboard.content?.fragment)
            let json = try XCTUnwrap(JSONSerialization.jsonObject(with: payload) as? [String: Any])
            XCTAssertEqual(json["plain_text"] as? String, clipboard.content?.plainText)
            let matrix = try XCTUnwrap(json["table_cells"] as? [[[String: Any]]])
            XCTAssertEqual(matrix.count, 3); XCTAssertTrue(matrix.allSatisfy { $0.count == 2 })
            XCTAssertEqual(matrix[0][0]["plain_text"] as? String, "bold\nHello, world!")
            XCTAssertEqual(try session.tableContext().row, 1)
            XCTAssertEqual(try session.tableContext().column, 1)

            XCTAssertFalse(try backend.formattedText().contains("Hello, world!"))
            surface.editorView.pasteIntoDocument(nil)
            XCTAssertEqual(surface.statusBarState.message, "", "paste, insert=\(insertMode)")
            XCTAssertEqual(try backend.formattedText(), before, "insert=\(insertMode)")
            let pastedSource = try backend.serializedSource(typeName: EVDocument.markdownType)
            surface.refreshPresentation()
            let current = try XCTUnwrap(surface.layoutSnapshot)
            let pastedCell = try XCTUnwrap(current.tableCells.first { $0.row == 1 && $0.column == 1 })
            surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: Int(pastedCell.text_start),
                length: Int(pastedCell.text_end - pastedCell.text_start)))
            surface.refreshPresentation()
            XCTAssertEqual(surface.editorView.accessibilitySelectedText(), "bold\nHello, world!",
                "AX selection must target the pasted multiline cell; requested=\(pastedCell.text_start)..<\(pastedCell.text_end), actual=\(surface.selectedUTF8Ranges()), rows=\(current.rows.map { ($0.text_start, $0.text_end, $0.flags) })")
            for (command, style) in [(EVMenuCommand.bold, UInt32(VIEM_SEMANTIC_STYLE_STRONG)),
                                     (.italic, UInt32(VIEM_SEMANTIC_STYLE_EMPHASIS))] {
                surface.editorView.setAccessibilitySelectedTextRange(NSRange(location: Int(pastedCell.text_start),
                    length: Int(pastedCell.text_end - pastedCell.text_start)))
                surface.refreshPresentation()
                let capability = try session.semanticStylePresentation(style)
                XCTAssertNotEqual(capability.flags & UInt32(VIEM_SEMANTIC_STYLE_CAN_SET), 0,
                    "Multiline cell style \(style) must remain available after paste: flags=\(capability.flags), kind=\(capability.selection.kind), range=\(capability.selection.text_start)..<\(capability.selection.text_end)\n\(String(decoding: pastedSource, as: UTF8.self))")
                XCTAssertTrue(surface.presentation(for: command).isEnabled)
                _ = try session.setSemanticStyle(style, enabled: true, expected: capability.selection)
                surface.refreshPresentation()
                let applied = try session.semanticStylePresentation(style)
                let styledSource = try backend.serializedSource(typeName: EVDocument.markdownType)
                XCTAssertEqual(applied.state, UInt32(VIEM_SEMANTIC_STYLE_STATE_ON),
                    "style=\(style), before=\(capability.selection.text_start)..<\(capability.selection.text_end), after=\(applied.selection.text_start)..<\(applied.selection.text_end), selected=\(surface.selectedUTF8Ranges())\n\(String(decoding: styledSource, as: UTF8.self))")
                _ = try session.undo(); surface.refreshPresentation()
                XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), pastedSource)
            }
        }
    }

    func testGiantCellEndMotionAndTypingKeepNativeCaretVisible() async throws {
        let text = "# Long cell interaction test\n\n| Text | Neighbor |\n| --- | --- |\n| "
            + String(repeating: "readable text ", count: 40_000)
            + " | still a separate cell |\n\nAfter the table.\n"
        for warm in [false, true] {
            let (backend, surface, session) = try surface(text, source: true)
            defer { session.detach() }
            surface.formattingToolbar.formattedView.performClick(nil)
            XCTAssertEqual(backend.sourceFormat, .markdown)
            func finishRefinement() async throws {
                for _ in 0..<1000 {
                    if session.tableWidthRefinement.isIdle && session.tableWidthRefinement.started > 0 { break }
                    try await Task.sleep(nanoseconds: 20_000_000)
                }
                XCTAssertTrue(session.tableWidthRefinement.isIdle)
                XCTAssertNil(session.tableWidthRefinement.lastError)
            }
            if warm { try await finishRefinement() }
            let initial = try session.layoutExport()
            let cell = try XCTUnwrap(initial.tableCells.first { $0.row == 1 && $0.column == 0 })
            let originalFormatted = Array(try backend.formattedText().utf8)
            var point = ViemLayoutCaretPointV1()
            point.document_id = initial.info.identity.document_id
            point.document_revision = initial.info.identity.document_revision
            point.layout_revision = initial.info.identity.layout_revision
            point.text_offset = cell.text_start + 3
            point.affinity = UInt32(VIEM_BOUNDARY_AFFINITY_DOWNSTREAM)
            _ = try session.placeCursor(point, extendSelection: false)
            let startedBeforeMotion = session.tableWidthRefinement.started
            let noReplacement = NSRange(location: NSNotFound, length: 0)
            surface.editorView.insertText("$", replacementRange: noReplacement)

            func verifyCaret(_ stage: String, expectedOffset: UInt64) throws {
                let presentation = try session.presentation()
                let viewport = try session.viewportState()
                let snapshot = try session.layoutExport()
                let nearest = snapshot.carets.filter { abs(Int64($0.text_offset) - Int64(expectedOffset)) < 5 }
                let context = "warm=\(warm), \(stage), cursor=\(presentation.cursor_utf8_offset), affinity=\(presentation.cursor_affinity), viewport=\(viewport.left)..\(viewport.left + snapshot.info.viewport_width), max=\(viewport.maximum_left), clusters=\(snapshot.clusters.map(\.text_start).min() ?? 0)..\(snapshot.clusters.map(\.text_end).max() ?? 0), rows=\(snapshot.rows.map { "\($0.text_start)..\($0.text_end)" }), nearest carets=\(nearest.map { "\($0.text_offset)@\($0.x),affinity\($0.affinity)" })"
                XCTAssertEqual(presentation.cursor_utf8_offset, expectedOffset, context)
                do {
                    let geometry = try session.caretGeometry(offset: presentation.cursor_utf8_offset,
                        affinity: presentation.cursor_affinity, in: snapshot.info)
                    XCTAssertGreaterThanOrEqual(geometry.rect.x, viewport.left, context)
                    XCTAssertLessThanOrEqual(geometry.rect.x, viewport.left + snapshot.info.viewport_width, context)
                } catch { XCTFail("\(context): \(error)") }
                XCTAssertGreaterThan(viewport.left, 1000, context)
                XCTAssertEqual(surface.viewPresentation.cursor_utf8_offset, expectedOffset, context)
                XCTAssertNotEqual(surface.statusBarState.location, "Ln 1, Col 1", context)
                let drawn = try XCTUnwrap(surface.editorView.caretItemGeometry(snapshot).rect, context)
                XCTAssertTrue(surface.editorView.bounds.intersects(drawn), "Drawn caret \(drawn); \(context)")
            }

            try verifyCaret("after $", expectedOffset: cell.text_end - 1)
            surface.refreshPresentation()
            try verifyCaret("after refresh", expectedOffset: cell.text_end - 1)
            if warm {
                try await finishRefinement()
                try verifyCaret("after worker refinement", expectedOffset: cell.text_end - 1)
            } else {
                // Typing while a worker holds the pre-edit snapshot must
                // cancel that work and retain the new native insertion caret.
                for _ in 0..<100 {
                    if session.tableWidthRefinement.started > startedBeforeMotion { break }
                    try await Task.sleep(nanoseconds: 1_000_000)
                }
                XCTAssertGreaterThan(session.tableWidthRefinement.started, startedBeforeMotion)
                XCTAssertFalse(session.tableWidthRefinement.isIdle)
            }
            let discardedBeforeTyping = session.tableWidthRefinement.discarded
            XCTAssertEqual(try backend.serializedSource(typeName: EVDocument.markdownType), Data(text.utf8))
            surface.editorView.insertText("a", replacementRange: noReplacement)
            try verifyCaret("after a", expectedOffset: cell.text_end)
            surface.viewDidLayout()
            try verifyCaret("after native layout following a", expectedOffset: cell.text_end)
            surface.editorView.insertText("Z", replacementRange: noReplacement)
            try verifyCaret("after aZ", expectedOffset: cell.text_end + 1)
            surface.viewDidLayout()
            try verifyCaret("after native layout following aZ", expectedOffset: cell.text_end + 1)
            try await finishRefinement()
            try verifyCaret("after aZ worker refinement", expectedOffset: cell.text_end + 1)
            if !warm { XCTAssertGreaterThan(session.tableWidthRefinement.discarded, discardedBeforeTyping) }
            var expectedFormatted = originalFormatted
            expectedFormatted.insert(contentsOf: "Z".utf8, at: Int(cell.text_end))
            XCTAssertTrue(Array(try backend.formattedText().utf8) == expectedFormatted,
                "Typing must insert Z at the giant cell's end and preserve all other text; warm=\(warm)")
        }
    }

}
