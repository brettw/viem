# macOS frontend development

The native frontend is an AppKit application backed by the Rust core through
`include/viem_core.h`. Swift Package Manager builds the frontend modules and
their tests; the helper script assembles the executable and resources into a
locally signed application bundle.

Requirements:

- macOS 26 or later;
- the Xcode command-line tools with Swift 6.2 or later; and
- a Rust toolchain capable of building the workspace.

Build and launch a debug application:

```sh
./scripts/run-mac-app.sh debug
```

Build the bundle without launching it:

```sh
./scripts/build-mac-app.sh debug
```

The resulting application is `.build/Viem.app`. Use `release` in place of
`debug` for an optimized local build.

Run the portable and native test suites independently:

```sh
cargo test --all-targets
swift test --disable-sandbox
```

The Swift package is divided into the same ownership boundaries as the source:

- `CViemCore` exposes the C ABI;
- `ViemCoreTextProvider` implements shaping and owns native render handles;
- `ViemEditor` adapts core document/view state to the custom AppKit surface;
- `ViemAppShell` owns documents, windows, menus, and application lifecycle; and
- `Viem` is the small composition root.

The AppKit view is a renderer and input adapter. It must not become a second
text store, selection model, or undo authority.
