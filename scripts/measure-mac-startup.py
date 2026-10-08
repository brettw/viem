#!/usr/bin/env python3
"""Measure fresh native processes through the requested document's first draw."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import tempfile
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, default=root / ".build/release-app/Viem.app/Contents/MacOS/Viem")
    parser.add_argument("--document", type=Path, default=root / "AGENTS.md")
    parser.add_argument("--profile-directory", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--sample", action="store_true", help="Capture startup stacks; use separate runs from timing comparisons")
    parser.add_argument("--sample-delay", type=float, default=0, help="Seconds after launch before sampling")
    args = parser.parse_args()
    if not 1 <= args.runs <= 20:
        parser.error("--runs must be between 1 and 20")
    if not 0 <= args.sample_delay <= 10:
        parser.error("--sample-delay must be between 0 and 10 seconds")
    executable, document = args.executable.resolve(), args.document.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    reports = []
    with tempfile.TemporaryDirectory(prefix="viem-startup-") as scratch:
        for index in range(args.runs):
            run = Path(scratch) / str(index)
            profile = run / "profile"
            profile.mkdir(parents=True)
            if args.profile_directory:
                for name in ("config.json", "startup.viem", "themes"):
                    original = args.profile_directory / name
                    if original.is_dir():
                        shutil.copytree(original, profile / name)
                    elif original.is_file():
                        shutil.copy2(original, profile / name)
            # Keep source and recovery ownership separate from an open editor.
            fixture = run / document.name
            shutil.copy2(document, fixture)
            # Preserve reopening semantics: do not turn an existing recent-file
            # entry into an extra preferences write just because it is isolated.
            configuration = profile / "config.json"
            if configuration.exists():
                settings = json.loads(configuration.read_text())
                if "recentDocuments" in settings:
                    settings["recentDocuments"] = [
                        str(fixture) if Path(path).resolve() == document else path
                        for path in settings["recentDocuments"]
                    ]
                    configuration.write_text(json.dumps(settings))
            report = output / f"run-{index + 1}.json"
            if report.exists():
                parser.error(f"report already exists: {report}")
            environment = dict(os.environ, VIEM_CONFIG_DIR=str(profile),
                               VIEM_INSTANCE_DIRECTORY=str(run / "instance"),
                               VIEM_STARTUP_DOCUMENT=str(fixture), VIEM_STARTUP_REPORT=str(report),
                               VIEM_STARTUP_EXIT="1")
            # Foundation's reference date is 2001-01-01; this includes dyld/main
            # bootstrap that the in-process monotonic milestones cannot cover.
            environment["VIEM_STARTUP_LAUNCH_TIME"] = str(time.time() - 978307200)
            process = subprocess.Popen([str(executable), str(fixture)], env=environment,
                                       cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                if args.sample:
                    time.sleep(args.sample_delay)
                    subprocess.run(["sample", str(process.pid), "1", "1", "-file", str(output / f"sample-{index + 1}.txt")],
                                   capture_output=True, text=True, timeout=10, check=True)
                stdout, stderr = process.communicate(timeout=30)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.communicate()
            result = subprocess.CompletedProcess(process.args, process.returncode, stdout, stderr)
            (output / f"run-{index + 1}.log").write_text(result.stdout + result.stderr)
            result.check_returncode()
            reports.append(json.loads(report.read_text()))
            print(f"Run {index + 1}: {reports[-1]['processToFirstDrawMilliseconds']:.1f} ms", flush=True)
    summary = {"executable": str(executable), "binarySha256": digest(executable),
               "document": str(document), "documentSha256": digest(document),
               "profileDirectory": str(args.profile_directory) if args.profile_directory else None,
               "sampled": args.sample,
               "platform": platform.platform(), "runs": reports,
               "medianProcessToFirstDrawMilliseconds": statistics.median(
                   r["processToFirstDrawMilliseconds"] for r in reports)}
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
