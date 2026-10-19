# Unicode data

[LICENSE.txt](LICENSE.txt) is an exact copy of the Unicode license supplied with
the vendored Unicode 15.0.0 / UAX #14 revision 49 data. It covers
`LineBreak.txt`, `DerivedGeneralCategory.txt`, `EastAsianWidth.txt`,
`emoji-data.txt`, and the `LineBreakTest.txt` conformance fixture.

The original [data README](../../../../data/unicode/15.0.0/README.md) and
[manifest](../../../../data/unicode/15.0.0/manifest.json) identify the official
source URLs and SHA-256 hashes. Copyright notices in those data files and the
original adjacent license remain intact. The generated line-break lookup in
`src/core/layout/unicode_break_data.rs` is derived from this data.
