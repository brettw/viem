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
make run-debug
```

`make run-release` builds and launches an optimized application; `make run` is
an alias for `make run-release`. All run targets launch a fresh instance of
`.build/Viem.app`, even if another instance is already running.

Build the bundle without launching it:

```sh
make debug
make release
```

Both targets build the Rust core and Swift frontend, then assemble and locally
sign `.build/Viem.app`. `make debug` (also the default for `make`) builds with
debug information; `make release` builds an optimized application. Each replaces
the same application bundle.

After moving or renaming the checkout, remove cached build artifacts before
rebuilding. Swift and Clang precompiled modules can retain absolute source paths:

```sh
make clean
make debug
```

`make clean` removes the repository's `.build` and `target` directories, including
the application bundle, Swift/Clang module caches, and Rust build artifacts.

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
