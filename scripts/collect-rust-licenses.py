#!/usr/bin/env python3
"""Collect locked Rust licenses, or verify the committed inventory offline.

Requires Python 3.11+ (tomllib). Downloads are opt-in. Crate archives are checked
against Cargo.lock before any content is used; extra upstream texts are pinned
by URL and SHA-256 in rust-license-supplements.json. No archive is extracted.
"""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import sys
import tarfile
import tempfile
import urllib.request

try:
    import tomllib
except ImportError:
    sys.exit("Python 3.11+ is required; use python3.12 scripts/collect-rust-licenses.py")

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "docs/licenses/rust"
SUPPLEMENTS = Path(__file__).with_name("rust-license-supplements.json")
LOCKFILES = ("Cargo.lock", "fuzz/Cargo.lock")
LICENSE_NAME = re.compile(
    r"^(?:unlicen[cs]e|licen[cs]e|copying|notice|copyright|authors|credits|ofl)(?:[._-]|$)",
    re.IGNORECASE,
)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def json_bytes(value):
    return (json.dumps(value, indent=2, ensure_ascii=False) + "\n").encode("utf-8")


def package_id(package):
    return f"{package['name']}-{package['version']}"


def safe_relative(value):
    path = PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts or "\\" in value:
        raise ValueError(f"Unsafe relative path: {value}")
    return path


def locked_packages():
    packages = {}
    for filename in LOCKFILES:
        lock = tomllib.loads((ROOT / filename).read_text())
        for item in lock["package"]:
            if not item.get("source"):
                continue  # First-party workspace/path packages have no registry source.
            if item["source"] != "registry+https://github.com/rust-lang/crates.io-index":
                raise ValueError(f"Unreviewed dependency source: {item['source']}")
            key = package_id(item)
            if key in packages:
                if packages[key]["checksum"] != item["checksum"]:
                    raise ValueError(f"Lockfiles disagree on checksum: {key}")
                packages[key]["lockfiles"].append(filename)
            else:
                packages[key] = {
                    "name": item["name"], "version": item["version"],
                    "source": item["source"], "checksum": item["checksum"],
                    "lockfiles": [filename],
                }
    return dict(sorted(packages.items()))


