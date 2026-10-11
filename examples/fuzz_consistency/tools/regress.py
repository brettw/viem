"""Replay the scripted BUGS.md reproductions against a fuzz_consistency build.

Usage (from the repository root):
    cargo build --release --example fuzz_consistency
    python3 examples/fuzz_consistency/tools/regress.py [ID_REGEX]
    FC=path/to/older/fuzz_consistency python3 examples/fuzz_consistency/tools/regress.py

Cases live in regress_cases.py. Each runs `fuzz_consistency case FILE` in its
own process with a timeout, so a hang is reported instead of stalling the run.
Results are also written to regress_results.json in the current directory.
"""
import json, os, re, subprocess, sys, tempfile, concurrent.futures as cf

BIN = os.environ.get("FC", "target/release/examples/fuzz_consistency")
TMP = tempfile.mkdtemp(prefix="viem-regress")

NAMED = {"CR": "Enter", "Esc": "Esc", "BS": "BS", "Del": "Del", "Tab": "Tab"}


def K(spec):
    out = []
    i = 0
    while i < len(spec):
        c = spec[i]
        if c == "<":
            j = spec.find(">", i)
            if j > i + 1:
                name = spec[i + 1:j]
                out.append({"op": "key", "key": NAMED.get(name, name)})
                i = j + 1
                continue
        out.append({"op": "key", "key": c})
        i += 1
    return out


def T(text):
    return [{"op": "text", "text": text}]


def P(offset, down=True, extend=False):
    return [{"op": "place", "offset": offset, "downstream": down, "extend": extend}]


def SEL(a, b):
    """Native selection a..b via pointer gesture then extend."""
    return [{"op": "pointer", "offset": a}, {"op": "place", "offset": b, "downstream": True, "extend": True}]


def W(offset, extend=False):
    return [{"op": "word", "offset": offset, "extend": extend}]


def A(op, **kw):
    d = {"op": op}
    d.update(kw)
    return [d]


def EX(cmd):
    return K(":" + cmd + "<CR>")


CASES = []


def case(cid, fmt, source, acts, check="ok", width=600, height=2000, filename=None, note="", timeout=30):
    flat = []
    for a in acts:
        flat.extend(a if isinstance(a, list) else [a])
    CASES.append(dict(index=len(CASES), id=cid, fmt=fmt, source=source, acts=flat, check=check, width=width, height=height,
                      filename=filename, note=note, timeout=timeout))


def run(c):
    path = os.path.join(TMP, "c%05d.json" % c["index"])
    with open(path, "w") as f:
        json.dump({"fmt": c["fmt"], "source": c["source"], "acts": c["acts"], "width": c["width"],
                   "height": c["height"], "filename": c["filename"]}, f)
    try:
        p = subprocess.run([BIN, "case", path], capture_output=True, text=True, timeout=c["timeout"])
        if p.returncode != 0:
            return c, None, f"exit {p.returncode}: {p.stderr[-400:]}"
        return c, json.loads(p.stdout.strip().splitlines()[-1]), None
    except subprocess.TimeoutExpired:
        return c, None, "TIMEOUT (hang)"


def evaluate(c, res, err):
    if err:
        return False, err
    if "open_error" in res:
        return False, res["open_error"]
    steps = res["steps"]
    check = c["check"]
    bad = [(i, s) for i, s in enumerate(steps) if not s["ok"] or s["problems"]]
    final = steps[-1] if steps else None
    if callable(check):
        try:
            r = check(steps)
        except Exception as e:  # noqa
            r = f"check raised {e!r}"
        if r is True or r is None:
            return True, ""
        return False, r if isinstance(r, str) else "custom check failed"
    if check == "ok":
        if bad:
            i, s = bad[0]
            return False, f"step {i} {s['act']} -> {s['result']} problems={s['problems'][:2]} src={s['source'][:120]!r}"
        return True, ""
    if isinstance(check, tuple):
        kind, want = check
        if kind == "src":
            if bad:
                i, s = bad[0]
                return False, f"step {i} {s['act']} -> {s['result']} problems={s['problems'][:2]} src={s['source'][:120]!r}"
            if final["source"] != want:
                return False, f"source {final['source']!r} != expected {want!r}"
            return True, ""
        if kind == "text":
            if bad:
                i, s = bad[0]
                return False, f"step {i} {s['act']} -> {s['result']} problems={s['problems'][:2]}"
            if final["text"] != want:
                return False, f"text {final['text']!r} != expected {want!r} (src {final['source']!r})"
            return True, ""
        if kind == "noproblems":
            # Errors allowed (policy) but no invariant problems and no internal errors.
            probs = [(i, s) for i, s in enumerate(steps) if s["problems"]]
            if probs:
                i, s = probs[0]
                return False, f"step {i} {s['act']} problems={s['problems'][:2]}"
            internal = [(i, s) for i, s in enumerate(steps) if re.search(want, s["result"])]
            if internal:
                i, s = internal[0]
                return False, f"step {i} {s['act']} -> {s['result']}"
            return True, ""
    return False, f"unknown check {check!r}"


def main():
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import regress_cases  # noqa: F401  (registers cases)
    filt = re.compile(sys.argv[1]) if len(sys.argv) > 1 else None
    cases = [c for c in CASES if not filt or filt.search(c["id"])]
    results = []
    with cf.ThreadPoolExecutor(int(os.environ.get("JOBS", "6"))) as ex:
        for c, res, err in ex.map(run, cases):
            ok, why = evaluate(c, res, err)
            results.append((c, ok, why, res))
            print(("PASS " if ok else "FAIL ") + c["id"] + ("" if ok else "  :: " + why), flush=True)
    fails = [r for r in results if not r[1]]
    print(f"\n{len(results) - len(fails)}/{len(results)} pass")
    with open("regress_results.json", "w") as f:
        json.dump([{"id": c["id"], "ok": ok, "why": why, "fmt": c["fmt"], "source": c["source"], "acts": c["acts"],
                    "note": c["note"], "steps": (res or {}).get("steps")} for c, ok, why, res in results], f)


if __name__ == "__main__":
    sys.modules["regress"] = sys.modules[__name__]
    main()
