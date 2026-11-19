#!/usr/bin/env python3
"""Live handoff eval (R4): agent A does half, agent B (other provider, same ledger) finishes.

One command, real agents:

  python3 evals/handoff/live/eval.py                      # 16 real-work cases, one run each
  python3 evals/handoff/live/eval.py --matrix --repeat 3  # baseline + each ablation, 3 runs per cell

Without agents (what CI or a cloud box can do):

  python3 evals/handoff/live/eval.py --dry-run            # validate every case, run no agent

Every number printed comes from a real run: pass/fail from deterministic checks on the
resulting files, seconds from wall clock, tokens and cost from the ledger's own
`session_usage` row (NULL stays null, it is never filled in). Dry-run prints no metrics
at all. See README.md next to this file for how to read the output.
"""
import argparse, json, math, os, shutil, sqlite3, subprocess, sys, tempfile, threading, time
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import cases as toy_cases
import real_cases

ROOT = os.path.abspath(os.path.join(HERE, "../../.."))
CH = os.environ.get("CHORUS_EVAL_BIN") or os.path.join(ROOT, "target/debug/chorus")  # override only for harness tests
CARGO_TARGET = os.path.join(os.path.expanduser("~"), ".cache", "chorus-handoff-eval", "target")
CARGO_LOCK = threading.Lock()  # one cargo at a time across worker threads


# ── cases ────────────────────────────────────────────────────────────────────

def load_cases(suite):
    out = []
    if suite in ("real", "all"):
        out += [dict(c, suite="real") for c in real_cases.CASES]
    if suite in ("toy", "all"):
        for i, c in enumerate(toy_cases.CASES):
            out.append(dict(c, suite="toy", source="fixture:blank",
                            dir="claude->codex" if i % 2 == 0 else "codex->claude",
                            goal=c["p1"][:120], a_checks=[], oracle1=[], oracle2=[]))
    return out


REQUIRED = ["id", "source", "files", "dir", "goal", "p1", "p2", "a_checks", "checks"]
CHECK_ARITY = {"exists": 2, "has": 3, "lacks": 3, "run": 2, "slow": 2}


def validate_case(c, externals):
    """Static problems with a case's definition (no workspace needed)."""
    errs = []
    for k in REQUIRED:
        if k not in c:
            errs.append(f"missing {k}")
    if errs:
        return errs
    if c["dir"] not in ("claude->codex", "codex->claude"):
        errs.append(f"dir {c['dir']!r}")
    src = c["source"]
    kind, _, name = src.partition(":")
    if src != "chorus" and kind not in ("fixture", "external"):
        errs.append(f"source {src!r}")
    if kind == "fixture" and name not in real_cases.FIXTURES and c["suite"] == "real":
        errs.append(f"unknown fixture {name!r}")
    if kind == "external" and name not in externals:
        errs.append(f"external repo {name!r} not given (--external {name}=PATH)")
    if not c["checks"]:
        errs.append("no checks")
    for chk in list(c["checks"]) + list(c["a_checks"]):
        if chk[0] not in CHECK_ARITY or len(chk) != CHECK_ARITY[chk[0]]:
            errs.append(f"bad check {chk!r}")
    if c["p1"].strip() == c["p2"].strip():
        errs.append("p1 == p2")
    return errs


# ── workspaces ───────────────────────────────────────────────────────────────

def sh(cmd, cwd, timeout=90, env=None):
    try:
        return subprocess.run(cmd, shell=True, cwd=cwd, capture_output=True, text=True, timeout=timeout,
                              env=env, executable="/bin/bash").returncode
    except subprocess.TimeoutExpired:
        return 124


_chorus_tmpl = {}


def chorus_template():
    """`git archive` of the pinned snapshot, extracted once per process."""
    if "dir" not in _chorus_tmpl:
        d = tempfile.mkdtemp(prefix="chorus-eval-base-")
        p = subprocess.run(f"git -C '{ROOT}' archive {real_cases.BASE_SHA} | tar -x -C '{d}'", shell=True,
                           capture_output=True, text=True)
        if p.returncode != 0:
            raise RuntimeError(f"cannot extract {real_cases.BASE_SHA} from {ROOT}: {p.stderr.strip()[:200]} "
                               f"(fetch full history: git fetch --unshallow)")
        _chorus_tmpl["dir"] = d
    return _chorus_tmpl["dir"]


