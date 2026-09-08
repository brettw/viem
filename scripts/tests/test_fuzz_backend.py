"""Watchdog and artifact regressions; no Rust build or third-party packages needed."""

import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "fuzz-backend.py"
SPEC = importlib.util.spec_from_file_location("fuzz_backend_supervisor", SCRIPT)
fuzz = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fuzz)


class SupervisorTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="viem-fuzz-test-")
        self.directory = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def run_child(self, code, **limits):
        return fuzz.run_monitored([sys.executable, "-c", code], self.directory,
                                  timeout=limits.get("timeout", 5),
                                  rss_limit_bytes=limits.get("rss_limit_bytes", 512 * fuzz.MIB),
                                  sample_interval=0.02)

    def test_completion_preserves_output_and_per_child_peak(self):
        result = self.run_child("import sys; data=bytearray(12*1024*1024); print('hello'); print('diagnostic',file=sys.stderr)")
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["exit_code"], 0)
        self.assertIsNone(result["signal"])
        self.assertGreater(result["memory"]["os_peak_rss_bytes"], 10 * fuzz.MIB)
        self.assertEqual((self.directory / "stdout.log").read_text(), "hello\n")
        self.assertEqual((self.directory / "stderr.log").read_text(), "diagnostic\n")

    @unittest.skipUnless(sys.platform == "darwin", "macOS libproc regression")
    def test_macos_rss_sampling_does_not_require_launching_ps(self):
        child = subprocess.Popen(
            [sys.executable, "-c", "import time; data=bytearray(8*1024*1024); time.sleep(5)"]
        )
        try:
            time.sleep(0.05)
            with mock.patch.object(fuzz.subprocess, "run", side_effect=PermissionError("ps denied")):
                self.assertGreater(fuzz.sample_rss(child.pid), 8 * fuzz.MIB)
        finally:
            child.terminate()
            child.wait(timeout=5)

    def test_nonzero_and_signal_are_distinct(self):
        result = self.run_child("raise SystemExit(7)")
        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["exit_code"], 7)
        result = self.run_child("import os,signal; os.kill(os.getpid(),signal.SIGTERM)")
        self.assertEqual(result["signal"], signal.SIGTERM)
        self.assertEqual(result["signal_name"], "SIGTERM")
        self.assertIsNone(result["exit_code"])

    def test_timeout_stops_descendant_but_not_unrelated_process(self):
        marker = self.directory / "escaped-child"
        descendant = "import time,pathlib; time.sleep(.8); pathlib.Path({!r}).write_text('escaped')".format(str(marker))
        parent = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',{!r}]); time.sleep(5)".format(descendant)
        unrelated = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(5)"], start_new_session=True)
        try:
            result = self.run_child(parent, timeout=0.2)
            self.assertEqual(result["status"], "timeout")
            self.assertLess(result["elapsed_seconds"], 2)
            self.assertIsNone(unrelated.poll())
            time.sleep(0.85)
            self.assertFalse(marker.exists())
        finally:
            unrelated.terminate()
            unrelated.wait(timeout=5)

    def test_rss_ceiling_is_reported(self):
        result = self.run_child("import time; data=bytearray(16*1024*1024); time.sleep(5)", rss_limit_bytes=fuzz.MIB)
        self.assertEqual(result["status"], "rss_limit")
        self.assertGreater(result["memory"]["observed_peak_rss_bytes"], fuzz.MIB)
        self.assertLess(result["elapsed_seconds"], 2)

    def test_unresponsive_child_is_force_killed(self):
        result = self.run_child("import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(5)", timeout=0.2)
        self.assertEqual(result["status"], "timeout")
        self.assertEqual(result["signal"], signal.SIGKILL)
        self.assertLess(result["elapsed_seconds"], 2)

    def test_requested_memory_ceiling_fails_closed_without_live_sampling(self):
        with mock.patch.object(fuzz, "sample_rss", return_value=None):
            result = self.run_child("import time; time.sleep(5)")
        self.assertEqual(result["status"], "rss_monitor_unavailable")
        self.assertFalse(result["memory"]["live_rss_sampling_available"])
        self.assertLess(result["elapsed_seconds"], 2)

    def test_trace_reports_partial_tail_and_live_heap_after_session_drop(self):
        records = [
            {"type": "header", "memory": {"live_allocated_bytes": 10, "peak_allocated_bytes": 10}},
            {"type": "action", "session": 0, "index": 0, "action": {"insert": "hello"}},
            {"type": "memory", "memory": {"live_allocated_bytes": 1000, "peak_allocated_bytes": 1500}},
            {"type": "session_end", "index": 0, "memory": {"live_allocated_bytes": 100, "peak_allocated_bytes": 1500}},
            {"type": "session_end", "index": 1, "memory": {"live_allocated_bytes": 110, "peak_allocated_bytes": 2000}},
        ]
        trace = self.directory / "trace.jsonl"
        original = "".join(json.dumps(record) + "\n" for record in records).encode() + b'{"type":"action"'
        trace.write_bytes(original)
        summary = fuzz.summarize_trace(trace)
        self.assertTrue(summary["incomplete_tail"])
        self.assertEqual(summary["last_action"], {"session": 0, "index": 0})
        self.assertEqual(summary["session_end_live_growth_bytes"], 10)
        self.assertEqual(summary["max_live_allocated_bytes"], 1000)
        self.assertEqual(summary["peak_allocated_bytes"], 2000)
        replay_dir = self.directory / "replay"
        replay_dir.mkdir()
        prepared = fuzz.prepare_replay(trace, replay_dir)
        self.assertEqual(Path(prepared["original"]).read_bytes(), original)
        self.assertEqual(Path(prepared["input"]).read_bytes(), original[:-len(b'{"type":"action"')])
        self.assertGreater(prepared["discarded_incomplete_tail_bytes"], 0)
        self.assertEqual(trace.read_bytes(), original)

    def test_replay_rejects_corruption_before_tail(self):
        trace = self.directory / "broken.jsonl"
        trace.write_text('{"type":"header"}\nnot-json\n')
        with self.assertRaisesRegex(ValueError, "line 2"):
            fuzz.prepare_replay(trace, self.directory)

    def test_campaign_and_replay_artifacts_without_cargo_build(self):
        binary = self.directory / "fake-runner"
        binary.write_text("#!{}\n".format(sys.executable) + """
import json,pathlib,sys
args=dict(zip(sys.argv[1::2],sys.argv[2::2]))
records=[{'type':'header','suite':args.get('--suite','core')}]
records += [{'type':'session_end','index':index,'memory':{'live_allocated_bytes':10,'peak_allocated_bytes':20}} for index in range(int(args.get('--sessions','1')))]
records += [{'type':'result','status':'ok'}]
pathlib.Path(args['--trace']).write_text(''.join(json.dumps(r)+'\\n' for r in records))
print(json.dumps(records[-1]))
""")
        binary.chmod(0o755)
        output = self.directory / "campaign"
        command = [sys.executable, str(SCRIPT), "--no-build", "--binary", str(binary),
                   "--output", str(output), "--suite", "all", "--seed", "41", "--cases", "2", "--sessions", "2"]
        completed = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        summary = json.loads((output / "summary.json").read_text())
        self.assertEqual(summary["cases_completed"], 6)
        self.assertEqual([case["name"] for case in summary["cases"]], [
            "00001-core-seed-41", "00002-core-seed-42", "00003-model-seed-41", "00004-model-seed-42",
            "00005-projection-seed-41", "00006-projection-seed-42"])
        metadata = json.loads((output / "metadata.json").read_text())
        self.assertIn("git_dirty", metadata)
        self.assertEqual(metadata["binary_sha256"], fuzz.file_sha256(binary))
        case = output / summary["cases"][0]["name"]
        result = json.loads((case / "result.json").read_text())
        self.assertEqual(result["trace"]["session_end_live_growth_bytes"], 0)
        for artifact in ["command.json", "trace.jsonl", "stdout.log", "stderr.log", "memory.csv"]:
            self.assertTrue((case / artifact).exists())
        replay_output = self.directory / "replayed"
        completed = subprocess.run([sys.executable, str(SCRIPT), "--no-build", "--binary", str(binary),
            "--output", str(replay_output), "--replay", str(case / "trace.jsonl")],
            text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        replay_case = replay_output / "00001-replay"
        self.assertEqual((replay_case / "replay-original.jsonl").read_bytes(), (case / "trace.jsonl").read_bytes())
        repeated = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
        self.assertEqual(repeated.returncode, 2)

    def test_missing_trace_is_reportable_after_launch_failure(self):
        result = fuzz.run_monitored([str(self.directory / "missing")], self.directory, 1, fuzz.MIB, .02)
        self.assertEqual(result["status"], "launch_error")
        summary = fuzz.summarize_trace(self.directory / "trace.jsonl")
        self.assertTrue(summary["missing"])
        self.assertIsNone(summary["session_end_live_growth_bytes"])


if __name__ == "__main__":
    unittest.main()
