# Bundled nvim-treesitter query attributions

Viem includes unmodified highlight and injection query files from
[nvim-treesitter](https://github.com/nvim-treesitter/nvim-treesitter), distributed
under the [Apache License 2.0](nvim.LICENSE). This license covers the copied
nvim-treesitter contributions. The original Rust and Python highlight-query
copyright notices and MIT terms, and Markdown's upstream attributions, are
retained separately below.

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
| `markdown` | `markdown.scm` | `markdown_injections.scm` |
| `markdown_inline` | `markdown_inline.scm` | `markdown_inline_injections.scm` |
| `ecma` | `ecma.scm` | `ecma_injections.scm` |
| `jsx` | `jsx.scm` | `jsx_injections.scm` |

The files retain their original bytes and attribution headers. When compiling
queries, Viem resolves their inheritance and removes the loader-only `inherits`
headers from the combined query strings. The stored upstream files are unchanged.

## Retained upstream notices

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
- `markdown.scm` and `markdown_inline.scm` identify
  [tree-sitter-markdown](https://github.com/tree-sitter-grammars/tree-sitter-markdown)
  as an origin. Copyright (c) 2021 Matthias Deiml. Its complete MIT license is in
  [markdown.LICENSE](markdown.LICENSE), copied from the upstream
  [v0.5.3 license](https://github.com/tree-sitter-grammars/tree-sitter-markdown/blob/v0.5.3/LICENSE).
  The same license covers the bundled `tree-sitter-md` grammar.
- `markdown.scm` also identifies [Helix](https://github.com/helix-editor/helix)
  as an origin. Its MPL-2.0 license is retained in [helix.LICENSE](helix.LICENSE),
  copied from the upstream
  [25.07.1 license](https://github.com/helix-editor/helix/blob/25.07.1/LICENSE).
  The unmodified query source is available at the pinned nvim-treesitter revision
  and paths above, and in this source distribution.

These license files and this attribution inventory ship together in
`Contents/Resources/Licenses/nvim-treesitter` on macOS and
`Resources/Licenses/nvim-treesitter` in Windows build and publish output.
