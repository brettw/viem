# Viem for Windows

A native C# / WinUI 3 shell over the same Rust core used by the Swift frontend.
The editor uses Win2D/DirectWrite for shaping and drawing, with native Windows
menus, file dialogs, controls, clipboard and input-method hosting.

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
.\scripts\run_windows.ps1
.\scripts\build-win.ps1 -Configuration Debug -Run
.\scripts\test-win.ps1
cargo test --locked
```

`run_windows.ps1` only launches the existing Release build. Rebuild explicitly
with `build-win.ps1 -Configuration Release` after code changes; add `-Offline`
to use already-restored dependencies. `build-win.ps1 -Configuration Debug -Run`
builds and launches Debug.

The executable is under
`target/windows/Viem.Windows/bin/x64/Debug/net10.0-windows10.0.26100.0/win-x64/Viem.exe`
(replace `Debug` with `Release` for that build). Keep its adjacent files together:
the output includes `viem_core.dll`, WinUI and Win2D dependencies, and
`Resources/vim` with the shared pinned syntax snapshot, license and provenance.
Both `dotnet build` and `dotnet publish` verify and replace that runtime subtree;
publishing with `--no-build` also packages it. No installed Vim is required.
Resource lookup uses the executable directory, so the app can be relocated or
launched from another working directory. Windows App SDK
is self-contained; this development build uses the installed .NET 10 runtime.
It does not install file associations or an application package.

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
and mouse use. Settings uses sidebar categories with a live Theme preview.
Document windows remember their last normal size and position, restore onto an
available monitor, and cascade additional windows. Maximized, minimized and
fullscreen bounds do not replace the saved normal frame.

- Ctrl+C/X/V always copy/cut/paste; Ctrl+Shift+V pastes plain text.
- Ctrl+Q retains Visual Block and literal-next input.
- F8 opens or raises the style inspector outside literal-next input. Font
  pickers list sorted families and installed variants such as Light or Bold.
- Ctrl+S / Ctrl+Shift+S save / save as, outside literal-next input.
- Ctrl+Z / Ctrl+Shift+Z undo / redo, including from Insert mode, outside
  literal-next input. Normal-mode `u` and Ctrl+R remain available.
- Ctrl+0–5 select Base Paragraph / Headings 1–5. Heading 6 is menu-only to
  preserve vi's Ctrl+6 / Ctrl+^.
- Other vi control bindings remain available. Use the menus for native actions
  whose usual Windows shortcuts conflict with vi, including bold, italic,
  underline, find and select all. AltGr and IME text go through native input.

Preferences, startup commands and style defaults use `%USERPROFILE%\.viem`.
Set `VIEM_CONFIG_DIR` before launch for an isolated profile. Portable settings
use the same JSON schema as macOS; unknown nested keys survive updates and
invalid settings are not overwritten. Recovery uses separate owned swap files,
never autosaves over the original source, and retains crash leftovers for review.

Code settings provides Edit Code Styles, filename associations, and syntax load
diagnostics. Vim syntax always uses application resources. Retired
`code.vimSyntaxDirectory` values are ignored and removed on the next successful
settings write; unrelated fields survive and resource paths are never saved.

The normative list of known Mac differences is in the **Windows frontend
requirements** section of [`AGENTS.md`](../../AGENTS.md). In particular,
printing, AppKit text services and Versions, full document accessibility,
advanced typography controls and outgoing RTF are not claimed as implemented.

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
`scripts/test-win-vim-runtime.ps1` additionally checks build/publish inventories,
stale-file removal, native syntax paint after relocation into a path containing
spaces and non-ASCII characters, and editing/Tree-sitter with missing resources.
Use `-NoRustBuild` to reuse the Rust DLL while still exercising MSBuild packaging.
It checks native WinUI keyboard focus, text and command-key routing, status-line
prompt painting and command output (including hidden bars and scrolled documents), vi editing,
F8/inspector focus, compact controls, font variants and inheritance,
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
with isolated empty profiles, checks that startup loads no font-picker lists
or face descriptions, and writes JSON traces under `target/windows-validation`.
`-Runs` changes the launch count. Timings end at the first editor draw callback;
they measure elapsed startup time, not physical display presentation or a cold boot.
Normal launches do not collect traces. Font discovery uses cached, indexed
lookups for requested families/faces; complete family lists load on picker use.

Regenerate `Assets/Viem.ico` with `python src/win/tools/build_icon.py` when the
existing iconset changes. The script only packages those PNGs into an ICO;
it does not redraw or resample them.
