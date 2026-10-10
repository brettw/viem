# Viem for Windows

A native C# / WinUI 3 shell over the same Rust core used by the Swift frontend.
The editor uses Win2D/DirectWrite for shaping and drawing, with native Windows
menus, file dialogs, controls, clipboard and input-method hosting.

## Git commit messages

The build script places `blocking-viem.exe` beside `Viem.exe`. Keep them and
all their adjacent runtime files together. Put that directory on `PATH`, then:

```powershell
git config --global core.editor 'blocking-viem'
```

Git supplies the filename. To try the launcher directly, run
`blocking-viem path/to/file`. Relative paths use the caller's working directory;
paths containing spaces must be quoted. The launcher starts Viem if necessary
or opens the file in its existing instance. It waits until **all panes and
windows showing that document have closed**, including copies opened during
editing. Saving alone does not release the wait. Reloading keeps it waiting;
closing unrelated files does not affect it.

Use `:wq` to save and close a view. Use `:cq` / `:cquit` to abort: like Vim,
this closes **all Viem windows without saving** and returns status 1 to every
pending blocking caller. `:7cq` or `:cq 7` specifies another exit status;
`!` is accepted and has no additional effect. Already saved changes stay saved.
Ordinary discard/`:q!` completes successfully using the file's existing disk
contents, so use `:cq` when Git should abort. Cancelling a close dialog keeps
waiting. Launch/open failure or a GUI crash returns failure.

macOS also ships a blocking helper inside its app bundle. See
[Blocking editor](../../docs/blocking-editor.md) for both platforms.

## Build and run

Prerequisites:

- Windows x64; development was verified on Windows 10 build 19045.
- Visual Studio 2026 with WinUI development, MSVC x64 tools, and Windows SDK
  10.0.26100.0.
- .NET SDK 10 and a current `x86_64-pc-windows-msvc` Rust toolchain.
- Python 3.9+ for packaging/verifying the bundled Vim runtime and for the C ABI
  declarations and icon tools.

From the repository root in PowerShell:

```powershell
.\scripts\build-win.ps1 -Configuration Release
.\scripts\run-win.ps1
.\scripts\build-win.ps1 -Configuration Debug -Run
.\scripts\test-win.ps1
.\scripts\test-win.ps1 -NativeAot
cargo test --locked
```

`run-win.ps1` only launches the existing Release build. Rebuild explicitly
with `build-win.ps1 -Configuration Release` after code changes; add `-Offline`
to use already-restored dependencies. `build-win.ps1 -Configuration Debug -Run`
builds and launches Debug. Both launch paths preserve the invoking PowerShell
directory as the application's working directory.

Release uses `dotnet publish` and NativeAOT compilation, including Viem's
WinUI/Win2D projections, to eliminate startup JIT compilation. The script
preserves the executable path below. A direct `dotnet build -c Release` still
produces a managed build; use the build script for startup measurements.
Ordinary Debug editor builds remain managed and debuggable. The blocking launcher
is published with NativeAOT in both configurations. The first online restore
also fetches the SDK's native compiler/runtime packs; later `-Offline` builds
reuse them. The pinned Windows SDK .NET projection supplies the C#/WinRT
runtime support required for Win2D's non-blittable font and line-metric arrays.

Use `test-win.ps1 -NativeAot` to publish the native UI diagnostics with optimized
C# and Rust code into `target/windows-native-tests`, including the blocking
launcher. This exercises NativeAOT behavior with the same native rendering and
UI checks as the managed tests. `-NoBuild -NativeAot` reuses that test build;
`-Optimized` alone retains the managed optimized test mode.

The executable is under
`target/windows/Viem.Windows/bin/x64/Debug/net10.0-windows10.0.26100.0/win-x64/Viem.exe`
(replace `Debug` with `Release` for that build). Keep its adjacent files together:
the output includes `viem_core.dll`, WinUI and Win2D dependencies, and
`Resources/vim` with the shared pinned syntax snapshot, license and provenance.
Both `dotnet build` and `dotnet publish` verify and replace that runtime subtree;
publishing with `--no-build` also packages it. No installed Vim is required.
Resource lookup uses the executable directory, so the app can be relocated or
launched from another working directory. Windows App SDK
is self-contained. The published Release editor and blocking launcher need no
separately installed .NET runtime; managed Debug editor builds use .NET 10.
It does not install file associations or an application package.