def seed(c, externals):
    d = tempfile.mkdtemp(prefix="chorus-live-")
    w = os.path.join(d, "w")
    kind, _, name = c["source"].partition(":")
    if c["source"] == "chorus":
        shutil.copytree(chorus_template(), w, symlinks=True)
    else:
        os.makedirs(w)
        if kind == "external":
            subprocess.run(f"git -C '{externals[name]}' archive HEAD | tar -x -C '{w}'", shell=True, check=True)
        else:
            files = real_cases.FIXTURES.get(name, {}) if c["suite"] == "real" else {}
            for f, t in files.items():
                write(w, f, t)
    for f, t in c["files"].items():
        write(w, f, t)
    sh("git init -q && git config user.email e@e && git config user.name e && git add -A && git commit -qm seed --allow-empty", w, 120)
    return d, w


def write(w, path, text):
    p = os.path.join(w, path)
    os.makedirs(os.path.dirname(p) or w, exist_ok=True)
    open(p, "w", encoding="utf8").write(text)


def apply_ops(w, ops):
    for op in ops:
        if op[0] == "write":
            write(w, op[1], op[2])
        elif op[0] == "sub":
            p = os.path.join(w, op[1])
            t = open(p, encoding="utf8").read()
            assert t.count(op[2]) == 1, f"oracle sub: {op[2][:50]!r} occurs {t.count(op[2])}x in {op[1]}"
            open(p, "w", encoding="utf8").write(t.replace(op[2], op[3]))
        elif op[0] == "append":
            p = os.path.join(w, op[1])
            open(p, "a", encoding="utf8").write(op[2])
        elif op[0] == "fn":
            op[1](w)
        else:
            raise ValueError(op)


# ── checks ───────────────────────────────────────────────────────────────────

def check(c, w, slow):
    """True/False, or None when skipped (a slow check without --slow)."""
    k = c[0]
    if k == "exists":
        return os.path.exists(os.path.join(w, c[1]))
    if k in ("has", "lacks"):
        p = os.path.join(w, c[1])
        if not os.path.exists(p):
            return False
        return (c[2] in open(p, encoding="utf8", errors="replace").read()) == (k == "has")
    if k == "run":
        return sh(c[1], w) == 0
    if k == "slow":
        if not slow:
            return None
        env = dict(os.environ, CARGO_TARGET_DIR=CARGO_TARGET)
        with CARGO_LOCK:
            return sh(c[1], w, 1500, env) == 0
    raise ValueError(c)


def run_checks(checks, w, slow):
    return [check(c, w, slow) for c in checks]


# ── dry run ──────────────────────────────────────────────────────────────────

def dry_case(c, externals, slow):
    """Prove the case is well-formed and satisfiable without any agent.

    seed: no check may pass for free (a no-op must fail), a_checks must not be met
    oracle1: a_checks pass, and the final checks are still not all met (B has work)
    oracle2: every final check passes
    """
    errs = validate_case(c, externals)
    if errs:
        return errs, []
    notes = []
    try:
        d, w = seed(c, externals)
    except Exception as e:  # noqa: BLE001 - report, don't crash the batch
        return [f"seed: {e}"], []
    try:
        s_final = run_checks(c["checks"], w, slow)
        s_a = run_checks(c["a_checks"], w, slow)
        if all(r in (True, None) for r in s_final) and any(r is True for r in s_final):
            errs.append("a no-op already satisfies every check")
        if c["a_checks"] and all(s_a):
            errs.append("a_checks already hold on the seed")
        if not c["oracle1"] and not c["oracle2"]:
            notes.append("no oracle (satisfiability not proven)")
            return errs, notes
        apply_ops(w, c["oracle1"])
        a = run_checks(c["a_checks"], w, slow)
        if not all(a):
            errs.append(f"after oracle1 a_checks = {a}")
        mid = run_checks(c["checks"], w, slow)
        if all(r in (True, None) for r in mid):
            errs.append("oracle1 alone already satisfies every check (turn 2 would be unnecessary)")
        apply_ops(w, c["oracle2"])
        fin = run_checks(c["checks"], w, slow)
        if not all(r in (True, None) for r in fin):
            errs.append(f"after oracle2 checks = {fin}")
        if None in fin:
            notes.append("slow checks skipped (--slow to run cargo)")
    except Exception as e:  # noqa: BLE001
        errs.append(f"{type(e).__name__}: {e}")
    finally:
        shutil.rmtree(d, ignore_errors=True)
    return errs, notes


