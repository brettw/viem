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
an alias for `make run-release`. After building, all run targets check for an
existing Viem process belonging to the current user and fail if one is running.
Quit Viem and rerun the command to use the newly built executable. A failed
process check also prevents launch.
The targets open `docs/markdown_demo.md` in the bundled executable, in the
background with make's working directory. Normally this is the invoking shell's
directory; `make -C` uses the directory selected by `-C`.
`scripts/run-mac-app.sh [debug|release]` also builds and launches in the background,
with the same process check and demo, preserving the invoking shell's directory
even when called from another directory.

Build the bundle without launching it:

```sh
make debug
make release
```

Both targets build the Rust core and Swift frontend, then assemble and locally
sign `.build/Viem.app`. `make debug` (also the default for `make`) builds with
debug information; `make release` builds an optimized application. Each replaces
the same application bundle.

Both configurations explicitly build `Viem` and `blocking-viem`. Release builds
also package `.build/release-app/Viem.app`, which remains a release build after
later debug builds. `make run` and `make run-release` build and launch this release
bundle. Use its `Contents/MacOS/blocking-viem` for a release-only editor redirector;
the complete bundle supplies the GUI's syntax runtime, fonts, and themes.

The bundle also includes `Contents/MacOS/blocking-viem`, a command-line editor
launcher that waits for its requested document to close. See
[Blocking editor](blocking-editor.md) for Git setup and cancellation behavior.

The bundle includes the shared desktop fonts and their attribution files from
[`assets/fonts`](../assets/fonts/README.md) in `Contents/Resources/fonts`.
Startup uses installed fonts with matching face names and registers bundled
files for Viem's process when any of their faces are missing, before opening
font pickers or shaping text. They are available without systemwide
installation; saved styles retain ordinary family and face names.

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
scripts/test-mac.sh
```

The native test script checks the C header against the Rust ABI, builds the
current app, and runs the Swift tests with isolated settings. Run `make check-abi`
for the header check alone. See [C ABI validation](abi-validation.md) for Windows
setup and CI usage. Ordinary Rust tests do not run the C compiler check.

Native tests link the separate `native-test` Rust profile, which inherits the
Rust test suite's optimization while retaining debug assertions and overflow
checks. The first run builds this archive;
subsequent runs reuse it. Normal `make debug` builds remain unoptimized.
The native-test archive keeps source-line backtraces but omits Rust variable/type
debug information to reduce native relink work. Use an ordinary debug build when
inspecting Rust locals in a debugger; Swift debug information is unchanged.
The native-test configuration also optimizes only `ViemCoreTextProvider`, whose
shaping loops dominate large native fixtures. Explicit Debug assertion settings
retain Swift assertions and checked arithmetic; testability, `DEBUG`, and debug
symbols remain enabled. Other Swift targets and all test bodies stay unoptimized.
Ordinary debug builds keep the provider unoptimized too. Switching between these
configurations may rebuild the provider; repeated native runs reuse it.
The app builder fingerprints the Rust archive and linked executable, so an
unchanged build avoids relinking. Changed archive bytes or profile selections
still force a relink; the stamp is published only after a successful build.
The test command wakes the display and temporarily prevents display/system idle
sleep, plus system sleep while on AC, so native UI and font-service waits do not
stall an unattended run. Saved power settings are unchanged.
The test bundle also disables automatic AppKit window animations in its own
volatile preference domain. This avoids blocked animation workers starving
file I/O in the test runner, without changing saved preferences or app behavior.
It holds a user-initiated process activity while running so App Nap does not
throttle tests when their windows are behind other applications.

The Swift package is divided into the same ownership boundaries as the source:

- `CViemCore` exposes the C ABI;
- `ViemCoreTextProvider` implements shaping and owns native render handles;
- `ViemEditor` adapts core document/view state to the custom AppKit surface;
- `ViemAppShell` owns documents, windows, menus, and application lifecycle; and
- `Viem` is the small composition root.

The AppKit view is a renderer and input adapter. It must not become a second
text store, selection model, or undo authority.

## Startup commands

Viem creates an empty `~/.viem/startup.viem` on launch if it is missing. Edit it
to configure mappings and supported Ex settings:

```vim
" Yank from the cursor through the current line end (using $ semantics).
map Y y$

" Control-F2 prepares :sp; include <CR> to split immediately.
map <C-F2> :sp
map <C-F3> :sp<CR>

set nowrap
set hlsearch incsearch
```

Restart Viem after editing the file. Startup commands apply to every document
and new view. `VIEM_CONFIG_DIR` overrides the profile directory for `config.json`,
the `themes` directory, and `startup.viem` together; native code resolves it through
`EVProfileDirectory`. The startup file uses UTF-8, accepts a BOM and
CRLF, and is limited to 1 MiB. Invalid lines report their path and line number
without preventing later valid settings from loading.

`incsearch` is enabled by default and highlights all matches while typing a
search, even when `hlsearch` is off. `set noincsearch` disables this preview.
Accepting or cancelling restores normal `hlsearch` behavior: highlight the saved
search pattern when enabled, or show no highlights when disabled. `hlsearch`
defaults off. `:noh` clears saved-pattern highlights until the next
search. Customize their internal **Incremental match** character style in the
Style Editor; its explicit properties overlay the text's existing formatting.
The default changes only the background, and the style is absent from style
application menus.

Use `noremap` to prevent recursive expansion, or mode-specific forms such as
`nnoremap` and `imap`. Multi-key mappings wait up to one second for another key.
Interactive mapping commands affect the current buffer and its views. Startup
accepts configuration commands only; document/window actions belong in mapping
replacements. Mapping listing, mapping attributes, Vimscript, and startup
assignments to `fileformat` or `fileformats` are not supported.
