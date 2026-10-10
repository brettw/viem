import json, sys, glob
root, out = sys.argv[1], sys.argv[2]
merged = {}
for f in sorted(glob.glob(f"{root}/c*/walk-*-findings.jsonl")):
    seen = set()
    for line in open(f):
        try:
            d = json.loads(line)
            key = d['key']
        except Exception:
            continue
        m = merged.setdefault(key, {'count': 0, 'examples': []})
        if key not in seen:
            seen.add(key); m['count'] += d['count']
        if len(m['examples']) < 12: m['examples'].append(d['finding'])
with open(out, 'w') as fh:
    for k, m in merged.items():
        for e in m['examples']:
            fh.write(json.dumps({"key": k, "count": m['count'], "finding": e}) + "\n")
print(len(merged))
