#!/usr/bin/env python3
"""Staged, reproducible validation for Viem changes.

Run the cheapest stage that can find a problem first, then freeze the
candidate for the expensive ones. Every invocation records a manifest with the
source identity, each command, its duration, exit status and log, and the
exact artifacts it ran, under target/validation/<timestamp>/ by default.

    python3 scripts/validate.py check
    python3 scripts/validate.py focused --filter markdown_local_paragraph_edits::
    python3 scripts/validate.py consistency --baseline main
    python3 scripts/validate.py suites consistency-full memory --baseline main

Stages (run in the order given):

  check             cargo check of the library, tests and examples.
  focused           Integration (or library) tests matching --filter. Shadow
                    verification compares regional edits with complete
                    reprojection (see .cargo/config.toml).
  suites            The complete library and integration test suites.
  consistency       The quick consistency probe (a curated seed subset that
                    runs in seconds), compared case by case with the same
                    probe at --baseline.
  consistency-full  The full consistency probe, compared with --baseline.
                    Only for large or risky refactors or when asked to run
                    all tests.
  memory            The standard, supplemental and rich-paste allocation
                    runners with their checked-in limits. Only when asked,
                    or for changes with plausible memory impact.
  native            The macOS native tests (scripts/test-mac.sh), after a
                    preflight, with a stall watchdog.

Baselines are built from a detached worktree of the requested revision in
target/validation/baselines/<commit>/ with the current consistency harness, and
their probe results are reused while the revision, harness and probe options
are unchanged. After deleting target/, run `git worktree prune`. Timing-sensitive stages never overlap; run one invocation at a
time when measurements matter.
"""

import argparse
import datetime
import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HARNESS = ["examples/fuzz_consistency.rs", "examples/fuzz_consistency/corpus.rs"]
STAGES = ["check", "focused", "suites", "consistency", "consistency-full", "memory", "native"]


class StageFailure(Exception):
    pass


def git(*args, cwd=ROOT):
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def source_identity():
    """Commit plus a digest of uncommitted changes, so dirty runs are distinguishable."""
    commit = git("rev-parse", "HEAD")
    status = git("status", "--porcelain")
    digest = hashlib.sha256()
    digest.update(subprocess.run(["git", "diff", "HEAD", "--binary"], cwd=ROOT, check=True,
                                 capture_output=True).stdout)
    for line in status.splitlines():
        if line.startswith("??"):
            path = ROOT / line[3:].strip().strip('"')
            if path.is_file():
                digest.update(path.name.encode())
                digest.update(path.read_bytes())
    return {"commit": commit, "dirty": bool(status), "status": status.splitlines(),
            "change_digest": digest.hexdigest()[:16]}


def file_digest(paths, root=ROOT):
    digest = hashlib.sha256()
    for path in paths:
        digest.update(path.encode())
        digest.update((root / path).read_bytes())
    return digest.hexdigest()[:16]


class Run:
    def __init__(self, out, keep_going):
        self.out = out
        self.keep_going = keep_going
        self.out.mkdir(parents=True, exist_ok=True)
        self.manifest = {"started": datetime.datetime.now().isoformat(timespec="seconds"),
                         "source": source_identity(), "stages": []}
        self.save()

    def save(self):
        (self.out / "manifest.json").write_text(json.dumps(self.manifest, indent=2) + "\n")

    def command(self, stage, name, argv, cwd=ROOT, env=None, stall_seconds=None, allow_failure=False):
        """Run one command, streaming its output to a log. Returns (exit code, log path)."""
        log = self.out / f"{stage}-{name}.log"
        merged_env = dict(os.environ)
        merged_env.update(env or {})
        started = time.time()
        record = {"stage": stage, "name": name, "argv": argv, "cwd": str(cwd), "log": str(log),
                  "env": env or {}}
        print(f"[{stage}] {name}: {' '.join(argv)}", flush=True)
        with open(log, "wb") as output:
            process = subprocess.Popen(argv, cwd=cwd, env=merged_env, stdout=subprocess.PIPE,
                                       stderr=subprocess.STDOUT, start_new_session=True)
            last_output = [time.time()]
            stalled = [False]

            def pump():
                for chunk in iter(lambda: process.stdout.read1(65536), b""):
                    output.write(chunk)
                    output.flush()
                    last_output[0] = time.time()

            reader = threading.Thread(target=pump, daemon=True)
            reader.start()
            while process.poll() is None:
                time.sleep(1)
                if stall_seconds and time.time() - last_output[0] > stall_seconds:
                    stalled[0] = True
                    sample_stalled(process.pid, self.out / f"{stage}-{name}-stall.txt")
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=30)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                    break
            reader.join(timeout=10)
        record.update({"seconds": round(time.time() - started, 1), "exit": process.returncode,
                       "stalled": stalled[0]})
        self.manifest["stages"].append(record)
        self.save()
        status = "stalled" if stalled[0] else f"exit {process.returncode}"
        print(f"[{stage}] {name}: {status} in {record['seconds']}s (log {log})", flush=True)
        if (process.returncode != 0 or stalled[0]) and not allow_failure:
            raise StageFailure(f"{stage}/{name} failed; see {log}")
        return process.returncode, log


