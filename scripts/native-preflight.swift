import AppKit
import CoreText
import Foundation

func require(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() {
        FileHandle.standardError.write(Data((message + "\n").utf8))
        exit(1)
    }
}
let application = NSApplication.shared
require(!NSScreen.screens.isEmpty, "Native preflight: no usable screen")
let pasteboard = NSPasteboard(name: .init("viem-preflight-\(UUID().uuidString)"))
defer { pasteboard.releaseGlobally() }
require(pasteboard.setString("probe", forType: .string), "Native preflight: private clipboard write failed")
require(pasteboard.string(forType: .string) == "probe", "Native preflight: private clipboard read failed")
let font = NSFont.systemFont(ofSize: 12)
let line = CTLineCreateWithAttributedString(NSAttributedString(string: "probe", attributes: [.font: font]))
require(CTLineGetTypographicBounds(line, nil, nil, nil) > 0, "Native preflight: font service unavailable")
print("Native capabilities available: screen, private clipboard, fonts")
