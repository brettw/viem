# Bundled Vim syntax

This is an unmodified snapshot of the user's installed **MacVim 9.1.1887**
runtime syntax directory. `runtime/syntax` retains the entire directory tree,
including nested helpers and each file's original attribution. `runtime/LICENSE`
is copied from the same installation. The personal `~/.vim` directory contained
a color scheme but no syntax files at import time.

`manifest.json` pins the exact imported bytes with SHA-256 hashes. Its source
path records provenance only; builds never read from that installation or
download syntax files. Git attributes disable text conversion for these files,
including on Windows. The [native compiler profile](../../src/core/document/syntax/vim/PROFILE.md)
documents compatibility and current audit procedures; distribution does not
imply that every file is supported by Viem's bounded native compiler.

## Build and update

macOS packaging verifies this directory, copies it to
`Viem.app/Contents/Resources/vim`, verifies the copy, and signs the app. Python 3
with its standard library is required by the packaging verifier. Windows builds
and publishes the same assets to `Resources/vim` beside the executable, using
`scripts/vim-runtime.py package` to replace only that subtree and verify both
copies. This also runs for direct MSBuild builds and publishing without a build;
see [Windows development](../../src/win/README.md) for validation.

To intentionally replace the snapshot from an installed Vim runtime:

```sh
python3 scripts/vim-runtime.py import /path/to/vim/runtime --version 'Exact distribution and revision'
python3 scripts/vim-runtime.py verify
cargo run --release --offline --example audit_vim_syntax -- assets/vim/runtime/syntax
```

Review the byte and compatibility changes together before accepting an update.
The import replaces the owned `runtime` subtree, removing obsolete files, and
preserves this README. Runtime globs and includes require preserving the helper
directories. The compiler never runs a Vim executable during normal editing.

## Language and filename maintenance

Whenever this snapshot gains, loses or renames syntax files, follow the
[language maintenance checklist](../../src/core/document/syntax/detection/PROFILE.md#keeping-language-support-and-filename-rules-in-sync).
Review the catalogue/aliases and update
[`detection/filenames.rs`](../../src/core/document/syntax/detection/filenames.rs)
and detection/opening tests in the same change, or document intentional omissions.
The manifest feeds the language catalogue, but import does not update filename
rules. Syntax-file stems are language/helper names, not an extension inventory.

These `.vim` programs are consumed by the native Vim compiler. The separate
[Tree-sitter packages](../../src/core/document/syntax/treesitter/PROFILE.md) use
compiled grammars and `.scm` queries for a smaller set of languages. Vim-only
languages can have automatic filename detection too. Keep maintenance notes here,
outside the byte-preserved upstream `runtime` subtree and its manifest.

## Resource discovery

Viem always uses its bundled syntax files. The frontend resolves the bundle's
current installation path at runtime and passes it through the existing C ABI.
Includes stay within that syntax root. There is no directory preference or
associated settings UI, and resource paths are never persisted.

The retired `code.vimSyntaxDirectory` key is ignored and removed on the next
settings write; unrelated settings remain intact. Missing resources report a
diagnostic while editing and bundled Tree-sitter remain available.