def sample_stalled(pid, path):
    """Record what a stalled process tree is doing before it is stopped."""
    lines = []
    try:
        tree = subprocess.run(["ps", "-o", "pid=,ppid=,pcpu=,etime=,command=", "-g", str(pid)],
                              capture_output=True, text=True).stdout
        lines.append(tree)
        if shutil.which("sample"):
            for row in tree.splitlines():
                child = row.split()[0]
                result = subprocess.run(["sample", child, "3"], capture_output=True, text=True)
                lines.append(result.stdout[-20000:])
    except OSError as error:
        lines.append(f"sampling failed: {error}")
    path.write_text("\n".join(lines))


def cargo_artifact(run, stage, cargo_args, target, kind="example", cwd=ROOT, env=None):
    """Build with JSON messages and return the executable Cargo actually produced.

    Never guess paths under target/: a stale binary from another profile,
    worktree or CARGO_TARGET_DIR silently invalidates every later result.
    """
    log = run.out / f"{stage}-build-{target}.json"
    merged = dict(os.environ)
    merged.update(env or {})
    argv = ["cargo", "build", "--message-format=json-render-diagnostics", *cargo_args]
    print(f"[{stage}] build {target}: {' '.join(argv)}", flush=True)
    started = time.time()
    result = subprocess.run(argv, cwd=cwd, env=merged, capture_output=True, text=True)
    log.write_text(result.stdout + result.stderr)
    if result.returncode != 0:
        sys.stderr.write(result.stderr[-4000:])
        raise StageFailure(f"{stage}: building {target} failed; see {log}")
    executable = None
    for line in result.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (message.get("reason") == "compiler-artifact" and message["target"]["name"] == target
                and kind in message["target"]["kind"] and message.get("executable")):
            executable = message["executable"]
    if not executable:
        raise StageFailure(f"{stage}: Cargo reported no executable for {target}")
    digest = hashlib.sha256(Path(executable).read_bytes()).hexdigest()[:16]
    run.manifest.setdefault("artifacts", []).append(
        {"stage": stage, "target": target, "path": executable, "sha256": digest,
         "build_seconds": round(time.time() - started, 1)})
    run.save()
    return executable


def stage_check(run, args):
    run.command("check", "cargo-check", ["cargo", "check", "--lib", "--tests", "--examples"])


def stage_focused(run, args):
    if not args.filter:
        raise StageFailure("focused: pass at least one --filter (for example module_name::)")
    # Cargo takes one test-name filter before `--`; libtest takes several after it.
    if args.tests in ("all", "both"):
        run.command("focused", "integration", ["cargo", "test", "--test", "all", "--", *args.filter])
    if args.tests in ("lib", "both"):
        run.command("focused", "lib", ["cargo", "test", "--lib", "--", *args.filter])


def stage_suites(run, args):
    run.command("suites", "lib", ["cargo", "test", "--lib"])
    run.command("suites", "integration", ["cargo", "test", "--test", "all"])


def baseline_worktree(run, revision):
    commit = git("rev-parse", f"{revision}^{{commit}}")
    base = ROOT / "target" / "validation" / "baselines" / commit
    tree = base / "src"
    if not (tree / ".git").exists():
        base.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "worktree", "add", "--detach", str(tree), commit], cwd=ROOT,
                       check=True, capture_output=True)
    # The comparison needs one case identity, so the baseline core runs the
    # current harness. A harness change that the baseline cannot build is
    # reported rather than silently compared with a different harness.
    for path in HARNESS:
        (tree / path).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / path, tree / path)
    return commit, base, tree


def probe_arguments(full, args):
    probe = ["probe", "--probes", args.probes]
    if not full:
        probe.append("--quick")
    if args.positions:
        probe += ["--positions", str(args.positions)]
    for name in ("seed_filter", "op_filter", "fmt"):
        value = getattr(args, name)
        if value:
            probe += [{"seed_filter": "--filter", "op_filter": "--op-filter", "fmt": "--fmt"}[name], value]
    return probe


