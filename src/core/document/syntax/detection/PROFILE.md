# Portable language detection profile

Profile version 4 uses decoded normalized physical source and never executes Vimscript,
filetype autocommands, shell commands, regular expressions, or package callbacks.
An explicit language (including None) precedes modelines, user filename rules,
bundled filename rules with content disambiguation, shebangs, and finally content
signatures. Modelines retain unknown language names instead of guessing another.

Bundled filename rules match basenames, using the curated tables in
[`filenames.rs`](filenames.rs). Exact names precede compound suffixes and final
extensions. Windows Batch (`.bat`/`.cmd`), PowerShell, Windows script host, INI, HTML,
and Just extensions have explicit ASCII case-insensitive rules. Other rules
remain case-sensitive, including `.c` versus `.C` and `.h` versus `.H`.
User globs still match case-sensitive basenames only; directory components never
participate. Files without a recognized name or marker retain the Text fallback.

The filename audit compared the bundled syntax inventory with the literal maps
and declarative rules in Vim 9.2's `filetype.vim` and `autoload/dist/ft.vim`.
Common script, programming-language, build, markup, and data filename rules are
recorded explicitly. MSBuild project/resource and NuGet manifest suffixes use XML.
Tests ensure every rule targets a bundled language, tables are sorted and unique,
and overrides, source bytes, explicit formats, and filename case retain their
semantics. The syntax-file inventory is not an extension list: many names are
aliases, dialects, or helpers, and syntax availability does not guarantee that
Viem's bounded native compiler supports every declaration in that syntax file.

