# Highlighter compatibility work

Every bundled language entry point should load and implement the highlighting
semantics its syntax uses. This target is not yet met. The language catalogue is
currently derived from the runtime manifest, not verified provider availability.
A successful load only covers the setup branches taken by that input; even exact
coverage can contain incorrect groups. Native presentation continues to follow
Viem's literal-source, theme, folding, spelling, and explicit-link policies.

## Outstanding work

These are concrete gaps found by compiling the pinned syntax inventory, reading
its dependencies, and comparing representative source against Vim. The listed
failures are first active failures, not exhaustive diagnoses of each program.

| Area | Bundled evidence | Required work |
| --- | --- | --- |
| Setup expression and function semantics | `java.vim:130` calls a dictionary function; `shared/debarchitectures.vim:21` uses lambdas; `help.vim:18` uses `#{}` and later method calls; `csv.vim:17` uses interpolated strings; `neomuttrc.vim:441` mutates a local collection | Extend the bounded setup evaluator with correct identities, scope and execution order. Cover legacy and the syntax-setup subset of Vim9 without enabling a general editor interpreter. |
| Builtins and document context | `cs.vim:163` uses `getline('1')`; `gitrebase.vim:13` uses `getline('$')`; `sed.vim:60` calls `char2nr()`; `hostconf.vim:84` calls `remove()`; `tera.vim:13` calls `strridx()` | Implement supported argument forms and deterministic helpers. Test filename, prefix, EOF and edit-dependent branches; retain query dependencies for successful and failed loads. |
| Declarations inside functions and wrappers | `ant.vim:41`, `doxygen.vim:407`, `haml.vim:18`, `pandoc.vim:223`, and `cucumber.vim:132` fail on function execution, wrappers, command attributes or parsing | Use an ordered, bounded setup execution model. Validate declarations and reads as they occur; deferred output cannot substitute for synchronous setup semantics. |
| Runtime dependency closure | Ada reads `g:ada#Keywords`; FreeBASIC and Modula-2 call autoload functions; R probes `globpath(&rtp, ...)`; DocBook uses filetype autocommands; Cmod/Pike load `matchit` | Inventory and pin required helper data, not only `syntax/`. Provide confined read-only resolution and explicit adapters for needed setup effects. Do not enable arbitrary plugins, host runtime discovery, or process execution. |
| Options and capability queries | `acedb.vim:110`, `fasm.vim:18`, `gift.vim:16`, `dune.vim:17`, `spec.vim:172`; `exists('*...')` and `has(...)` in setup | Separate syntax-affecting options from passive editor hints. Model the former and validate the latter without mutating views. Capability answers must describe the host consistently rather than accidentally skipping required rules. |
| Regex and region behavior | `bst.vim:19` and `gitconfig.vim:21` use cursor assertions; `m17ndb.vim:21` uses `\Z`; `raku.vim:184` uses a numeric lookahead modifier; Groovy, Tcl, Procmail and others use rejected region offsets | Add proven matching semantics and offset handling. Cursor-dependent syntax needs view identity/invalidation because the current syntax service is shared by a buffer. Do not substitute a fixed cursor or silently drop atoms. |
| Default resource budgets | Clojure's keyword data exceeds the setup list limit; Hollywood exceeds the pattern-byte limit | Measure and change representation or compilation strategy while preserving finite work/memory. A blanket limit increase is not a compatibility proof. |
| Highlight link semantics | `zsh.vim:339` links `zshVariable` to `None`; native code treats every case spelling as the `NONE` unlink sentinel | Match Vim's declaration-token semantics separately from case-insensitive group identity. Compare resulting group names as well as theme resolution. |

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
`latex` also lacks a provider/alias even though `tex.vim` exists. Doxygen is
present but fails its Vim load. Dynamic captured language names need the same
dependency accounting. Missing children retain host colors, so this loss is
easy to miss in visual smoke tests.

The Vim injection availability check in `service/providers.rs` compiles with a
default context and caches a Boolean by language. This rejects languages whose
setup requires a document even when their actual child snapshot would load
(for example Git commit). Separate resource availability from child-context
compilation, and key negative results by their real dependencies. Test a fenced
language with context-sensitive setup, then edit the relevant context.

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

Start with shared setup failures affecting Java and its dependents, the missing
`getline` forms and deterministic builtins, then dependency/option adapters.
Handle view-dependent atoms and retroactive offsets as architectural work.
Keep intentional non-highlighting effects (concealment, folding, editor plugins)
separate from missing highlighting support.
