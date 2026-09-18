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
- Python 3 only for verifying/regenerating the C ABI declarations and icon.

From the repository root in PowerShell:

```powershell
.\scripts\build-win.ps1 -Run
.\scripts\build-win.ps1 -Configuration Release
.\scripts\test-win.ps1
cargo test --locked
```

The executable is under
`target/windows/Viem.Windows/bin/x64/Debug/net10.0-windows10.0.26100.0/win-x64/Viem.exe`
(replace `Debug` with `Release` for that build). Keep its adjacent files together:
the output includes `viem_core.dll`, WinUI and Win2D dependencies. Windows App SDK
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

- Ctrl+C/X/V always copy/cut/paste; Ctrl+Shift+V pastes plain text.
- Ctrl+Q retains Visual Block and literal-next input.
- Ctrl+S / Ctrl+Shift+S save / save as, outside literal-next input.
- Ctrl+0–5 select Base Paragraph / Headings 1–5. Heading 6 is menu-only to
  preserve vi's Ctrl+6 / Ctrl+^.
- Other vi control bindings remain available. Use the menus for native actions
  whose usual Windows shortcuts conflict with vi, including bold, italic,
  underline, find, select all and undo. AltGr and IME text go through native input.

Preferences, startup commands and style defaults use `%USERPROFILE%\.viem`.
Set `VIEM_CONFIG_DIR` before launch for an isolated profile. Portable settings
use the same JSON schema as macOS; unknown nested keys survive updates and
invalid settings are not overwritten. Recovery uses separate owned swap files,
never autosaves over the original source, and retains crash leftovers for review.

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
  WinUI/DirectWrite UI thread. No stand-in core or test compiler is involved.

`scripts/test-win.ps1` creates a unique profile under `target/windows-validation`.
It checks native WinUI keyboard focus, text and command-key routing, vi editing,
Unicode and bidi, composition, shared views, source/style
round trips, clipboard policy, atomic settings/save behavior, recovery ownership,
second-process launch handoff, menu visibility and large-document layout/cache
behavior. It also writes screenshots of the real editor, split prompt, style
inspector and Settings. It does not overwrite the system clipboard or use the
normal application profile. Atomic file replacement must be permitted by the
host: the Codex workspace sandbox on this machine blocks `File.Replace`, so the
native integration harness was run outside that sandbox against isolated test data.

Regenerate `Assets/Viem.ico` with `python src/win/tools/build_icon.py` when the
existing iconset changes. The script only packages those PNGs into an ICO;
it does not redraw or resample them.
