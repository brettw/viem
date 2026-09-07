#!/usr/bin/env python3
"""Run deterministic backend fuzz campaigns with replay artifacts and memory limits."""

from __future__ import annotations

import argparse
from collections import deque
import ctypes
import ctypes.util
import csv
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import signal
import statistics
import subprocess
import sys
import time
from typing import Any, Dict, List, Optional


REPO = Path(__file__).resolve().parents[1]
MIB = 1024 * 1024
U64_MAX = (1 << 64) - 1


class DarwinProcTaskInfo(ctypes.Structure):
    """Public ``proc_taskinfo`` prefix from macOS ``<sys/proc_info.h>``."""

    _fields_ = [
        ("virtual_size", ctypes.c_uint64),
        ("resident_size", ctypes.c_uint64),
        ("total_user", ctypes.c_uint64),
        ("total_system", ctypes.c_uint64),
        ("threads_user", ctypes.c_uint64),
        ("threads_system", ctypes.c_uint64),
        ("policy", ctypes.c_int32),
        ("faults", ctypes.c_int32),
        ("pageins", ctypes.c_int32),
        ("cow_faults", ctypes.c_int32),
        ("messages_sent", ctypes.c_int32),
        ("messages_received", ctypes.c_int32),
        ("syscalls_mach", ctypes.c_int32),
        ("syscalls_unix", ctypes.c_int32),
        ("csw", ctypes.c_int32),
        ("threadnum", ctypes.c_int32),
        ("numrunning", ctypes.c_int32),
        ("priority", ctypes.c_int32),
    ]


def load_darwin_proc_pidinfo():
    if sys.platform != "darwin":
        return None
    try:
        library = ctypes.CDLL(ctypes.util.find_library("proc") or "/usr/lib/libproc.dylib",
                              use_errno=True)
        function = library.proc_pidinfo
        function.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64,
                             ctypes.c_void_p, ctypes.c_int]
        function.restype = ctypes.c_int
        # Keep the CDLL alive for as long as its function pointer is used.
        function._library = library
        return function
    except (AttributeError, OSError):
        return None


DARWIN_PROC_PIDINFO = load_darwin_proc_pidinfo()
PROC_PIDTASKINFO = 4


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def write_json(path: Path, value: Any) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def capture(command: List[str]) -> Dict[str, Any]:
    try:
        result = subprocess.run(command, cwd=REPO, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=10)
        return {"command": command, "returncode": result.returncode,
                "stdout": result.stdout.strip(), "stderr": result.stderr.strip()}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {"command": command, "error": str(error)}


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(MIB), b""):
            digest.update(block)
    return digest.hexdigest()


def sample_rss(pid: int) -> Optional[int]:
    """Resident bytes of this child, never unrelated processes or global usage."""
    if sys.platform.startswith("linux"):
        try:
            for line in Path("/proc/{}/status".format(pid)).read_text().splitlines():
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) * 1024
        except (OSError, ValueError):
            pass
        return None
    if sys.platform == "darwin" and DARWIN_PROC_PIDINFO is not None:
        info = DarwinProcTaskInfo()
        size = ctypes.sizeof(info)
        try:
            copied = DARWIN_PROC_PIDINFO(pid, PROC_PIDTASKINFO, 0,
                                         ctypes.byref(info), size)
            if copied == size:
                return int(info.resident_size)
        except (OSError, ValueError):
            pass
    try:
        result = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)],
                                text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, timeout=1)
        value = result.stdout.strip()
        return int(value) * 1024 if result.returncode == 0 and value else None
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return None


def signal_owned_group(pid: int, signum: int) -> None:
    # Every monitored child starts a new session whose process-group ID is its PID.
    try:
        os.killpg(pid, signum)
    except ProcessLookupError:
        pass


