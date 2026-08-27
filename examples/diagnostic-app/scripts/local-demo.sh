#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
SDK_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../.." && pwd)
DOMAIN_ID=${AUKI_DIAGNOSTIC_DOMAIN_ID:-11111111-2222-4333-8444-555555555555}
PORT_A=${AUKI_DIAGNOSTIC_PORT_A:-39441}
PORT_B=${AUKI_DIAGNOSTIC_PORT_B:-39442}

if [[ -n "${AUKI_DIAGNOSTIC_BIN:-}" ]]; then
    APP=$AUKI_DIAGNOSTIC_BIN
else
    cargo build --locked --manifest-path "$SDK_ROOT/Cargo.toml" -p auki-diagnostic-app
    APP="$SDK_ROOT/target/debug/auki-diagnostic-app"
fi

if [[ -n "${AUKI_DIAGNOSTIC_WORKDIR:-}" ]]; then
    WORKDIR=$AUKI_DIAGNOSTIC_WORKDIR
    mkdir -p -- "$WORKDIR"
    REMOVE_WORKDIR=0
else
    WORKDIR=$(mktemp -d "${TMPDIR:-/tmp}/auki-domain-demo.XXXXXX")
    REMOVE_WORKDIR=1
fi

PID_A=
PID_B=
cleanup() {
    if [[ -n "$PID_A" ]]; then kill "$PID_A" 2>/dev/null || true; fi
    if [[ -n "$PID_B" ]]; then kill "$PID_B" 2>/dev/null || true; fi
    if [[ "$REMOVE_WORKDIR" == 1 ]]; then rm -r -- "$WORKDIR"; fi
}
trap cleanup EXIT INT TERM

MATERIAL="$WORKDIR/material"
"$APP" demo-material --output "$MATERIAL" --domain "$DOMAIN_ID"
PEER_A=$(tr -d '\r\n' < "$MATERIAL/peer-a.peer-id")
PEER_B=$(tr -d '\r\n' < "$MATERIAL/peer-b.peer-id")

"$APP" run \
    --domain "$DOMAIN_ID" \
    --identity "$MATERIAL/peer-a.identity" \
    --dds-public-key "$MATERIAL/dds-public.pem" \
    --credential "$MATERIAL/peer-a.jwt" \
    --listen "/ip4/127.0.0.1/tcp/$PORT_A" \
    --route "$PEER_B=/ip4/127.0.0.1/tcp/$PORT_B" \
    --fetch-peer "$PEER_B" \
    --resource peer-a-camera \
    --storage "$WORKDIR/storage-a" \
    > "$WORKDIR/peer-a.log" 2>&1 &
PID_A=$!

"$APP" run \
    --domain "$DOMAIN_ID" \
    --identity "$MATERIAL/peer-b.identity" \
    --dds-public-key "$MATERIAL/dds-public.pem" \
    --credential "$MATERIAL/peer-b.jwt" \
    --listen "/ip4/127.0.0.1/tcp/$PORT_B" \
    --route "$PEER_A=/ip4/127.0.0.1/tcp/$PORT_A" \
    --fetch-peer "$PEER_A" \
    --resource peer-b-camera \
    --storage "$WORKDIR/storage-b" \
    > "$WORKDIR/peer-b.log" 2>&1 &
PID_B=$!

set +e
wait "$PID_A"
STATUS_A=$?
PID_A=
wait "$PID_B"
STATUS_B=$?
PID_B=
set -e

if [[ "$STATUS_A" != 0 || "$STATUS_B" != 0 ]]; then
    sed 's/^/[peer-a] /' "$WORKDIR/peer-a.log"
    sed 's/^/[peer-b] /' "$WORKDIR/peer-b.log"
    echo "two-peer demo failed: peer-a=$STATUS_A peer-b=$STATUS_B" >&2
    exit 1
fi

grep -q "^CATALOG peer_id=$PEER_B count=1 resource_ids=peer-b-camera$" "$WORKDIR/peer-a.log"
grep -q "^CATALOG peer_id=$PEER_A count=1 resource_ids=peer-a-camera$" "$WORKDIR/peer-b.log"
grep -q "^PEERS count=1 peer_ids=$PEER_B$" "$WORKDIR/peer-a.log"
grep -q "^PEERS count=1 peer_ids=$PEER_A$" "$WORKDIR/peer-b.log"
grep -q "^LEFT peer_id=$PEER_A$" "$WORKDIR/peer-a.log"
grep -q "^LEFT peer_id=$PEER_B$" "$WORKDIR/peer-b.log"

assert_join_rejected() {
    LABEL=$1
    CREDENTIAL=$2
    LOG="$WORKDIR/$LABEL.log"
    set +e
    "$APP" run \
        --domain "$DOMAIN_ID" \
        --identity "$MATERIAL/peer-a.identity" \
        --dds-public-key "$MATERIAL/dds-public.pem" \
        --credential "$CREDENTIAL" \
        --listen /ip4/127.0.0.1/tcp/0 \
        --storage "$WORKDIR/storage-$LABEL" \
        --run-for-secs 1 \
        > "$LOG" 2>&1
    NEGATIVE_STATUS=$?
    set -e

    if [[ "$NEGATIVE_STATUS" == 0 ]]; then
        echo "$LABEL credential unexpectedly joined" >&2
        exit 1
    fi
    grep -q '^JOIN_FAILED ' "$LOG"
    if grep -q '^CATALOG ' "$LOG"; then
        echo "$LABEL credential exposed a catalog" >&2
        exit 1
    fi
}

assert_join_rejected wrong-domain "$MATERIAL/peer-a-wrong-domain.jwt"
assert_join_rejected wrong-peer "$MATERIAL/peer-a-wrong-peer.jwt"
printf '%s\n' 'not-a-compact-jwt' > "$WORKDIR/malformed.jwt"
assert_join_rejected malformed "$WORKDIR/malformed.jwt"

sed 's/^/[peer-a] /' "$WORKDIR/peer-a.log"
sed 's/^/[peer-b] /' "$WORKDIR/peer-b.log"
sed 's/^/[wrong-domain] /' "$WORKDIR/wrong-domain.log"
sed 's/^/[wrong-peer] /' "$WORKDIR/wrong-peer.log"
sed 's/^/[malformed] /' "$WORKDIR/malformed.log"
echo "DEMO_OK domain_id=$DOMAIN_ID peer_a=$PEER_A peer_b=$PEER_B"