Both build and publish also copy [`assets/fonts`](../../assets/fonts/README.md)
to `Resources/fonts`, retaining the original fonts, licenses and attribution.
The font picker includes these app-local families and variants alongside system
fonts. Matching installed variable designs are reused after checking their
original identity and variable-font metadata; other designs continue to resolve
bundled files relative to the executable. Both paths retain the same portable
names, presets, and separate italic designs in rendering, previews, and whitespace
markers. No systemwide font installation is required.

The first build restores the locked Cargo/NuGet dependencies. After restoration,
`-Offline` uses cached dependencies and skips NuGet restore. If compiling native
Rust dependencies cannot find MSVC, run from a VS 2026 Developer PowerShell.
Open `Viem.Windows.csproj` in Visual Studio to debug the C# frontend, after
building the Rust library. All intermediate output stays under `target/windows`.

Examples:

```powershell
& $exe README.md
& $exe +123 first.md second.md
& $exe -o2 first.md second.md
```

Here `$exe` is the executable path above. Without `-o`, only the first file is
opened; `:next` and `:previous` navigate deferred arguments. A second launch
forwards its arguments and working directory to the existing process for the
same user/profile.

## Windows behavior

The title-bar menu button sits immediately left of the window controls.
It toggles the top menu bar and remembers its state. The editor, vertically
stacked panes, status bars and command prompt follow `docs/mac_references`.
Application windows, menus and dialogs use WinUI compact sizing for keyboard
and mouse use. Settings has View, Theme and Editing sidebar categories, with a
live Theme preview.
Document windows remember their last normal size and position, restore onto an
available monitor, and cascade additional windows. Maximized, minimized and
fullscreen bounds do not replace the saved normal frame.

- Ctrl+C/X/V always copy/cut/paste; Ctrl+Shift+V pastes plain text.
- Ctrl+Q retains Visual Block and literal-next input.
- Alt activates menu access keys and their native keytip badges. Alt+F, then O
  opens a file; fixed commands in each menu and submenu have their own keys.
  When the menu bar is hidden, Alt temporarily reveals it; Alt+F also opens File
  directly. Choosing a command, dismissing the menu, or leaving the window hides
  it again without changing the saved Show menu bar setting. AltGr remains text input.
- F8 opens or raises the style inspector outside literal-next input. Font
  pickers list sorted families and installed/bundled variants such as Light or Bold.
- The formatting toolbar provides Bold, Italic, and Strikethrough for Markdown;
  there is no Format menu. Fonts, colors, and paragraph properties remain
  available in the independent theme style inspector. Its title identifies the
  Code, Markdown, or Plain Text stylesheet and the selected theme.
- Markdown's right-aligned **Formatted view** toolbar toggle switches between
  Source and WYSIWYG while retaining source bytes. Its document/Aa icon is pressed
  for WYSIWYG and unpressed for Source. The last choice is saved as
  `editing.markdownFormattedView` in `config.json` and used for subsequent Markdown
  opens, including Ex commands. With no saved choice, Markdown opens in Source.
  Existing documents and recovered sessions retain their view.
  The status bar shows mode, the current file path, and messages on
  the left and cursor position on the right. Paths are relative to the working
  directory unless that would traverse the root or cross drives; narrow panes
  trim paths from the left while keeping the mode column a fixed width. Right-click
  the filename to copy its full path or its path relative to the working directory.
- View starts with Plain text, Markdown, and Code mode overrides. Code lists Auto,
  an Obscure languages flyout, a divider, and the primary languages. Each language
  group follows the shared catalogue in case-insensitive alphabetical order;
  every bundled choice remains available. Markdown code-block language pickers use
  the same groups, with None in place of Auto.
  Auto shows the detected language, keeps Markdown in literal Code, and uses
  Plain text when no language is detected. Checkmarks reflect the current mode
  and language choice. Mode switches preserve source bytes and the bound filename.
- Ctrl+S / Ctrl+Shift+S save / save as, outside literal-next input.
- File > Export… writes a styled HTML copy through the shared core exporter.
  Markdown exports formatted content and Code exports syntax-highlighted text.
  Export keeps the open document's filename, source, and unsaved state intact.
  HTML files open in Code; HTML interpretation and conversion modes are no
  longer offered in the File menu.
