import json, sys, re
path = sys.argv[1]
include = re.compile(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[2] else None
exclude = re.compile(sys.argv[3]) if len(sys.argv) > 3 and sys.argv[3] else None
n = int(sys.argv[4]) if len(sys.argv) > 4 else 3
groups = {}
for line in open(path):
    d = json.loads(line)
    k = d['key']
    if include and not include.search(k): continue
    if exclude and exclude.search(k): continue
    g = groups.setdefault(k, [d['count'], []])
    g[1].append(d['finding'])
for k, (count, fs) in sorted(groups.items(), key=lambda kv: -kv[1][0]):
    fs.sort(key=lambda f: (len(f['source']), len(f['trace'])))
    print(f"{count:6} {k[:160]}")
    seen = set()
    shown = 0
    for f in fs:
        if f['source'] in seen: continue
        seen.add(f['source'])
        print(f"          {f['source'][:60]!r:64} {f['context'][:110]}")
        shown += 1
        if shown >= n: break
