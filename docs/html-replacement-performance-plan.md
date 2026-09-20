# Plan: local HTML replacement context

Status: implemented with measured acceptance tests. See
[the results and remaining limits](html-replacement-work-results.md) for
end-to-end work counts, native validation, and reproducible profiling commands.
The original implementation plan follows.

## Problem and measurable goal

`Document::replacement_typing_context` and `html_typing::replacement_insertion`
currently materialize the complete source, decode/normalize it, tokenize it,
and walk the token stack to recover inline scopes. A short replacement in a
large HTML document can therefore scan the document twice, even when the actual
source patch and projection update are local. IME preparation can repeat this
work. The retained replacement context is small; the expensive part is finding
it and finding the insertion site's current scopes.

The goal is logarithmic lookup plus the actual inline nesting depth and changed
source region. Unrelated paragraphs and the untouched prefix of a multi-megabyte
paragraph must not be scanned for a small replacement. Continued typing must
stay local too. This work should not change editing, formatting, source spelling,
history, or encoding behavior.

Some other HTML structural/style operations also perform complete parses. This
followup must measure their contribution to the complete replacement operation;
optimizing the context lookup alone does not establish that every replacement
is bounded. Legitimate nonlocal HTML recovery and global style changes remain
separately identified workloads.

## 1. Establish an end-to-end work baseline

Instrument the replacement turn from selection capture through the first
committed input and subsequent typing, including scratch transactions and IME.
Extend the existing work counters rather than counting only candidate projection:
context lookup happens before preparation and currently escapes those totals.

Record source bytes materialized, decoded and tokenized; scope entries visited
and copied; index maintenance; projection work; and retained index memory.
Record why a query or transaction fell back to a broader parse. Keep counters
available in tests without making production queries allocate diagnostic logs.

Fixtures should replace one character near the start, middle and end of 1,000,
10,000 and 100,000 short HTML paragraphs, plus a multi-megabyte paragraph. Include
plain/bold/link boundaries, deep nesting, large attribute values, UTF-16, entities,
and two views. Measure the initial open separately from repeated edits. Use
wall-clock measurements as supplementary diagnostics, not the sole test oracle.

Deliverable: reproducible current work counts and failing bounded-work tests
for the two context queries and the full native/IME replacement path.

## 2. Retain a source-backed scope index during projection

Add a portable HTML scope index under `src/core/document`; retain it with the
immutable document/projection state. Build it during the existing tokenization
and HTML tree-construction work, with no second complete decode pass.

The index should support three related questions:

- Which paragraph owner and structural ancestors belong to the beginning of
  the selection, including an empty first paragraph?
- Which source-backed inline scopes give a selected text contributor its
  character/link context?
- Which lexical scopes surround an insertion boundary, and which adjacent
  closing tokens can be crossed without consuming source content?

Keep those questions distinct. HTML recovery can make semantic ancestry differ
from lexical nesting; a single naive stack is insufficient for misnested tags,
fostered table text, implied elements, and unclosed formatting.
The first selected character can also follow an empty paragraph or leading
paragraph separator. Its paragraph owner must not replace the independently
captured paragraph context. Full-document replacement needs that first owner's
original attributes and list/quote ancestry when no owner survives the clear.

Each entry retains stable source identity, local token boundaries, structural
owner identity, and a shared parent-scope handle. Original opening/closing token
bytes remain slices of immutable source storage, preserving attributes, quoting,
case and duplicate attributes. Decode only the small syntax needed to author an
insertion. Cache compact parsed metadata such as tag names and link destinations
without copying every ancestor's text into every entry.

Use a persistent balanced index with subtree lengths, not a flat vector of
absolute offsets that shifts after each edit. Reuse the repository's snapshot,
source-piece and range-index machinery where it provides the needed identities.
Tag every query with source/projection identity; stale state is rejected or
explicitly rebased. Do not retain naked document offsets as cache identities.

Deliverable: indexed queries compared against the existing implementation across
the replacement and recovery fixtures, while the old implementation remains a
test oracle.

## 3. Maintain the index through existing incremental transactions

For ordinary text edits, reuse unchanged token/scope structure and update only
the affected leaves. For inserted/split/removed inline wrappers, rebuild the
changed token region and share the unaffected suffix. Source changes earlier in
the file must not rewrite every later entry.

Integrate with both regional projection paths and full reprojection. A regional
candidate receives an explicit inherited scope context and must prove that its
exit state matches the retained suffix. State includes the relevant lexical,
semantic recovery, paragraph-owner and whitespace context; token spelling alone
does not prove convergence.

Reuse existing safe regional boundaries first. When an edit changes grammar
outside them, expand to a trustworthy boundary or use the established full
projection path, rebuilding the index as part of that parse. Record the reason.
Do not invent a partially resumable HTML5 parser or guess recovery state merely
to make a performance assertion pass.

Include document-wide encoding/line-ending conversion, source-view edits,
format switches, style changes, undo/redo, cancelled composition and failed
transactions. Undo/redo should restore the matching persistent index with its
snapshot. Failed preparation must not publish partial cache state. Independent
views share the immutable index; their replacement contexts remain view-local.

Deliverable: incrementally maintained index with explicit invalidation tests and
memory accounting through the existing history-retention mechanism.

## 4. Switch the replacement path to indexed queries

Replace both full-source scans with indexed lookups. Capture the first selected
text character's original context before deletion, respecting paragraph-separator
rules. After deletion, query the new insertion boundary against the new snapshot;
never reuse a pre-edit positional result.

Retain only the small, owned semantic/syntax context needed for continued typing
and IME, as the current implementation does. Explicit character-style choices
must still override inheritance. Dot repeat resolves the context at its new
destination. Rich clipboard input supplies its own context.

Audit downstream style restoration and whitespace helpers reached by these
transactions. Add local variants for any unconditional full-source scans that
would otherwise dominate the same small replacement. Keep this followup scoped
to replacement paths; list remaining genuinely nonlocal structural operations
with their measured causes.

Deliverable: ordinary/native/IME replacement no longer invokes the old whole-
document scope scanner. Remove the production fallback once parity is established;
retain an independent full-parse comparison in tests.

## 5. Acceptance and rollout

- Existing replacement, paragraph inheritance, entity, link, encoding,
  malformed-HTML, clipboard, IME and undo tests retain identical results.
- Differential tests compare indexed scope answers and edited saved/reopened
  documents against a fresh full parse after randomized edit sequences.
- At a fixed edit location and local structure, growing unrelated document
  content does not grow decoded/tokenized/materialized bytes. Index traversal
  scales logarithmically; scope work scales with real nesting depth.
- A small edit near the end of a huge paragraph does not decode its entire
  prefix. A huge affected token or genuinely changed nesting context may cost
  its actual size and is measured separately.
- Source edits to ancestor tags, link attributes, paragraph boundaries and
  malformed recovery points invalidate the correct context. Distant ordinary
  text edits do not invalidate unrelated scope branches.
- Repeated edits, two views, IME cancellation, undo/redo and history pruning keep
  retained index memory bounded by live snapshots and configured history limits.
- macOS native replacement tests and Windows shared-core/ABI tests pass; native
  latency measurements are recorded on both platforms before shipping.

Implement in reviewable stages: instrumentation, full-build indexed queries,
incremental maintenance, then replacement-path migration and performance gates.
This makes correctness parity reviewable before removing the existing scanner.