Broad `Dockerfile.*`, `Containerfile.*`, `Makefile.*`, and `*vimrc* variants
run after recognized extensions: `Dockerfile.py` and `vimrc.py` select Python.
Exact `.exrc`, `_exrc`, and `.netrwhist` names also select Vim. Compound suffixes
include `.cmake.in` and specific MSBuild project `.user` files; generic `.user`,
`.config`, and `.conf` files are not treated as project XML.

Git's exact `COMMIT_EDITMSG`, `MERGE_MSG`, `SQUASH_MSG`, `TAG_EDITMSG`,
`NOTES_EDITMSG`, and `EDIT_DESCRIPTION` basenames select Git Commit (`gitcommit`),
including inside worktree Git directories. These names are case-sensitive and
do not match backup suffixes or parent-directory names. Explicit formats,
language choices, modelines, and user filename associations retain precedence.

The audit deliberately leaves ambiguous suffixes such as `.tex`, `.r`, `.f`,
`.d`, `.cl`, `.cls`, `.edn`, `.sc`, `.pp`, `.tf`, and `.reg` without new blanket
rules. They need content disambiguation or a user association. Likewise, `.obj`,
`.pdb`, `.mat`, and `.mo` can name binary files; do not infer text languages from
those suffixes. Extensionless `.prettierrc` and `.stylelintrc` can be JSON or YAML.
Formats without a bundled highlighter (for example Elixir, F#, Svelte, and `.sln`
solution files) are not added as automatic selections. Path-specific Vim rules
such as `.git/config` cannot be reduced to a generic `config` basename rule.

These are portable declarations, not execution of a configured Vim runtime's
`filetype.vim`. Builds and startup have no installed-Vim or network dependency.

## Keeping language support and filename rules in sync

Treat language additions, removals, renames and runtime/package updates as a
review of all three layers below. Update affected layers in the same change;
shipping a highlighter or a menu entry alone does not provide auto-detection.

| Layer | Source of truth | What it provides |
| --- | --- | --- |
| Language catalogue | [`../languages.rs`](../languages.rs) reads the Vim manifest and adds explicit non-Vim entries | Canonical IDs and names shared by native language menus |
| Filename detection | [`filenames.rs`](filenames.rs), aliases/shebangs in [`../detection.rs`](../detection.rs), and disambiguators in [`profile.rs`](profile.rs) | Filename/marker to canonical language selection, independent of the backend |
| Highlighting | [Bundled Vim assets](../../../../../assets/vim/README.md), [Tree-sitter packages](../treesitter/PROFILE.md), and [`../service/providers.rs`](../service/providers.rs) | Syntax runs for the selected language, subject to compatibility and work limits |

Vim `runtime/syntax/*.vim` files describe pattern-based highlighting and bounded
setup, interpreted by Viem's native Vim compiler. They are not extension maps;
the snapshot includes helpers and dialects, and does not run Vim's `filetype.vim`.
Tree-sitter uses compiled grammar dependencies and separate `.scm` highlight and
injection queries. Adding a `.scm` file alone does not register a grammar or a
language. Its bundled package set is smaller than the Vim catalogue. Vim script
uses the Vim provider; other languages prefer a compatible Tree-sitter package
and use available Vim coverage or default styling when needed. Neither backend
should own a duplicate extension table, and neither needs a counterpart for
every supported language.

For each affected language:

1. Verify the canonical ID, catalogue entry and aliases. Check
   `canonical_language` in `detection.rs`, Tree-sitter's package aliases, and
   `vim_language` in `service/providers.rs`; these IDs need not match the upstream
   syntax/grammar filenames. A new Tree-sitter-only language needs an explicit
   catalogue entry because the catalogue is not derived from that registry.
2. Audit its common extensions, compound suffixes and special basenames, then
   update `filenames.rs` with canonical targets. Keep tables sorted and unique;
   choose casing deliberately. Preserve precedence, user overrides and unknown
   Text fallback. Do not assign every syntax-file stem as an extension. Use
   bounded disambiguation for shared suffixes, or document the intentional
   omission here with the other ambiguous cases. Helper/injection-only entries
   do not require standalone filename rules. Review stale mappings on removal.
3. Follow the relevant provider's update guide, preserving pinned upstream bytes,
   licenses and provenance. Validate actual loading/highlighting for new support;
   an inventory entry does not prove compiler compatibility. Existing filename
   rules remain valid when merely adding a second backend for the same language.
4. Extend detection cases in `detection.rs` and opening/source-preservation cases
   in `tests/all/code_syntax.rs` for new automatic selections. The table integrity
   test checks valid catalogue targets, sorting and uniqueness; it cannot detect
   missing real-world extensions, so new filename examples are required. Run
   `cargo test --locked --lib document::syntax::detection::` and
   `cargo test --locked --test all code_syntax::`, plus the relevant provider
   checks. Update `PROFILE_VERSION` when detection behavior changes.

Keep the filename policy portable in the Rust core. Native menus consume the
shared catalogue and must not introduce their own language/extension inventories.

## Content profiles and bounds

DetectionProfile compiles an ordered declarative list of ContentSignature and
ExtensionDisambiguator values. Its default includes the PHP/XML signatures and
C/C++/Objective-C header and Objective-C/MATLAB disambiguators. An unresolved .h
uses C and an unresolved .m uses Objective-C. Registration inserts a new rule
before existing rules; the most recent matching registration wins within that
stage. User filename associations still precede every extension disambiguator.
A custom extension disambiguator can also recognize an otherwise unknown suffix.

A rule specifies a language, a literal, and one of three match scopes: anywhere
within a sampled complete line, after whitespace in logical line zero, or after
leading whitespace in the first nonempty sampled line. Literals never cross
logical line boundaries. Rules are case-sensitive; language aliases are
canonicalized after selection. No regex dialect or arbitrary native callback is
part of this interface.

The detector reads at most 64 KiB total, from complete first/last-five lines
inside fixed 32 KiB head/tail windows. Core samples physical source windows and
also caps decoded UTF-8 at 64 KiB, so Markdown presentation cannot change the
samples. It uses line indexes and bounded slices;
it skips a giant or truncated line. Modelines, shebangs and registered content
rules reuse these samples. Content rules share one compiled finite automaton
pass; they do not reread or join document ranges. A full-capacity adversarial
fixture verifies at most twice the sample byte count plus 64 output steps for
one line. General scan work is at most twice the sampled bytes plus 64 output
steps per sampled line, with at most ten sampled lines.

Compilation accepts at most 64 rules and 4096 total literal bytes, at most 256
bytes per literal and 128 per language/extension. It rejects empty, multiline or
NUL literals and invalid identifiers. Its trie has at most 4097 states, and
failed registration preserves the prior profile. Filename globs retain their
separate bounded count, pattern size and shared matching fuel.

A caller can use detect_with_profile directly or install an Arc<DetectionProfile>
through Core::set_code_detection_profile. Installation is an explicit policy
change and redetects an initialized buffer without changing source or undo.
Ordinary edits do not rerun detection. The detect wrapper uses the shared bundled
profile. Additional package integrations can supply this portable policy without
adding platform UI or C ABI dependencies.
