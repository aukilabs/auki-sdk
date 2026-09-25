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

## Stalled direct connection recovery

The `stalled_connection` module in `core/auki-p2p/tests/relay_transport.rs`
pauses one established publisher TCP tunnel while a healthy sibling sends an
echoed byte every 100 ms through the same relay. This models a connection that
stops making protocol progress; it does not establish the original cause of the
September 24, 2026 load-test stall, nor simulate a particular NAT failure.

```sh
# Rust relay: no additional process required.
cargo test --locked --offline -p auki-p2p --test relay_transport \
  stalled_connection::stalled_connection_is_retired_automatically \
  -- --exact --nocapture

# Build the loopback Go fixture using the commands above first.
AUKI_HANDOVER_GO_RELAY=/tmp/auki-handover-go-relay \
  cargo test --locked --offline -p auki-p2p --test relay_transport \
  stalled_connection::go_stalled_connection_is_retired_automatically \
  -- --exact --ignored --nocapture

AUKI_HANDOVER_GO_RELAY=/tmp/auki-handover-go-relay \
  cargo test --locked --offline -p auki-sdk --lib \
  caller_timeouts_recover_through_dms_without_creating_a_new_booking \
  -- --ignored --nocapture
```

Both transport regressions require the SDK to close the affected connection
automatically after sustained source-admission negotiation timeouts. The proxy
remains stalled; the test does not close it. The tests verify reservation
retirement, a fresh connection, restored authenticated echo, and continued
healthy-sibling delivery. The low-level transport tests explicitly reserve again.

The SDK regression uses the actual booking coordinator and native Node with the
Go relay, but a scripted `RelayBookingApi` boundary. Every caller cancels its
open before the negotiation timer fires. The test checks exactly one fenced
`ReservationLost` report, route unpublication, recovery under a new provider
epoch, no new booking, restored delivery, and awaited booking deletion/shutdown.
It does not test the live DMS service or its database.

The shared fixtures in `fixtures.rs` use loopback DNS, fixture credentials, and
mock relay admission. No API/DDS/DMS endpoints or real credentials are used.
Reservation lifetimes exceed the injected stall, excluding scheduled renewal.
Existing streams on the retired connection are interrupted; independent healthy
connections remain intact. TCP fault injection is local; WSS and language-binding
runtime checks must be reported separately. Native test success or a WASM build
alone does not qualify every platform.
