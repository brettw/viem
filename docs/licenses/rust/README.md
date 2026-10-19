# Rust dependency licenses

This inventory covers all 70 third-party package versions in the root
and fuzz lockfiles: 65 occur in `Cargo.lock`, and 5 occur only in
`fuzz/Cargo.lock`. It includes transitive, build, development, and target-specific
entries, whether or not the current platform compiles them. Lockfile presence
does not imply inclusion in every shipped binary.

License and attribution files are copied byte-for-byte from the exact published
crate archives, with nested notices retained. Crate SHA-256 checksums are verified
against the lockfiles. Where an archive omits its license, the supplemental
source and hash are recorded in [the inventory](inventory.json) and
[the collector configuration](../../../scripts/rust-license-supplements.json).
The declared license column reproduces Cargo metadata; the notes below record
important details and additional notices. These third-party licenses do not
change Viem's own license.

## Updating and checking

Use Python 3.11 or newer (for example `python3.12`):

```sh
python3.12 scripts/collect-rust-licenses.py --check
python3.12 scripts/collect-rust-licenses.py --collect --allow-network
python3.12 scripts/collect-rust-licenses.py --verify-sources --allow-network
```

`--check` is offline and needs no Cargo cache: it checks lockfile coverage,
checksums, recorded file hashes, supplemental records, and the generated index.
Collection uses Cargo's cached archives and an optional `--cache-dir`
(default: the system temporary directory's `viem-rust-license-cache`).
Downloads require `--allow-network`; dependency builds are never run.
`--verify-sources` compares every collected byte with checksum-verified archives
and the pinned supplements, using the same cache/network policy.
Review new package licenses and supplemental notices when dependencies change;
filename discovery is an aid to collection, not a replacement for that review.
Remove obsolete package directories after review; the collector reports them
instead of silently deleting files. Update the `docs/licenses/README.md` index too.

## Packages

| Package | Declared license | Lockfile scope | License and attribution texts |
| --- | --- | --- | --- |
| [aho-corasick-1.1.5](https://github.com/BurntSushi/aho-corasick) | `Unlicense OR MIT` | Root and fuzz | [COPYING](aho-corasick-1.1.5/COPYING), [LICENSE-MIT](aho-corasick-1.1.5/LICENSE-MIT), [UNLICENSE](aho-corasick-1.1.5/UNLICENSE) |
| [arbitrary-1.5.0](https://github.com/rust-fuzz/arbitrary/) | `MIT OR Apache-2.0` | Fuzz only | [LICENSE-APACHE](arbitrary-1.5.0/LICENSE-APACHE), [LICENSE-MIT](arbitrary-1.5.0/LICENSE-MIT) |
| [bitflags-2.13.1](https://github.com/bitflags/bitflags) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](bitflags-2.13.1/LICENSE-APACHE), [LICENSE-MIT](bitflags-2.13.1/LICENSE-MIT) |
| [cc-1.4.5](https://github.com/rust-lang/cc-rs) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](cc-1.4.5/LICENSE-APACHE), [LICENSE-MIT](cc-1.4.5/LICENSE-MIT) |
| [cfg-if-1.0.0](https://github.com/alexcrichton/cfg-if) | `MIT/Apache-2.0` | Root and fuzz | [LICENSE-APACHE](cfg-if-1.0.0/LICENSE-APACHE), [LICENSE-MIT](cfg-if-1.0.0/LICENSE-MIT) |
| [equivalent-1.0.2](https://github.com/indexmap-rs/equivalent) | `Apache-2.0 OR MIT` | Root and fuzz | [LICENSE-APACHE](equivalent-1.0.2/LICENSE-APACHE), [LICENSE-MIT](equivalent-1.0.2/LICENSE-MIT) |
| [fastrand-2.5.0](https://github.com/smol-rs/fastrand) | `Apache-2.0 OR MIT` | Root and fuzz | [LICENSE-APACHE](fastrand-2.5.0/LICENSE-APACHE), [LICENSE-MIT](fastrand-2.5.0/LICENSE-MIT) |
| [find-msvc-tools-0.1.12](https://github.com/rust-lang/cc-rs) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](find-msvc-tools-0.1.12/LICENSE-APACHE), [LICENSE-MIT](find-msvc-tools-0.1.12/LICENSE-MIT) |
| [getrandom-0.4.3](https://github.com/rust-random/getrandom) | `MIT OR Apache-2.0` | Fuzz only | [LICENSE-APACHE](getrandom-0.4.3/LICENSE-APACHE), [LICENSE-MIT](getrandom-0.4.3/LICENSE-MIT) |
| [hashbrown-0.15.5](https://github.com/rust-lang/hashbrown) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](hashbrown-0.15.5/LICENSE-APACHE), [LICENSE-MIT](hashbrown-0.15.5/LICENSE-MIT) |
| [html5ever-0.39.0](https://github.com/servo/html5ever) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](html5ever-0.39.0/LICENSE-APACHE), [LICENSE-MIT](html5ever-0.39.0/LICENSE-MIT) |
| [indexmap-2.7.1](https://github.com/indexmap-rs/indexmap) | `Apache-2.0 OR MIT` | Root and fuzz | [LICENSE-APACHE](indexmap-2.7.1/LICENSE-APACHE), [LICENSE-MIT](indexmap-2.7.1/LICENSE-MIT) |
| [itoa-1.0.10](https://github.com/dtolnay/itoa) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](itoa-1.0.10/LICENSE-APACHE), [LICENSE-MIT](itoa-1.0.10/LICENSE-MIT) |
| [jobserver-0.1.35](https://github.com/rust-lang/jobserver-rs) | `MIT OR Apache-2.0` | Fuzz only | [LICENSE-APACHE](jobserver-0.1.35/LICENSE-APACHE), [LICENSE-MIT](jobserver-0.1.35/LICENSE-MIT) |
| [libc-0.2.189](https://github.com/rust-lang/libc) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](libc-0.2.189/LICENSE-APACHE), [LICENSE-MIT](libc-0.2.189/LICENSE-MIT) |
| [libfuzzer-sys-0.4.13](https://github.com/rust-fuzz/libfuzzer) | `(MIT OR Apache-2.0) AND NCSA` | Fuzz only | [CREDITS-LLVM.txt](libfuzzer-sys-0.4.13/CREDITS-LLVM.txt), [FuzzerSHA1-NOTICE.txt](libfuzzer-sys-0.4.13/FuzzerSHA1-NOTICE.txt), [LICENSE-APACHE](libfuzzer-sys-0.4.13/LICENSE-APACHE), [LICENSE-LLVM.txt](libfuzzer-sys-0.4.13/LICENSE-LLVM.txt), [LICENSE-MIT](libfuzzer-sys-0.4.13/LICENSE-MIT) |
| [lock_api-0.4.14](https://github.com/Amanieu/parking_lot) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](lock_api-0.4.14/LICENSE-APACHE), [LICENSE-MIT](lock_api-0.4.14/LICENSE-MIT) |
| [log-0.4.34](https://github.com/rust-lang/log) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](log-0.4.34/LICENSE-APACHE), [LICENSE-MIT](log-0.4.34/LICENSE-MIT) |
| [markup5ever-0.39.0](https://github.com/servo/html5ever) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](markup5ever-0.39.0/LICENSE-APACHE), [LICENSE-MIT](markup5ever-0.39.0/LICENSE-MIT) |
| [memchr-2.8.3](https://github.com/BurntSushi/memchr) | `Unlicense OR MIT` | Root and fuzz | [COPYING](memchr-2.8.3/COPYING), [LICENSE-MIT](memchr-2.8.3/LICENSE-MIT), [UNLICENSE](memchr-2.8.3/UNLICENSE) |
| [new_debug_unreachable-1.0.6](https://github.com/mbrubeck/rust-debug-unreachable) | `MIT` | Root and fuzz | [LICENSE-MIT](new_debug_unreachable-1.0.6/LICENSE-MIT) |
| [parking_lot-0.12.5](https://github.com/Amanieu/parking_lot) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](parking_lot-0.12.5/LICENSE-APACHE), [LICENSE-MIT](parking_lot-0.12.5/LICENSE-MIT) |
| [parking_lot_core-0.9.12](https://github.com/Amanieu/parking_lot) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](parking_lot_core-0.9.12/LICENSE-APACHE), [LICENSE-MIT](parking_lot_core-0.9.12/LICENSE-MIT) |
| [phf-0.13.1](https://github.com/rust-phf/rust-phf) | `MIT` | Root and fuzz | [LICENSE](phf-0.13.1/LICENSE) |
| [phf_codegen-0.13.1](https://github.com/rust-phf/rust-phf) | `MIT` | Root and fuzz | [LICENSE](phf_codegen-0.13.1/LICENSE) |
| [phf_generator-0.13.1](https://github.com/rust-phf/rust-phf) | `MIT` | Root and fuzz | [LICENSE](phf_generator-0.13.1/LICENSE) |
| [phf_shared-0.13.1](https://github.com/rust-phf/rust-phf) | `MIT` | Root and fuzz | [LICENSE](phf_shared-0.13.1/LICENSE) |
| [precomputed-hash-0.1.1](https://github.com/emilio/precomputed-hash) | `MIT` | Root and fuzz | [LICENSE](precomputed-hash-0.1.1/LICENSE) |
| [proc-macro2-1.0.107](https://github.com/dtolnay/proc-macro2) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](proc-macro2-1.0.107/LICENSE-APACHE), [LICENSE-MIT](proc-macro2-1.0.107/LICENSE-MIT) |
| [pulldown-cmark-0.13.4](https://github.com/raphlinus/pulldown-cmark) | `MIT` | Root and fuzz | [LICENSE](pulldown-cmark-0.13.4/LICENSE) |
| [quote-1.0.47](https://github.com/dtolnay/quote) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](quote-1.0.47/LICENSE-APACHE), [LICENSE-MIT](quote-1.0.47/LICENSE-MIT) |
| [r-efi-6.0.0](https://github.com/r-efi/r-efi) | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | Fuzz only | [AUTHORS](r-efi-6.0.0/AUTHORS) |
| [redox_syscall-0.5.18](https://gitlab.redox-os.org/redox-os/syscall) | `MIT` | Root and fuzz | [LICENSE](redox_syscall-0.5.18/LICENSE) |
| [regex-1.13.1](https://github.com/rust-lang/regex) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](regex-1.13.1/LICENSE-APACHE), [LICENSE-MIT](regex-1.13.1/LICENSE-MIT) |
| [regex-automata-0.4.18](https://github.com/rust-lang/regex) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](regex-automata-0.4.18/LICENSE-APACHE), [LICENSE-MIT](regex-automata-0.4.18/LICENSE-MIT) |
| [regex-syntax-0.8.11](https://github.com/rust-lang/regex) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](regex-syntax-0.8.11/LICENSE-APACHE), [LICENSE-MIT](regex-syntax-0.8.11/LICENSE-MIT), [src/unicode_tables/LICENSE-UNICODE](regex-syntax-0.8.11/src/unicode_tables/LICENSE-UNICODE) |
| [ryu-1.0.17](https://github.com/dtolnay/ryu) | `Apache-2.0 OR BSL-1.0` | Root and fuzz | [LICENSE-APACHE](ryu-1.0.17/LICENSE-APACHE), [LICENSE-BOOST](ryu-1.0.17/LICENSE-BOOST) |
| [scopeguard-1.2.0](https://github.com/bluss/scopeguard) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](scopeguard-1.2.0/LICENSE-APACHE), [LICENSE-MIT](scopeguard-1.2.0/LICENSE-MIT) |
| [serde-1.0.229](https://github.com/serde-rs/serde) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](serde-1.0.229/LICENSE-APACHE), [LICENSE-MIT](serde-1.0.229/LICENSE-MIT) |
| [serde_core-1.0.229](https://github.com/serde-rs/serde) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](serde_core-1.0.229/LICENSE-APACHE), [LICENSE-MIT](serde_core-1.0.229/LICENSE-MIT) |
| [serde_derive-1.0.229](https://github.com/serde-rs/serde) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](serde_derive-1.0.229/LICENSE-APACHE), [LICENSE-MIT](serde_derive-1.0.229/LICENSE-MIT) |
| [serde_json-1.0.140](https://github.com/serde-rs/json) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](serde_json-1.0.140/LICENSE-APACHE), [LICENSE-MIT](serde_json-1.0.140/LICENSE-MIT) |
| [shlex-2.0.1](https://github.com/comex/rust-shlex) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](shlex-2.0.1/LICENSE-APACHE), [LICENSE-MIT](shlex-2.0.1/LICENSE-MIT) |
| [siphasher-1.0.3](https://github.com/jedisct1/rust-siphash) | `MIT/Apache-2.0` | Root and fuzz | [COPYING](siphasher-1.0.3/COPYING), [LICENSE-APACHE-2.0.txt](siphasher-1.0.3/LICENSE-APACHE-2.0.txt) |
| [smallvec-1.16.0](https://github.com/servo/rust-smallvec) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](smallvec-1.16.0/LICENSE-APACHE), [LICENSE-MIT](smallvec-1.16.0/LICENSE-MIT) |
| [streaming-iterator-0.1.9](https://github.com/sfackler/streaming-iterator) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](streaming-iterator-0.1.9/LICENSE-APACHE), [LICENSE-MIT](streaming-iterator-0.1.9/LICENSE-MIT) |
| [string_cache-0.9.0](https://github.com/servo/string-cache) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](string_cache-0.9.0/LICENSE-APACHE), [LICENSE-MIT](string_cache-0.9.0/LICENSE-MIT) |
| [string_cache_codegen-0.6.1](https://github.com/servo/string-cache) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](string_cache_codegen-0.6.1/LICENSE-APACHE), [LICENSE-MIT](string_cache_codegen-0.6.1/LICENSE-MIT) |
| [syn-3.0.5](https://github.com/dtolnay/syn) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](syn-3.0.5/LICENSE-APACHE), [LICENSE-MIT](syn-3.0.5/LICENSE-MIT) |
| [tendril-0.5.1](https://github.com/servo/html5ever) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](tendril-0.5.1/LICENSE-APACHE), [LICENSE-MIT](tendril-0.5.1/LICENSE-MIT) |
| [tree-sitter-0.25.10](https://github.com/tree-sitter/tree-sitter) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-0.25.10/LICENSE), [src/unicode/LICENSE](tree-sitter-0.25.10/src/unicode/LICENSE) |
| [tree-sitter-c-0.23.4](https://github.com/tree-sitter/tree-sitter-c) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-c-0.23.4/LICENSE) |
| [tree-sitter-c-sharp-0.23.1](https://github.com/tree-sitter/tree-sitter-c-sharp) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-c-sharp-0.23.1/LICENSE) |
| [tree-sitter-cpp-0.23.4](https://github.com/tree-sitter/tree-sitter-cpp) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-cpp-0.23.4/LICENSE) |
| [tree-sitter-javascript-0.23.1](https://github.com/tree-sitter/tree-sitter-javascript) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-javascript-0.23.1/LICENSE) |
| [tree-sitter-json-0.24.8](https://github.com/tree-sitter/tree-sitter-json) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-json-0.24.8/LICENSE) |
| [tree-sitter-language-0.1.3](https://github.com/tree-sitter/tree-sitter) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-language-0.1.3/LICENSE) |
| [tree-sitter-md-0.5.3](https://github.com/tree-sitter-grammars/tree-sitter-markdown) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-md-0.5.3/LICENSE) |
| [tree-sitter-objc-3.0.2](https://github.com/tree-sitter-grammars/tree-sitter-objc) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-objc-3.0.2/LICENSE) |
| [tree-sitter-python-0.23.5](https://github.com/tree-sitter/tree-sitter-python) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-python-0.23.5/LICENSE) |
| [tree-sitter-rust-0.23.2](https://github.com/tree-sitter/tree-sitter-rust) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-rust-0.23.2/LICENSE) |
| [tree-sitter-swift-0.7.3](https://github.com/alex-pinkus/tree-sitter-swift) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-swift-0.7.3/LICENSE), [node_modules/isexe/LICENSE](tree-sitter-swift-0.7.3/node_modules/isexe/LICENSE), [node_modules/node-gyp-build/LICENSE](tree-sitter-swift-0.7.3/node_modules/node-gyp-build/LICENSE), [node_modules/tree-sitter-cli/LICENSE](tree-sitter-swift-0.7.3/node_modules/tree-sitter-cli/LICENSE), [node_modules/which/LICENSE](tree-sitter-swift-0.7.3/node_modules/which/LICENSE) |
| [tree-sitter-typescript-0.23.2](https://github.com/tree-sitter/tree-sitter-typescript) | `MIT` | Root and fuzz | [LICENSE](tree-sitter-typescript-0.23.2/LICENSE) |
| [unicase-2.9.0](https://github.com/seanmonstar/unicase) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](unicase-2.9.0/LICENSE-APACHE), [LICENSE-MIT](unicase-2.9.0/LICENSE-MIT) |
| [unicode-bidi-0.3.18](https://github.com/servo/unicode-bidi) | `MIT OR Apache-2.0` | Root and fuzz | [AUTHORS](unicode-bidi-0.3.18/AUTHORS), [COPYRIGHT](unicode-bidi-0.3.18/COPYRIGHT), [LICENSE-APACHE](unicode-bidi-0.3.18/LICENSE-APACHE), [LICENSE-MIT](unicode-bidi-0.3.18/LICENSE-MIT) |
| [unicode-ident-1.0.24](https://github.com/dtolnay/unicode-ident) | `(MIT OR Apache-2.0) AND Unicode-3.0` | Root and fuzz | [LICENSE-APACHE](unicode-ident-1.0.24/LICENSE-APACHE), [LICENSE-MIT](unicode-ident-1.0.24/LICENSE-MIT), [LICENSE-UNICODE](unicode-ident-1.0.24/LICENSE-UNICODE) |
| [unicode-segmentation-1.12.0](https://github.com/unicode-rs/unicode-segmentation) | `MIT OR Apache-2.0` | Root and fuzz | [COPYRIGHT](unicode-segmentation-1.12.0/COPYRIGHT), [LICENSE-APACHE](unicode-segmentation-1.12.0/LICENSE-APACHE), [LICENSE-MIT](unicode-segmentation-1.12.0/LICENSE-MIT) |
| [unicode-width-0.1.14](https://github.com/unicode-rs/unicode-width) | `MIT OR Apache-2.0` | Root and fuzz | [COPYRIGHT](unicode-width-0.1.14/COPYRIGHT), [LICENSE-APACHE](unicode-width-0.1.14/LICENSE-APACHE), [LICENSE-MIT](unicode-width-0.1.14/LICENSE-MIT) |
| [web_atoms-0.2.6](https://github.com/servo/html5ever) | `MIT OR Apache-2.0` | Root and fuzz | [LICENSE-APACHE](web_atoms-0.2.6/LICENSE-APACHE), [LICENSE-MIT](web_atoms-0.2.6/LICENSE-MIT) |
| [windows-link-0.2.1](https://github.com/microsoft/windows-rs) | `MIT OR Apache-2.0` | Root and fuzz | [license-apache-2.0](windows-link-0.2.1/license-apache-2.0), [license-mit](windows-link-0.2.1/license-mit) |

## Source and license notes

- **libfuzzer-sys-0.4.13:** Cargo metadata declares (MIT OR Apache-2.0) AND NCSA, while the bundled libfuzzer source headers declare Apache-2.0 WITH LLVM-exception. LICENSE-LLVM.txt preserves the LLVM Apache license, exception, and historical NCSA/MIT notices; CREDITS-LLVM.txt supplies the referenced attribution. These are from LLVM commit a47b42eb9f9b302167b4fc413e6c92798d65dd0b, recorded in update-libfuzzer.sh at the crate source commit. The SHA-1 source notice also preserves its public-domain attribution.
- **r-efi-6.0.0:** AUTHORS contains the full MIT permission text and package-specific copyright, plus the alternative Apache-2.0 and LGPL-2.1-or-later grants. The MIT option supplies the complete license text included here.
- **siphasher-1.0.3:** COPYING contains the package-specific copyright and Apache-2.0/MIT grant, but neither the published crate nor its source commit supplies the referenced full license files. The Apache-2.0 option is accompanied here by the complete text from the Apache Software Foundation; COPYING is retained unchanged.
- **tree-sitter-0.25.10:** The published crate omits its main MIT license. LICENSE is copied from the exact source commit recorded by the crate's .cargo_vcs_info.json.
- **tree-sitter-c-0.23.4:** The published crate omits its main MIT license. LICENSE is copied from the exact source commit recorded by the crate's .cargo_vcs_info.json.
- **tree-sitter-c-sharp-0.23.1:** The published crate omits its main MIT license. LICENSE is copied from the exact source commit recorded by the crate's .cargo_vcs_info.json.
- **tree-sitter-cpp-0.23.4:** The published crate omits its main MIT license. LICENSE is copied from the exact source commit recorded by the crate's .cargo_vcs_info.json.
- **tree-sitter-javascript-0.23.1:** The published crate omits its main MIT license. LICENSE is copied from the exact source commit recorded by the crate's .cargo_vcs_info.json.
- **tree-sitter-json-0.24.8:** The published crate omits its main MIT license. LICENSE is copied from the exact source commit recorded by the crate's .cargo_vcs_info.json.
- **tree-sitter-language-0.1.3:** The published crate omits its main MIT license. LICENSE is copied from the exact source commit recorded by the crate's .cargo_vcs_info.json.
- **tree-sitter-md-0.5.3:** The published crate omits its main MIT license. LICENSE is copied from the exact source commit recorded by the crate's .cargo_vcs_info.json.
- **tree-sitter-objc-3.0.2:** The published crate omits its main MIT license. LICENSE is copied from the exact source commit recorded by the crate's .cargo_vcs_info.json.
- **tree-sitter-python-0.23.5:** The published crate omits its main MIT license. LICENSE is copied from the exact source commit recorded by the crate's .cargo_vcs_info.json.
- **tree-sitter-rust-0.23.2:** The published crate omits its main MIT license. LICENSE is copied from the exact source commit recorded by the crate's .cargo_vcs_info.json.
- **tree-sitter-swift-0.7.3:** The crate archive also includes node_modules license files for isexe, node-gyp-build, tree-sitter-cli, and which. Their original nested notices are retained.
- **tree-sitter-typescript-0.23.2:** The published crate omits its main MIT license. LICENSE is copied from the exact source commit recorded by the crate's .cargo_vcs_info.json.
