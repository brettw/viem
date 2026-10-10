"""Run walk chunks in separate processes with a timeout, then merge findings.
Usage: JOBS=J walk_driver.py BINARY OUTDIR FIRST_SEED CHUNKS CASES_PER_CHUNK STEPS"""
import json, os, subprocess, sys, time, concurrent.futures as cf
binary, out = sys.argv[1], sys.argv[2]
first, chunks, per, steps = map(int, sys.argv[3:7])
os.makedirs(out, exist_ok=True)
def run(i):
    seed = first + i * per
    d = os.path.join(out, f"c{i:04d}")
    os.makedirs(d, exist_ok=True)
    t = time.time()
    try:
        p = subprocess.run([binary, "walk", "--seed", str(seed), "--cases", str(per), "--steps", str(steps),
                            "--threads", "1", "--out", d], capture_output=True, text=True, timeout=900)
        status = p.returncode
    except subprocess.TimeoutExpired:
        status = "timeout"
    return i, seed, status, time.time() - t
with cf.ThreadPoolExecutor(int(os.environ.get("JOBS", "18"))) as ex:
    for i, seed, status, secs in ex.map(run, range(chunks)):
        if status != 0:
            print(f"chunk {i} seeds {seed}..{seed+per}: {status} {secs:.0f}s", flush=True)
merged = {}
for i in range(chunks):
    seed = first + i * per
    f = os.path.join(out, f"c{i:04d}", f"walk-{seed}-findings.jsonl")
    if not os.path.exists(f): continue
    seen = set()
    for line in open(f):
        d = json.loads(line)
        m = merged.setdefault(d['key'], {'count': 0, 'examples': []})
        if d['key'] not in seen:
            seen.add(d['key']); m['count'] += d['count']
        if len(m['examples']) < 12:
            m['examples'].append(d['finding'])
with open(os.path.join(out, "walk-findings.jsonl"), "w") as fh:
    for k, m in merged.items():
        for e in m['examples']:
            fh.write(json.dumps({"key": k, "count": m['count'], "finding": e}) + "\n")
print("merged", len(merged), "signatures")
