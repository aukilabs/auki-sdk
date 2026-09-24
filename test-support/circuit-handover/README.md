# Circuit replacement regression fixture

This local fixture runs Hagall's vendored Go libp2p relay with a short circuit
limit. It binds loopback, mocks relay admission, and uses SDK test credentials
for real mutual peer authentication. It makes no API/DDS/DMS requests. Do not
deploy the fixture.

```sh
SDK_CHECKOUT=/path/to/auki-sdk
HAGALL_CHECKOUT=/path/to/hagall
cd "$HAGALL_CHECKOUT"
go build -mod=vendor -o /tmp/auki-handover-go-relay \
  "$SDK_CHECKOUT/test-support/circuit-handover/go-relay/main.go"
cd "$SDK_CHECKOUT"
AUKI_HANDOVER_GO_RELAY=/tmp/auki-handover-go-relay \
  cargo test --locked --offline -p auki-p2p --test relay_transport \
  handover_experiment -- --ignored --test-threads=1 --nocapture
```

The eleven opt-in tests cover forced expiry, sequential and overlapping planned
replacement, failed dial, wrong-Domain authentication, cancellation, concurrent
preparation, old expiry, traffic during preparation, shutdown, foreign-node
rejection, and the two-generation bound. Tests own and reap the relay process.
DNS fixture names resolve locally. Receivers validate payloads, sequence numbers,
and per-stream drain acknowledgements; relay counters check overlap and cleanup.

`HANDOVER_RESULT` counts complete 3,200-byte application frames, not packets.
These short debug tests are correctness checks. Rotation times include fixture
control work; they are not production latency benchmarks. Forced expiry can
lose successfully queued bytes, but does not necessarily do so on every run.
A clean planned run does not establish reliability during arbitrary failures.

The fixture uses TCP. Native WSS and browser execution require separate runtime
validation; a WASM compile alone does not establish browser behavior.

## Stalled direct connection experiment

The `stalled_connection` module in `core/auki-p2p/tests/relay_transport.rs`
reproduces the repeated source-admission negotiation timeout observed in the
September 24, 2026 load test. It uses the same three-peer scenario with either
the local Rust relay fixture or the Go fixture above:

```sh
# Rust relay: no additional process required.
cargo test --locked --offline -p auki-p2p --test relay_transport \
  stalled_connection::stalled_connection_retries_same_transport_until_explicit_close \
  -- --exact --nocapture

# Go relay: build the fixture using the commands above first.
AUKI_HANDOVER_GO_RELAY=/tmp/auki-handover-go-relay \
  cargo test --locked --offline -p auki-p2p --test relay_transport \
  stalled_connection::go_stalled_connection_retries_same_transport_until_explicit_close \
  -- --exact --ignored --nocapture
```

A loopback TCP proxy pauses reads/forwarding on one established publisher
tunnel while a second publisher sends an echoed byte every 100 ms through the
same relay. The fault models a connection that stops making protocol progress;
it is not a packet-loss or NAT-timeout simulation. The target and healthy source
do not use the proxy. Each test takes approximately 31 seconds.

On the load-test SDK revision `443658147c6e1fd965f28f9bb1860d0af6153dc6`, both
relay backends produced three successive approximately ten-second
`NegotiationTimeout` errors for `/auki-p2p/relay-auth/1` on the same connection
ID, without opening another tunnel. The healthy sibling kept delivering.
Closing only that tunnel during the fourth open produced
`SelectedConnectionClosed`. Explicitly retiring the old reservation (including
an already-retired stale handle), reserving again on a new connection, and
opening the application stream restored authenticated echo delivery.

This establishes a reproducible stalled-connection retry behavior and verifies
manual recovery. It does not prove what originally stalled the live publishers,
or validate an automatic SDK recovery policy. No production SDK behavior changes
are included. In particular:

- No DMS booking, provider epoch recovery, or full `AukiPeer` coordinator runs in
  this fixture. It does not establish whether a live recovery needs DMS calls.
- Traffic on other peers' independent connections is preserved. Existing streams
  on the deliberately stalled connection are not tested or promised to survive.
- TCP native execution is tested. WSS, WASM, Python and Swift runtime validation
  remains necessary before claiming a cross-platform fix.
- The Rust fixture uses a 180-second reservation and the Go fixture its normal
  reservation lifetime, keeping reservation renewal out of the 30-second stall.

A recovery implementation should be tested for bounded retry, caller timeout /
cancellation, exact connection/generation fencing, concurrent opens and cleanup,
and healthy traffic preservation before using this experiment as a regression
for automatic recovery.
