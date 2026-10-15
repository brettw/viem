import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVCodeStyleLayoutTests: XCTestCase {

    private func configuration() throws -> EVConfigurationStore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-code-style-layout-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let configuration = EVConfigurationStore(directory: directory, legacyDefaults: nil)
        try EVStyleTestFixtures.configure(configuration, format: .code, declarations: [
            (.baseParagraph, .characterSize, .float(14)),
            (.baseParagraph, .characterFontFamilies, .stringList(["system-ui"])),
            (.baseParagraph, .characterFontAxes, .string("{}")),
            (.baseParagraph, .characterWeight, .unsigned(400)),
            (.baseParagraph, .paragraphLineSpacing, .lineSpacing(EVLineSpacing(kind: UInt32(VIEM_STYLE_LINE_SPACING_NORMAL), value: 0))),
            (EVStyleTestFixtures.character("syntax:Keyword"), .characterSize, .float(14)),
            (EVStyleTestFixtures.character("syntax:Keyword"), .characterFontAxes, .string("{}")),
            (EVStyleTestFixtures.character("syntax:Function"), .characterSize, .float(14)),
        ])
        return configuration
    }

}

