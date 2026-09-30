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
