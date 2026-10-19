# Bundled assets and copied source

These copies collect the licenses and attribution for material stored directly
in the Viem repository. Original notices remain beside the bundled resources,
where packaging and integrity checks require them.

| Component | License and attribution |
| --- | --- |
| Recursive 1.085 font | [SIL OFL 1.1](recursive/LICENSE.txt), [font attribution](recursive/README.md) |
| Flightline Code 0.002 fonts, derived from B612 and including Cascadia Code glyphs | [SIL OFL 1.1](flightline/OFL.txt), [Cascadia Code OFL](flightline/Cascadia-Code-LICENSE.txt), [complete font notices](flightline/README.md) |
| MacVim/Vim 9.1.1887 runtime syntax and helpers | [Vim license](vim-runtime/LICENSE.txt), [individual file licenses and notices](vim-runtime/README.md) |
| Vim syntax test fixtures | [Vim license](vim-fixtures/LICENSE.txt), [fixture scope](vim-fixtures/README.md) |
| Vim-derived color names and builtin signature metadata | [Vim license](../LICENSE_vim.txt) |
| nvim-treesitter queries and their upstream sources | [Apache 2.0](nvim-treesitter/nvim.LICENSE), [MIT/MPL notices and query provenance](nvim-treesitter/README.md) |
| Unicode 15.0.0 data and conformance fixtures | [Unicode license](unicode/LICENSE.txt), [data provenance](unicode/README.md) |

[sources.json](sources.json) records copied files, exact source URLs or local
paths, extraction ranges where applicable, and SHA-256 checksums. License URLs
pin the retrieved document, not an inferred version of the bundled code. The
runtime and Unicode data retain their separate original provenance manifests.

When an asset changes, update its central license copies and provenance together
with the original distribution notices. Do not edit a license to normalize
copyright years, whitespace, line endings, or terminology.
