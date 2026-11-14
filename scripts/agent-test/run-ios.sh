#!/usr/bin/env bash
# Closed-loop test on the iOS Simulator: build, install, seed a synthetic
# library, launch the app in agent test mode, wait for its self-test report,
# take a screenshot, and print a verdict. No device and no real photos needed.
#
#   scripts/agent-test/run-ios.sh
#
# Environment:
#   DEVICE      simulator name or UDID        (default: "iPhone 17 Pro")
#   COUNT       synthetic geotagged photos    (default: 20000)
#   SKIP_BUILD  1 = reuse the last built .app (default: build)
#   FRESH       1 = uninstall first           (default: keep the container)
#   SCENARIO    passed to the in-app probe    (default: "default";
#               "keep-viewer" leaves the viewer open for the screenshot)
#   TIMEOUT     seconds to wait for the report (default: 180)
#
# Output: scripts/agent-test/out/{report.json,screenshot.png,app.log}
# Exit code: 0 when the report says passed, 1 otherwise.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HERE="$ROOT/scripts/agent-test"
OUT="$HERE/out"
APP_DIR="$ROOT/packages/desktop"
BUNDLE_ID="app.footos"

DEVICE="${DEVICE:-iPhone 17 Pro}"
COUNT="${COUNT:-20000}"
SCENARIO="${SCENARIO:-default}"
TIMEOUT="${TIMEOUT:-180}"

mkdir -p "$OUT"
rm -f "$OUT/report.json" "$OUT/screenshot.png" "$OUT/app.log"

log() { printf '\033[1m[agent-test]\033[0m %s\n' "$*"; }

# --- simulator ---------------------------------------------------------------
UDID="$(xcrun simctl list devices available | grep -F "$DEVICE (" | head -1 | sed -E 's/.*\(([0-9A-F-]{36})\).*/\1/')"
if [[ -z "$UDID" ]]; then
    if [[ "$DEVICE" =~ ^[0-9A-F-]{36}$ ]]; then UDID="$DEVICE"; else
        echo "No available simulator named '$DEVICE'" >&2; exit 2
    fi
fi
if ! xcrun simctl list devices | grep -F "$UDID" | grep -q Booted; then
    log "booting $DEVICE"
    xcrun simctl boot "$UDID"
    xcrun simctl bootstatus "$UDID" -b >/dev/null
fi
log "simulator $DEVICE ($UDID)"

# --- build -------------------------------------------------------------------
if [[ "${SKIP_BUILD:-0}" != "1" ]]; then
    log "building (pnpm tauri ios build --debug --target aarch64-sim)"
    # Tauri cannot overwrite the previous bundle ("Directory not empty").
    rm -rf "$APP_DIR/src-tauri/gen/apple/build/arm64-sim"
    (cd "$APP_DIR" && pnpm tauri ios build --debug --target aarch64-sim) >"$OUT/build.log" 2>&1 || {
        tail -40 "$OUT/build.log" >&2; echo "build failed, see $OUT/build.log" >&2; exit 2;
    }
fi
APP="$(find "$APP_DIR/src-tauri/gen/apple/build/arm64-sim" -maxdepth 1 -type d -name '*.app' 2>/dev/null | head -1)"
if [[ -z "$APP" ]]; then
    echo "No simulator .app found. Run without SKIP_BUILD=1." >&2; exit 2
fi
log "app $APP"

# --- install -----------------------------------------------------------------
xcrun simctl terminate "$UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
if [[ "${FRESH:-0}" == "1" ]]; then
    xcrun simctl uninstall "$UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
fi
xcrun simctl install "$UDID" "$APP"

# No permission dialogs: they would cover the UI and block the probe.
# Photos is left alone: in agent test mode the app never touches the photo
# library, and `simctl privacy grant photos` only gives add-only access, which
# reads as "denied" and would upset manual permission tests on this simulator.
xcrun simctl privacy "$UDID" grant location "$BUNDLE_ID" || true
xcrun simctl spawn "$UDID" defaults write "$BUNDLE_ID" footos.networkAlertShown -bool YES

# --- seed --------------------------------------------------------------------
CONTAINER="$(xcrun simctl get_app_container "$UDID" "$BUNDLE_ID" data)"
EXISTING_DB="$(find "$CONTAINER" -name footos.db -not -path '*/tmp/*' 2>/dev/null | head -1)"
if [[ -n "$EXISTING_DB" ]]; then
    DATA_DIR="$(dirname "$EXISTING_DB")"
else
    DATA_DIR="$CONTAINER/Library/Application Support/$BUNDLE_ID"
fi
log "seeding $COUNT photos into $DATA_DIR"
python3 "$HERE/seed.py" "$DATA_DIR" --count "$COUNT"

# --- run ---------------------------------------------------------------------
log "launching in agent test mode (scenario: $SCENARIO)"
(xcrun simctl spawn "$UDID" log stream --style compact --predicate 'process CONTAINS "footos" OR processImagePath CONTAINS "footos"' >"$OUT/app.log" 2>&1) &
LOG_PID=$!
trap 'kill $LOG_PID >/dev/null 2>&1 || true' EXIT

SIMCTL_CHILD_FOOTOS_AGENT_TEST="$SCENARIO" xcrun simctl launch "$UDID" "$BUNDLE_ID" >/dev/null

REPORT=""
for ((i = 0; i < TIMEOUT; i++)); do
    REPORT="$(find "$CONTAINER" -name agent-report.json 2>/dev/null | head -1)"
    [[ -n "$REPORT" ]] && break
    sleep 1
done

sleep 1
xcrun simctl io "$UDID" screenshot "$OUT/screenshot.png" >/dev/null 2>&1 || true

if [[ -z "$REPORT" ]]; then
    log "no report after ${TIMEOUT}s (the app may have crashed or the UI is blocked)"
    log "screenshot: $OUT/screenshot.png   log: $OUT/app.log"
    exit 1
fi
cp "$REPORT" "$OUT/report.json"

python3 "$HERE/summarize.py" "$OUT/report.json"
