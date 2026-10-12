# Code formatting and comments

`gq` and `gw` reflow selected hard lines in Text and Code. `textwidth` defaults
to 80 logical columns; changing it does not enable automatic wrapping. Markdown
Source and WYSIWYG reflow remain unsupported. Fonts and soft wrapping do not
change reflow boundaries or widths.

Code buffers additionally use Vim's `formatoptions`, `comments`, and
`formatlistpat`. These options are shared by every view of a buffer and do not
enter document undo. Text retains its existing prose/list rules; Markdown's
structural editing does not use these options.

The Code default is `formatoptions=qj`. Supported flags are:

| Flag | Behavior |
| --- | --- |
| `q` | Recognize configured comment leaders during `gq`/`gw`, preserving leaders on continuation lines. Without it, comments reflow as ordinary text. |
| `j` | Remove a repeated comment leader when `J` or `:join` joins compatible comment lines. `gJ` and `:join!` remain literal. |
| `r` | Continue comments after Enter in Insert mode. |
| `o` | Continue comments when opening lines with `o`/`O`. |
| `/` | With lowercase `o`, suppress continuation of every trailing line-comment kind. Enter and full-line comments still continue. This deliberately extends Vim's `//` rule. |
| `n` | Recognize list prefixes using `formatlistpat` and give their continuations a hanging indent. Requires `autoindent`. |
| `2` | Use the paragraph's second-line indentation for its continuations. Requires `autoindent`. |
| `w` | Treat trailing whitespace as a paragraph-continuation marker; wrapped continuation lines retain a trailing space. |
| `1` | Keep one-letter words, including a run of them, with the following word when choosing a wrap. |

For example:

```vim
:set fo+=ro
:set fo+=/
:set fo-=j
:set fo?
:set fo&
```

`+=`, `^=`, and `-=` modify values; `?` queries them. `&` and `<` return to
inherited defaults. Unsupported flags and invalid option values fail atomically,
including a `:set` command that also contains otherwise valid changes. The
existing comment-continuation settings and `continuecommentsonenter` /
`continuecommentsonopenline` options control `r` and `o`; an explicit
`formatoptions` override survives application-default changes.

## Language context and formatting profiles

Comment editing has two independent inputs. A bounded lexical provider identifies
native comments, strings and other protected literals. The effective `comments`
profile chooses which eligible leaders to preserve, strip, repeat, or close.
Syntax highlighting can be disabled or unfinished without changing these rules.
No external formatter, interpreter, or Vimscript runs.

[Comment language profiles and coverage](comment-language-coverage.md) records
every default, its pinned Vim 9.2 reference, deliberate adaptations, and the
languages using literal-leader compatibility. Selecting a language updates its
inherited profile; the language catalogue and file detection rules are unchanged.

Each comma-separated entry has `flags:literal-leader` syntax. Repeated leaders
may use `b` (require a following blank), `n` (nested leaders), `f` (first line
only), and `O` (do not continue above). Three-piece comments use consecutive
`s`, `m`, and `e` entries. Alignment and numeric offsets control generated
leaders; `x` allows the final character of the end marker to close an untouched
generated middle. Matching honors configured order across line and block
leaders. Put longer overlapping leaders first. Unsupported flag combinations
fail atomically; the linked inventory documents the audited subset.

Native comment boundaries take precedence over formatting preferences. Manual
`///`, `//!`, and `// TODO:` can customize a native `//` start. A nonnative `#`
in C++ can establish a custom formatting region in code. It cannot establish one
inside a string or an existing native comment. Configuring `comments=:"` cannot
redefine C++ strings. A native marker used where the language forbids a comment,
such as shell `$#`, does not become a custom comment merely because it matches
the profile. Block middles decorate the containing block, with its identity
preserved through nested native comments. A manual block rule whose generated
leaders would open or terminate native syntax incorrectly is ineligible in that
language; it does not redefine the native grammar.

### Manual overrides

The setting is buffer-local and shared by all views; changing it is not a source
edit and does not enter undo. Assignment replaces the whole profile, including
assignment equal to the current default. An explicit empty value disables
comment-specific assistance but retains lexical knowledge and highlighting.
`comments&` and `comments<` restore inheritance from the current language.

```vim
:set comments=
:set comments=:#
:set comments&
```

