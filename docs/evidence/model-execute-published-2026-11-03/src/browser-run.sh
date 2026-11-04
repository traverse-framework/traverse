#!/usr/bin/env bash
# Runs the demo page in headless Chrome. Set CHROME to the binary and CHROME_FLAGS for extra flags.
set -euo pipefail
cd "$(dirname "$0")/.."
rm -f proof/browser.json proof/browser-failed.txt
node src/server.mjs > proof/server.log 2>&1 &
server=$!
chrome=""
trap 'kill $server ${chrome:+$chrome} 2>/dev/null || true' EXIT
until curl -fs -o /dev/null http://127.0.0.1:8791/; do sleep 1; done
# shellcheck disable=SC2086
"${CHROME:-google-chrome}" --headless=new --disable-gpu --no-first-run --remote-debugging-port=0 \
  --user-data-dir="$(mktemp -d)" ${CHROME_FLAGS:-} http://127.0.0.1:8791/ > proof/chrome.log 2>&1 &
chrome=$!
for _ in $(seq 1 180); do
  if [ -s proof/browser-failed.txt ]; then
    cat proof/browser-failed.txt >&2
    exit 1
  fi
  if [ -s proof/browser.json ]; then
    sleep 1
    echo "browser run finished"
    exit 0
  fi
  sleep 1
done
echo "browser run timed out" >&2
tail -20 proof/chrome.log >&2
exit 1
