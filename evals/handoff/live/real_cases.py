"""Live handoff cases built from work shaped like the maintainer's real tasks.

Where `cases.py` (16 toy cases, one-file Python) asks "does a decision survive a
handoff?", these ask it on work that looks like what the app is used for:
five tasks on a pinned snapshot of this repo (the same kind of chores that fill
its history: a classifier phrase, a script flag, a doc, an ADR) and eleven
small projects in other stacks (Node, Python, sqlite, bash) with seeded files.

Shape of a case (all keys required unless marked optional):

  id        unique, sortable
  source    "chorus"            -> `git archive` of BASE_SHA below
            "fixture:<name>"    -> files from FIXTURES[name] (empty = blank repo)
            "external:<name>"   -> a repo you pass with --external name=PATH
  files     extra seed files {path: text}, written on top of the source
  dir       "claude->codex" | "codex->claude"   (agent A -> agent B)
  goal      one line, only used by `--via handoff` for `chorus handoff --goal`
  p1        turn 1, to A. States a decision and asks for half the work.
  p2        turn 2, to B in the same ledger session. Never restates the decision.
  a_checks  must hold after turn 1; otherwise A never did its half and the case
            is reported "a_dead" and left out of the handoff rate
  checks    must all hold after turn 2. Deterministic (files, grep, running code);
            no model grades a model. A check is
              ("exists", path) · ("has", path, text) · ("lacks", path, text)
              ("run", shell) · ("slow", shell)   slow = needs cargo; skipped by
                                                 `--dry-run` unless --slow
  oracle1   ops that turn the seed into "A finished its half"
  oracle2   ops that turn that into "B finished"
            op = ("write", path, text) | ("sub", path, old, new) |
                 ("append", path, text) | ("fn", callable(workdir))
            Dry-run uses these to prove the checks are satisfiable and that a
            no-op fails them. They are never shown to an agent.
  implied   optional: True when the pending work is only implied by turn 1 or by
            a rule written down in the repo (the failures worth finding)

Chosen to be unfair in one way on purpose: p2 is always a plain continuation
that does not repeat the decision from p1.
"""
# The snapshot every source="chorus" case starts from. Fixed so that results
# stay comparable after main moves on.
BASE_SHA = "937d2fe"

CONT = "Continue where we left off and finish the work. Do not ask me questions."

FIXTURES = {
    "blank": {},
    "node-http": {
        "package.json": '{ "name": "svc", "version": "1.0.0", "private": true }\n',
        "server.js": (
            "const http = require('http');\n"
            "\n"
            "function createServer() {\n"
            "  return http.createServer((req, res) => {\n"
            "    res.writeHead(200, { 'content-type': 'text/plain' });\n"
            "    res.end('ok\\n');\n"
            "  });\n"
            "}\n"
            "\n"
            "module.exports = { createServer };\n"
            "if (require.main === module) createServer().listen(3000);\n"
        ),
    },
    "dotfiles": {
        "dotfiles/.zshrc": "export EDITOR=vim\n",
        "dotfiles/.gitconfig": "[user]\n  name = Perth\n",
    },
    "orders-csv": {
        "data/orders.csv": (
            "id,placed_at,amount_cents\n"
            "1,2026-03-04 18:30,1000\n"
            "2,2026-03-05 02:00,500\n"
            "3,2026-03-05 09:00,250\n"
        ),
    },
    "migrations": {
        "migrations/0001_init.sql": "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);\n",
        "migrations/0001_init.down.sql": "DROP TABLE users;\n",
        "migrations/0002_add_email.sql": "ALTER TABLE users ADD COLUMN email TEXT;\n",
    },
}


def _w(path, text):
    return ("write", path, text)


def _s(path, old, new):
    return ("sub", path, old, new)


def _wrap_relay(w):
    """Oracle for r04: skip step 3 of the smoke script when asked to."""
    p = f"{w}/scripts/smoke-real-agents.sh"
    t = open(p, encoding="utf8").read()
    head = '    echo "── 3. relay: broken $A → $B ──"\n'
    assert head in t
    t = t.replace(
        head,
        '    if [ -n "${CHORUS_SMOKE_SKIP_RELAY:-}" ]; then\n'
        '        record "${A}→${B}" "failover" "SKIP (CHORUS_SMOKE_SKIP_RELAY)"\n'
        "    else\n" + head,
    )
    tail = '        record "${A}→${B}" "failover" "FAIL (no relay to ${B} — see log)"\n    fi\n'
    assert tail in t
    t = t.replace(tail, tail + "    fi\n")
    open(p, "w", encoding="utf8").write(t)


def _header_note(w):
    p = f"{w}/scripts/smoke-real-agents.sh"
    t = open(p, encoding="utf8").read()
    t = t.replace(
        "#   scripts/smoke-real-agents.sh claude codex\n",
        "#   scripts/smoke-real-agents.sh claude codex\n"
        "#   CHORUS_SMOKE_SKIP_RELAY=1 scripts/smoke-real-agents.sh   # skip step 3 (failover)\n",
        1,
    )
    open(p, "w", encoding="utf8").write(t)


