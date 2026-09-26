# Bundled nvim-treesitter query attributions

Viem includes unmodified highlight and injection query files from
[nvim-treesitter](https://github.com/nvim-treesitter/nvim-treesitter), distributed
under the [Apache License 2.0](nvim.LICENSE). This license covers the copied
nvim-treesitter contributions. The original Rust and Python highlight-query
copyright notices and MIT terms are retained separately below.

The main source revision is
[`f603a2f4da48728f80257fb5fbb90145fd1dc173`](https://github.com/nvim-treesitter/nvim-treesitter/tree/f603a2f4da48728f80257fb5fbb90145fd1dc173/runtime/queries).
Swift uses the compatible revision
[`13ddd4d7522ce3e5a1abc0ea34e10ec4e445908a`](https://github.com/nvim-treesitter/nvim-treesitter/tree/13ddd4d7522ce3e5a1abc0ea34e10ec4e445908a/runtime/queries/swift).
Both revisions contain the same Apache license text and no separate upstream
NOTICE file. This document is Viem's attribution inventory.

## Copied files

Each row maps the local files to `runtime/queries/<language>/highlights.scm`
and `runtime/queries/<language>/injections.scm` at the revision above.

| Upstream language directory | Local highlight file | Local injection file |
| --- | --- | --- |
| `c` | `c.scm` | `c_injections.scm` |
| `cpp` | `cpp.scm` | `cpp_injections.scm` |
| `rust` | `rust.scm` | `rust_injections.scm` |
| `swift` | `swift.scm` | `swift_injections.scm` |
| `objc` | `objc.scm` | `objc_injections.scm` |
| `c_sharp` | `c_sharp.scm` | `c_sharp_injections.scm` |
| `javascript` | `javascript.scm` | `javascript_injections.scm` |
| `typescript` | `typescript.scm` | `typescript_injections.scm` |
| `tsx` | `tsx.scm` | `tsx_injections.scm` |
| `python` | `python.scm` | `python_injections.scm` |
| `json` | `json.scm` | `json_injections.scm` |
| `ecma` | `ecma.scm` | `ecma_injections.scm` |
| `jsx` | `jsx.scm` | `jsx_injections.scm` |

The files retain their original bytes and attribution headers. When compiling
queries, Viem resolves their inheritance and removes the loader-only `inherits`
headers from the combined query strings. The stored upstream files are unchanged.

## Retained MIT notices

- `rust.scm` identifies its origin as
  [tree-sitter-rust](https://github.com/tree-sitter/tree-sitter-rust).
  Copyright (c) 2017 Maxim Sokolov. Its complete MIT license is in
  [rust.LICENSE](rust.LICENSE), copied from the upstream
  [v0.23.2 license](https://github.com/tree-sitter/tree-sitter-rust/blob/v0.23.2/LICENSE).
- `python.scm` identifies its origin as
  [tree-sitter-python](https://github.com/tree-sitter/tree-sitter-python).
  Copyright (c) 2016 Max Brunsfeld. Its complete MIT license is in
  [python.LICENSE](python.LICENSE), copied from the upstream
  [v0.23.5 license](https://github.com/tree-sitter/tree-sitter-python/blob/v0.23.5/LICENSE).

These three license files and this attribution inventory ship together in
`Contents/Resources/Licenses/nvim-treesitter` on macOS and
`Resources/Licenses/nvim-treesitter` in Windows build and publish output.
