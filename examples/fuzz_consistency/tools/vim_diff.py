"""Compare Viem's Normal-mode results with reference Vim.

Usage:
    fuzz_consistency vimcases cases.jsonl [--positions N] [--sources list.json]
    python3 vim_diff.py cases.jsonl report.jsonl

The first command runs every Normal-mode op from `normal_op_specs` at each
character of the literal (Plain/Code) seeds and records Viem's resulting
source. This script replays every case in the installed Vim (no user
configuration, Viem's indentation defaults, `nofixeol` so final line endings
are preserved as written) and writes one JSON line per disagreement.

Cases that Vim cannot represent are skipped: sources containing CR, and
cursors on Viem's empty line after a final line ending.
"""
import collections
import json
import os
import re
import subprocess
import sys
import tempfile

VIM_SETTINGS = (
    "set nocp ai et sw=2 sts=2 ts=2 sta bs=indent,eol,start ff=unix ffs=unix "
    "nofixeol tw=80 fo=q nojs encoding=utf-8 shortmess+=A noswapfile"
)


def vim_keys(spec):
    """Turn a fuzzer key spec such as `ccx<Esc>` into a Vim double-quoted string body."""
    out = []
    i = 0
    while i < len(spec):
        if spec[i] == "<" and ">" in spec[i:]:
            j = spec.index(">", i)
            out.append("\\<" + spec[i + 1 : j] + ">")
            i = j + 1
        else:
            ch = spec[i]
            out.append("\\\\" if ch == "\\" else '\\"' if ch == '"' else ch)
            i += 1
    return "".join(out)


def line_col(source, offset):
    raw = source.encode()
    before = raw[:offset]
    line = before.count(b"\n") + 1
    col = offset - (before.rfind(b"\n") + 1) + 1
    return line, col


def run_vim(source, cases, workdir):
    path = os.path.join(workdir, "buffer.txt")
    with open(path, "wb") as fh:
        fh.write(source.encode())
    written = os.path.join(workdir, "written.txt")
    script = [VIM_SETTINGS, "let g:results = []"]
    for case in cases:
        line, col = line_col(source, case["cursor"])
        script.append(f"silent! edit! {path}")
        script.append(VIM_SETTINGS)
        script.append(f"call cursor({line}, {col})")
        script.append(f'try | silent exe "normal {vim_keys(case["op"])}" | catch | endtry')
        script.append(f"silent! write! {written}")
        script.append(f"call add(g:results, readblob('{written}'))")
    out = os.path.join(workdir, "results.json")
    script.append(f"call writefile([json_encode(g:results)], '{out}')")
    script.append("qa!")
    script_path = os.path.join(workdir, "run.vim")
    with open(script_path, "w") as fh:
        fh.write("\n".join(script) + "\n")
    subprocess.run(
        ["vim", "-u", "NONE", "-i", "NONE", "-N", "-es", "-S", script_path],
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        timeout=600,
    )
    with open(out) as fh:
        raw = json.loads(fh.read())
    return [bytes(blob).decode("utf-8", "replace") for blob in raw]


def main():
    cases_path, report_path = sys.argv[1], sys.argv[2]
    by_source = collections.defaultdict(list)
    for line in open(cases_path):
        case = json.loads(line)
        source = case["source"]
        if "\r" in source:
            continue
        if source.endswith("\n") and case["cursor"] >= len(source.encode()) :
            continue
        by_source[source].append(case)
    mismatches = 0
    total = 0
    with tempfile.TemporaryDirectory() as workdir, open(report_path, "w") as report:
        for source, cases in by_source.items():
            vim_results = run_vim(source, cases, workdir)
            for case, vim in zip(cases, vim_results):
                total += 1
                if case["result"] != vim:
                    mismatches += 1
                    case["vim"] = vim
                    report.write(json.dumps(case, ensure_ascii=False) + "\n")
    print(f"{total} cases, {mismatches} differ from Vim", file=sys.stderr)


if __name__ == "__main__":
    main()