# ── ablations ────────────────────────────────────────────────────────────────

def ablation_env(names):
    env, report = {}, []
    for n in names:
        if n not in real_cases.ABLATIONS:
            raise SystemExit(f"unknown ablation {n!r}; known: {', '.join(real_cases.ABLATIONS)}")
        a = real_cases.ABLATIONS[n]
        env[a["env"]] = a["value"]
        effective = sh(f"grep -rqF '{a['env']}' '{ROOT}/crates' --include=*.rs", ROOT, 30) == 0
        report.append(dict(name=n, env=a["env"], value=a["value"], read_by_core=effective, what=a["what"]))
    return env, report


# ── a real run ───────────────────────────────────────────────────────────────

def turn(args, cwd, timeout, env):
    t = time.time()
    try:
        p = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout, env=env)
        return p.returncode, p.stdout + p.stderr, time.time() - t
    except subprocess.TimeoutExpired:
        return 124, "timeout", time.time() - t


def usage(db):
    try:
        c = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        r = c.execute("select tokens_in, tokens_out, cost_usd, context_used from session_usage where sess=1").fetchone()
        c.close()
        return dict(tokens_in=r[0], tokens_out=r[1], cost_usd=r[2], context_used=r[3]) if r else None
    except sqlite3.Error:
        return None


def run_one(c, rep, cfg, a_to_b, via, timeout, externals, slow, keep):
    a, b = a_to_b
    d, w = seed(c, externals)
    db = os.path.join(d, "s.db")
    env = dict(os.environ, **cfg["env"])
    rc1, o1, t1 = turn([CH, "run", w, c["p1"], "--chain", a, "--yes", "--db", db], w, timeout, env)
    a_ok = rc1 == 0 and all(run_checks(c["a_checks"], w, slow))
    u1 = usage(db)
    th = 0.0
    rch = 0
    if via == "handoff" and a_ok:
        rch, oh, th = turn([CH, "handoff", "--db", db, "--session", "1", "--to", b, "--reason", "rate_limit",
                            "--goal", c["goal"], "--yes"], w, timeout, env)
    rc2, o2, t2 = (0, "", 0.0)
    if a_ok:
        rc2, o2, t2 = turn([CH, "run", c["p2"], "--session", "1", "--chain", b, "--yes", "--db", db], w, timeout, env)
    results = run_checks(c["checks"], w, slow) if a_ok else []
    u2 = usage(db)
    done = [r for r in results if r is not None]
    status = "a_dead" if not a_ok else ("pass" if done and all(done) and rc2 == 0 else "fail")
    row = dict(id=c["id"], suite=c["suite"], rep=rep, config=cfg["name"], direction=f"{a}->{b}", via=via,
               status=status, implied=bool(c.get("implied")), checks=results, rc=[rc1, rch, rc2],
               secs=dict(a=round(t1, 1), handoff=round(th, 1), b=round(t2, 1)),
               usage_after_a=u1, usage_end=u2,
               relayed=("relay:" in o1, "relay:" in o2), dir=d if keep or status != "pass" else None,
               tail=(o2 or o1)[-500:] if status != "pass" else "")
    if not keep and status == "pass":
        shutil.rmtree(d, ignore_errors=True)
    return row


# ── statistics ───────────────────────────────────────────────────────────────

def wilson(k, n, z=1.96):
    if n == 0:
        return (0.0, 1.0)
    p = k / n
    den = 1 + z * z / n
    mid = (p + z * z / (2 * n)) / den
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / den
    return (max(0.0, mid - half), min(1.0, mid + half))