def stage_consistency(run, args, full):
    stage = "consistency-full" if full else "consistency"
    probe = probe_arguments(full, args)
    candidate = cargo_artifact(run, stage, ["--release", "--example", "fuzz_consistency"], "fuzz_consistency")
    baseline_dir = None
    if args.baseline:
        commit, base, tree = baseline_worktree(run, args.baseline)
        key = hashlib.sha256(json.dumps([commit, file_digest(HARNESS), probe]).encode()).hexdigest()[:16]
        baseline_dir = base / f"consistency-{key}"
        if (baseline_dir / "manifest.json").exists():
            print(f"[{stage}] reusing baseline results {baseline_dir}", flush=True)
        else:
            try:
                executable = cargo_artifact(run, stage, ["--release", "--example", "fuzz_consistency"],
                                            "fuzz_consistency", cwd=tree,
                                            env={"CARGO_TARGET_DIR": str(base / "target")})
            except StageFailure as failure:
                raise StageFailure(f"{failure}. The baseline {args.baseline} cannot build the current "
                                   "consistency harness (for example it predates an API the harness "
                                   "uses); choose a newer baseline") from failure
            if baseline_dir.exists():
                shutil.rmtree(baseline_dir)
            run.command(stage, "baseline-probe", [executable, *probe, "--out", str(baseline_dir)], cwd=tree)
        run.manifest["baseline"] = {"revision": args.baseline, "commit": commit, "results": str(baseline_dir)}
        run.save()
    out = run.out / stage
    argv = [candidate, *probe, "--out", str(out)]
    if baseline_dir:
        argv += ["--baseline", str(baseline_dir)]
    code, _ = run.command(stage, "probe", argv, allow_failure=True)
    if baseline_dir:
        run.command(stage, "compare", [candidate, "compare", str(baseline_dir), str(out)], allow_failure=True)
        if code != 0:
            raise StageFailure(f"{stage}: newly failing cases; replayable traces are in {out}/*-new.jsonl "
                               f"(replay with `{candidate} replay FILE`)")
    elif code != 0:
        raise StageFailure(f"{stage}: probe failed")


def stage_memory(run, args):
    cargo_artifact(run, "memory", ["--release", "--offline", "--example", "large_file_memory"], "large_file_memory")
    limits = "scripts/fixtures/large-file-memory-limits.json"
    for name, flags in [("standard", []), ("supplemental", ["--supplemental"]), ("rich-paste", ["--rich-paste"])]:
        run.command("memory", name, ["python3", "scripts/measure-large-file-memory.py", *flags,
                                     "--output", str(run.out / f"memory-{name}.json"), "--limits", limits],
                    allow_failure=args.keep_going)


def stage_native(run, args):
    problems = []
    if sys.platform != "darwin":
        problems.append("native tests require macOS")
    for tool in ("swift", "xcrun", "caffeinate"):
        if not shutil.which(tool):
            problems.append(f"{tool} is not on PATH")
    session = subprocess.run(["launchctl", "managername"], capture_output=True, text=True).stdout.strip()
    if session != "Aqua":
        problems.append(f"no GUI login session (launchctl managername: {session or 'unknown'}); "
                        "AppKit and font-service tests need one, and sandboxed shells usually lack it")
    if problems:
        raise StageFailure("native preflight: " + "; ".join(problems))
    run.command("native", "test-mac", ["scripts/test-mac.sh"], stall_seconds=args.stall_minutes * 60)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("stages", nargs="*", default=["check"], choices=STAGES, metavar="STAGE")
    parser.add_argument("--filter", action="append", default=[], help="test name filter for `focused`")
    parser.add_argument("--tests", choices=["all", "lib", "both"], default="all",
                        help="which test binaries `focused` runs (default: integration tests only)")
    parser.add_argument("--baseline", help="git revision to compare consistency results with")
    parser.add_argument("--probes", default="insert,normal,selection,copypaste")
    parser.add_argument("--positions", type=int, help="override probe positions per seed")
    parser.add_argument("--seed-filter", help="only consistency seeds containing this text")
    parser.add_argument("--op-filter", help="only consistency operations whose label contains this text")
    parser.add_argument("--fmt", help="only consistency seeds of this format (wysiwyg, code, plain)")
    parser.add_argument("--stall-minutes", type=float, default=10, help="native watchdog (default 10)")
    parser.add_argument("--keep-going", action="store_true", help="continue after a failed stage")
    parser.add_argument("--out", type=Path, help="results directory")
    args = parser.parse_args()
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    run = Run(args.out or ROOT / "target" / "validation" / stamp, args.keep_going)
    print(f"results: {run.out}", flush=True)
    failures = []
    for stage in args.stages:
        started = time.time()
        try:
            {"check": stage_check, "focused": stage_focused, "suites": stage_suites,
             "consistency": lambda run, args: stage_consistency(run, args, False),
             "consistency-full": lambda run, args: stage_consistency(run, args, True),
             "memory": stage_memory, "native": stage_native}[stage](run, args)
            outcome = "passed"
        except StageFailure as failure:
            outcome = str(failure)
            failures.append(outcome)
        run.manifest.setdefault("summary", []).append(
            {"stage": stage, "outcome": outcome, "seconds": round(time.time() - started, 1)})
        run.save()
        print(f"== {stage}: {outcome} ({time.time() - started:.0f}s)", flush=True)
        if failures and not args.keep_going:
            break
    run.manifest["finished"] = datetime.datetime.now().isoformat(timespec="seconds")
    run.save()
    if failures:
        print("FAILED:\n  " + "\n  ".join(failures))
        return 1
    print("all requested stages passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
