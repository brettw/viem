# First-party Unicode line breaking

Viem implements Unicode line breaking a according to the default
[Unicode 15.0 UAX #14 rules](https://www.unicode.org/reports/tr14/tr14-49.html),
matching the previous Unicode version.

The algorithm, compact data lookup, and generator are original repository code.
The external material is the Unicode Consortium's published character
properties and conformance examples, with their data license and checksums in
[data/unicode/15.0.0](../data/unicode/15.0.0/README.md). Regeneration is offline and uses only
Python's standard library:

```sh
python3 scripts/generate-unicode-break-data.py --check
```

The generated lookup occupies 15,027 static bytes and allocates nothing. ASCII
classification uses a direct lookup; other scalars use a compact range search.
The streaming rule state has a tested maximum size of 32 bytes, independent of
the input length. Scanners check cancellation by consumed input, including long
runs without any break opportunity. A common alphabetic-pair shortcut avoids
walking the remaining rules where they cannot change LB28's no-break decision;
the same conformance and restart tests cover that path.

Partial layout starts at a hard-line boundary or a verified prior break
checkpoint. That allows exact restart without reconstructing context from an
arbitrary captured prefix. Shaping retains its separate context requirements.
The generator checks its packed data against every Unicode code point.

The rule tests cover all 7,654 published Unicode 15.0 examples. The published
fixture enables optional numeric-expression tailoring; Viem retains default
LB25 rules. Exactly 34 individual break expectations differ for that documented
policy, with the specific rule annotations and total difference count checked.
No fixture cases are skipped. Fixed numeric examples test the default behavior.

Restart equivalence is checked across the entire fixture and all 74,088 triples
of valid-scalar break-class representatives. Integration regressions compare
full and resumed layout after resizing, including regional indicators, combining
marks, emoji joiners, giant punctuation/space runs, and flowed source newlines.

There is no new user option or intended appearance change. Unicode updates and
optional tailoring remain explicit policy decisions. This change retains the
existing giant-line overflow and memory-budget behavior.

## Correctness validation

The final Rust library run passes 1,158 tests, with 3 existing ignored tests.
Another 94 tests pass across 15 selected integration targets. These cover the
new continuation/Unicode regressions, layout cache invalidation and bounds,
direction and list decorations, paragraph flow, Markdown Source layout,
viewport navigation, input atomicity, and the Vim command matrix. The Unicode
fixture and exhaustive representative triples run inside those library tests;
they are not counted as thousands of separate Rust test functions.

The final native `scripts/test-mac.sh` run rebuilds the Rust library and macOS
application, then passes all 487 XCTest and 36 Swift Testing tests. These include
real Core Text giant-line geometry, wrapping, resizing, source formats, editing,
and native render-resource ownership. The wrapper uses a temporary configuration
directory and the AppKit service access required by the native tests.

The offline generator verifies all 1,114,112 Unicode code points and reproduces
the checked-in lookup exactly. Cargo's dependency tree, manifest and lockfile
contain no `unicode-linebreak` dependency. Formatting checks pass for the new
Rust modules and integration tests, and `git diff --check` is clean.

Reproduce the Rust and native checks:

```sh
python3 scripts/generate-unicode-break-data.py --check
cargo test --offline --lib
cargo test --offline --no-fail-fast --test all -- \
  first_party_line_breaks:: layout_jobs:: layout_cache_bounds:: \
  layout_direction:: layout_list_decorations:: command_layout_viewport:: \
  view_line_modes:: viewport_horizontal_range:: viewport_end:: \
  paragraph_flow:: markdown_source_layout:: \
  prose_block_layout:: input_layout_atomicity:: vim_command_matrix::
scripts/test-mac.sh
```
