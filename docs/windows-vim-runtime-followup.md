# Windows bundled Vim runtime

Windows uses the same pinned MacVim 9.1.1887 snapshot as macOS:
`assets/vim/runtime/syntax`. The runtime bytes and `manifest.json` are unchanged.
Git attributes retain original encodings and line endings on Windows.

## Packaging and discovery

`Viem.Windows.csproj` verifies the source before building and invokes
`scripts/vim-runtime.py package` after both Build and Publish. This covers
`scripts/build-win.ps1`, direct MSBuild/Visual Studio builds, and
`dotnet publish --no-build`. Python 3.9+ and its standard library are sufficient;
`VimRuntimePython` may select the Python executable for MSBuild.
The publish inventory also retains compiled WinUI XAML and the application PRI
resource index; the SDK previously copied these only to build output.

The packager checks the source inventory and hashes before replacing only
`Resources/vim` under the explicitly supplied output directory, then verifies
its copy. It preserves nested helpers, LICENSE, README.md and manifest.json;
obsolete runtime files disappear while adjacent application resources remain.
Redirected destinations and overlapping source/output trees are rejected before
removal. No build reads an installed Vim or downloads a runtime.

`BundledVimRuntime` resolves `Resources/vim/runtime/syntax` from
`AppContext.BaseDirectory`. Every `CoreDocument` passes that UTF-8 path through
`viem_core_configure_syntax` before language detection and view creation. The
core remains platform-neutral. Preference updates do not reconfigure syntax.
Relocation and the caller's working directory do not change resource selection.

## Configuration and fallback

The main Settings window has no Code category. The shared Code stylesheet remains
editable through the ordinary modeless style inspector opened from a Code view
by F8 or the menu. Filename associations remain supported in
`code.filenameAssociations` in `config.json`. The directory field and preference
override have been removed. Legacy `code.vimSyntaxDirectory` values of any JSON
type are ignored on read and removed on the next successful write, preserving
unknown fields and associations. Resolved application paths are never saved.

An unavailable resource tree and individual syntax load/compiler failures are
reported through the existing diagnostic API. Editing and bundled Tree-sitter
remain available. Includes remain confined by the portable
loader. Shipping a syntax file does not expand the native compiler's supported
Vim subset or suppress its diagnostics.

## Reproducible validation

```powershell
python -m unittest discover -s scripts/tests -p test_vim_runtime.py -v
.\scripts\test-win-vim-runtime.ps1
.\scripts\test-win.ps1 -NoBuild
```

The runtime script accepts `-NoRustBuild` to reuse the Rust DLL while still
building the frontend and testing both MSBuild packaging targets. It verifies
all 777 runtime files, attribution bytes, nested helper bytes, stale-file
removal and preservation of neighboring resources. It publishes into a path
containing spaces and Japanese characters, launches from another working
directory, moves the application and launches again, then temporarily removes
only that test application's runtime to check graceful fallback.

Native checks require actual paint for `.vimrc`, Makefile and Rust tokens and
for a Debian distribution keyword loaded through `shared/debversions.vim`;
Code mode alone does not satisfy them. They cover clean state/revision/source
preservation, editing and undo, retired settings with an external no-highlight
fixture, migration without losing unrelated JSON, and missing Vim resources
with Rust highlighting still available. The standard Windows harness also
checks that Settings has no Code category and directly validates the global
Styles inspector against a Code view. Reports and isolated profiles are
retained in `target/windows-validation`.

Validated on Windows on 2026-09-19: Debug and Release builds completed with no
warnings or errors; all 8 Python packaging tests passed; build, publish and
relocated launches each passed 26 native syntax checks, and the missing-runtime
launch passed 16. The full Windows integration harness passed 257 checks and
verified all 137 generated C ABI declarations. Native tests used isolated
profiles outside the restricted sandbox because Windows atomic file replacement
is blocked inside it. The vendored runtime files and manifest remain unchanged.