class Samples:
    def __init__(self) -> None:
        self.count = 0
        self.maximum = 0
        self.first = []  # type: List[int]
        self.last = deque(maxlen=10)  # type: deque
        self.sum_t = self.sum_y = self.sum_tt = self.sum_ty = 0.0

    def add(self, elapsed: float, value: int) -> None:
        self.count += 1
        self.maximum = max(self.maximum, value)
        if len(self.first) < 10:
            self.first.append(value)
        self.last.append(value)
        self.sum_t += elapsed
        self.sum_y += value
        self.sum_tt += elapsed * elapsed
        self.sum_ty += elapsed * value

    def summary(self) -> Dict[str, Any]:
        denominator = self.count * self.sum_tt - self.sum_t * self.sum_t
        enough = self.count >= 20
        return {
            "sample_count": self.count,
            "sampled_peak_rss_bytes": self.maximum if self.count else None,
            "first_window_median_rss_bytes": statistics.median(self.first) if self.count else None,
            "last_window_median_rss_bytes": statistics.median(self.last) if self.count else None,
            "rss_window_growth_bytes": (statistics.median(self.last) - statistics.median(self.first)) if enough else None,
            "rss_slope_bytes_per_second": ((self.count * self.sum_ty - self.sum_t * self.sum_y) / denominator)
                if enough and denominator > 0 else None,
        }


def run_monitored(command: List[str], directory: Path, timeout: float,
                  rss_limit_bytes: int, sample_interval: float,
                  cwd: Path = REPO) -> Dict[str, Any]:
    """Reap with wait4 so even a between-samples crash retains per-child peak RSS."""
    result = {"command": command, "started_at": utc_now(), "status": "launch_error"}  # type: Dict[str, Any]
    samples = Samples()
    began = time.monotonic()
    process = None
    usage = None
    reason = None
    terminate_at = None
    kill_sent = False
    with (directory / "stdout.log").open("wb") as stdout, \
            (directory / "stderr.log").open("wb") as stderr, \
            (directory / "memory.csv").open("w", newline="") as memory:
        writer = csv.writer(memory)
        writer.writerow(["elapsed_seconds", "rss_bytes"])
        try:
            process = subprocess.Popen(command, cwd=cwd, stdout=stdout, stderr=stderr,
                                       start_new_session=True)
            result["pid"] = process.pid
            while True:
                try:
                    waited_pid, wait_status, child_usage = os.wait4(process.pid, os.WNOHANG)
                    if waited_pid:
                        process.returncode = os.waitstatus_to_exitcode(wait_status)
                        usage = child_usage
                        break
                    elapsed = time.monotonic() - began
                    rss = sample_rss(process.pid)
                    if rss is not None:
                        samples.add(elapsed, rss)
                        writer.writerow([round(elapsed, 6), rss])
                        memory.flush()
                    elif rss_limit_bytes and reason is None:
                        # A very fast child may exit before ps or /proc sees it.
                        # Otherwise a requested ceiling must not silently stop working.
                        waited_pid, wait_status, child_usage = os.wait4(process.pid, os.WNOHANG)
                        if waited_pid:
                            process.returncode = os.waitstatus_to_exitcode(wait_status)
                            usage = child_usage
                            break
                        reason = "rss_monitor_unavailable"
                        result["error"] = "Cannot sample RSS of the running child; refusing to run without the requested memory ceiling."
                        terminate_at = time.monotonic()
                        signal_owned_group(process.pid, signal.SIGTERM)
                    if reason is None:
                        if rss_limit_bytes and rss is not None and rss > rss_limit_bytes:
                            reason = "rss_limit"
                        elif elapsed >= timeout:
                            reason = "timeout"
                        if reason is not None:
                            terminate_at = time.monotonic()
                            signal_owned_group(process.pid, signal.SIGTERM)
                    if terminate_at is not None and not kill_sent and time.monotonic() - terminate_at >= 0.5:
                        signal_owned_group(process.pid, signal.SIGKILL)
                        kill_sent = True
                    time.sleep(sample_interval)
                except KeyboardInterrupt:
                    reason = "interrupted"
                    if terminate_at is None:
                        terminate_at = time.monotonic()
                        signal_owned_group(process.pid, signal.SIGTERM)
                    else:
                        signal_owned_group(process.pid, signal.SIGKILL)
                        kill_sent = True
            result["returncode"] = process.returncode
            result["exit_code"] = process.returncode if process.returncode >= 0 else None
            result["signal"] = -process.returncode if process.returncode < 0 else None
            result["signal_name"] = signal.Signals(-process.returncode).name if process.returncode < 0 else None
            result["status"] = reason or ("passed" if process.returncode == 0 else "failed")
        except OSError as error:
            result["error"] = str(error)
        finally:
            if process is not None:
                # Also stop descendants left behind when their leader exits/crashes.
                signal_owned_group(process.pid, signal.SIGKILL)
                if process.returncode is None:
                    _, wait_status, usage = os.wait4(process.pid, 0)
                    process.returncode = os.waitstatus_to_exitcode(wait_status)
    result["elapsed_seconds"] = time.monotonic() - began
    result["finished_at"] = utc_now()
    result["memory"] = samples.summary()
    peak = None
    if usage is not None:
        peak = int(usage.ru_maxrss) * (1 if sys.platform == "darwin" else 1024)
        result["cpu_user_seconds"] = usage.ru_utime
        result["cpu_system_seconds"] = usage.ru_stime
    result["memory"]["os_peak_rss_bytes"] = peak
    result["memory"]["observed_peak_rss_bytes"] = max(peak or 0, samples.maximum) if peak is not None or samples.count else None
    result["memory"]["rss_limit_bytes"] = rss_limit_bytes or None
    result["memory"]["live_rss_sampling_available"] = samples.count > 0
    observed_peak = result["memory"]["observed_peak_rss_bytes"]
    result["memory"]["rss_limit_exceeded"] = bool(rss_limit_bytes and observed_peak is not None and observed_peak > rss_limit_bytes)
    if peak is not None and rss_limit_bytes and peak > rss_limit_bytes and result["status"] == "passed":
        result["status"] = "rss_limit"
        result["limit_detected_after_exit"] = True
    return result


