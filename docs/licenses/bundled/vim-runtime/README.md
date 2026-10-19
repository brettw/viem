# Vim runtime syntax and helpers

The bundled runtime is the unmodified **MacVim 9.1.1887** snapshot described in
[assets/vim/README.md](../../../../assets/vim/README.md). The three autoload
helpers are from Vim's `v9.1.1887` tag. The original
[manifest](../../../../assets/vim/manifest.json) pins every source file's bytes.

[LICENSE.txt](LICENSE.txt) is an exact copy of the runtime's Vim license.
[FILE-NOTICES.txt](FILE-NOTICES.txt) preserves the leading attribution blocks
and other license/copyright comment blocks from the bundled `.vim` files,
with their source paths and line ranges. Those notices also remain in the
complete original sources under [assets/vim/runtime](../../../../assets/vim/runtime).
Some individual files identify other terms, collected below; the Vim license
does not replace those notices.

| Bundled syntax file(s) | License text and attribution |
| --- | --- |
| `asciidoc.vim` | [AsciiDoc GPL v2 text](AsciiDoc-LICENSE.txt) and [copyright / v2-or-later notice](AsciiDoc-COPYRIGHT.txt) from AsciiDoc 9.0.0. The bundled file's header says GPL without specifying a version. |
| `bitbake.vim` | [MIT terms](MIT.txt); Chris Larson and Ricardo Salveti notices in [FILE-NOTICES.txt](FILE-NOTICES.txt). |
| `cf.vim` | [ColdFusion MIT license](ColdFusion-LICENSE.txt). |
| `cmake.vim` | [CMake copyright and BSD terms](CMake-Copyright.txt), from CMake 3.27.0, matching the bundled file's 3.27 series. |
| `dockerfile.vim` | [Dockerfile BSD license](Dockerfile-LICENSE.txt). |
| `doxygen.vim` | [Michael Geddes's permission notice](doxygen-NOTICE.txt). |
| `elm.vim` | [Elm BSD license](Elm-LICENSE.txt). |
| `go.vim` | [vim-go and Go BSD notices](Go-LICENSE.txt); the bundled file's 2009 Go Authors notice remains in [FILE-NOTICES.txt](FILE-NOTICES.txt). |
| `graphql.vim` | [GraphQL MIT license](GraphQL-LICENSE.txt). |
| `hcl.vim`, `terraform.vim` | Full ISC terms embedded in [hcl-NOTICE.txt](hcl-NOTICE.txt) and [terraform-NOTICE.txt](terraform-NOTICE.txt). |
| `initng.vim` | [GPL v2](GPL-2.0.txt); Elan Ruusamäe's source attribution in [FILE-NOTICES.txt](FILE-NOTICES.txt). |
| `json.vim` | [Embedded MIT notice](json-NOTICE.txt). |
| `jsonc.vim` | [vim-jsonc MIT license](JSONC-LICENSE.txt); Izhak Jakov's source attribution in [FILE-NOTICES.txt](FILE-NOTICES.txt). |
| `kotlin.vim` | [Kotlin Apache 2.0 license](Kotlin-LICENSE.txt); Alexander Udalov's attribution in [FILE-NOTICES.txt](FILE-NOTICES.txt). |
| `nix.vim` | [vim-nix MIT license](Nix-LICENSE.txt); additional James Fleming attribution in [FILE-NOTICES.txt](FILE-NOTICES.txt). |
| `nu.vim` | [Nushell MIT license](Nushell-LICENSE.txt). |
| `poke.vim` | [GPL v3](GPL-3.0.txt); Matthew T. Ihlenfield's copyright and v3-or-later grant in [FILE-NOTICES.txt](FILE-NOTICES.txt). |
| `proto.vim` | [Embedded Protocol Buffers BSD notice](proto-NOTICE.txt). |
| `ps1.vim`, `ps1xml.vim` | [PowerShell upstream copyright and Apache grant](PowerShell-README.markdown), with the [full Apache 2.0 text](Kotlin-LICENSE.txt). |
| `requirements.vim` | [Embedded MIT notice](requirements-NOTICE.txt). |
| `rust.vim` | [Rust Apache 2.0 license](Rust-LICENSE-APACHE.txt) and [MIT license](Rust-LICENSE-MIT.txt). |
| `scheme.vim`, `chicken.vim` | [Unlicense](Unlicense.txt), with Evan Hanson's attribution in [FILE-NOTICES.txt](FILE-NOTICES.txt) and the supplemental distributor inventory below. |
| `sil.vim`, `swift.vim`, `swiftgyb.vim` | [Swift Apache 2.0 license with Runtime Library Exception](Swift-LICENSE.txt), including the exception named by the bundled source headers. |
| `thrift.vim` | [Apache Thrift license](Thrift-LICENSE.txt) and [NOTICE](Thrift-NOTICE.txt). |
| `tmux.vim` | [BSD 2-clause terms](BSD-2-Clause.txt), [upstream license statement](Tmux-README.md), and Eric Pruitt's source attribution in [FILE-NOTICES.txt](FILE-NOTICES.txt). |
| `tpp.vim` | [BSD 3-clause terms](BSD-3-Clause.txt), Gerfried Fuchs's source attribution in [FILE-NOTICES.txt](FILE-NOTICES.txt), and the supplemental distributor inventory below. |
| `wat.vim` | [WebAssembly syntax MIT license](WebAssembly-LICENSE.txt). |

[Debian-Vim-copyright.txt](Debian-Vim-copyright.txt) is an unmodified
supplemental inventory from the Debian Vim maintainers. It supplies the named
BSD, MIT, and Unlicense sections copied here and records the Scheme and TPP
attributions. It describes a broader, newer Vim distribution: its other file
entries are not a claim that Viem includes those files. The installed snapshot's
own source notices and manifest remain the inventory of what Viem actually
ships.

Upstream license documents are preserved without rewriting their copyright
years or replacing them with generic templates. [sources.json](../sources.json)
records the exact source revision for each retrieved document. These revisions
identify the license documents; they do not claim that the bundled syntax files
were refreshed to those revisions. Copied upstream READMEs keep their original
relative links, which refer to the original projects.

The separate [Vim license](../../LICENSE_vim.txt) covers Vim-derived builtin
signature metadata and color-name data in the core. Test fixture licensing is
listed [separately](../vim-fixtures/README.md).
