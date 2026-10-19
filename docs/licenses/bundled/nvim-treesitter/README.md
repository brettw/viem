# nvim-treesitter queries

These are exact copies of the license and attribution files beside the bundled
[Tree-sitter queries](../../../../src/core/document/syntax/treesitter).

| Origin | License |
| --- | --- |
| nvim-treesitter contributions | [Apache 2.0](nvim.LICENSE) |
| tree-sitter-rust query origin, Maxim Sokolov | [MIT](rust.LICENSE) |
| tree-sitter-python query origin, Max Brunsfeld | [MIT](python.LICENSE) |
| tree-sitter-markdown query origin, Matthias Deiml | [MIT](markdown.LICENSE) |
| Helix Markdown query origin | [MPL 2.0](helix.LICENSE) |

[nvim.NOTICES.md](nvim.NOTICES.md) records the pinned upstream revisions, maps
each copied query to its origin, and preserves the full attribution inventory.
It is copied unchanged, including its description of the original query-specific
packaging directory. The source query files and their adjacent notices remain
available in this repository; the notices also ship in their existing resource
locations in both native packages. Query source is embedded in the core.

The grammar crates and Tree-sitter runtime are separately included in the Rust
dependency license inventory; this directory covers the copied query sources.
