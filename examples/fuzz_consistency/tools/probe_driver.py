"""Run one fuzz_consistency probe per seed in its own process with a timeout,
then merge findings.
Usage: SEEDS=N JOBS=J probe_driver.py BINARY OUTDIR PROBE [extra probe args...]"""
import json, os, subprocess, sys, time, concurrent.futures as cf
binary, out, probe = sys.argv[1], sys.argv[2], sys.argv[3]
extra = sys.argv[4:]
timeout = 600
os.makedirs(out, exist_ok=True)
# Count seeds: run the binary with an impossible filter? Instead ask via --seed-index probing.
count = int(os.environ.get("SEEDS", "0"))
results = []
def run(i):
    d = os.path.join(out, f"s{i:04d}")
    os.makedirs(d, exist_ok=True)
    t = time.time()
    try:
        p = subprocess.run([binary, "probe", "--probes", probe, "--threads", "1", "--seed-index", str(i), "--out", d] + extra,
                           capture_output=True, text=True, timeout=timeout)
        status = p.returncode
        err = p.stderr[-2000:]
    except subprocess.TimeoutExpired:
        status = "timeout"; err = ""
    return i, status, time.time() - t, err
with cf.ThreadPoolExecutor(int(os.environ.get("JOBS", "18"))) as ex:
    for i, status, secs, err in ex.map(run, range(count)):
        if status != 0 or secs > 60:
            print(f"seed {i}: status {status} {secs:.1f}s {err.strip()[-300:]}", flush=True)
# Merge
merged = {}
for i in range(count):
    f = os.path.join(out, f"s{i:04d}", f"{probe}-findings.jsonl")
    if not os.path.exists(f): continue
    for line in open(f):
        d = json.loads(line)
        m = merged.setdefault(d['key'], {'count': 0, 'seen': set(), 'examples': []})
        if (i, d['key']) not in m['seen']:
            m['seen'].add((i, d['key'])); m['count'] += d['count']
        if len(m['examples']) < 12:
            m['examples'].append(d['finding'])
with open(os.path.join(out, f"{probe}-findings.jsonl"), "w") as fh:
    for k, m in merged.items():
        for e in m['examples']:
            fh.write(json.dumps({"key": k, "count": m['count'], "finding": e}) + "\n")
print("merged", len(merged), "signatures")
