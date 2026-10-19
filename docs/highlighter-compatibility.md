# Highlighter compatibility work

Every bundled language entry point should load and implement the highlighting
semantics its syntax uses. This target is not yet met. The language catalogue is
currently derived from the runtime manifest, not verified provider availability.
A successful load only covers the setup branches taken by that input; even exact
coverage can contain incorrect groups. Native presentation continues to follow
Viem's literal-source, theme, folding, spelling, and explicit-link policies.

## Shared interpreter and repaired setup gaps

Editor scripts, syntax files and imported color schemes now use the one
[`vimscript` interpreter](../src/core/vimscript/mod.rs). The separate setup
expression parser, value evaluator and function executor have been removed.
The [confined syntax host](../src/core/document/syntax/vim/loader/script.rs)
provides declarations, options, immutable document queries, and pinned resource
imports without exposing editor effects or ambient file/process access.

This repairs shared failures involving dictionary functions, lambdas, collection
identity, interpolated strings/heredocs, command wrappers, `execute()`, and
synchronous declarations followed by queries. `getline()` accepts the numbered
and `$` forms used by the runtime. Included scripts and lazy autoload helpers
share the same runtime while retaining script-local scope. `exists()` does not
source an autoload file. The manifest now pins the Ada, FreeBASIC and Modula-2
autoload dependencies at the original runtime revision.

Setup uses private modeled options and a scratch query cursor. Syntax events
resolve bundled language resources. It never changes a user's view. Highlights
retain declaration precedence and distinguish the `NONE` unlink token from a
group named `None`. Large keyword lists and provably exclusive, word-delimited
literal unions split into bounded rules; arbitrary oversized patterns remain
subject to the ordinary limits. Numeric lookahead modifiers use Vim's semantics.

## Outstanding work

Successful loading still does not establish complete highlighting compatibility.
These architectural gaps remain:

| Area | Bundled evidence | Required work |
| --- | --- | --- |
| Region and match offsets | Java/Groovy, Tcl, Procmail, RST, Yacc and dependents | Match Vim's offset scheduling, including valid negative starts/end offsets, without rewriting published exact coverage. Verify containment, skips, boundaries and incremental checkpoints against Vim. |
| View-dependent patterns | `bst.vim`, `gitconfig.vim` use `\%#` | Carry actual view cursor identity and invalidate dependent results. The private setup cursor is not a substitute. |
| Composing-character matching | `m17ndb.vim` uses `\Z`; Vim also defines `\%C` | Implement composing-character semantics in both matching paths, including leading combining marks and ordinary literal behavior. |
| Vim9 branch conformance | Some bundled files use Vim9 declarations; XPM contains a typed helper | Typed calls, scopes, imports and object types now use the shared evaluator. Continue differential coverage of alternative setup branches; the language fixtures and a passing default branch do not prove every bundled script path. |
| Setup host coverage | Alternative runtime branches, option combinations, highlight queries and buffer-sensitive helpers | Add named nonempty fixtures and differential branch coverage; successful empty-input loads do not prove every branch. Keep effects confined and resource budgets explicit. |

The top-level `chicken.vim` is supplemental Scheme syntax and produces no rules
when loaded alone. It is currently exposed by the manifest-derived catalogue.
Audit it through Scheme with its intended setup and model that ownership in the
catalogue; an empty program is not evidence that the dialect works.

## Embedded languages and Tree-sitter

The bundled Tree-sitter packages have tests that compile all their composed
queries and parse/highlight a representative fixture for each grammar. This
does not establish the availability of every child requested by those queries.

Fixed injection targets without a bundled provider include `comment`, `re2c`,
`printf`, `jsdoc`, `groq`, `glimmer`, `styled`, `regex`, and `angular`.
`latex` now resolves to the bundled `tex` provider, and Doxygen loads through
the shared interpreter. Dynamic captured language names need the same dependency
accounting. Missing children retain host colors, so this loss is easy to miss
in visual smoke tests.

Injection discovery now checks resource availability without compiling a Vim
program against an invented default context. The child compiles against its
actual immutable source, retains setup-query dependencies, and recompiles when
a dependency changes. A regression exercises a context-dependent fence and
compares its edited result with fresh analysis.

## Acceptance and audit procedure

Use the [Vim profile's audit command](../src/core/document/syntax/vim/PROFILE.md#runtime-compatibility-audit).
Keep generated JSON, fixtures and comparisons under `target/validation/`, with
source identity, runtime manifest digest, command, input, oracle version and
capability settings. Do not modify the pinned upstream syntax to obtain passes.

1. Enumerate language entry points, aliases, utilities and owner-only helpers.
   Compile each language with an empty snapshot and representative named input.
   Record transitive dependencies and zero-rule results. Check helpers through
   owners; standalone helper failures must not inflate language-failure totals.
2. Add branch fixtures for filename/dialect selection, buffer queries, embedded
   languages and setup options used in these files. First-failure reporting must
   be followed by another audit after each shared evaluator fix.
3. Compare effective groups byte for byte with a reference Vim on identical
   source and filename. Normalize deliberate presentation choices explicitly:
   TeX's conceal-generated superscript groups differ from literal-source groups.
   Record oracle setup errors separately. Pin the oracle for release evidence;
   system Vim of a different patch level gives narrower diagnostic evidence.
4. For repaired features, assert standalone and injected behavior, incremental
   versus fresh results, cancellation, bounded work, and stale-result rejection.
   Load tests alone cannot catch a matcher or group-resolution defect.
5. Make the full inventory an acceptance gate once its known failures are fixed.
   Until then, retain an explicit failure inventory and reject new failures;
   neither hiding picker entries nor silently ignoring syntax repairs support.

Continue with view-dependent atoms and offset scheduling as architectural work,
then close the remaining injection providers and fixture/branch inventory.
Keep intentional non-highlighting effects (concealment, folding, editor plugins)
separate from missing highlighting support.