def summarize(rows, cfg_name):
    rs = [r for r in rows if r["config"] == cfg_name]
    valid = [r for r in rs if r["status"] != "a_dead"]
    k = sum(r["status"] == "pass" for r in valid)
    lo, hi = wilson(k, len(valid))

    def total(key, who):
        v = [r[who][key] for r in valid if r.get(who) and r[who].get(key) is not None]
        return sum(v) if len(v) == len(valid) and v else None  # only when every run reported it

    return dict(config=cfg_name, runs=len(rs), a_dead=len(rs) - len(valid), valid=len(valid), passed=k,
                rate=(k / len(valid)) if valid else None, ci95=(round(lo, 3), round(hi, 3)),
                secs_b_mean=round(sum(r["secs"]["b"] + r["secs"]["handoff"] for r in valid) / len(valid), 1) if valid else None,
                tokens_in_end=total("tokens_in", "usage_end"), tokens_out_end=total("tokens_out", "usage_end"),
                cost_usd_end=total("cost_usd", "usage_end"))


def print_summary(rows, cfgs):
    print("\nconfig".ljust(24) + "pass".rjust(8) + "   95% CI".ljust(16) + "a_dead".rjust(7) + "  B secs".rjust(9) + "   tokens in/out".ljust(22) + "cost")
    sums = [summarize(rows, c["name"]) for c in cfgs]
    for s in sums:
        rate = "—" if s["rate"] is None else f"{s['passed']}/{s['valid']} {100*s['rate']:.0f}%"
        tok = "—" if s["tokens_in_end"] is None else f"{s['tokens_in_end']}/{s['tokens_out_end']}"
        cost = "—" if s["cost_usd_end"] is None else f"${s['cost_usd_end']:.2f}"
        print(f"{s['config']:<24}{rate:>8}   {str(s['ci95']):<13}{s['a_dead']:>7}{str(s['secs_b_mean']):>9}   {tok:<19}{cost}")
    return sums


def print_failures(rows):
    for r in rows:
        if r["status"] != "pass":
            print(f"\n{r['status'].upper()} {r['id']} [{r['config']}#{r['rep']}] {r['direction']} checks={r['checks']} rc={r['rc']}")
            if r["tail"]:
                print("  …" + r["tail"].strip().replace("\n", "\n  ")[-400:])
            print(f"  workspace kept: {r['dir']}")


