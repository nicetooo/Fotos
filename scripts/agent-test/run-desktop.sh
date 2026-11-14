#!/usr/bin/env bash
# Agent self-test for the desktop app, on seeded data in a throwaway data dir.
#
# The app's data dir comes from $HOME, so the debug binary is started with HOME
# pointing into scripts/agent-test/out/: the real library is never opened.
# (WebKit keeps localStorage under the real home; the probe does not rely on it.)
#
#   COUNT=20000 SKIP_BUILD=1 FRESH=1 SCENARIO=default TIMEOUT=120 ./run-desktop.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
APP="$ROOT/packages/desktop"
OUT="$HERE/out/desktop"
FAKE_HOME="$OUT/home"
DATA_DIR="$FAKE_HOME/Library/Application Support/app.footos"
REPORT="$DATA_DIR/agent-report.json"
COUNT="${COUNT:-20000}"
SCENARIO="${SCENARIO:-default}"
TIMEOUT="${TIMEOUT:-120}"
BIN="$APP/src-tauri/target/debug/footos"

log() { printf '[agent-test] %s\n' "$*"; }

if [ -z "${SKIP_BUILD:-}" ]; then
    log "building debug binary"
    (cd "$APP/src-tauri" && cargo build 2>&1 | tail -2)
fi

if [ -n "${FRESH:-}" ] || [ ! -f "$DATA_DIR/footos.db" ]; then
    rm -rf "$FAKE_HOME"
    mkdir -p "$DATA_DIR"
    log "seeding $COUNT photos"
    python3 "$HERE/seed.py" "$DATA_DIR" --count "$COUNT"
fi
rm -f "$REPORT"

# The debug binary loads the frontend from the Vite dev server.
VITE_PID=""
if ! curl -s -o /dev/null http://localhost:1420; then
    log "starting vite"
    (cd "$APP" && pnpm dev >"$OUT/vite.log" 2>&1) &
    VITE_PID=$!
    for _ in $(seq 1 40); do
        curl -s -o /dev/null http://localhost:1420 && break
        sleep 0.5
    done
fi

log "launching (scenario: $SCENARIO)"
HOME="$FAKE_HOME" FOOTOS_AGENT_TEST="$SCENARIO" "$BIN" >"$OUT/app.log" 2>&1 &
APP_PID=$!
cleanup() {
    kill "$APP_PID" >/dev/null 2>&1 || true
    if [ -n "$VITE_PID" ]; then
        pkill -P "$VITE_PID" >/dev/null 2>&1 || true
        kill "$VITE_PID" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

for _ in $(seq 1 "$TIMEOUT"); do
    [ -f "$REPORT" ] && break
    sleep 1
done
if [ ! -f "$REPORT" ]; then
    log "no report after ${TIMEOUT}s (see $OUT/app.log)"
    exit 2
fi
cp "$REPORT" "$OUT/agent-report.json"
python3 "$HERE/summarize.py" "$OUT/agent-report.json"
