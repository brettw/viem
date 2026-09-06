// swift-tools-version: 6.2

import Foundation
import PackageDescription

let packageRoot = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .path

let rustProfile = ProcessInfo.processInfo.environment["EVIM_RUST_PROFILE"] ?? "debug"
let rustArchive = "\(packageRoot)/target/\(rustProfile)/libevim_core.a"

let package = Package(
    name: "eVim",
    platforms: [
        .macOS("26.0"),
    ],
    products: [
        .executable(name: "eVim", targets: ["eVim"]),
        .library(name: "EvimAppShell", targets: ["EvimAppShell"]),
        .library(name: "EvimCoreTextProvider", targets: ["EvimCoreTextProvider"]),
    ],
    targets: [
        .target(
            name: "CEvimCore",
            path: "src/mac/CEvimCore",
            publicHeadersPath: "include",
            linkerSettings: [
                .unsafeFlags([rustArchive]),
            ]
        ),
        .target(
            name: "EvimCoreTextProvider",
            dependencies: ["CEvimCore"],
            path: "src/mac/CoreTextProvider/Sources",
            linkerSettings: [
                .linkedFramework("AppKit"),
                .linkedFramework("CoreGraphics"),
                .linkedFramework("CoreText"),
            ]
        ),
        .target(
            name: "EvimAppShell",
            path: "src/mac/AppShell",
            linkerSettings: [
                .linkedFramework("AppKit"),
            ]
        ),
        .target(
            name: "EvimEditor",
            dependencies: ["CEvimCore", "EvimCoreTextProvider", "EvimAppShell"],
            path: "src/mac/Editor/Sources",
            linkerSettings: [
                .linkedFramework("AppKit"),
            ]
        ),
        .executableTarget(
            name: "eVim",
            dependencies: [
                "EvimAppShell",
                "EvimCoreTextProvider",
                "EvimEditor",
            ],
            path: "src/mac/App",
            exclude: ["Resources"]
        ),
        .testTarget(
            name: "EvimAppShellTests",
            dependencies: ["EvimAppShell"],
            path: "src/mac/AppShellTests"
        ),
        .testTarget(
            name: "EvimCoreTextProviderTests",
            dependencies: ["EvimCoreTextProvider", "CEvimCore"],
            path: "src/mac/CoreTextProvider/Tests"
        ),
        .testTarget(
            name: "EvimEditorTests",
            dependencies: ["EvimEditor", "CEvimCore", "EvimCoreTextProvider"],
            path: "src/mac/Editor/Tests"
        ),
    ],
    swiftLanguageModes: [.v5]
)
