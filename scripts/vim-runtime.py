#!/usr/bin/env python3
"""Import or verify the shared Vim syntax snapshot using only Python's stdlib.

Import does not update language aliases or filename detection. When the inventory
changes, follow assets/vim/README.md and the language maintenance checklist in
src/core/document/syntax/detection/PROFILE.md; update filename rules/tests too.
"""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unicodedata


ASSETS = Path(__file__).resolve().parents[1] / "assets" / "vim"


def inventory(runtime):
    """Hash raw bytes and reject trees whose names differ across platforms."""
    if runtime.is_symlink() or not runtime.is_dir():
        raise ValueError(f"Missing or symlinked runtime directory: {runtime}")
    names = set()
    files = {}
    for path in sorted(runtime.rglob("*")):
        relative = path.relative_to(runtime.parent).as_posix()
        if path.is_symlink():
            raise ValueError(f"Runtime symlinks are unsupported: {relative}")
        folded = unicodedata.normalize("NFC", relative).casefold()
        if folded in names:
            raise ValueError(f"Runtime filename collision: {relative}")
        names.add(folded)
        if path.is_dir():
            continue
        if not path.is_file():
            raise ValueError(f"Unsupported runtime entry: {relative}")
        data = path.read_bytes()
        files[relative] = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    if "runtime/LICENSE" not in files or "runtime/syntax/vim.vim" not in files:
        raise ValueError("Runtime must contain LICENSE and syntax/vim.vim")
    return files


def verify(assets):
    manifest = json.loads((assets / "manifest.json").read_text(encoding="utf-8"))
    if manifest.get("version") != 1:
        raise ValueError("Unsupported Vim runtime manifest version")
    expected = manifest["files"]
    actual = inventory(assets / "runtime")
    if expected != actual:
        changed = sorted(name for name in expected.keys() | actual.keys()
                         if expected.get(name) != actual.get(name))
        raise ValueError("Vim runtime differs from manifest: " + ", ".join(changed[:10]))
    if manifest["file_count"] != len(actual) or manifest["total_bytes"] != sum(
            entry["bytes"] for entry in actual.values()):
        raise ValueError("Vim runtime manifest totals do not match")
    return len(actual)


def package(assets, output):
    """Replace only the app output's owned Resources/vim subtree."""
    assets = assets.resolve()
    count = verify(assets)
    # Read required attribution before touching an existing installation.
    (assets / "README.md").read_bytes()
    output = output.resolve()
    destination = output / "Resources" / "vim"
    # Reject redirected resource parents (including Windows junctions) and
    # overlapping source/output trees before any recursive removal.
    if (destination.resolve() != destination or destination.is_symlink()
            or not destination.is_relative_to(output)
            or assets.is_relative_to(destination)
            or destination.is_relative_to(assets)):
        raise ValueError(f"Unsafe Vim resource destination: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(assets, destination, symlinks=True)
    verify(destination)
    return count


def import_runtime(source, version, assets=ASSETS):
    source = source.resolve()
    if not (source / "LICENSE").is_file() or not (source / "syntax").is_dir():
        raise ValueError("Source must be a Vim runtime directory with LICENSE and syntax/")
    assets.mkdir(parents=True, exist_ok=True)
    # Validate the complete candidate before replacing the previous snapshot.
    with tempfile.TemporaryDirectory(prefix=".vim-import-", dir=assets.parent) as temporary:
        staged = Path(temporary)
        runtime = staged / "runtime"
        runtime.mkdir()
        shutil.copyfile(source / "LICENSE", runtime / "LICENSE")
        shutil.copytree(source / "syntax", runtime / "syntax", symlinks=True)
        files = inventory(runtime)
        manifest = {
            "version": 1,
            "source": {"distribution": version, "runtime_directory": str(source)},
            "file_count": len(files),
            "total_bytes": sum(entry["bytes"] for entry in files.values()),
            "files": files,
        }
        (staged / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        verify(staged)
        if (assets / "runtime").exists():
            shutil.rmtree(assets / "runtime")
        shutil.move(str(runtime), assets / "runtime")
        shutil.move(str(staged / "manifest.json"), assets / "manifest.json")
    return len(files)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    check = commands.add_parser("verify", help="verify a source or packaged snapshot")
    check.add_argument("assets", nargs="?", type=Path, default=ASSETS)
    bundle = commands.add_parser("package", help="verify and package a Windows app output")
    bundle.add_argument("output", type=Path, help="application build or publish directory")
    bundle.add_argument("--assets", type=Path, default=ASSETS)
    update = commands.add_parser("import", help="replace the repository snapshot from a local runtime")
    update.add_argument("runtime", type=Path)
    update.add_argument("--version", required=True, help="exact source distribution/revision")
    args = parser.parse_args()
    try:
        if args.command == "verify":
            count = verify(args.assets)
        elif args.command == "package":
            count = package(args.assets, args.output)
        else:
            count = import_runtime(args.runtime, args.version)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, f"Vim runtime: {error}\n")
    print(f"Vim runtime: {count} files verified")


if __name__ == "__main__":
    main()