- Ctrl+Z / Ctrl+Shift+Z undo / redo, including from Insert mode, outside
  literal-next input. Normal-mode `u` and Ctrl+R remain available.
- Ctrl+= / Ctrl+- zoom in / out, including from Insert mode, outside literal-next
  input. These use the same zoom steps as the View menu and Ctrl+mouse wheel.
- Ctrl+0–5 select Base Paragraph / Headings 1–5. Heading 6 is menu-only to
  preserve vi's Ctrl+6 / Ctrl+^.
- Other vi control bindings remain available. Use the menus for native actions
  whose usual Windows shortcuts conflict with vi, including bold, italic,
  find and select all. AltGr and IME text go through native input.

Ex file-opening commands such as `:e`, `:sp`, and `:E` accept a directory: the
Open dialog starts there, and the selected file replaces the originating pane,
opens in a split, or opens in a new window respectively. Cancel leaves the
existing documents and layout unchanged.

Windows file commands and launch arguments expand `~`, `~/…`, and `~\…` to
the current user's profile directory. This includes open, write, Save As,
`:read`, `:source`, `:file`, and `:cd`. Other tildes are literal; use `./~` to
refer to a file or directory actually named `~` in the working directory.

Preferences, startup commands and style defaults use `%USERPROFILE%\.viem`.
On launch, Viem creates an empty `startup.viem` there if it is missing.
Set `VIEM_CONFIG_DIR` before launch for an isolated profile. Portable settings
use the same JSON schema as macOS; unknown nested keys survive updates and
invalid settings are not overwritten. Recovery uses separate owned swap files,
never autosaves over the original source, and retains crash leftovers for review.

The main Settings window has no Code category. From a Code view, F8 or Style > Edit Styles… opens the shared Code stylesheet in the ordinary modeless
style inspector. Filename associations remain supported through
`code.filenameAssociations` in `config.json`, and syntax load/compiler messages
remain available through the existing diagnostics mechanisms. Vim syntax always
uses application resources. Retired `code.vimSyntaxDirectory` values are
ignored and removed on the next successful settings write; unrelated fields
survive and resource paths are never saved.

The normative list of known Mac differences is in the **Windows frontend
requirements** section of [`AGENTS.md`](../../AGENTS.md). In particular,
printing, AppKit text services and Versions, full document accessibility,
per-font OpenType discovery are not claimed as implemented.

## Code boundaries and validation

- `Interop`: generated blittable C ABI structures, callbacks and explicit memory
  ownership. Regenerate with `python src/win/tools/generate_bindings.py` after a
  header change. Core and frontend must be rebuilt together.
- `Core`: checked C# wrappers for core snapshots, views, effects, styles,
  clipboard exports and composition. Editing semantics remain in Rust.
- `Rendering`: bounded contextual DirectWrite shaping, UTF-8/UTF-16 mapping,
  bidi geometry, font fallback and leased glyph resources.
- `Editor` / `Input`: viewport painting, caret/selection, native key and IME
  ingress, completion, command prompts and clipboard formats.
- `Shell`: windows, menus, settings, style inspector, file operations, recovery
  and same-user process handoff.
- `Diagnostics`: Debug-only tests running against the actual Rust DLL and
  WinUI/DirectWrite UI thread, plus opt-in Release startup tracing. No stand-in
  core or test compiler is involved.

