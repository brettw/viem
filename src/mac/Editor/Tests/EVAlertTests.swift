import AppKit
import CViemCore
import XCTest
@testable import ViemAppShell
@testable import ViemEditor

@MainActor
final class EVAlertTests: XCTestCase {
    private func surface(_ source: String) throws -> (EVCoreDocumentBackend, EVEditorSurfaceController) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-alert-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let backend = EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
        try backend.readUntitled(source: Data(source.utf8), typeName: EVDocument.markdownType)
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        surface.view.frame = NSRect(x: 0, y: 0, width: 600, height: 400)
        surface.viewDidLayout()
        return (backend, surface)
    }

    private func source(_ backend: EVCoreDocumentBackend) throws -> String {
        String(decoding: try backend.serializedSource(typeName: EVDocument.markdownType), as: UTF8.self)
    }

    func testToolbarMenuAndInsertCommandsSetSwitchAndClearAlertKindsWithExactUndo() throws {
        let (backend, surface) = try surface("Words")
        let toolbar = surface.formattingToolbar
        toolbar.refresh()
        XCTAssertTrue(toolbar.blockQuoteAlerts.isEnabled)
        let menu = toolbar.makeAlertMenu()
        XCTAssertEqual(menu.items.map(\.title), ["Note", "Tip", "Important", "Warning", "Caution"])
        XCTAssertTrue(menu.items.allSatisfy { $0.image != nil && $0.isEnabled && $0.state == .off })

        toolbar.chooseAlert(UInt32(VIEM_ALERT_NOTE))
        XCTAssertNil(surface.commandOutput)
        XCTAssertEqual(try source(backend), "> [!NOTE]\n> Words")
        XCTAssertEqual(toolbar.blockQuote.state, .on)
        XCTAssertEqual(toolbar.makeAlertMenu().items.map(\.state), [.on, .off, .off, .off, .off])
        XCTAssertEqual(surface.presentation(for: .alertNote).state, .on)

        surface.perform(menuCommand: .alertWarning, sender: nil)
        XCTAssertEqual(try source(backend), "> [!WARNING]\n> Words")
        // Choosing the current kind again leaves a plain quote.
        surface.perform(menuCommand: .alertWarning, sender: nil)
        XCTAssertEqual(try source(backend), "> Words")
        XCTAssertEqual(toolbar.blockQuote.state, .on)
        toolbar.chooseAlert(UInt32(VIEM_ALERT_TIP))
        // The quote toggle removes the quote and its marker together.
        toolbar.blockQuote.performClick(nil)
        XCTAssertEqual(try source(backend), "Words")
        for expected in ["> [!TIP]\n> Words", "> Words", "> [!WARNING]\n> Words", "> [!NOTE]\n> Words", "Words"] {
            surface.perform(menuCommand: .undo, sender: nil)
            XCTAssertEqual(try source(backend), expected)
        }
    }

    func testAlertKindsAreUnavailableInsideLists() throws {
        let (_, surface) = try surface("- Item")
        surface.formattingToolbar.refresh()
        XCTAssertFalse(surface.formattingToolbar.blockQuoteAlerts.isEnabled)
        XCTAssertFalse(surface.presentation(for: .alertNote).isEnabled)
    }

    func testEveryIconDrawsInsideItsSquareInTheRequestedColor() throws {
        let scale: CGFloat = 2, size: CGFloat = 16, padding: CGFloat = 4
        let pixels = Int((size + padding * 2) * scale)
        for kind in [VIEM_ALERT_NOTE, VIEM_ALERT_TIP, VIEM_ALERT_IMPORTANT, VIEM_ALERT_WARNING, VIEM_ALERT_CAUTION].map({ UInt32($0) }) {
            let bitmap = try XCTUnwrap(CGContext(data: nil, width: pixels, height: pixels, bitsPerComponent: 8,
                bytesPerRow: pixels * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue))
            bitmap.translateBy(x: 0, y: CGFloat(pixels))
            bitmap.scaleBy(x: scale, y: -scale)
            let red = CGColor(srgbRed: 1, green: 0, blue: 0, alpha: 1)
            EVAlertDrawing.drawIcon(kind: kind, in: NSRect(x: padding, y: padding, width: size, height: size), color: red, context: bitmap)
            let bytes = try XCTUnwrap(bitmap.data).assumingMemoryBound(to: UInt8.self)
            var painted = 0, outside = 0
            for y in 0..<pixels {
                for x in 0..<pixels where bytes[(y * pixels + x) * 4 + 3] > 0 {
                    let offset = (y * pixels + x) * 4
                    XCTAssertEqual(bytes[offset + 1], 0, "kind \(kind) paints only the requested color")
                    let inside = (CGFloat(x) / scale, CGFloat(y) / scale)
                    if inside.0 < padding - 1 || inside.0 > padding + size + 1 || inside.1 < padding - 1 || inside.1 > padding + size + 1 { outside += 1 }
                    painted += 1
                }
            }
            XCTAssertGreaterThan(painted, 40, "kind \(kind)")
            XCTAssertEqual(outside, 0, "kind \(kind)")
        }
    }
}
