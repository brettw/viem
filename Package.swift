// swift-tools-version: 6.2

import Foundation
import PackageDescription

let packageRoot = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .path

let rustProfile = ProcessInfo.processInfo.environment["VIEM_RUST_PROFILE"] ?? "debug"
let rustArchive = "\(packageRoot)/target/\(rustProfile)/libviem_core.a"

let package = Package(
    name: "Viem",
    platforms: [
        .macOS("26.0"),
    ],
    products: [
        .executable(name: "Viem", targets: ["Viem"]),
        .library(name: "ViemAppShell", targets: ["ViemAppShell"]),
        .library(name: "ViemCoreTextProvider", targets: ["ViemCoreTextProvider"]),
    ],
    targets: [
        .target(
            name: "CViemCore",
            path: "src/mac/CViemCore",
            publicHeadersPath: "include",
            linkerSettings: [
                .unsafeFlags([rustArchive]),
            ]
        ),
        .target(
            name: "ViemCoreTextProvider",
            dependencies: ["CViemCore"],
            path: "src/mac/CoreTextProvider/Sources",
            linkerSettings: [
                .linkedFramework("AppKit"),
                .linkedFramework("CoreGraphics"),
                .linkedFramework("CoreText"),
            ]
        ),
        .target(
            name: "ViemAppShell",
            path: "src/mac/AppShell",
            linkerSettings: [
                .linkedFramework("AppKit"),
            ]
        ),
        .target(
            name: "ViemEditor",
            dependencies: ["CViemCore", "ViemCoreTextProvider", "ViemAppShell"],
            path: "src/mac/Editor/Sources",
            linkerSettings: [
                .linkedFramework("AppKit"),
            ]
        ),
        .executableTarget(
            name: "Viem",
            dependencies: [
                "ViemAppShell",
                "ViemCoreTextProvider",
                "ViemEditor",
            ],
            path: "src/mac/App",
            exclude: ["Resources"]
        ),
        .testTarget(
            name: "ViemAppShellTests",
            dependencies: ["ViemAppShell"],
            path: "src/mac/AppShellTests"
        ),
        .testTarget(
            name: "ViemCoreTextProviderTests",
            dependencies: ["ViemCoreTextProvider", "CViemCore"],
            path: "src/mac/CoreTextProvider/Tests"
        ),
        .testTarget(
            name: "ViemEditorTests",
            dependencies: ["ViemEditor", "CViemCore", "ViemCoreTextProvider"],
            path: "src/mac/Editor/Tests"
        ),
    ],
    swiftLanguageModes: [.v5]
)