`scripts/test-win.ps1` creates a unique profile under `target/windows-validation`.
Set `VIEM_TEST_ZOOM_ONLY=1` to run just native zoom shortcut checks.
Set `VIEM_TEST_MENU_KEYS_ONLY=1` to run just native menu access-key checks.
Set `VIEM_TEST_POINTER_ONLY=1` to run just native click mode preservation, click
jitter, subsequent keyboard input, and drag-selection checks.
Set `VIEM_TEST_TOOLBAR_ONLY=1` to run just the formatting-toolbar native controls,
visibility, selection/undo, formatted-view switching, overflow, and large-document latency checks.
The toolbar uses the active pane's core selection, the shared style catalogue,
and the same commands as the menus. Refreshes reuse exact-revision style exports
and native selector items; visibility uses the shared per-format
`formattingToolbar` configuration keys.
The formatted-view toggle stays pinned at the right edge while the formatting
controls scroll on narrow windows.
`scripts/test-win-vim-runtime.ps1` additionally checks build/publish inventories,
stale-file removal, native syntax paint after relocation into a path containing
spaces and non-ASCII characters, and editing/Tree-sitter with missing resources.
Use `-NoRustBuild` to reuse the Rust DLL while still exercising MSBuild packaging.
It checks native WinUI keyboard focus, text and command-key routing, status-line
prompt painting and command output (including hidden bars and scrolled documents), vi editing,
F8/inspector focus, compact controls, font variants and inheritance,
toolbar formatting actions, exclusive script controls, style font/color controls and
coalesced caret synchronization,
Unicode and bidi, composition, shared views, source/style
round trips, clipboard policy, atomic settings/save behavior, recovery ownership,
second-process launch handoff, window placement across sessions and monitor
changes, menu visibility and large-document layout/cache
behavior. It also writes screenshots of the real editor, split prompt, style
inspector and Settings. It does not overwrite the system clipboard or use the
normal application profile. Atomic file replacement must be permitted by the
host: the Codex workspace sandbox on this machine blocks `File.Replace`, so the
native integration harness was run outside that sandbox against isolated test data.

To profile selection with a local document, run
`scripts/test-win.ps1 -ProfileDocument AGENTS.md` (add `-NoBuild` to reuse the
Debug build). The isolated app loads a copy as Markdown and reports p50/p95 CPU
times for 100 deterministic drag updates, layout export and Win2D drawing,
plus shaping and drawing-cache rebuild counts. Drawing uses an offscreen
surface on the UI thread; these are CPU measurements, not display-present or
mouse-to-photon latency. The source file is never changed.

Add `-ProfileScenario resize` to measure 100 width changes through the same
core resize, presentation refresh, and drawing path. This reports CPU work for
reflow and rendering; it does not measure native window-manager presentation.
The default scenario is `drag`.

To measure empty-editor startup, run `scripts/test-win-startup.ps1` (add
`-NoBuild` to reuse the Release build). It launches three separate processes
with isolated empty profiles, checks that startup loads no font-picker lists,
and writes JSON traces under `target/windows-validation`. Targeted face metadata
for the active theme is allowed even for an empty document and is reported along
with installed-design and bundled-file counts.
`-Runs` changes the launch count. Timings end at the first editor draw callback;
they measure elapsed startup time, not physical display presentation or a cold boot.
Use `-ProfileDocument <path>` for a real command-line file open, `-ConfigFile
<path>` to copy settings into each isolated profile, and `-Executable <path>`
to compare a preserved build. First launches of new/rebuilt output should be
reported separately from repeated launches. Traces include framework, shell,
document and layout phases plus JIT CPU time; overlapping scopes are not additive.
`-ProfileDirectory <path>` copies that profile's settings, startup commands and
style defaults into each isolated test profile, leaving the original untouched.
Normal launches do not collect traces. Font discovery uses cached, indexed
lookups for requested families/faces; complete family lists load on picker use.
Font indexing overlaps WinUI initialization. Hidden formatting toolbars are
created only when shown; visible Markdown reserves their space before the first
editor layout. Primary Code language controls are created when View first opens;
Obscure languages controls are deferred until Code first opens. Both paths use
native row loading, including keyboard, access-key and UI Automation entry.
`VIEM_TEST_TOOLBAR_ONLY=startup` selects the focused native startup/toolbar checks;
use `VIEM_TEST_MENU_KEYS_ONLY=1` for access-key and window-focus coverage.
Set `VIEM_FORCE_BUNDLED_FONTS=1` to exercise the bundled fallback even when matching
fonts are installed. This works in Release for startup comparisons and with
`VIEM_TEST_STYLES_ONLY=variable` for the native font/preset/inspector regressions.
Run both modes in fresh processes, preserve matching settings and document bytes,
and include the installed-font check in measurements of the optimized path.
Keep local measurement output under `target/windows-validation`; record the
build, settings, document bytes and viewport when comparing runs. See the
[measurement guide](../../docs/performance.md) for scope and interpretation.

Regenerate `Assets/Viem.ico` with `python src/win/tools/build_icon.py` when the
existing iconset changes. The script only packages those PNGs into an ICO;
it does not redraw or resample them.