def summarize_trace(path: Path) -> Dict[str, Any]:
    result = {"records": 0, "actions": 0, "sessions_completed": 0,
              "malformed_records": 0, "incomplete_tail": False,
              "last_action": None, "result": None,
              "max_live_allocated_bytes": None, "peak_allocated_bytes": None,
              "first_session_end": None, "last_session_end": None,
              "session_end_live_growth_bytes": None,
              "session_end_live_growth_bytes_per_session": None}  # type: Dict[str, Any]
    if not path.exists():
        result["missing"] = True
        return result
    with path.open("rb") as stream:
        for line in stream:
            try:
                record = json.loads(line)
                if not isinstance(record, dict):
                    raise ValueError("record is not an object")
            except (ValueError, UnicodeError):
                result["malformed_records"] += 1
                result["incomplete_tail"] = not line.endswith(b"\n")
                continue
            result["records"] += 1
            kind = record.get("type")
            if kind == "action":
                result["actions"] += 1
                result["last_action"] = {"session": record.get("session"), "index": record.get("index")}
            if kind == "result":
                result["result"] = record
            memory = record.get("memory", {})
            for field, target in [("live_allocated_bytes", "max_live_allocated_bytes"),
                                  ("peak_allocated_bytes", "peak_allocated_bytes")]:
                value = memory.get(field) if isinstance(memory, dict) else None
                if isinstance(value, int) and value >= 0:
                    result[target] = max(result[target] or 0, value)
            if kind == "session_end":
                sample = {"index": record.get("index"), "memory": memory, "stats": record.get("stats")}
                result["sessions_completed"] += 1
                if result["first_session_end"] is None:
                    result["first_session_end"] = sample
                result["last_session_end"] = sample
    first, last = result["first_session_end"], result["last_session_end"]
    growth = None
    if result["sessions_completed"] >= 2:
        first_live = first["memory"].get("live_allocated_bytes")
        last_live = last["memory"].get("live_allocated_bytes")
        if isinstance(first_live, int) and isinstance(last_live, int):
            growth = last_live - first_live
    result["session_end_live_growth_bytes"] = growth
    result["session_end_live_growth_bytes_per_session"] = growth / (result["sessions_completed"] - 1) if growth is not None else None
    return result


