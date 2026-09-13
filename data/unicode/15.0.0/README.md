# Unicode 15.0.0 line-break data

These are unmodified character-property data and conformance examples published
by the Unicode Consortium. No third-party line-breaking implementation or
generated library table is included. Viem's rule implementation, data generator,
and compact lookup are original code in this repository.

The implementation is pinned to **Unicode 15.0.0, UAX #14 revision 49**, matching
the Unicode version used by the previous line-breaking dependency. The exact
official URLs, retrieval date, sizes, and SHA-256 checksums are in
[manifest.json](manifest.json). All files are covered by the Unicode data
copyright and permission notice in [LICENSE.txt](LICENSE.txt); the original
copyright notices also remain in the data files.

| File | Purpose |
| --- | --- |
| `LineBreak.txt` | All `Line_Break` character classes, including explicit unassigned-range defaults. |
| `DerivedGeneralCategory.txt` | `Mn`/`Mc` handling for SA in LB1 and `Cn` handling for LB30b. |
| `EastAsianWidth.txt` | Fullwidth, wide, and halfwidth exclusions in LB30. |
| `emoji-data.txt` | Extended pictographic characters in LB30b. |
| `LineBreakTest.txt` | 7,654 official examples containing 31,624 boundary markers and 282 distinct code points. |

Run the original generator from the repository root:

```sh
python3 scripts/generate-unicode-break-data.py
python3 scripts/generate-unicode-break-data.py --check
```

The generator uses only Python's standard library, verifies every vendored
file's checksum, and requires no network access. It reads the original property
ranges, then packs each combined class/flag run into a four-byte entry. The
generated Rust lookup uses 3,682 entries plus an ASCII fast table and class table,
totalling 15,027 static bytes. It allocates nothing at runtime. Generation checks
the packed representation against the input-derived properties at every one of
the 1,114,112 Unicode code points, including surrogate values that Rust `char`
cannot represent. Future Unicode updates require an explicit version/data
change, regeneration, and conformance review.

The official test file states that it uses the numeric tailoring from UAX #14
section 8.2, example 7. That tailoring differs from the default LB25 rules used
by Viem. The test source is retained unchanged, including this notice; the rule
tests account explicitly for this policy difference instead of silently
modifying the published fixture.
