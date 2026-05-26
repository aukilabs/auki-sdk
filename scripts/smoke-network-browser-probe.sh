#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

package_dir="bindings/javascript/auki-network"
if [[ ! -f "$package_dir/browser-probe-smoke.mjs" ]]; then
  echo "missing generated browser-probe smoke script; run: just generate-javascript-bindings auki-network" >&2
  exit 1
fi

if [[ ! -d "$package_dir/node_modules" ]]; then
  npm install --prefix "$package_dir"
fi

log_file="$(mktemp "${TMPDIR:-/tmp}/auki-browser-probe.XXXXXX.log")"
listener_pid=""
cleanup() {
  if [[ -n "$listener_pid" ]] && kill -0 "$listener_pid" 2>/dev/null; then
    kill "$listener_pid" 2>/dev/null || true
    wait "$listener_pid" 2>/dev/null || true
  fi
  rm -f "$log_file"
}
trap cleanup EXIT

cargo run -p auki-network --features browser_probe --example browser_probe_listener >"$log_file" 2>&1 &
listener_pid="$!"

probe_addr=""
for _ in $(seq 1 120); do
  if ! kill -0 "$listener_pid" 2>/dev/null; then
    cat "$log_file" >&2
    exit 1
  fi
  probe_addrs="$(sed -n 's/^PARK_BROWSER_PROBE_ADDR=//p' "$log_file")"
  probe_addr="$(printf '%s\n' "$probe_addrs" | awk '/^\/ip4\/127\.0\.0\.1\// { addr = $0 } END { print addr }')"
  if [[ -z "$probe_addr" ]]; then
    probe_addr="$(printf '%s\n' "$probe_addrs" | tail -n 1)"
  fi
  if [[ -n "$probe_addr" ]]; then
    break
  fi
  sleep 0.1
done

if [[ -z "$probe_addr" ]]; then
  cat "$log_file" >&2
  echo "native browser-probe listener did not print PARK_BROWSER_PROBE_ADDR" >&2
  exit 1
fi

if ! node "$package_dir/browser-probe-smoke.mjs" "$probe_addr"; then
  cat "$log_file" >&2
  exit 1
fi