def download(url, allow_network):
    if not allow_network:
        raise ValueError(f"Missing cached input; rerun with --allow-network to fetch {url}")
    request = urllib.request.Request(url, headers={"User-Agent": "Viem-license-collector"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def archive_bytes(package, args):
    key = package_id(package)
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    candidates = [args.cache_dir / f"{key}.crate"]
    candidates.extend(sorted((cargo_home / "registry/cache").glob(f"*/{key}.crate")))
    for path in candidates:
        if path.is_file():
            data = path.read_bytes()
            if digest(data) != package["checksum"]:
                raise ValueError(f"Crate checksum mismatch: {path}")
            return data
    url = f"https://static.crates.io/crates/{package['name']}/{key}.crate"
    data = download(url, args.allow_network)
    if digest(data) != package["checksum"]:
        raise ValueError(f"Downloaded crate checksum mismatch: {key}")
    args.cache_dir.mkdir(parents=True, exist_ok=True)
    (args.cache_dir / f"{key}.crate").write_bytes(data)
    return data


def supplemental_bytes(key, spec, args):
    safe_relative(spec["path"])
    candidates = [args.cache_dir / key / spec["path"], OUTPUT / key / spec["path"]]
    for path in candidates:
        if path.is_file() and digest(path.read_bytes()) == spec["sha256"]:
            return path.read_bytes()
    data = download(spec["url"], args.allow_network)
    if digest(data) != spec["sha256"]:
        raise ValueError(f"Supplement checksum mismatch: {key}/{spec['path']}")
    target = args.cache_dir / key / spec["path"]
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    return data


def readme(packages):
    root_count = sum("Cargo.lock" in item["lockfiles"] for item in packages)
    fuzz_count = len(packages) - root_count
    lines = [
        "# Rust dependency licenses", "",
        f"This inventory covers all {len(packages)} third-party package versions in the root",
        f"and fuzz lockfiles: {root_count} occur in `Cargo.lock`, and {fuzz_count} occur only in",
        "`fuzz/Cargo.lock`. It includes transitive, build, development, and target-specific",
        "entries, whether or not the current platform compiles them. Lockfile presence",
        "does not imply inclusion in every shipped binary.", "",
        "License and attribution files are copied byte-for-byte from the exact published",
        "crate archives, with nested notices retained. Crate SHA-256 checksums are verified",
        "against the lockfiles. Where an archive omits its license, the supplemental",
        "source and hash are recorded in [the inventory](inventory.json) and",
        "[the collector configuration](../../../scripts/rust-license-supplements.json).",
        "The declared license column reproduces Cargo metadata; the notes below record",
        "important details and additional notices. These third-party licenses do not",
        "change Viem's own license.", "",
        "## Updating and checking", "",
        "Use Python 3.11 or newer (for example `python3.12`):", "",
        "```sh",
        "python3.12 scripts/collect-rust-licenses.py --check",
        "python3.12 scripts/collect-rust-licenses.py --collect --allow-network",
        "python3.12 scripts/collect-rust-licenses.py --verify-sources --allow-network",
        "```", "",
        "`--check` is offline and needs no Cargo cache: it checks lockfile coverage,",
        "checksums, recorded file hashes, supplemental records, and the generated index.",
        "Collection uses Cargo's cached archives and an optional `--cache-dir`",
        "(default: the system temporary directory's `viem-rust-license-cache`).",
        "Downloads require `--allow-network`; dependency builds are never run.",
        "`--verify-sources` compares every collected byte with checksum-verified archives",
        "and the pinned supplements, using the same cache/network policy.",
        "Review new package licenses and supplemental notices when dependencies change;",
        "filename discovery is an aid to collection, not a replacement for that review.",
        "Remove obsolete package directories after review; the collector reports them",
        "instead of silently deleting files. Update the `docs/licenses/README.md` index too.", "",
        "## Packages", "",
        "| Package | Declared license | Lockfile scope | License and attribution texts |",
        "| --- | --- | --- | --- |",
    ]
    for item in packages:
        key = package_id(item)
        scope = "Root and fuzz" if len(item["lockfiles"]) == 2 else (
            "Root" if item["lockfiles"] == ["Cargo.lock"] else "Fuzz only"
        )
        texts = ", ".join(f"[{f['path']}]({key}/{f['path']})" for f in item["files"])
        lines.append(f"| [{key}]({item['repository'] or item['crate_url']}) | `{item['license']}` | {scope} | {texts} |")
    noted = [item for item in packages if item.get("note")]
    if noted:
        lines.extend(["", "## Source and license notes", ""])
        for item in noted:
            lines.append(f"- **{package_id(item)}:** {item['note']}")
    return ("\n".join(lines) + "\n").encode("utf-8")


def collect(packages, supplements, args):
    output = {}
    inventory = []
    for key, locked in packages.items():
        with tarfile.open(fileobj=io.BytesIO(archive_bytes(locked, args)), mode="r:gz") as archive:
            members = {}
            prefix = key + "/"
            for member in archive.getmembers():
                if member.name.startswith(prefix) and member.isfile():
                    relative = member.name[len(prefix):]
                    safe_relative(relative)
                    members[relative] = member
            def read_member(path):
                return archive.extractfile(members[path]).read()
            metadata = tomllib.loads(read_member("Cargo.toml").decode("utf-8"))["package"]
            if (metadata["name"], metadata["version"]) != (locked["name"], locked["version"]):
                raise ValueError(f"Crate metadata identity mismatch: {key}")
            selected = {
                path for path in members
                if LICENSE_NAME.match(PurePosixPath(path).name)
                or any(part.lower() == "licenses" for part in PurePosixPath(path).parts[:-1])
            }
            if metadata.get("license-file"):
                selected.add(metadata["license-file"])
            files = []
            for path in sorted(selected):
                data = read_member(path)
                output[f"{key}/{path}"] = data
                files.append({"path": path, "sha256": digest(data), "crate_member": path})
            supplement = supplements.get(key, {})
            for spec in supplement.get("archive_excerpts", []):
                first, last = spec["lines"]
                data = b"".join(read_member(spec["member"]).splitlines(keepends=True)[first - 1:last])
                path = spec["path"]
                safe_relative(path)
                output[f"{key}/{path}"] = data
                files.append({"path": path, "sha256": digest(data), "crate_member": spec["member"], "lines": spec["lines"]})
            for spec in supplement.get("files", []):
                data = supplemental_bytes(key, spec, args)
                output[f"{key}/{spec['path']}"] = data
                files.append(dict(spec))
            if not files:
                raise ValueError(f"No license texts found for {key}; review and add a pinned supplement")
            if len({f["path"] for f in files}) != len(files):
                raise ValueError(f"Duplicate collected license path for {key}")
            item = dict(locked)
            item.update({"license": metadata.get("license", "See license-file"),
                         "repository": metadata.get("repository"),
                         "crate_url": f"https://crates.io/crates/{locked['name']}/{locked['version']}",
                         "archive_url": f"https://static.crates.io/crates/{locked['name']}/{key}.crate",
                         "files": sorted(files, key=lambda f: f["path"])})
            if ".cargo_vcs_info.json" in members:
                item["vcs"] = json.loads(read_member(".cargo_vcs_info.json"))
            for field in ("note", "evidence_url"):
                if field in supplement:
                    item[field] = supplement[field]
            inventory.append(item)
    output["inventory.json"] = json_bytes({"schema_version": 1, "lockfiles": list(LOCKFILES), "packages": inventory})
    output["README.md"] = readme(inventory)
    return output


def check(packages, supplements):
    manifest = json.loads((OUTPUT / "inventory.json").read_text())
    recorded = {package_id(item): item for item in manifest["packages"]}
    if len(recorded) != len(manifest["packages"]) or set(recorded) != set(packages):
        raise ValueError("Inventory package set differs from the lockfiles; recollect licenses")
    expected_paths = {"inventory.json", "README.md"}
    for key, locked in packages.items():
        item = recorded[key]
        for field in ("name", "version", "source", "checksum", "lockfiles"):
            if item[field] != locked[field]:
                raise ValueError(f"Inventory mismatch for {key}: {field}")
        if not item["files"]:
            raise ValueError(f"No license texts for {key}")
        files = {f["path"]: f for f in item["files"]}
        if len(files) != len(item["files"]):
            raise ValueError(f"Duplicate license paths for {key}")
        for path, file in files.items():
            safe_relative(path)
            full = f"{key}/{path}"
            expected_paths.add(full)
            if digest((OUTPUT / full).read_bytes()) != file["sha256"]:
                raise ValueError(f"License text changed: {full}")
        supplement = supplements.get(key, {})
        for spec in supplement.get("files", []):
            if files.get(spec["path"]) != spec:
                raise ValueError(f"Supplement record changed: {key}/{spec['path']}")
        for spec in supplement.get("archive_excerpts", []):
            file = files.get(spec["path"], {})
            if file.get("crate_member") != spec["member"] or file.get("lines") != spec["lines"]:
                raise ValueError(f"Archive excerpt record changed: {key}/{spec['path']}")
        for field in ("note", "evidence_url"):
            if item.get(field) != supplement.get(field):
                raise ValueError(f"Supplement note changed: {key}")
    actual_paths = {str(p.relative_to(OUTPUT)) for p in OUTPUT.rglob("*") if p.is_file()}
    if actual_paths != expected_paths:
        raise ValueError(f"Unexpected or missing inventory files: {sorted(actual_paths ^ expected_paths)}")
    if (OUTPUT / "README.md").read_bytes() != readme(manifest["packages"]):
        raise ValueError("Rust license README is stale; recollect licenses")
    return len(expected_paths) - 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="Verify inventory and copies offline")
    mode.add_argument("--collect", action="store_true", help="Collect licenses and generate inventory")
    mode.add_argument("--verify-sources", action="store_true", help="Compare copies to verified source archives")
    parser.add_argument("--allow-network", action="store_true", help="Permit downloading missing inputs")
    parser.add_argument("--cache-dir", type=Path, default=Path(tempfile.gettempdir()) / "viem-rust-license-cache")
    args = parser.parse_args()
    packages = locked_packages()
    supplements = json.loads(SUPPLEMENTS.read_text())["packages"]
    unused = set(supplements) - set(packages)
    if unused:
        raise ValueError(f"Obsolete supplemental records need review: {sorted(unused)}")
    if args.check:
        count = check(packages, supplements)
    else:
        output = collect(packages, supplements, args)
        if args.verify_sources:
            check(packages, supplements)
            for path, data in output.items():
                if (OUTPUT / path).read_bytes() != data:
                    raise ValueError(f"Collected source differs: {path}")
        else:
            for path, data in output.items():
                target = OUTPUT / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
            check(packages, supplements)
        count = len(output) - 2
    print(f"Verified {len(packages)} locked Rust package versions and {count} license/attribution files.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, tarfile.TarError) as error:
        sys.exit(f"License collection failed: {error}")