CASES = [
    # ── on a snapshot of this repo ────────────────────────────────────────────
    dict(id="r01-score-json", source="chorus", files={}, dir="claude->codex",
         goal="Add a --json flag to evals/handoff/score.py and document it",
         p1=("In evals/handoff/score.py add a --json flag. With --json print exactly one JSON object and "
             "nothing else: {\"passed\": int, \"total\": int, \"pass_rate\": float between 0 and 1 (a fraction, "
             "not a percent), \"per_key\": {criterion: points}}. Without the flag the Thai table output must "
             "stay byte-identical to today. Do the flag only; the docs come after."),
         p2=CONT + " Document the new flag where this harness is described.",
         a_checks=[("run", "python3 evals/handoff/score.py --json | python3 -c \"import sys,json;d=json.load(sys.stdin);"
                           "assert isinstance(d['passed'],int) and d['total']>=7 and 0<=d['pass_rate']<=1 "
                           "and {'understood_goal','output_usable'}<=set(d['per_key'])\"")],
         checks=[("run", "python3 evals/handoff/score.py --json | python3 -c \"import sys,json;d=json.load(sys.stdin);"
                         "assert 0<=d['pass_rate']<=1 and d['passed']<=d['total']\""),
                 ("run", "t=$(mktemp -d); git show HEAD:evals/handoff/score.py > evals/handoff/.orig_score.py && "
                         "python3 evals/handoff/.orig_score.py > $t/a; rm evals/handoff/.orig_score.py; "
                         "python3 evals/handoff/score.py > $t/b; cmp $t/a $t/b"),
                 ("run", "grep -q -- '--json' evals/handoff/README.md")],
         oracle1=[_s("evals/handoff/score.py", "import json, pathlib, sys", "import json, pathlib, sys\nJSON = \"--json\" in sys.argv"),
                  _s("evals/handoff/score.py", "print(f\"{'case':<28} {'คะแนน':>6}  ผล\")",
                     "if JSON:\n    print(json.dumps({\"passed\": passed, \"total\": len(rows), \"pass_rate\": passed / len(rows), \"per_key\": per_key}))\n    sys.exit(0)\n\nprint(f\"{'case':<28} {'คะแนน':>6}  ผล\")")],
         oracle2=[("append", "evals/handoff/README.md", "\n`python3 score.py --json` prints the summary as one JSON object.\n")]),

    dict(id="r02-classifier-verbatim", source="chorus", files={}, dir="codex->claude", implied=True,
         goal="Classify the new Claude weekly-cap limit message as quota exhausted",
         p1=("A new Claude limit message was seen in the wild, verbatim: "
             "\"You've reached your weekly cap · resets Monday 9am (Asia/Bangkok)\". Make "
             "HandoffReason::from_failure classify it as QuotaExhausted by adding the key "
             "\"reached your weekly cap\" to the EXHAUSTED list in crates/chorus-core/src/handoff.rs, with a "
             "short comment quoting the message. Do not touch the other lists."),
         p2=CONT,
         a_checks=[("has", "crates/chorus-core/src/handoff.rs", "reached your weekly cap")],
         checks=[("has", "crates/chorus-core/src/handoff.rs", "reached your weekly cap"),
                 ("run", "grep -qF \"You've reached your weekly cap · resets Monday 9am (Asia/Bangkok)\" crates/chorus-core/tests/handoff.rs"),
                 ("slow", "cargo test -q -p chorus-core --test handoff")],
         oracle1=[_s("crates/chorus-core/src/handoff.rs", '            "hit your limit",\n',
                     '            "hit your limit",\n            // "You\'ve reached your weekly cap · resets Monday 9am (Asia/Bangkok)"\n            "reached your weekly cap",\n')],
         oracle2=[("append", "crates/chorus-core/tests/handoff.rs",
                   '\n#[test]\nfn the_real_weekly_cap_message_is_classified_as_exhausted() {\n'
                   '    let observed = "Internal error: You\'ve reached your weekly cap · resets Monday 9am (Asia/Bangkok)";\n'
                   '    assert_eq!(HandoffReason::from_failure(observed), Some(HandoffReason::QuotaExhausted));\n}\n')]),

    dict(id="r03-adr-pair", source="chorus", files={}, dir="claude->codex", implied=True,
         goal="Write ADR 0001 (ledger is source of truth) and then 0002 plus an index",
         p1=("Create docs/adr/0001-ledger-is-source-of-truth.md, in Thai, as an ADR with exactly these four "
             "second-level headings in this order: ## Status, ## Context, ## Decision, ## Consequences. "
             "Status is Accepted. Base it on rule 2.2 in CLAUDE.md. That is the format every ADR here will use."),
         p2=CONT + " Next the one for rule 2.1, and an index page for the folder.",
         a_checks=[("run", "grep -c '^## ' docs/adr/0001-ledger-is-source-of-truth.md | grep -q '^4$'")],
         checks=[("run", "ls docs/adr/0002-*.md | grep -q ."),
                 ("run", "f=$(ls docs/adr/0002-*.md | head -1); grep '^## ' $f | tr -d '\\n' | grep -q '^## Status## Context## Decision## Consequences$'"),
                 ("run", "grep -q 'Accepted' $(ls docs/adr/0002-*.md | head -1)"),
                 ("run", "python3 -c \"import re;t=open('docs/adr/0002-' + [f for f in __import__('os').listdir('docs/adr') if f.startswith('0002-')][0][5:],encoding='utf8').read();assert len(re.findall('[\\u0e00-\\u0e7f]',t))>60\""),
                 ("run", "test -f docs/adr/README.md && grep -q '0001' docs/adr/README.md && grep -q '0002' docs/adr/README.md")],
         oracle1=[_w("docs/adr/0001-ledger-is-source-of-truth.md",
                     "# 0001 ledger เป็นแหล่งความจริงเดียว\n\n## Status\nAccepted\n\n## Context\nsession เป็นของแอป ไม่ใช่ของ provider ถ้าพึ่ง session ฝั่ง agent เมื่อ provider ติดลิมิตเราก็เสียบริบททั้งหมดไปด้วย\n\n## Decision\nทุกอย่างอ่านจาก ledger ของ Chorus provider เป็นเพียงผู้ถูกเรียกมาตอบหนึ่งเทิร์น\n\n## Consequences\nพังตัวไหนไป ledger ยังอยู่ครบ แต่ต้องประกอบ context ใหม่ทุกเทิร์น\n")],
         oracle2=[_w("docs/adr/0002-storage-is-not-context.md",
                     "# 0002 storage ไม่ใช่ context\n\n## Status\nAccepted\n\n## Context\nที่เก็บบนดิสก์กับสิ่งที่ส่งให้โมเดลต้องการสิ่งที่ต่างกัน ดิสก์ต้องการไบต์น้อยและ query เร็ว ส่วนโมเดลต้องการ token น้อย\n\n## Decision\nแยก format เป็นสองฝั่งเสมอ ฝั่งเข้าโมเดลประกอบใหม่ทุกครั้งและไม่เขียนกลับลงดิสก์\n\n## Consequences\nต้องมีตัวประกอบ context แยก แต่ไม่ต้องจ่ายทั้งไบต์และ token เกินจำเป็น\n"),
                  _w("docs/adr/README.md", "# ADR\n\n- [0001](0001-ledger-is-source-of-truth.md)\n- [0002](0002-storage-is-not-context.md)\n")]),

    dict(id="r04-smoke-skip-relay", source="chorus", files={}, dir="codex->claude",
         goal="Let scripts/smoke-real-agents.sh skip the relay step with an env var",
         p1=("In scripts/smoke-real-agents.sh add an env var CHORUS_SMOKE_SKIP_RELAY. When it is set to anything "
             "non-empty, step 3 (the relay/failover check) must not run and its result row must read exactly "
             "SKIP (CHORUS_SMOKE_SKIP_RELAY) instead of PASS/FAIL. The script must still pass bash -n."),
         p2=CONT + " The usage lines at the top of the script should mention it.",
         a_checks=[("run", "bash -n scripts/smoke-real-agents.sh && grep -q 'SKIP (CHORUS_SMOKE_SKIP_RELAY)' scripts/smoke-real-agents.sh")],
         checks=[("run", "bash -n scripts/smoke-real-agents.sh"),
                 ("run", "grep -q 'SKIP (CHORUS_SMOKE_SKIP_RELAY)' scripts/smoke-real-agents.sh"),
                 ("run", "sed -n 1,20p scripts/smoke-real-agents.sh | grep -q CHORUS_SMOKE_SKIP_RELAY")],
         oracle1=[("fn", _wrap_relay)], oracle2=[("fn", _header_note)]),

    dict(id="r05-refused-config-key", source="chorus", files={}, dir="claude->codex", implied=True,
         goal="Treat the unknown-model refusal as a reason to move on",
         p1=("Codex answered a turn with the short text \"Unknown model 'gpt-9-nova' for this account.\" "
             "instead of failing. Teach is_refused_configuration in crates/chorus-core/src/handoff.rs to treat "
             "that wording as a refused configuration by adding the key \"unknown model\". Keep the rule that long "
             "answers never count."),
         p2=CONT,
         a_checks=[("has", "crates/chorus-core/src/handoff.rs", "\"unknown model\"")],
         checks=[("has", "crates/chorus-core/src/handoff.rs", "\"unknown model\""),
                 ("run", "grep -qF \"Unknown model 'gpt-9-nova' for this account.\" crates/chorus-core/tests/handoff.rs"),
                 ("slow", "cargo test -q -p chorus-core --test handoff")],
         oracle1=[_s("crates/chorus-core/src/handoff.rs", '        "model_not_found",\n',
                     '        "model_not_found",\n        "unknown model",\n')],
         oracle2=[("append", "crates/chorus-core/tests/handoff.rs",
                   '\n#[test]\nfn an_unknown_model_answer_is_a_refused_configuration() {\n'
                   '    use chorus_core::handoff::is_refused_configuration;\n'
                   '    assert!(is_refused_configuration("Unknown model \'gpt-9-nova\' for this account."));\n}\n')]),

    # ── other stacks, seeded files ────────────────────────────────────────────
    dict(id="f01-node-ratelimit", source="fixture:node-http", files={}, dir="claude->codex",
         goal="Add a fixed-window rate limiter and wire it into server.js",
         p1=("Add ratelimit.js (CommonJS, no dependencies) exporting makeLimiter({limit, windowMs, now = Date.now}) "
             "that returns allow(key) -> boolean, a fixed window per key. The clock is injected through `now` so "
             "tests never sleep. Only the limiter now; do not wire it into server.js yet."),
         p2=CONT + " Wire it into server.js so a client over the limit gets HTTP 429, with the limit read from the "
                   "RATE_LIMIT environment variable (default 60), and add tests.",
         a_checks=[("run", "node -e \"const {makeLimiter}=require('./ratelimit');let t=0;const a=makeLimiter({limit:2,windowMs:1000,now:()=>t});"
                           "if(!(a('k')&&a('k')&&!a('k')))process.exit(1);t=1001;if(!a('k'))process.exit(1)\"")],
         checks=[("has", "server.js", "RATE_LIMIT"),
                 ("run", "RATE_LIMIT=2 node -e \"const {createServer}=require('./server');const http=require('http');const s=createServer().listen(0,async()=>{"
                         "const p=s.address().port;const get=()=>new Promise(r=>http.get({port:p,path:'/'},x=>{x.resume();r(x.statusCode)}));"
                         "const c=[await get(),await get(),await get()];s.close();process.exit(c.join()==='200,200,429'?0:1)})\""),
                 ("run", "ls *.test.js test/*.js tests/*.js 2>/dev/null | grep -q ."),
                 ("run", "! grep -l setTimeout *.test.js test/*.js tests/*.js 2>/dev/null | grep -q ."),
                 ("run", "node --test >/dev/null 2>&1")],
         oracle1=[_w("ratelimit.js",
                     "function makeLimiter({ limit, windowMs, now = Date.now }) {\n  const m = new Map();\n  return function allow(key) {\n    const t = now();\n    let e = m.get(key);\n    if (!e || t - e.start >= windowMs) { e = { start: t, n: 0 }; m.set(key, e); }\n    e.n += 1;\n    return e.n <= limit;\n  };\n}\nmodule.exports = { makeLimiter };\n")],
         oracle2=[_w("server.js",
                     "const http = require('http');\nconst { makeLimiter } = require('./ratelimit');\n\nfunction createServer() {\n  const allow = makeLimiter({ limit: Number(process.env.RATE_LIMIT || 60), windowMs: 60000 });\n  return http.createServer((req, res) => {\n    if (!allow(req.socket.remoteAddress)) { res.writeHead(429); return res.end('slow down\\n'); }\n    res.writeHead(200, { 'content-type': 'text/plain' });\n    res.end('ok\\n');\n  });\n}\n\nmodule.exports = { createServer };\nif (require.main === module) createServer().listen(3000);\n"),
                  _w("ratelimit.test.js",
                     "const test = require('node:test');\nconst assert = require('node:assert');\nconst { makeLimiter } = require('./ratelimit');\ntest('window resets', () => {\n  let t = 0;\n  const a = makeLimiter({ limit: 1, windowMs: 10, now: () => t });\n  assert.ok(a('k')); assert.ok(!a('k')); t = 11; assert.ok(a('k'));\n});\n")]),

    dict(id="f02-expenses-satang", source="fixture:blank", files={}, dir="codex->claude",
         goal="Expense tracker CLI: add command, then a total command",
         p1=("Create expenses.py (standard library only): `python3 expenses.py add <amount> <note>` stores a row in "
             "expenses.db (sqlite3). Decision: amounts are stored as integer satang (baht x 100) in a column named "
             "amount_satang, never as a float, so `add 12.50 coffee` stores 1250. The `add` command only for now."),
         p2=CONT + " Now the total command: it prints the sum in baht with exactly two decimals.",
         a_checks=[("run", "rm -f expenses.db; python3 expenses.py add 12.50 coffee && python3 -c \"import sqlite3;c=sqlite3.connect('expenses.db');"
                           "r=c.execute('select amount_satang from expenses').fetchall();assert r==[(1250,)],r\"")],
         checks=[("run", "rm -f expenses.db; python3 expenses.py add 12.50 coffee && python3 expenses.py add 0.10 gum && python3 expenses.py total | grep -qx '12.60'"),
                 ("run", "python3 -c \"import sqlite3;c=sqlite3.connect('expenses.db');assert all(isinstance(x[0],int) for x in c.execute('select amount_satang from expenses'))\"")],
         oracle1=[_w("expenses.py",
                     "import sqlite3, sys\nfrom decimal import Decimal\n\ndb = sqlite3.connect('expenses.db')\ndb.execute('create table if not exists expenses (id integer primary key, amount_satang integer not null, note text)')\n\nif sys.argv[1] == 'add':\n    s = int(Decimal(sys.argv[2]) * 100)\n    db.execute('insert into expenses (amount_satang, note) values (?, ?)', (s, sys.argv[3]))\n    db.commit()\n")],
         oracle2=[("append", "expenses.py",
                   "elif sys.argv[1] == 'total':\n    t = db.execute('select coalesce(sum(amount_satang),0) from expenses').fetchone()[0]\n    print(f'{t // 100}.{t % 100:02d}')\n")]),

    dict(id="f03-dotfiles-installer", source="fixture:dotfiles", files={}, dir="claude->codex",
         goal="Idempotent dotfiles installer, then an --uninstall flag",
         p1=("Write install.sh (bash) that symlinks every file in ./dotfiles into $TARGET_HOME (default $HOME; the "
             "override exists for testing). Rule: never overwrite an existing real file, move it to <name>.bak first; "
             "a symlink that already points at ours is skipped, so running twice changes nothing."),
         p2=CONT + " Add an --uninstall flag.",
         a_checks=[("run", "h=$(mktemp -d); echo mine > $h/.zshrc; TARGET_HOME=$h bash install.sh && [ -L $h/.zshrc ] && [ \"$(cat $h/.zshrc.bak)\" = mine ] "
                           "&& TARGET_HOME=$h bash install.sh && [ ! -e $h/.zshrc.bak.bak ] && [ -L $h/.gitconfig ]")],
         checks=[("run", "h=$(mktemp -d); echo mine > $h/.zshrc; TARGET_HOME=$h bash install.sh && TARGET_HOME=$h bash install.sh --uninstall && "
                         "[ ! -L $h/.gitconfig ] && [ ! -e $h/.gitconfig ] && [ ! -L $h/.zshrc ] && [ \"$(cat $h/.zshrc)\" = mine ] && [ ! -e $h/.zshrc.bak ]"),
                 ("run", "bash -n install.sh")],
         oracle1=[_w("install.sh",
                     "#!/usr/bin/env bash\nset -euo pipefail\nT=\"${TARGET_HOME:-$HOME}\"\nD=\"$(cd \"$(dirname \"$0\")\" && pwd)/dotfiles\"\nfor f in \"$D\"/.[!.]*; do\n  n=$(basename \"$f\")\n  if [ -L \"$T/$n\" ] && [ \"$(readlink \"$T/$n\")\" = \"$f\" ]; then continue; fi\n  [ -e \"$T/$n\" ] && mv \"$T/$n\" \"$T/$n.bak\"\n  ln -s \"$f\" \"$T/$n\"\ndone\n")],
         oracle2=[_w("install.sh",
                     "#!/usr/bin/env bash\nset -euo pipefail\nT=\"${TARGET_HOME:-$HOME}\"\nD=\"$(cd \"$(dirname \"$0\")\" && pwd)/dotfiles\"\nif [ \"${1:-}\" = --uninstall ]; then\n  for f in \"$D\"/.[!.]*; do\n    n=$(basename \"$f\")\n    if [ -L \"$T/$n\" ] && [ \"$(readlink \"$T/$n\")\" = \"$f\" ]; then rm \"$T/$n\"; [ -e \"$T/$n.bak\" ] && mv \"$T/$n.bak\" \"$T/$n\"; fi\n  done\n  exit 0\nfi\nfor f in \"$D\"/.[!.]*; do\n  n=$(basename \"$f\")\n  if [ -L \"$T/$n\" ] && [ \"$(readlink \"$T/$n\")\" = \"$f\" ]; then continue; fi\n  [ -e \"$T/$n\" ] && mv \"$T/$n\" \"$T/$n.bak\"\n  ln -s \"$f\" \"$T/$n\"\ndone\n")]),

    dict(id="f04-orders-utc", source="fixture:orders-csv", files={}, dir="codex->claude",
         goal="Orders loader with UTC timestamps, then daily totals",
         p1=("In pipeline.py write load_orders(path) -> list of dicts from data/orders.csv. The placed_at column is "
             "Bangkok local time (UTC+7) like '2026-03-04 18:30'. Decision: every timestamp we expose is ISO 8601 in "
             "UTC ending in Z, e.g. '2026-03-04T11:30:00Z', and amount_cents becomes an int. Only the loader now."),
         p2=CONT + " Add daily_totals(orders) returning {date: total_cents}.",
         a_checks=[("run", "python3 -c \"import pipeline as p;o=p.load_orders('data/orders.csv');assert o[0]['placed_at']=='2026-03-04T11:30:00Z' and o[0]['amount_cents']==1000\"")],
         checks=[("run", "python3 -c \"import pipeline as p;t=p.daily_totals(p.load_orders('data/orders.csv'));assert t=={'2026-03-04':1500,'2026-03-05':250},t\"")],
         oracle1=[_w("pipeline.py",
                     "import csv\nfrom datetime import datetime, timedelta\n\ndef load_orders(path):\n    out = []\n    for r in csv.DictReader(open(path)):\n        t = datetime.strptime(r['placed_at'], '%Y-%m-%d %H:%M') - timedelta(hours=7)\n        out.append({'id': int(r['id']), 'placed_at': t.strftime('%Y-%m-%dT%H:%M:%SZ'), 'amount_cents': int(r['amount_cents'])})\n    return out\n")],
         oracle2=[("append", "pipeline.py",
                   "\ndef daily_totals(orders):\n    d = {}\n    for o in orders:\n        k = o['placed_at'][:10]\n        d[k] = d.get(k, 0) + o['amount_cents']\n    return d\n")]),

    dict(id="f05-migrations-down", source="fixture:migrations", files={}, dir="claude->codex", implied=True,
         goal="Migration runner: up now, down and the missing down file next",
         p1=("Write migrate.py with up(db_path) that applies every migrations/NNNN_name.sql in numeric order and "
             "records each applied version in a table schema_migrations. Down migrations are separate files named "
             "NNNN_name.down.sql and up() must never run those. Calling up() twice must change nothing."),
         p2=CONT + " Add down(db_path) that reverts the latest applied migration.",
         a_checks=[("run", "rm -f t.db; python3 -c \"import migrate,sqlite3;migrate.up('t.db');migrate.up('t.db');c=sqlite3.connect('t.db');"
                           "assert c.execute('select count(*) from schema_migrations').fetchone()[0]==2;c.execute('select email from users')\"")],
         checks=[("run", "rm -f t.db; python3 -c \"import migrate,sqlite3;migrate.up('t.db');migrate.down('t.db');c=sqlite3.connect('t.db');"
                         "assert c.execute('select count(*) from schema_migrations').fetchone()[0]==1;"
                         "assert 'email' not in [r[1] for r in c.execute('pragma table_info(users)')]\""),
                 ("exists", "migrations/0002_add_email.down.sql")],
         oracle1=[_w("migrate.py",
                     "import glob, os, re, sqlite3\n\ndef _db(p):\n    c = sqlite3.connect(p)\n    c.execute('create table if not exists schema_migrations (version text primary key)')\n    return c\n\ndef _files():\n    return sorted(f for f in glob.glob('migrations/*.sql') if not f.endswith('.down.sql'))\n\ndef up(path):\n    c = _db(path)\n    done = {r[0] for r in c.execute('select version from schema_migrations')}\n    for f in _files():\n        v = os.path.basename(f)[:4]\n        if v not in done:\n            c.executescript(open(f).read())\n            c.execute('insert into schema_migrations values (?)', (v,))\n    c.commit()\n")],
         oracle2=[("append", "migrate.py",
                   "\ndef down(path):\n    c = _db(path)\n    r = c.execute('select max(version) from schema_migrations').fetchone()[0]\n    if r is None:\n        return\n    f = glob.glob(f'migrations/{r}_*.down.sql')[0]\n    c.executescript(open(f).read())\n    c.execute('delete from schema_migrations where version=?', (r,))\n    c.commit()\n"),
                  _w("migrations/0002_add_email.down.sql", "ALTER TABLE users DROP COLUMN email;\n")]),

    dict(id="f06-json-log-keys", source="fixture:blank", files={}, dir="codex->claude",
         goal="Structured logger with fixed key names, then use it in a worker",
         p1=("Create jlog.py with log(level, msg, **fields) that writes one JSON object per line to stderr. The key "
             "names are fixed by our log shipper: ts (epoch milliseconds, int), lvl (upper-case: INFO, WARN or "
             "ERROR), msg, plus the extra fields flattened into the same object. Just the logger for now."),
         p2=CONT + " Create worker.py with run_job(job_id) and log when it starts and when it ends, with the job id.",
         a_checks=[("run", "python3 -c \"import jlog;jlog.log('info','hi',job_id=7)\" 2>&1 | python3 -c \"import sys,json;d=json.loads(sys.stdin.read());"
                           "assert d['lvl']=='INFO' and d['msg']=='hi' and isinstance(d['ts'],int) and d['job_id']==7\"")],
         checks=[("run", "python3 -c \"import worker;worker.run_job(42)\" 2>&1 >/dev/null | python3 -c \"import sys,json;ls=[json.loads(l) for l in sys.stdin if l.strip()];"
                         "assert len(ls)>=2 and all(l['job_id']==42 and l['lvl'] in('INFO','WARN','ERROR') and isinstance(l['ts'],int) for l in ls)\"")],
         oracle1=[_w("jlog.py",
                     "import json, sys, time\n\ndef log(level, msg, **fields):\n    d = {'ts': int(time.time() * 1000), 'lvl': level.upper(), 'msg': msg}\n    d.update(fields)\n    print(json.dumps(d), file=sys.stderr)\n")],
         oracle2=[_w("worker.py",
                     "from jlog import log\n\ndef run_job(job_id):\n    log('info', 'job start', job_id=job_id)\n    log('info', 'job end', job_id=job_id)\n")]),

    dict(id="f07-config-precedence", source="fixture:blank", files={}, dir="claude->codex",
         goal="Settings loader with env > file > defaults, then a typed timeout",
         p1=("Create config.py with get(name) returning a setting. Precedence, highest first: environment variable "
             "APP_<NAME in upper case>, then config.json in the current directory, then a built-in DEFAULTS dict "
             "({port: 8080, debug: false}). Env values are strings; that is handled later."),
         p2=CONT + " Add a new setting timeout_s with default 30, and make get() convert env values to the type "
                   "of the default (int, bool).",
         a_checks=[("run", "echo '{\"port\": 7000}' > config.json; python3 -c \"import config;assert config.get('port')==7000\" && "
                           "APP_PORT=9 python3 -c \"import config;assert str(config.get('port'))=='9'\"; r=$?; rm -f config.json; exit $r")],
         checks=[("run", "rm -f config.json; python3 -c \"import config;assert config.get('timeout_s')==30 and config.get('port')==8080\""),
                 ("run", "echo '{\"port\": 7000}' > config.json; APP_PORT=9 python3 -c \"import config;assert config.get('port')==9 and isinstance(config.get('port'),int)\"; r=$?; rm -f config.json; exit $r"),
                 ("run", "echo '{\"port\": 7000}' > config.json; python3 -c \"import config;assert config.get('port')==7000\"; r=$?; rm -f config.json; exit $r"),
                 ("run", "APP_DEBUG=true python3 -c \"import config;assert config.get('debug') is True\"")],
         oracle1=[_w("config.py",
                     "import json, os\n\nDEFAULTS = {'port': 8080, 'debug': False}\n\ndef get(name):\n    e = os.environ.get('APP_' + name.upper())\n    if e is not None:\n        return e\n    try:\n        f = json.load(open('config.json'))\n    except FileNotFoundError:\n        f = {}\n    return f.get(name, DEFAULTS[name])\n")],
         oracle2=[_w("config.py",
                     "import json, os\n\nDEFAULTS = {'port': 8080, 'debug': False, 'timeout_s': 30}\n\ndef _cast(v, like):\n    if isinstance(like, bool):\n        return v.lower() in ('1', 'true', 'yes')\n    if isinstance(like, int):\n        return int(v)\n    return v\n\ndef get(name):\n    e = os.environ.get('APP_' + name.upper())\n    if e is not None:\n        return _cast(e, DEFAULTS[name])\n    try:\n        f = json.load(open('config.json'))\n    except FileNotFoundError:\n        f = {}\n    return f.get(name, DEFAULTS[name])\n")]),

    dict(id="f08-textstats-plan", source="fixture:blank", files={}, dir="codex->claude", implied=True,
         goal="textstats.py: word_count now, the rest of the plan after",
         p1=("We are building textstats.py with three functions: word_count(s), char_count(s) (letters only, no "
             "spaces or punctuation) and top_words(s, n) (lower-cased, ties broken alphabetically). "
             "Do word_count now; I want the other two after."),
         p2=CONT,
         a_checks=[("run", "python3 -c \"import textstats as t;assert t.word_count('a b  c')==3\"")],
         checks=[("run", "python3 -c \"import textstats as t;assert t.char_count('Hi, you!')==5\""),
                 ("run", "python3 -c \"import textstats as t;assert t.top_words('b a B c a',2)==['a','b']\"")],
         oracle1=[_w("textstats.py", "def word_count(s):\n    return len(s.split())\n")],
         oracle2=[("append", "textstats.py",
                   "\ndef char_count(s):\n    return sum(c.isalpha() for c in s)\n\ndef top_words(s, n):\n    from collections import Counter\n    c = Counter(s.lower().split())\n    return [w for w, _ in sorted(c.items(), key=lambda kv: (-kv[1], kv[0]))[:n]]\n")]),

    dict(id="f09-money-half-even", source="fixture:blank", files={}, dir="claude->codex",
         goal="Money helpers in integer cents, rounding half-even for percentages",
         p1=("Create money.js (CommonJS) exporting add(a, b) and format(cents). All amounts are integer cents. "
             "format(123450) returns '฿1,234.50'. Standing rule for later functions: any rounding in this file is "
             "half-even (banker's rounding), never half-up. add and format only for now."),
         p2=CONT + " Add percent(cents, pct) that returns the integer cents for pct percent of an amount.",
         a_checks=[("run", "node -e \"const m=require('./money');if(m.add(1,2)!==3||m.format(123450)!=='฿1,234.50')process.exit(1)\"")],
         checks=[("run", "node -e \"const m=require('./money');if(m.percent(5,50)!==2||m.percent(1,50)!==0||m.percent(3,50)!==2||m.percent(1000,10)!==100)process.exit(1)\"")],
         oracle1=[_w("money.js",
                     "const add = (a, b) => a + b;\nconst format = (c) => '฿' + (Math.floor(c / 100)).toLocaleString('en-US') + '.' + String(c % 100).padStart(2, '0');\nmodule.exports = { add, format };\n")],
         oracle2=[_w("money.js",
                     "const add = (a, b) => a + b;\nconst format = (c) => '฿' + (Math.floor(c / 100)).toLocaleString('en-US') + '.' + String(c % 100).padStart(2, '0');\nfunction percent(c, p) {\n  const n = c * p; const q = Math.floor(n / 100); const r = n % 100;\n  if (r > 50 || (r === 50 && q % 2 === 1)) return q + 1;\n  return q;\n}\nmodule.exports = { add, format, percent };\n")]),

    dict(id="f10-cli-exit-codes", source="fixture:blank", files={}, dir="codex->claude",
         goal="mytool.py with fixed exit codes: cat now, rm next",
         p1=("Create mytool.py (argparse, standard library) with subcommand `cat <path>`. Exit codes are fixed by "
             "our wrapper script: 0 ok, 2 usage error, 3 file not found, 4 permission denied. Only `cat` now."),
         p2=CONT + " Add the `rm <path>` subcommand.",
         a_checks=[("run", "echo hi > x.txt; python3 mytool.py cat x.txt | grep -q hi; python3 mytool.py cat nope.txt; [ $? -eq 3 ]")],
         checks=[("run", "python3 mytool.py rm nope.txt; [ $? -eq 3 ]"),
                 ("run", "echo y > y.txt; python3 mytool.py rm y.txt && [ ! -e y.txt ]"),
                 ("run", "python3 mytool.py; [ $? -eq 2 ]")],
         oracle1=[_w("mytool.py",
                     "import argparse, sys\n\np = argparse.ArgumentParser()\nsub = p.add_subparsers(dest='cmd', required=True)\nsub.add_parser('cat').add_argument('path')\nsub.add_parser('rm').add_argument('path')\na = p.parse_args()\n\ndef run(fn):\n    try:\n        fn()\n    except FileNotFoundError:\n        sys.exit(3)\n    except PermissionError:\n        sys.exit(4)\n\nif a.cmd == 'cat':\n    run(lambda: sys.stdout.write(open(a.path).read()))\n")],
         oracle2=[("append", "mytool.py", "if a.cmd == 'rm':\n    import os\n    run(lambda: os.remove(a.path))\n")]),

    dict(id="f11-retry-attempts", source="fixture:blank", files={}, dir="claude->codex",
         goal="retry decorator with total-attempt semantics, then use it in a client",
         p1=("Create retry.py with a decorator retry(times, exceptions=(Exception,)). `times` is the TOTAL number of "
             "attempts including the first one (not the number of retries). Only the listed exception types are "
             "retried; anything else propagates immediately. After the last attempt the last exception is re-raised."),
         p2=CONT + " Create client.py with fetch(url) that uses it for flaky network errors, three attempts.",
         a_checks=[("run", "python3 -c \"import retry\nn=[0]\n@retry.retry(3)\ndef f():\n    n[0]+=1; raise ValueError\ntry: f()\nexcept ValueError: pass\nassert n[0]==3,n\"")],
         checks=[("run", "python3 -c \"import client,retry\nassert hasattr(client,'fetch')\""),
                 ("run", "grep -q 'retry' client.py && grep -qE 'ConnectionError|OSError|URLError|TimeoutError' client.py"),
                 ("run", "python3 -c \"import client,retry\nn=[0]\nimport client as c\nsrc=open('client.py').read()\nassert 'times=3' in src.replace(' ','') or 'retry(3' in src.replace(' ','')\"")],
         oracle1=[_w("retry.py",
                     "import functools\n\ndef retry(times, exceptions=(Exception,)):\n    def deco(fn):\n        @functools.wraps(fn)\n        def w(*a, **k):\n            for i in range(times):\n                try:\n                    return fn(*a, **k)\n                except exceptions:\n                    if i == times - 1:\n                        raise\n        return w\n    return deco\n")],
         oracle2=[_w("client.py",
                     "import urllib.request\nfrom retry import retry\n\n@retry(3, exceptions=(ConnectionError, TimeoutError, OSError))\ndef fetch(url):\n    return urllib.request.urlopen(url, timeout=5).read()\n")]),
]

# Names the ablation switches hold fixed so a result can be traced to one part.
# `env` is read by the part named in `reads`; the runner greps the source for it
# and says so when nothing reads it yet (an ablation that changes nothing is not
# an ablation).
ABLATIONS = {
    "r1": dict(env="CHORUS_R1_TOOL_REFS", value="off",
               what="old tool results collapse to one line with a blob reference (R1)"),
    "r2": dict(env="CHORUS_R2_MIN_RELEVANCE", value="off",
               what="memory facts below the relevance floor are dropped (R2)"),
    "evidence": dict(env="CHORUS_BRIEF_EVIDENCE", value="off",
                     what="handoff brief carries commands+exit codes and an unverified list (R5)"),
    "repomap": dict(env="CHORUS_REPO_MAP", value="off",
                    what="repo map at the top of the code tier (exists today)"),
}
