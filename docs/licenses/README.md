# Third-party licenses

The [root license](../../LICENSE.md) preserves Viem's MIT license and links to
this index of license and notice texts for its dependencies and bundled resources.
Third-party material retains its own terms and copyright notices.

- [Rust packages](rust/README.md): all 70 third-party package versions in
  `Cargo.lock` and `fuzz/Cargo.lock`, including transitive, build, target-specific,
  and fuzzing dependencies.
- [Native packages](native/README.md): all 19 locked Windows NuGet packages,
  the Windows SDK reference package, and three matching .NET runtime/host packs;
  Swift's package manifest has no external package dependencies.
- [Bundled resources](bundled/README.md): fonts, Vim runtime and derived tables,
  syntax queries, and Unicode data, including component-specific notices.

This is a source-tree inventory. A package's inclusion does not mean it is linked
into every app: some packages are build tools, fuzzing inputs, or platform-specific.
The component inventories record versions, origins, and any differences between
package metadata and the shipped license texts.

## Original notices and app packages

Upstream texts are copied without rewriting their terms or copyright lines.
Copies beside fonts, syntax files, and Unicode data remain in place to preserve
their provenance, checksums, and existing distribution structure.

macOS and Windows packaging include `Resources/LICENSE.md` and
`Resources/docs/licenses/`, so the root index's relative links also work in the
app distribution (`Resources` is inside `Contents` on macOS). Existing font,
Vim, and query resource notices continue to ship in their original locations.
Source-code and maintenance links in the inventories refer to the repository
checkout; all license and notice texts are included in the app copy.

## Updating dependencies

When changing a dependency or bundled resource:

1. Update the relevant lockfile or resource provenance first.
2. Copy all applicable license, copyright, exception, and notice files from that
   exact release or pinned source revision, preserving their bytes. Follow the
   collection instructions in the component inventory.
3. Update that inventory and this index, keeping `LICENSE.md`'s short link. Remove a version
   only after it is absent from every lockfile and bundled resource.
4. Verify lockfile coverage, recorded hashes, and local links. The
   [Rust collector](rust/README.md#updating-and-checking) provides offline checks
   and source verification. Verify packaged copies when changing native packaging.

Do not replace package-specific copyright notices with a generic license
template or assume that every component uses its parent project's license.
