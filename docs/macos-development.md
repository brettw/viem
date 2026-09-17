# macOS frontend development

The native frontend is an AppKit application backed by the Rust core through
`include/viem_core.h`. Swift Package Manager builds the frontend modules and
their tests; the helper script assembles the executable and resources into a
locally signed application bundle.

The Rust APIs and C ABI are internal to this repository and have no backward
compatibility requirement. Change the core, C declarations, Swift callers,
providers, and tests together; remove obsolete entry points rather than keeping
compatibility wrappers. Rebuild both core and frontend after an ABI change.
Persisted document formats and editing behavior have separate requirements in
`AGENTS.md` and are not covered by this API policy.

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

## Startup commands

Create `~/.viem/startup.viem` to configure mappings and supported Ex settings:

```vim
" Yank from the cursor through the current line end (using $ semantics).
map Y y$

" Control-F2 prepares :sp; include <CR> to split immediately.
map <C-F2> :sp
map <C-F3> :sp<CR>

set nowrap
```

Restart Viem after editing the file. Startup commands apply to every document
and new view. `VIEM_CONFIG_DIR` overrides the profile directory for `config.json`,
`code_style.json`, and `startup.viem` together; native code resolves it through
`EVProfileDirectory`. The optional startup file uses UTF-8, accepts a BOM and
CRLF, and is limited to 1 MiB. Invalid lines report their path and line number
without preventing later valid settings from loading.

Use `noremap` to prevent recursive expansion, or mode-specific forms such as
`nnoremap` and `imap`. Multi-key mappings wait up to one second for another key.
Interactive mapping commands affect the current buffer and its views. Startup
accepts configuration commands only; document/window actions belong in mapping
replacements. Mapping listing, mapping attributes, Vimscript, and startup
assignments to `fileformat` or `fileformats` are not supported.