# ── main ─────────────────────────────────────────────────────────────────────

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--suite", choices=["real", "toy", "all"], default="real")
    ap.add_argument("--cases", default="", help="comma list of id prefixes")
    ap.add_argument("--dry-run", action="store_true", help="validate cases (seed fails, oracle passes); run no agent")
    ap.add_argument("--slow", action="store_true", help="also run cargo-based checks (dry-run or real)")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--ablate", default="", help="comma list: " + ",".join(real_cases.ABLATIONS))
    ap.add_argument("--matrix", action="store_true", help="baseline plus each ablation on its own")
    ap.add_argument("--via", choices=["ledger", "handoff"], default="ledger",
                    help="ledger: turn 2 is `run --session` (same path as the app). handoff: also run `chorus handoff` (brief + Verify) first")
    ap.add_argument("--both", action="store_true", help="run every case in both directions")
    ap.add_argument("--repeat", type=int, default=1)
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--timeout", type=int, default=420)
    ap.add_argument("--external", action="append", default=[], metavar="NAME=PATH", help="repo for source=external:NAME cases")
    ap.add_argument("--keep", action="store_true", help="keep passing workspaces too")
    ap.add_argument("--out", default="")
    a = ap.parse_args()

    externals = dict(x.split("=", 1) for x in a.external)
    cs = [c for c in load_cases(a.suite) if not a.cases or any(c["id"].startswith(x) for x in a.cases.split(","))]
    if a.list:
        for c in cs:
            print(f"{c['id']:<28}{c['dir']:<15}{c['source']:<22}{'implied' if c.get('implied') else ''}")
        return 0
    ids = [c["id"] for c in cs]
    if len(set(ids)) != len(ids):
        print("duplicate case ids", file=sys.stderr)
        return 2

    if a.dry_run:
        bad = 0
        for c in cs:
            errs, notes = dry_case(c, externals, a.slow)
            tag = "OK  " if not errs else "FAIL"
            print(f"{tag} {c['id']:<28}{' · '.join(errs + notes)}")
            bad += bool(errs)
        both = sum(c["dir"] == "claude->codex" for c in cs), sum(c["dir"] == "codex->claude" for c in cs)
        print(f"\n{len(cs) - bad}/{len(cs)} cases valid · claude->codex {both[0]} · codex->claude {both[1]} · "
              f"implied-pending {sum(bool(c.get('implied')) for c in cs)}")
        print("dry-run: no agent was started and no metric was measured.")
        return 1 if bad else 0

    # real run
    for c in cs:
        errs = validate_case(c, externals)
        if errs:
            print(f"skip {c['id']}: {'; '.join(errs)}", file=sys.stderr)
    cs = [c for c in cs if not validate_case(c, externals)]
    names = [x for x in a.ablate.split(",") if x]
    cfgs = []
    if a.matrix:
        cfgs.append(dict(name="baseline", env={}, report=[]))
        for n in real_cases.ABLATIONS:
            env, rep = ablation_env([n])
            cfgs.append(dict(name=f"-{n}", env=env, report=rep))
    else:
        env, rep = ablation_env(names)
        cfgs.append(dict(name=("-" + "-".join(names)) if names else "baseline", env=env, report=rep))
    for cfg in cfgs:
        for r in cfg["report"]:
            if not r["read_by_core"]:
                print(f"warning: {r['env']} is not read by any crate yet, so ablating '{r['name']}' changes nothing "
                      f"yet (that part of config {cfg['name']} is a no-op)", file=sys.stderr)

    if not os.environ.get("CHORUS_EVAL_BIN"):
        subprocess.run(["cargo", "build", "-q", "-p", "chorus-cli", "--bin", "chorus"], cwd=ROOT, check=True)
    if any(c["source"] == "chorus" for c in cs):
        chorus_template()
    jobs = []
    for cfg in cfgs:
        for c in cs:
            for rep in range(a.repeat):
                ab = tuple(c["dir"].split("->"))
                jobs.append((c, rep, cfg, ab))
                if a.both:
                    jobs.append((c, rep, cfg, ab[::-1]))
    print(f"{len(jobs)} runs ≈ {len(jobs) * 2 * (1 if a.via == 'ledger' else 1.5):.0f} real agent turns "
          f"(configs: {', '.join(c['name'] for c in cfgs)}; jobs={a.jobs})", file=sys.stderr)
    t0 = time.time()
    with ThreadPoolExecutor(a.jobs) as ex:
        rows = list(ex.map(lambda j: run_one(j[0], j[1], j[2], j[3], a.via, a.timeout, externals, a.slow, a.keep), jobs))
    for r in rows:
        cs_ = "".join("·" if x is None else ("✓" if x else "✗") for x in r["checks"])
        print(f"{r['status'].upper():<7}{r['id']:<26}{r['config']:<12}{r['direction']:<14}{cs_:<8}{r['secs']['a']:>5}s {r['secs']['b']:>5}s")
    sums = print_summary(rows, cfgs)
    print_failures(rows)

    out = a.out or os.path.join(HERE, "runs", time.strftime("%Y%m%d-%H%M%S") + ".json")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    head = subprocess.run("git rev-parse --short HEAD", shell=True, cwd=ROOT, capture_output=True, text=True).stdout.strip()
    json.dump(dict(meta=dict(date=time.strftime("%Y-%m-%dT%H:%M:%S%z"), chorus_head=head, base_sha=real_cases.BASE_SHA,
                             via=a.via, repeat=a.repeat, wall_secs=round(time.time() - t0),
                             configs=[dict(name=c["name"], ablations=c["report"]) for c in cfgs]),
                   summary=sums, rows=rows), open(out, "w"), indent=1, ensure_ascii=False)
    print(f"\nwrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
