# Windows follow-up: bundle the shared Vim syntax snapshot

The macOS implementation vendors the existing MacVim 9.1.1887 syntax tree at
`assets/vim/runtime/syntax`. `assets/vim/runtime/LICENSE` and per-file notices
are unchanged. `assets/vim/manifest.json` pins all runtime files with their byte
lengths and SHA-256 hashes. `scripts/vim-runtime.py verify [assets-directory]`
validates the complete inventory, including nested helpers and extra files.
The `.gitattributes` entry preserves original encodings/line endings on Windows.

Copyable task for the Windows agent:

```text
Implement Windows packaging and resource discovery for Viem's bundled Vim
syntax runtime. Use the existing checked-in assets/vim snapshot; do not import
a second runtime or depend on Vim being installed. Read assets/vim/README.md
and the Code settings requirements in AGENTS.md before editing.

1. Update src/win/Viem.Windows.csproj and scripts/build-win.ps1 as needed so
   both build and publish output include assets/vim under Resources/vim,
   preserving runtime/syntax's nested directories, LICENSE, README.md, and
   manifest.json. The current application uses WindowsPackageType=None.
   Remove obsolete files only within the owned Resources/vim subtree during
   rebuilds. Verify source and packaged file inventories/hashes, using the
   existing Python verifier or equivalent PowerShell/.NET logic. Fail the
   build for missing, changed, or unexpected runtime files.

2. Resolve the default runtime from AppContext.BaseDirectory (or the actual
   package resource location if packaging changes), never the process working
   directory. Pass Resources/vim/runtime/syntax through the existing
   viem_core_configure_syntax ABI. Keep resource lookup in the Windows frontend;
   the Rust core now has an empty platform-neutral directory default.

3. Always use the bundled files. Remove the Vim syntax directory field from
   Code settings and remove its backing preference/override handling. There
   is no chooser or Restore Default action for syntax resources. Ignore any
   retired code.vimSyntaxDirectory value and remove that key on the next
   settings write, preserving unrelated fields. Never persist the resolved
   resource path. Includes stay within the bundled root. Missing resources
   report a diagnostic while editing and Tree-sitter remain available. Keep
   Code Styles and syntax load diagnostics in settings.

4. Test that legacy directory settings cannot override bundled resources and
   unrelated config fields are preserved. Test both build/publish resource
   inventories, stale-file removal, nested includes, installation paths with
   spaces/non-ASCII characters, relocation, and launching from another working
   directory. Verify bundled vim.vim highlights a .vimrc and make.vim highlights
   a Makefile without an installed Vim and without changing source bytes or
   dirty state. Keep unsupported native syntax diagnostics intact: shipping a
   file does not expand the compiler's compatibility subset.

Run the appropriate Windows build and test scripts, document validation, and
update AGENTS.md to mark Windows packaging implemented. Leave the shared
snapshot and its manifest unchanged unless a separate runtime update is needed.
```
