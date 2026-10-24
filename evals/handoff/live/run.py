#!/usr/bin/env python3
"""Tier-2 live handoff eval: A does half, B (other provider, same ledger) finishes.

  python3 evals/handoff/live/run.py [--cases 01,02] [--jobs 2] [--timeout 420]

Sends real requests to the logged-in agents (about 2 per case). Directions
alternate claude→codex / codex→claude. Output: results.json next to this file.
"""
import argparse, json, os, shutil, subprocess, sys, tempfile, time
from concurrent.futures import ThreadPoolExecutor
sys.path.insert(0, os.path.dirname(__file__))
from cases import CASES

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../.."))
CH = os.path.join(ROOT, "target/debug/chorus")

def sh(cmd, cwd, timeout=60):
    try:
        return subprocess.run(cmd, shell=True, cwd=cwd, capture_output=True, text=True, timeout=timeout).returncode
    except subprocess.TimeoutExpired:
        return 124

def check(c, w):
    k = c[0]
    if k == "exists": return os.path.exists(os.path.join(w, c[1]))
    if k in ("has", "lacks"):
        p = os.path.join(w, c[1])
        t = open(p, encoding="utf8", errors="replace").read() if os.path.exists(p) else None
        if t is None: return False
        return (c[2] in t) == (k == "has")
    if k == "run": return sh(c[1], w) == 0

def turn(args, cwd, timeout):
    t = time.time()
    try:
        p = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout + p.stderr, time.time() - t
    except subprocess.TimeoutExpired as e:
        return 124, "timeout", time.time() - t

def run_case(i, case, timeout):
    a, b = ("claude", "codex") if i % 2 == 0 else ("codex", "claude")
    d = tempfile.mkdtemp(prefix="chorus-live-")
    w = os.path.join(d, "w"); os.makedirs(w)
    sh("git init -q && git config user.email e@e && git config user.name e", w)
    for f, t in case["files"].items():
        os.makedirs(os.path.dirname(os.path.join(w, f)) or w, exist_ok=True)
        open(os.path.join(w, f), "w").write(t)
    sh("git add -A && git commit -qm seed --allow-empty", w)
    db = os.path.join(d, "s.db")
    rc1, o1, t1 = turn([CH, "run", w, case["p1"], "--chain", a, "--yes", "--db", db], w, timeout)
    # sanity: did A's half exist at all? (a dead A is not a handoff failure)
    rc2, o2, t2 = turn([CH, "run", case["p2"], "--session", "1", "--chain", b, "--yes", "--db", db], w, timeout)
    results = [bool(check(c, w)) for c in case["checks"]]
    return dict(id=case["id"], direction=f"{a}->{b}", passed=all(results) and rc1 == 0 and rc2 == 0,
                checks=results, rc=[rc1, rc2], secs=[round(t1), round(t2)], dir=d,
                tail=o2[-400:] if not all(results) else "")

if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--cases", default=""); ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--timeout", type=int, default=420)
    a = ap.parse_args()
    sel = [(i, c) for i, c in enumerate(CASES) if not a.cases or any(c["id"].startswith(x) for x in a.cases.split(","))]
    subprocess.run(["cargo", "build", "-q", "-p", "chorus-cli", "--bin", "chorus"], cwd=ROOT, check=True)
    with ThreadPoolExecutor(a.jobs) as ex:
        res = list(ex.map(lambda ic: run_case(ic[0], ic[1], a.timeout), sel))
    for r in res: print(("PASS" if r["passed"] else "FAIL"), r["id"], r["direction"], r["checks"], r["rc"], r["secs"])
    n = sum(r["passed"] for r in res)
    print(f"\n{n}/{len(res)} continued correctly ({100*n//max(1,len(res))}%)")
    json.dump(res, open(os.path.join(os.path.dirname(__file__), "results.json"), "w"), indent=1)
