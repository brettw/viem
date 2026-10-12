# Comment language profiles and coverage

A formatting profile says how to format an eligible comment; it does not prove
that an occurrence is a language comment. Language context establishes eligibility
independently of syntax highlighting. See [Code formatting](code-formatting.md)
for command behavior, manual overrides, bounded context, and known language forms.

## Reference baseline

Profiles were reviewed against **Vim v9.2.0000**, commit
[`e7e21018fc0b60c153c8e668f696d95e574cc5a4`](https://github.com/vim/vim/tree/e7e21018fc0b60c153c8e668f696d95e574cc5a4),
including the ftplugins linked below and
[`runtime/doc/change.txt`](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/doc/change.txt).
The annotated tag object is `ae86d7f73f5189af4f3df3a69c1c81ce5a54fb5e`.
This matches the project's Vim 9.2 behavior target. The separate bundled syntax
runtime remains MacVim 9.1.1887; its bytes, license, syntax rules, and language
catalogue are unchanged. These constants require no downloads, Vimscript,
ftplugins, or external process during build or editing. The provenance review
does not import ftplugin indentation, `formatoptions`, mappings, or other settings.

The registry in `src/core/document/comment_profiles.rs` is the implementation's
single source of truth. Language selection uses existing canonical aliases.
Unlisted languages inherit an empty profile; explicit buffer settings still work.
All requested sibling identities already exist in the catalogue, including
`proto`, `hcl`, `terraform`, and `ksh`/`dash`/`mksh`; no catalogue change was needed.

## Effective Viem defaults

Values below are decoded option values, not Ex command spellings. In the table,
`␠` denotes one ASCII space; there is no literal `␠` in the setting. An empty cell
shown as `(empty)` means the empty string. No profile enables continuation by
itself: `formatoptions` remains `qj` unless explicitly changed or a continuation
preference supplies its default. These entries include compatibility-only
languages; lexical coverage is documented separately.

| Canonical language | Effective `comments` | Pinned reference |
| --- | --- | --- |
| astro | `s:<!--,m:␠␠␠␠,e:-->` | [astro.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/astro.vim) |
| bash | `:#` | [bash.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/bash.vim), [sh.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/sh.vim) |
| c | `:///,://!,://,s1:/*,mb:*,ex:*/` | [c.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/c.vim) |
| c_sharp | `:///,://!,://,s1:/*,mb:*,ex:*/` | [cs.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/cs.vim) |
| cmake | `s:#[[,m:␠,e:]],:#` | [cmake.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/cmake.vim) |
| cpp | `:///,://!,://,s1:/*,mb:*,ex:*/` | [cpp.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/cpp.vim), [c.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/c.vim) |
| css | `s1:/*,mb:*,ex:*/` | [css.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/css.vim) |
| dart | `:///,://,s1:/*,mb:*,ex:*/` | [dart.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/dart.vim) |
| dash | `:#` | [sh.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/sh.vim) |
| dockerfile | `:#` | [dockerfile.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/dockerfile.vim) |
| dosbatch | `b:rem,b:@rem,b:REM,b:@REM,:::` | [dosbatch.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/dosbatch.vim) |
| dosini | `:;,:#` | [dosini.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/dosini.vim) |
| gitcommit | `:#` | [gitcommit.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/gitcommit.vim) |
| go | `s1:/*,mb:*,ex:*/,://` | [go.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/go.vim) |
| graphql | `:#` | [graphql.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/graphql.vim) |
| groovy | `s1:/*,mb:*,ex:*/,://` | [groovy.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/groovy.vim) |
| haskell | `s1fl:{-,mb:-,ex:-},:--` | [haskell.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/haskell.vim) |
| hcl | `s1:/*,mb:*,ex:*/,://,:#` | [hcl.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/hcl.vim), [terraform.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/terraform.vim) |
| html | `s:<!--,m:␠␠␠␠,e:-->` | [html.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/html.vim) |
| java | `:///,://,s1:/*,mb:*,ex:*/` | [java.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/java.vim) |
| javascript | `s1:/*,mb:*,ex:*/,://` | [javascript.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/javascript.vim) |
| json | (empty) | [json.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/json.vim) |
| json5 | `s1:/*,mb:*,ex:*/,://` | [json5.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/json5.vim) |
| jsonc | `s1:/*,mb:*,ex:*/,://` | [jsonc.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/jsonc.vim), [json.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/json.vim) |
| kotlin | `s1:/*,mb:*,ex:*/,://` | [kotlin.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/kotlin.vim) |
| ksh | `:#` | [sh.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/sh.vim) |
| lua | `s:--[[,m:␠,e:]],:---,:--` | [lua.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/lua.vim) |
| make | `:#` | [make.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/make.vim) |
| markdown | `s:<!--,m:␠␠␠␠,e:-->` | [markdown.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/markdown.vim), [html.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/html.vim) |
| mermaid | `:%%` | [mermaid.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/mermaid.vim) |
| mksh | `:#` | [sh.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/sh.vim) |
| objc | `:///,://!,://,s1:/*,mb:*,ex:*/` | [objc.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/objc.vim), [c.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/c.vim) |
| perl | `:#` | [perl.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/perl.vim) |
| php | `s1:/*,mb:*,ex:*/,://,:#` | [php.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/php.vim), [html.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/html.vim) |
| plaintex | `:%` | [plaintex.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/plaintex.vim) |
| proto | `s1:/*,mb:*,ex:*/,://` | [proto.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/proto.vim) |
| ps1 | `s:<#,m:␠,ex:#>,:#` | [ps1.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/ps1.vim) |
| python | `:#` | [python.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/python.vim) |
| r | `:#',:###,:##,:#` | [r.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/r.vim) |
| ruby | `:#` | [ruby.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/ruby.vim) |
| rust | `s:/*!,m:␠,ex:*/,s1:/*,mb:*,ex:*/,:///,://!,://` | [rust.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/rust.vim) |
| scala | `s1:/**,mb:*,ex:*/,s1:/*,mb:*,ex:*/,://` | [scala.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/scala.vim) |
| scss | `s1:/*,mb:*,ex:*/,://` | [scss.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/scss.vim), [sass.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/sass.vim) |
| sh | `:#` | [sh.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/sh.vim) |
| sql | `s1:/*,mb:*,ex:*/,:--` | [sql.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/sql.vim) |
| swift | `:///,://,s1:/*,mb:*,ex:*/` | [swift.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/swift.vim) |
| terraform | `s1:/*,mb:*,ex:*/,://,:#` | [terraform.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/terraform.vim) |
| tex | `:%` | [tex.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/tex.vim), [plaintex.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/plaintex.vim) |
| toml | `:#` | [toml.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/toml.vim) |
| tsx | `s1:/*,mb:*,ex:*/,://` | [typescriptreact.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/typescriptreact.vim), [typescript.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/typescript.vim) |
| typescript | `s1:/*,mb:*,ex:*/,://` | [typescript.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/typescript.vim) |
| typst | `s1:/*,mb:*,ex:*/,://` | [typst.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/typst.vim) |
| vim | `:",:#` | [vim.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/vim.vim) |
| vue | `s:<!--,m:␠␠␠␠,e:-->` | [vue.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/vue.vim) |
| xml | `s:<!--,m:␠␠␠␠,e:-->` | [xml.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/xml.vim) |
| yaml | `:#` | [yaml.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/yaml.vim) |
| zsh | `:#` | [zsh.vim](https://github.com/vim/vim/blob/e7e21018fc0b60c153c8e668f696d95e574cc5a4/runtime/ftplugin/zsh.vim) |

## Lexical coverage and explicit limits

Language-aware means the provider proves the listed forms. It does not mean a
complete language parser. An encountered unsupported or ambiguous construct in a
known provider produces **unknown** context until a new validated analysis can
establish it; it does not fall back to literal matching. Enter/open-line use
ordinary indentation, joins retain leaders, and comment-aware reflow rejects the
transaction in that state. Native ranges preserve opener, body, closer, origin,
and nesting identities for later features. They do not enable Markdown editing.

| Language-aware identities | Native comments and protected literals | Important unknown or excluded forms |
| --- | --- | --- |
| C, C++, Objective-C | `//`, `/*…*/`, ordinary quoted strings/characters, C++ raw delimiters, backslash-continued C line comments, and simple include/import header operands | Incomplete/ambiguous header operands, `__has_include`/`__has_embed`, trigraphs and ambiguous splices; no preprocessor execution. |
| Rust | `//`, nested `/*…*/`, normal and raw `#` strings | Incomplete/unsupported literal forms remain conservative. |
| Go | `//`, `/*…*/`, strings/characters and raw backticks | No language runtime or generated-code interpretation. |
| Java | `//`, `/*…*/`, ordinary strings/characters and triple-quoted text blocks | Unicode escape preprocessing is unknown. |
| JavaScript, TypeScript, TSX | `//`, `/*…*/`, ordinary strings and noninterpolated templates | Regular expressions, template interpolation, JSX/TSX embedding. |
| C# | `//`, `/*…*/`, ordinary supported quoted literals | Interpolated and raw strings. |
| Swift | `//`, nested `/*…*/`, ordinary and triple strings | Raw `#` literals and regex forms. |
| Kotlin, Scala, Dart | `//`, nested `/*…*/`, ordinary and triple strings, Dart raw strings, Kotlin/Scala raw triples | Scala interpolation/XML and other unsupported interpolation constructs. |
| Groovy | `//`, `/*…*/`, ordinary and triple strings | Slashy literals and ambiguous forms. |
| Protocol Buffers, CSS, SCSS, JSONC, JSON5, JSON | Declared language line/block forms; ordinary quoted strings. CSS has only block comments; JSON has no native comments. | CSS `url()` and SCSS interpolation. |
| Python | `#`, ordinary and multiline triple strings | f-strings, t-strings, and ambiguous/incomplete literals. |
| R, TOML, GraphQL | `#` and ordinary literals; TOML/GraphQL triple strings | Unsupported dialect/literal extensions. |
| HCL, Terraform | `#`, `//`, `/*…*/`, ordinary strings | Heredocs and interpolation. |
| Lua | `--`, arbitrary equals-level long-bracket comments and strings | Formatting a long bracket needs a matching block profile; default formats `--[[…]]`, while unmatched native blocks remain intact. |
| sh, Bash, zsh, ksh, dash, mksh | `#`, simple quotes, simple parameter expansion, `$#`, ordered simple quoted/unquoted `<<` and `<<-` heredocs | Command substitutions, arithmetic, compound parameter expansions, escaped physical continuations, and dialect-dependent constructs. Shebangs are protected. |
| YAML | `#`, quoted scalars, indented `|`/`>` block scalars | Unsupported scalar/header forms remain conservative. |
| SQL | `--`, nested `/*…*/`, ordinary supported quoted strings | Dollar-quoted, bracketed, and escaped-string dialect forms. |
| Haskell | `--`, nested `{-…-}`, ordinary literals | Pragmas and unsupported lexical forms. |
| Vim script | Legacy full-line `"`; full-line `#` after detected `vim9script`; protected ordinary strings | Ambiguous trailing markers and `=<<` heredocs; this does not parse the full Ex command grammar. |
| CMake | `#`, arbitrary equals-level bracket comments/strings, ordinary strings | Default formats `#[[…]]`; other equals levels require a matching explicit profile. |
| INI | `;`, `#`, ordinary quoted literals | Format-specific extensions are not a complete INI grammar. |
| TeX, Plain TeX | `%` with escape handling | Verbatim constructs and catcode changes. |
| XML | `<!--…-->`, quoted attributes, protected CDATA and processing instructions | DTD and embedded languages. |
| PowerShell | `#`, `<#…#>`, ordinary strings | Here-strings and unsupported interpolation/escape forms. |

**Literal-leader compatibility:** Ruby, Perl, PHP, HTML, Astro, Vue, Markdown in
Code format, Make, Dockerfile, Windows Batch, Mermaid, Typst, and Git commit.
Their profiles provide conventions only. There is no promise that markers in
strings, heredocs, raw bodies, interpolation, or embedded languages are protected.
Other languages without a provider use the same compatibility mode when the user
supplies an explicit profile. Languages with providers never silently enter this
mode because a construct or budget is unsupported. Mixed-language embedding is
not claimed by these providers.

## Upstream effective values and adaptations

This inventory resolves `set` escaping, inherited ftplugins, default options,
and relevant conditional choices. `D` below is Vim's clean default value:
`s1:/*,mb:*,ex:*/,://,b:#,:%,:XCOMM,n:>,fb:-`.
`L` is its dashed-list triplet: `sO:*␠-,mO:*␠␠,exO:*/`.
`B` is `s1:/*,mb:*,ex:*/`.
`H` is `s:<!--,m:␠␠␠␠,e:-->`.
Combinations such as `L,B,://` mean literal concatenation with commas. This
notation only abbreviates the inventory; no symbolic values are stored.

| Reference languages | Effective upstream value / choice | Viem adaptation |
| --- | --- | --- |
| C, C++, Objective-C, C# | `L,B,:///,://`; C++ and Objective-C inherit C | Remove list-only pseudo-block; put distinct `///` and `//!` before `//`, preserving Viem's prior documentation-leader behavior. |
| JavaScript, TypeScript, TSX, Kotlin, JSON5, JSONC | `L,B,://`; TSX inherits TypeScript; JSONC replaces JSON's empty value | Remove list-only pseudo-block. |
| Go, Protocol Buffers, SCSS, Typst | `B,://`; SCSS inherits Sass first, then replaces its value | Unchanged. |
| CSS | `B` | Unchanged; `//` is not a CSS comment default. |
| Dart | `L,B,:///,://` | Remove list-only pseudo-block; retain distinct `///`. |
| Java | `:///,L,D` (reset default, then prepend) | Keep Java block/line forms and documentation leader; omit unrelated default markers and dashed-list pseudo-block. |
| Swift | `B,:///,://` | Retain distinct `///`, ordered before `//`. |
| Rust | `s0:/*!,ex:*/,B,:///,://!,://` when `rust_bang_comment_leader` is absent/zero | Explicit whitespace middle for unstarred `/*!`; preserve its own block identity. Nonzero upstream chooses `B,s0:/*,mb:␠,ex:*/,:///,://!,://`; this alternative is not selected. |
| Scala | `L,s1:/**,mb:*,ex:*/,B,://` when `scala_scaladoc_indent` is absent/zero | Remove list-only pseudo-block. Nonzero upstream uses offset `s2` for `/**`; Viem chooses the default `s1`. |
| PHP | `B,://,b:#` with default `php_autocomment=1`; zero gives `B,://,:#` | Choose `:#`; language-aware recognition, when available, must distinguish attributes. PHP is currently literal-leader compatibility, so this is not attribute/string protection. |
| Python | `b:#,fb:-` | Remove the list marker and allow valid `#comment` without a following blank. |
| sh, Bash, ksh, dash, mksh | `b:#`; Bash inherits sh; other shell identities use the sh reference | Allow `#comment`; lexical context, not a blanket blank requirement, protects shell substitutions, parameters, and shebangs. |
| zsh, YAML, TOML, GraphQL, Dockerfile, Perl | `:#` | Unchanged. |
| Ruby | `b:#` | Allow `#comment`; remains compatibility-only, without Ruby literal protection. |
| R | `:#',:###,:##,:#` | Unchanged; distinct documentation leaders stay ordered. |
| Haskell | `s1fl:{-,mb:-,ex:-},:--` | Unchanged. `s` takes precedence over the later `f`; see flag audit below. |
| Lua | `:---,:--` | Add a block profile for `--[[…]]` before line leaders. Arbitrary equals-level long brackets remain lexically protected, but need a matching explicit profile to format their body. |
| HCL, Terraform | `://,:#`; HCL inherits Terraform | Add native `/*…*/` formatting. Heredocs require language context, not this table. |
| SQL | `B,:--,://` (final assignment in sql.vim) | Remove nonportable `//`; supported SQL dialect restrictions belong to the context provider. |
| HTML, Vue | `H` | Unchanged; neither row promises embedded-language protection. |
| XML | `s:<!--,e:-->` | Add an explicit four-space middle, matching the HTML convention. |
| Astro | Context-dependent `L,B,://`, `B`, or `H` from its script/style/template callback | Choose `H` for literal-leader compatibility; no attempt to execute its callbacks or infer embedded-language formatting. |
| Markdown | `fb:*,fb:-,fb:+,n:>` after loading HTML | Use `H` for Code-format HTML comments; Markdown's own structural list handling remains separate. |
| Make | `sO:#␠-,mO:#␠␠,b:#` | Use `:#`; omit list-only pseudo-block and its unrelated inheritance. |
| CMake, Groovy, PowerShell | Ftplugins leave `comments` at inherited `D` (only `commentstring` is changed) | Keep actual language delimiters; add `#[[…]]` and `<#…#>` block conventions with whitespace middles. |
| TeX, Plain TeX | Ftplugins leave `comments` at inherited `D`; TeX sources Plain TeX | Keep only `:%`. |
| INI | `:;,:#` | Unchanged. |
| Windows Batch | `b:rem,b:@rem,b:REM,b:@REM`, plus `:::` only when `dosbatch_colons_comment` exists | Include conventional `::` explicitly; compatibility-only, not a Batch parser. |
| Git commit | A content-selected single marker from `#;@!$%^&|:`, otherwise `:#` | Choose `:#`; no Git configuration or marker-detection side effect. |
| Mermaid | `:%%` | Unchanged. |
| Vim script | Legacy `sO:"␠-,mO:"␠␠,eO:"",:"\␠,:"`; Vim9 `sO:#␠-,mO:#␠␠,eO:##,:#\␠,:#` when the first 32 lines contain `vim9script` | Keep both basic markers `:"` and `:#`; the lexical provider chooses the active dialect and protects strings. Omit list and escaped-continuation special leaders. |
| JSON | Empty | Unchanged. |

Omitting list-only defaults does not remove manual first-only or nested line
leaders. Users can opt into those formatting conventions explicitly. Such
leaders do not become native language comments or future Markdown-comment owners.
Whitespace-middle adaptations are explicit policy, not silently repaired user
settings: an unsupported manual setting fails before option publication.

## Flag audit

The profile parser preserves matching order across line leaders and block
openers, including overlapping `s1:#{,mb:#,ex:#},:#`. It keeps block identity,
part-specific `b` and `O`, numeric/right/left alignment, and end `x`. Whitespace
middles retain their exact width; they are not looked up after stripping the
whitespace that constitutes them. Distinct line/documentation leaders remain separate during reflow. Vim `j`
joining may strip a configured repeating next-line leader after a different
line-comment leader; first-only next-line leaders are not stripped.

The earlier design concern about a dropped block-start `f` needed qualification.
Vim 9.2 visits flags in order in `src/change.c` and `src/textformat.c`: `s` takes
precedence over a later `f`, as in Haskell's `s1fl`. That `f` is retained as
metadata but does not suppress middle generation. An `f` before `s` is a
different hybrid and is rejected explicitly here. Standalone line `f` retains
first-line-only behavior.

The accepted subset is line flags `n`, `b`, `f`, `O`; block-start flags `s`, `b`,
trailing `f`, `l`, `r`, `O`, and one signed numeric offset; middle flags `m`, `b`,
`O`; and end flags `e`, `b`, `x`, `l`, `r`, `O`, and one signed numeric offset.
Offsets are bounded to 1024 columns. Numeric alignment takes precedence over
`l`/`r`, as in Vim. First-only nested line leaders, nested formatting-block
flags, missing block middles, conflicting left/right alignment without an overriding
number, misplaced flags, and split numeric offsets are
rejected atomically instead of being accepted with missing semantics. Native
block nesting is established by language context independently of those flags.

Viem deliberately applies configured alignment during its reflow, preserves
existing unstarred bodies, and generalizes `formatoptions=/` to every trailing
line-comment leader for lowercase `o`. The latter does not suppress Enter or
full-line comments. Generated closing is tied to its originating block and
option/language/source identity, rather than guessing from a shared final
closing character. It replaces the middle at its current column, following
Vim 9.2 `src/edit.c`; end offsets affect `O` before a closer, not this replacement.
This follows the implementation despite the broader alignment claim in Vim
help text.
