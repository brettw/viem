# Installed Vim syntax audit

The audited directory is the configured MacVim 9.1.1887 runtime:

`/opt/homebrew/Cellar/macvim/9.1.1887/MacVim.app/Contents/Resources/vim/runtime/syntax`

The inventory contains 774 `.vim` files: 763 top-level language packages and 11
nested helper fragments. The user's `~/.vim` contains `colors/bretts.vim`, a
color scheme rather than a syntax package.

The audited runtime is now copied unchanged into `assets/vim/runtime/syntax`
for distribution. `assets/vim/manifest.json` records the source distribution
and exact file hashes; the original runtime license is included. The report
below records the original audit, while its reproduction command now uses the
checked-in copy and needs no installed MacVim.

## Measured result

With native profile 3, **663 of 774 files compile**, up from 235 before the
changes. The remaining **111 files are rejected** with explicit diagnostics.
This includes 655 accepted top-level packages and 8 accepted helper fragments;
108 packages and 3 standalone fragments remain rejected. The run took 4.0
seconds, reached no watchdog deadline, and retained every baseline success.

The complete [per-file audit JSON](vim-syntax-audit.json) records every accepted
file and every remaining diagnostic, including the actual source filename and
line when a package fails in an included dependency. The results cover default
setup branches and compilation only, as described below.

| First blocker category | Rejected files |
| --- | ---: |
| Setup expressions, functions, commands, options, or unavailable context | 93 |
| Retroactive region offsets | 8 |
| Pattern language or declaration | 4 |
| Syntax macro options | 3 |
| Resource limits or unsupported pattern-limit qualifiers | 3 |

The most repeated first blockers are Java's `QueryFoldArgForSyntaxItems` helper
(9 entries), searches requiring flags beyond the bounded non-moving search
interface (8), retroactive region offsets (8), environment-variable expression
syntax (6), and remaining YAML setup-expression forms (5). Editor commands,
autoloaded helper functions, visual/cursor-dependent regex assertions, and
reads outside the supplied document context account for other failures. The
JSON contains the full inventory of these remaining failures.

Final review also repaired very-nomagic delimiters in `just.vim` and quoted
throw messages in inactive version guards in `raku.vim`. Just now compiles.
Raku advances to its first unsupported pattern at line 184: a numeric qualifier
on a negative lookahead (`\@2!`).

## Validation

The complete syntax suite passes **155 tests**, with 2 ignored performance
tests. All **4 native `EVVimOpeningTests`** pass, including opening and painting
the installed Makefile syntax while preserving source bytes, and full-path
setup followed by Save As. The pinned Vim release performance test passes at
one million lines; its largest measured edit repair on this machine was
**0.911 ms**.

## Reproduction and scope

```sh
cargo run --release --offline --example audit_vim_syntax -- \
  assets/vim/runtime/syntax \
  > target/vim-syntax-audit.json
```

An optional second argument filters relative file names. The example invokes
the same native compiler and default limits used by the application; it does
not execute the inspected scripts in Vim. Each file has a ten-second
cancellation watchdog. JSON retains every compiler diagnostic's source file,
logical statement's first physical line, message, and category. Progress goes
to stderr. Included dependencies remain attributed to their actual files.
The compiler now stops each package at its first active failure, so the remaining
inventory identifies the first blocker in each file and can conceal subsequent
features. The baseline compiler collected up to 256 diagnostics per package;
its diagnostic totals include cascades and cannot be compared directly with the
new first-blocker totals.

Top-level packages are loaded with their normal transitive dependencies and an
empty setup prefix and no document filename. Nested files are compiled independently as helper
fragments; missing script-local context or includes in that mode do not mean
the fragment fails when loaded by its owning package. The audit does not enable
optional globals or exercise every buffer-dependent setup branch. Successful
compilation means declarations were accepted; it is not evidence of complete
highlighting equivalence. Differential execution tests provide that evidence
for their particular inputs.

## Original failure and baseline

Before these changes, 235 files compiled and 539 were rejected. The 10,682
reported diagnostics corresponded to 4,556 distinct source file/line/message
triples; transitive dependencies account for repetition. The run took 4.6
seconds and no file reached the watchdog deadline.

`make.vim` had five diagnostics:

| Source line | Missing capability |
| --- | --- |
| 73, 82, 99 | Case-insensitive syntax option names (`nextGroup=makeCommands`) |
| 156, 157 | Named synchronization states (`groupthere makeCommands`) |

After compilation was repaired, byte-by-byte comparison also exposed the
region-body offset convention: `rs=e-1` must leave the Make target's colon
inside its delimiter highlight. Region-body boundaries (`rs`/`re`) now have a
separate conversion from character-based match/highlight offsets.

Two further execution cases were repaired: a transparent region's end inherits
the body when its matchgroup is the region's own group; and a `keepend` parent
with no separate end delimiter group retains contained coloring through the
end pattern. That second case preserves the last character of a trailing
comment on continued Make dependencies. External delimiter patterns also
use the final syntax keyword environment when instantiated, including
environment changes made after their declaration.

