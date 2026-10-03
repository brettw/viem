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
        .executable(name: "blocking-viem", targets: ["BlockingViem"]),
        .library(name: "ViemAppShell", targets: ["ViemAppShell"]),
        .library(name: "ViemCoreTextProvider", targets: ["ViemCoreTextProvider"]),
    ],
    targets: [
        .target(name: "ViemBlockingTransport", path: "src/mac/BlockingTransport"),
        .executableTarget(
            name: "BlockingViem",
            dependencies: ["ViemBlockingTransport"],
            path: "src/mac/Blocking"
        ),
        .testTarget(
            name: "ViemBlockingTransportTests",
            dependencies: ["ViemBlockingTransport"],
            path: "src/mac/BlockingTransportTests"
        ),
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
            // Native layout tests spend most of their shaping time in Swift
            // collection/hash loops. Optimize this provider while retaining
            // debug assertions, testability and symbols. Other debug targets,
            // including every test body, keep their normal build settings.
            swiftSettings: rustProfile == "native-test" ? [
                .unsafeFlags(["-O", "-assert-config", "Debug"], .when(configuration: .debug)),
            ] : [],
            linkerSettings: [
                .linkedFramework("AppKit"),
                .linkedFramework("CoreGraphics"),
                .linkedFramework("CoreText"),
            ]
        ),
        .target(
            name: "ViemAppShell",
            dependencies: ["CViemCore", "ViemBlockingTransport"],
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
        .target(
            name: "ViemNativeTestSupport",
            path: "src/mac/TestSupport",
            publicHeadersPath: "include"
        ),
        .testTarget(
            name: "ViemAppShellTests",
            dependencies: ["ViemAppShell", "ViemBlockingTransport", "ViemNativeTestSupport"],
            path: "src/mac/AppShellTests"
        ),
        .testTarget(
            name: "ViemCoreTextProviderTests",
            dependencies: ["ViemCoreTextProvider", "CViemCore", "ViemNativeTestSupport"],
            path: "src/mac/CoreTextProvider/Tests"
        ),
        .testTarget(
            name: "ViemEditorTests",
            dependencies: ["ViemEditor", "CViemCore", "ViemCoreTextProvider", "ViemBlockingTransport", "ViemNativeTestSupport"],
            path: "src/mac/Editor/Tests"
        ),
    ],
    swiftLanguageModes: [.v5]
)
