Make sure we support the HTML version of the basic markdown definitions, like <b>, etc. that github would support.

I want to add an option in code mode "Treat comments as markdown". Put it in the view menu. When the syntax highlighter produces comments.  TODO Figure out how this treats multiline comments.


# Per-language comment types

Extend Viem's per-language default `comments` profiles beyond the C family.

## Background

Comment leader recognition (Vim's `comments` option) is driven by
`CommentProfile` in src/core/document/format_options.rs. Today only
`C_FAMILY_COMMENT_LANGUAGES` get a default
(`C_FAMILY_COMMENTS = ":///,://!,://,s1:/*,mb:*,ex:*/"`), applied by
`ReflowOptions::set_language` (format_options.rs ~line 51). Every other language
(Python, shell, Ruby, Lua, SQL, YAML, TOML, HTML, Vim script, ...) has an empty
profile, so `gq`, `J` (with the default `formatoptions=qj`), Enter/`o`/`O` comment
continuation, and generated block-comment closing treat their comments as plain prose.

This is also a prerequisite for a planned "treat comments as Markdown" Code option
that needs exact leader bytes (otherwise a Python `# note` would parse as an ATX
heading). Do NOT implement that feature here; only make the profiles correct.

## What to build

1. Replace `C_FAMILY_COMMENTS` / `C_FAMILY_COMMENT_LANGUAGES` with one static,
   per-canonical-language table of default `comments` values (single source of
   truth). Look up by canonical language ID after alias resolution (see
   `canonical_language` in src/core/document/syntax/detection.rs and the catalogue
   in src/core/document/syntax/languages.rs). Keep `set_language` semantics:
   language refresh updates only the inherited default; explicit buffer `comments`
   overrides survive; `:set comments&` restores the language default; an explicit
   empty value disables recognition.
2. Source each value from Vim's upstream `runtime/ftplugin/<lang>.vim`
   `setlocal comments=...`, from the Vim release matching the bundled runtime
   (see assets/vim/README.md; AGENTS.md targets Vim 9.2 semantics, so note any
   difference). The bundled runtime (assets/vim/runtime) contains only syntax
   files. Do not add ftplugins, execute Vimscript, or fetch anything at build or
   run time; transcribe the values into the Rust table, citing the source file
   in a short comment. If a value uses something `CommentProfile::parse` rejects
   (allowed flags are `nbfsmelrOx`, digits and `-`), adapt it minimally and say so.
3. Coverage: every comment-bearing language in `is_primary` (languages.rs), plus
   obvious siblings (e.g. kotlin, dart, scala, groovy, proto, jsonc/json5, scss,
   hcl/terraform, zsh/ksh/dash/mksh, perl, r, haskell). Report each language you
   deliberately leave without a profile and why (e.g. json, diff, mixed-language
   astro/vue).

## Problems to resolve, not paper over

- **`css`** is currently in the C-family list, but CSS has no `//` comments; scss does.
- **Quote tracking:** `CommentProfile::trailing_line_leader` treats `'`, `"` and
  `` ` `` as quotes. That breaks Vim script, where `"` is the comment leader, and
  may misfire for other languages (shell `$#`/`${#x}`, Ruby `#{}`, Lua
  `--[[ ... ]]`). Decide whether quote characters must become per-language data.
  Either fix it or document the limitation with a test.
- **Hard-coded `//`:** src/core/command/indentation.rs (~line 133) hard-codes
  `leader.text.starts_with("//")` for trailing-comment `o`/`O` continuation.
  Generalize it or justify keeping it, with tests.
- **Leader ordering and `b`:** longer overlapping leaders must come first
  (`---` before `--`, `##` before `#` if used). `b:` matters for `#!` shebangs,
  `#include`-like lines and `#{`.
- **Lua's long brackets** (`--[==[`) can't be expressed as a three-piece comment;
  follow Vim's choice of line leaders.
- **Intended behaviour changes that need tests:**
  - `J` now strips repeated leaders in these languages.
  - `gq` recognizes their comment paragraphs.
  - Enter/`o` continuation applies only when the `r`/`o` flags are enabled.

Audit every consumer of `ReflowOptions::profile()` / `comments()`:
- src/core/document/reflow.rs
- src/core/command/line_mode.rs (~1802)
- src/core/command/ex_regions.rs (~465)
- src/core/command/mod.rs (~12663)
- src/core/command/indentation.rs (~131 and ~624)
- src/core/command/reflow.rs

Comment context must stay independent of syntax highlighting.

## Constraints

Follow AGENTS.md. Don't touch highlighting, the Viem Regex dialect, detection
rules or the language catalogue membership. If you find a catalogue or alias
gap, report it rather than fixing it here. No FFI or native-frontend changes
should be needed; if they are, stop and explain why.

## Tests

- Replace the unit test `every_c_family_language_recognizes_its_comment_leaders`
  (reflow.rs ~1259) with one that iterates the whole table. For each language,
  check that a two-line comment paragraph joins and wraps, and that block forms
  keep their delimiters.
- Rework `languages_without_a_profile_reflow_comments_as_prose` (~1285) to cover
  languages that still have no profile.
- Add integration tests in tests/all/ (registered in tests/all/main.rs; extend
  hard_line_reflow.rs where it fits). Cover the command entry paths: `gq`, `gw`,
  `J`, Enter/`o`/`O` with continuation enabled, generated block close, and leaders
  inside strings and shebangs not being recognized.
- Also test: explicit override surviving a language change, `:set comments&`,
  and `:set comments=`.
- Run `cargo test --test all hard_line_reflow::` plus the relevant lib unit tests,
  then the full suite. Build and test commands are in docs/macos-development.md.

## Docs

- Update the "Comment definitions" section of docs/code-formatting.md. It
  currently says only C-family languages get a default.
- Update AGENTS.md only if a requirement changes.

## Deliverable

A summary listing:
- each language → its value and the ftplugin it came from
- adaptations you made to Vim's values
- languages deliberately left without a profile
- user-visible behaviour changes
- remaining limitations