def prepare_replay(source: Path, directory: Path) -> Dict[str, Any]:
    """Keep the original; omit only a torn final JSON record from replay input."""
    original = directory / "replay-original.jsonl"
    shutil.copyfile(source, original)
    destination = directory / "replay-input.jsonl"
    discarded = 0
    with original.open("rb") as stream, destination.open("wb") as output:
        for number, line in enumerate(stream, 1):
            try:
                json.loads(line)
            except (ValueError, UnicodeError):
                if not line.endswith(b"\n") and not stream.read(1):
                    discarded = len(line)
                    break
                raise ValueError("invalid replay JSON on line {}".format(number))
            output.write(line)
    return {"source": str(source), "sha256": file_sha256(original),
            "original": str(original), "input": str(destination),
            "discarded_incomplete_tail_bytes": discarded}


def positive_int(value: str) -> int:
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def positive_float(value: str) -> float:
    number = float(value)
    if not 0 < number < float("inf"):
        raise argparse.ArgumentTypeError("must be finite and positive")
    return number


def options(argv: Optional[List[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=["core", "model", "projection", "all"], default="core")
    parser.add_argument("--seed", type=int, default=1, help="first unsigned 64-bit seed")
    parser.add_argument("--cases", type=positive_int, default=10, help="consecutive seeds per suite")
    parser.add_argument("--steps", type=positive_int, default=1000)
    parser.add_argument("--sessions", type=positive_int, default=1, help="fresh sessions in each child process")
    parser.add_argument("--timeout", type=positive_float, default=60, help="seconds per child (default: 60)")
    parser.add_argument("--rss-limit-mb", type=positive_float, default=2048, help="child RSS ceiling in MiB (default: 2048)")
    parser.add_argument("--sample-interval", type=positive_float, default=0.1, help="RSS polling seconds (default: .1)")
    parser.add_argument("--output", type=Path, help="new or empty artifact directory")
    parser.add_argument("--no-build", action="store_true")
    parser.add_argument("--binary", type=Path, help="runner executable; default: target/release/examples/fuzz_backend")
    parser.add_argument("--replay", type=Path, help="replay one trace instead of generating a campaign")
    parser.add_argument("--keep-going", action="store_true", help="continue after findings")
    args = parser.parse_args(argv)
    if not 0 <= args.seed <= U64_MAX or args.seed + args.cases - 1 > U64_MAX:
        parser.error("seed campaign must fit in unsigned 64-bit values")
    if args.sample_interval > args.timeout:
        parser.error("sample interval must not exceed timeout")
    if args.replay and not args.replay.is_file():
        parser.error("replay input does not exist")
    return args


def main(argv: Optional[List[str]] = None) -> int:
    args = options(argv)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    output = (args.output or REPO / "target" / "fuzz-campaigns" / "{}-{}".format(stamp, os.getpid())).resolve()
    if output.exists() and (not output.is_dir() or any(output.iterdir())):
        print("Refusing to overwrite nonempty output: {}".format(output), file=sys.stderr)
        return 2
    output.mkdir(parents=True, exist_ok=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR", str(REPO / "target")))
    if not target.is_absolute():
        target = REPO / target
    binary = (args.binary or target / "release" / "examples" / "fuzz_backend").resolve()
    git_status = capture(["git", "status", "--porcelain=v1", "--untracked-files=normal"])
    metadata = {"started_at": utc_now(), "argv": argv if argv is not None else sys.argv[1:],
                "repository": str(REPO), "platform": platform.platform(), "python": sys.version,
                "git_head": capture(["git", "rev-parse", "HEAD"]), "git_status": git_status,
                "git_dirty": bool(git_status["stdout"]) if git_status.get("returncode") == 0 else None,
                "rustc": capture(["rustc", "--version"]), "cargo": capture(["cargo", "--version"]),
                "binary": str(binary), "limits": {"timeout_seconds": args.timeout,
                    "rss_limit_bytes": int(args.rss_limit_mb * MIB), "sample_interval_seconds": args.sample_interval}}
    write_json(output / "metadata.json", metadata)
    print("Artifacts: {}".format(output), flush=True)
    if not args.no_build:
        build_command = ["cargo", "build", "--release", "--example", "fuzz_backend"]
        with (output / "build.stdout.log").open("wb") as stdout, (output / "build.stderr.log").open("wb") as stderr:
            try:
                code = subprocess.call(build_command, cwd=REPO, stdout=stdout, stderr=stderr)
            except OSError as error:
                code = 127
                stderr.write(str(error).encode())
        metadata["build"] = {"command": build_command, "returncode": code}
        write_json(output / "metadata.json", metadata)
        if code:
            print("Build failed; see {}".format(output / "build.stderr.log"), file=sys.stderr)
            write_json(output / "summary.json", {"status": "build_failed", "returncode": code})
            return 2
    if not binary.is_file() or not os.access(binary, os.X_OK):
        print("Runner is not executable: {}".format(binary), file=sys.stderr)
        return 2
    metadata["binary_sha256"] = file_sha256(binary)
    metadata["binary_mtime_ns"] = binary.stat().st_mtime_ns
    write_json(output / "metadata.json", metadata)
    suites = ["core", "model", "projection"] if args.suite == "all" else [args.suite]
    cases = [(suite, args.seed + index) for suite in suites for index in range(args.cases)]
    if args.replay:
        cases = [("replay", None)]
    summary = {"status": "passed", "cases_planned": len(cases), "cases_completed": 0,
               "failures": 0, "observed_peak_rss_bytes": 0, "cases": []}  # type: Dict[str, Any]
    for number, (suite, seed) in enumerate(cases, 1):
        name = "{:05d}-{}{}".format(number, suite, "-seed-{}".format(seed) if seed is not None else "")
        directory = output / name
        directory.mkdir()
        trace = directory / "trace.jsonl"
        command = [str(binary)]
        replay_info = None
        if args.replay:
            try:
                replay_info = prepare_replay(args.replay.resolve(), directory)
            except (OSError, ValueError) as error:
                write_json(directory / "result.json", {"status": "invalid_replay", "error": str(error)})
                print(str(error), file=sys.stderr)
                return 2
            command += ["--replay", replay_info["input"]]
        else:
            command += ["--suite", suite, "--seed", str(seed), "--steps", str(args.steps)]
            if args.sessions > 1:
                command += ["--sessions", str(args.sessions)]
        command += ["--trace", str(trace)]
        write_json(directory / "command.json", {"command": command, "replay": replay_info})
        result = run_monitored(command, directory, args.timeout, int(args.rss_limit_mb * MIB), args.sample_interval)
        result["trace"] = summarize_trace(trace)
        if replay_info:
            result["replay"] = replay_info
        trace_result = result["trace"].get("result")
        if result["status"] == "passed" and (result["trace"]["malformed_records"] or not trace_result or trace_result.get("status") != "ok"):
            result["status"] = "invalid_trace"
        replay_command = [sys.executable, str(Path(__file__).resolve()), "--no-build", "--binary", str(binary), "--replay", str(trace)]
        result["replay_command"] = replay_command
        write_json(directory / "result.json", result)
        peak = result["memory"]["observed_peak_rss_bytes"]
        summary["observed_peak_rss_bytes"] = max(summary["observed_peak_rss_bytes"], peak or 0)
        summary["cases_completed"] += 1
        summary["cases"].append({"name": name, "status": result["status"], "result": str(directory / "result.json")})
        if result["status"] != "passed":
            summary["failures"] += 1
            summary["status"] = "failed"
        growth = result["trace"]["session_end_live_growth_bytes"]
        memory_text = "RSS peak {:.1f} MiB".format(peak / MIB) if peak is not None else "RSS unavailable"
        if not result["memory"]["live_rss_sampling_available"]:
            memory_text += "; no live RSS samples"
        if growth is not None:
            memory_text += "; session-end live heap growth {:+d} bytes".format(growth)
        print("{}: {} ({})".format(name, result["status"], memory_text), flush=True)
        if result["status"] != "passed":
            print("Replay: {}".format(shlex.join(replay_command)), flush=True)
        write_json(output / "summary.json", summary)
        if result["status"] == "interrupted":
            return 130
        if result["status"] != "passed" and not args.keep_going:
            break
    print("{}: {}/{} cases completed, {} findings".format(summary["status"], summary["cases_completed"], summary["cases_planned"], summary["failures"]), flush=True)
    return 1 if summary["failures"] else 0


if __name__ == "__main__":
    sys.exit(main())
