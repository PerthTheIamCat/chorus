#!/usr/bin/env bash
# End-to-end check against the agents logged in on this Mac — what CI and the
# mock agent cannot cover (CLAUDE.md §3: the Claude and Antigravity bridges have
# never been run through a whole relay). Sends a few real requests per agent.
#
#   scripts/smoke-real-agents.sh                  # claude codex antigravity
#   scripts/smoke-real-agents.sh claude codex
#
# For every agent: one turn that must write a file. Then, across the first two
# that passed: the second must name the first one's file (memory comes from
# the ledger, not the provider's session — CLAUDE.md 2.2), and a relay whose
# first bridge is broken must fail over to the second.
#
# Works in a throwaway git repo under $TMPDIR; nothing in this checkout is
# touched. Agent output goes to the log printed at the end — it never contains
# credentials (Chorus does not read them), but check before sharing it.
set -uo pipefail
cd "$(dirname "$0")/.."

AGENTS="${*:-claude codex antigravity}"
TURN_SECS="${CHORUS_SMOKE_TIMEOUT:-300}"

echo "── build ──"
cargo build -q -p chorus-cli --bin chorus -p chorus-acp || exit 1
CH="$PWD/target/debug/chorus"

ROOT="$(mktemp -d "${TMPDIR:-/tmp}/chorus-smoke.XXXXXX")"
W="$ROOT/w"
LOG="$ROOT/smoke.log"
mkdir -p "$W"
(cd "$W" && git init -q && echo "# smoke" > README.md)
: > "$LOG"

echo "── doctor ──"
"$CH" doctor 2>&1 | tee -a "$LOG" | sed -n '1,40p'

# macOS ships no `timeout`; run "$@" and kill it after $1 seconds.
with_timeout() {
    local secs=$1; shift
    "$@" &
    local pid=$!
    ( sleep "$secs"; kill -TERM "$pid" 2>/dev/null ) &
    local watcher=$!
    wait "$pid"
    local rc=$?
    kill "$watcher" 2>/dev/null
    wait "$watcher" 2>/dev/null
    return $rc
}

bridge_env() {
    case "$1" in
        claude*) echo CHORUS_CLAUDE_BRIDGE ;;
        antigravity*|agy*) echo CHORUS_ANTIGRAVITY_BRIDGE ;;
        *) echo CHORUS_CODEX_BRIDGE ;;
    esac
}

RESULTS=""
record() { RESULTS="$RESULTS$1|$2|$3"$'\n'; }
PASSED=""

echo "── 1. one turn per agent ──"
for a in $AGENTS; do
    out="$ROOT/out-$a.txt"
    echo "  $a …"
    {
        echo "===== $a: single turn ====="
        with_timeout "$TURN_SECS" "$CH" run "$W" \
            "Create a file named smoke-$a.txt in the current directory whose entire content is the single word $a. Then reply DONE." \
            --chain "$a" --yes --db "$ROOT/$a.db"
    } > "$out" 2>&1
    rc=$?
    cat "$out" >> "$LOG"
    got=""
    [ -f "$W/smoke-$a.txt" ] && got="$(tr -d '[:space:]' < "$W/smoke-$a.txt")"
    if [ "$got" = "$a" ]; then
        record "$a" "writes a file" PASS
        PASSED="$PASSED $a"
    elif [ $rc -ne 0 ]; then
        record "$a" "writes a file" "FAIL (exit $rc — see log)"
    else
        record "$a" "writes a file" "FAIL (smoke-$a.txt missing or wrong)"
    fi
done

set -- $PASSED
if [ $# -ge 2 ]; then
    A=$1; B=$2
    echo "── 2. $B reads what $A did, from the ledger ──"
    out="$ROOT/out-memory.txt"
    {
        echo "===== $B continues $A's session ====="
        with_timeout "$TURN_SECS" "$CH" run \
            "What is the name of the file the previous agent created? Reply with only the filename." \
            --session 1 --db "$ROOT/$A.db" --chain "$B" --yes
    } > "$out" 2>&1
    cat "$out" >> "$LOG"
    if grep -q "smoke-$A.txt" "$out"; then
        record "$A→$B" "shared memory" PASS
    else
        record "$A→$B" "shared memory" "FAIL ($B did not name smoke-$A.txt)"
    fi

    echo "── 3. relay: broken $A → $B ──"
    out="$ROOT/out-relay.txt"
    {
        echo "===== relay with $A's bridge broken ====="
        env "$(bridge_env "$A")=/nonexistent/chorus-smoke-bridge" \
            "$CH" run "$W" "Reply with the single word PONG." \
            --chain "$A,$B" --yes --db "$ROOT/relay.db"
    } > "$out" 2>&1
    cat "$out" >> "$LOG"
    if grep -q "relay: $A → $B" "$out" && grep -qi "pong" "$out"; then
        record "$A→$B" "failover" PASS
    else
        record "$A→$B" "failover" "FAIL (no relay to $B — see log)"
    fi
else
    record "-" "shared memory + failover" "SKIP (needs two agents that passed step 1)"
fi

echo
echo "── result ──"
printf '%s' "$RESULTS" | while IFS='|' read -r who what res; do
    [ -n "$who" ] && printf '  %-16s %-26s %s\n' "$who" "$what" "$res"
done
echo
echo "log: $LOG"
case "$RESULTS" in
    *FAIL*) exit 1 ;;
esac