`+=`, `^=`, and `-=` modify the current effective value, then store the complete
result as an override. For example, adding a marker to C++ retains the C++
profile plus that marker after selecting Python; it is not a live addition to
Python's defaults. Empty overrides also survive language changes and selecting
no language. Profiles never implicitly merge with a later language default.

### Reflow, joining, and continuation

With `q`, `gq`/`gw` reflow eligible full-line comment paragraphs and block bodies.
Distinct documentation leaders, blank comment lines, paragraph/list boundaries,
block owners, and delimiters remain distinct. Lines mixing code with a trailing
comment remain byte-identical during reflow; body-only trailing reflow is deferred.
Native comments excluded by a nonempty profile remain unchanged. Disabling `q`
or setting an empty profile retains ordinary explicit text reflow.

With `r`, Enter continues inside an eligible comment, including a trailing comment
after code, preserving the code prefix and aligning its continuation. Lowercase
`o` continues trailing line comments unless `/` is set. Uppercase `O` never
copies a trailing line comment above its code statement. Openers, middles,
closers, first-only leaders, and part-specific `O` flags keep their separate
rules. Closed blocks resume the opener's indentation.

`J` and `:join` with `j` remove eligible repeated leaders according to the existing
join rules; unlike reflow, joining may remove a different repeating line-leader
spelling. First-only leaders are not stripped. Block joining requires the same
native/custom owner. `gJ` and `:join!` remain literal.

Insert-mode generated closing retains the actual generating block and resolved closing
spelling, including whitespace-only middles and blocks sharing a middle or final
closing character. It replaces that middle at its existing column; end offsets
apply to opening above a closer. A source revision, language, or effective-profile
change retires pending closing. Paste and IME commits bypass per-keystroke
continuation assistance. Counts and repeat resolve fresh context each time;
ordinary Insert undo grouping is preserved. Replace mode retains its existing
literal typing of closing characters and its replacement/Backspace journal.

### Bounded context and fallback

Language providers use immutable snapshot identities and reusable checkpoints.
Edits retain only verified unchanged-prefix state. Cache eviction changes
performance, not classification. Work is bounded by lines, bytes, nesting, and
retained results. Each native query scans at most 4096 lines or 512 KiB, with a
256 KiB limit per line and nesting limited to 128. An arena retains at most four
revision caches; each has four languages, with 256 checkpoints and 32 line results
per language. A cold far-away position can be unavailable until bounded
queries establish preceding state. Comment processing does not flatten the
whole document or wait for highlighting/background workers.

Insufficient budget, incomplete ambiguous syntax, or an unsupported form within
a supported provider yields **unknown**. Enter and `o`/`O` then use ordinary
indentation without leaders; `J`/`:join` join without stripping comment leaders.
Comment-aware `gq`/`gw` reject the entire requested transaction, preserving source,
cursor, selection, registers, mode, history, and repeat state. There is no silent
downgrade to generic matching.

Languages without a provider use **literal-leader compatibility**. Configured
leaders still support formatting and continuation, but this mode does not promise
string, heredoc, nesting, or embedded-language protection. Custom formatting
blocks also require bounded proof from the document start (4096 lines and 256 KiB
per operation, with a 256 KiB current-line cap); exceeding it produces unknown
instead of a guessed block state.
Native blocks use the incremental lexical checkpoints. Unknown state is never
presented as ordinary code.

Exact native body and delimiter ranges are available for future Markdown in
comments. Rendering, parsing, and editing Markdown in those bodies remain deferred;
custom formatting markers are not automatically eligible native comment bodies.

## List prefixes and limits

`formatlistpat` defaults to Vim's numbered-list pattern,
`^\s*\d\+[\]:.)}\t ]\s*`. To recognize bullet prefixes instead:

```vim
:set fo+=n
:set formatlistpat=^\s*[-+]\s\+
```

Patterns use the existing bounded Vim regular-expression engine, including
magic switches and `\ze`; no Vimscript or external formatter runs. The match
must start at the beginning of the body, consume a nonempty prefix, and leave
body text. A match ending inside a grapheme is rejected. Unsupported pattern
atoms, invalid expressions, and work-budget exhaustion produce an error rather
than silently changing the pattern's meaning. Reflow remains one verified,
undoable transaction and preserves untouched source bytes and encoding.