Included syntax files now retain the parent declarations when they call
`syntax clear`, matching Vim's include behavior. Nested include clusters own
their immediate declarations; synchronization group names participate in those
clusters while synchronization patterns remain recovery hints. Regression tests
also cover ignored offsets on the start, skip, and end patterns, trailing offset
commas, and rejection of a region-end offset that would require changing colors
already published before the end pattern.

A separate source/reference audit found 27 installed files with literal syntax
group references whose letter case differs from their declaration, including
`gitconfig`, `gitattributes`, `vim`, `dosbatch`, and `plsql`. Native Vim treats
group and cluster identities case-insensitively while retaining their first
spelling. Declaration-time name interning now preserves those identities across
containment, nextgroup, highlight links, cluster updates, and cycle checks.

Portable unit tests cover these cases independently. The unmodified Make
syntax file is pinned under `src/core/document/syntax/vim/fixtures/make.vim`,
with its original attribution and the adjacent Vim license. The native
differential test compares every non-newline byte for Make targets, variables,
directives, recipes, strings, define/endef, continued dependencies and trailing
comments, and malformed recipes. Existing `conf`, `dosini`, legacy Vim and
Vim9 samples remain in the comparison. The focused scanner suite and separate
pinned Make tests pass. The latter validate compilation
without any installed runtime and bounded edits in 10,000-line and
1,000,000-line documents. Beginning and distant viewports share the same
4-million-instruction, 128-KiB matcher-input, and 96-line ceilings; distant cold
results remain explicitly provisional. Deleting a target colon invalidates the
recipe context, repaired colors match a fresh exact local oracle, and cached
repaints use zero matcher instructions. The optional reference comparison uses
the pinned Make source and falls back to `/usr/bin/vim` when MacVim is absent.

## Baseline capability inventory

These counts are affected top-level or fragment audit entries, including entries
whose dependencies contain the feature. They overlap and must not be summed.

| Capability family | Representative baseline error | Affected entries |
| --- | --- | ---: |
| Highlight metadata | Direct highlight definitions rather than group links | 112 |
| Local setup options | `setlocal` | 88 |
| Synchronization | Named `grouphere`/`groupthere` states | 88 |
| Declaration names | Group names outside the initial identifier subset | 78 |
| Command spelling | `exec` abbreviation | 73 |
| Bracket expressions | Escaped quote inside `[]` | 70 |
| Setup calls | `call` and syntax-producing helper functions | 68 |
| Setup environment | `v:version` | 57 |
| Dependencies | Runtime globs and absent dependency paths | 56 |
| Variable lifecycle | Repeated removal of `b:current_syntax` | 51 |
| Setup expressions | Indexing, dictionaries, arithmetic and other expression forms | 48 |
| Synchronization | `ccomment` | 46 |
| Bracket expressions | Escaped dot inside `[]` | 45 |
| Setup control flow | `try`/`catch`/`endtry` | 41 |
| Generated declarations | Here-document setup data and loops | 35 |
| Pattern limits | Shared group-pattern predicate fuel exhaustion | 32 |
| Setup control flow | `for`/`endfor` | 31 |
| Pattern atoms | Additional percent atoms | 24 |
| Source encoding | `scriptencoding` | 24 |
| Setup environment | `&ft` and `&filetype` | 23 |
| Synchronization | `sync clear` | 20 |

Less frequent distinct families include syntax clearing and keyword options;
syntax command abbreviations and macros with different argument counts;
heredocs and Vim9script; pure helper function conditionals and local values;
nomagic modes, filename classes, numeric character escapes and alternate bracket
escapes; retroactive region offsets; concealment; editor commands, mappings,
autocommands, imports, and dynamic includes; direct color-scheme setup; and
resource caps on exceptionally large generated patterns/programs.

Some files in this directory are editor utilities (`2html.vim` and syntax menu
or color setup scripts), rather than self-contained highlighting declarations.
Executing those editor operations is outside the native syntax compiler's
contract. Unsupported behavior must continue to return a diagnostic rather
than silently produce an apparently complete program.

## Bounded document context

Production setup receives the current document filename and at most the first
32 complete hard lines, capped at 64 KiB. Filename changes, including Save As,
invalidate setup even when source bytes and detected language remain unchanged.
Edits beyond the prefix reuse the compiled setup. Provider tests compare those
changes with fresh compilation, and a native document test covers full-path
selection followed by Save As with unchanged source bytes.

The supplied context does not expose arbitrary document reads, total line count,
or the final line. Consequently, `line('$')` in `man.vim`, `getline('$')` in
`gitrebase.vim`, and the wider format-detection scan in `fortran.vim` remain
explicit limitations. Adding final-line/count values to every compilation key
would cause syntax recompilation on every newline or final-line edit; supporting
these cases correctly requires tracking which setup expressions depend on those
values. This audit does not substitute empty strings or guessed counts for them.

Numeric regex assertions for physical line and byte column are supported during
highlighting. They query the immutable input directly, and provider regressions
verify cache repair after line insertion and same-line edits, including Unicode
byte columns. Cursor positions and screen-relative positions still require
editor state outside the native syntax input.
