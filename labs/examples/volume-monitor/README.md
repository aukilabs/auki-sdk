# Two-peer Component volume monitor

Two independent `AukiPeer`s in one native process exchange volume through the
continuing Product subscription protocol over authenticated **loopback TCP**.
This replaces the older in-process serialized volume-monitor experiment.

Each peer creates:

- An explicitly synthetic audio Component, or a microphone Component on A when
  requested, emitting typed interleaved `f32` audio in 10 ms blocks.
- A 60-second audio Buffer, limited by source time, entry count, and accounted
  payload bytes. The volume-meter Component reads this same Buffer through
  `configured_buffer_input`; it does not receive a copied history.
- A volume-meter Component emitting `f64` RMS values in **dBFS**.
- A one-second volume Buffer for network subscriptions and a session-length
  volume Episode. These share the same immutable level payloads locally.
- A remote-volume display Component reading the other peer's imported volume
  Buffer through the same typed input API used locally.

The endpoint explicitly exports audio and volume Buffers. Each consumer fetches
the other peer's Catalog, selects the volume Product, and calls
`subscribe_product_exact` once. The host drives `next()`; there is no network
polling loop. The raw audio Buffer is also subscribable, but the normal demo
sends volume only. A regression test transfers raw audio through that same
protocol and checks its format and provenance.

## Run without hardware or credentials

From this branch's repository root:

```sh
cargo run --locked -p auki-component-volume-monitor -- --seconds 3
```

A produces a synthetic sine wave with peak amplitude 0.5, B uses 0.25. Expect
approximately `B observes A: -9.0 dBFS` and `A observes B: -15.1 dBFS`, followed
by both peers' retention and reception counts. A three-second run normally
concludes each local Episode with 300 observations. Peer IDs change each run.

`--seconds` accepts 1–300; the default is 3. Use `--seconds 65` to see audio
retention settle at 6,000 blocks while each volume Episode continues growing.
`--help` lists the options. Ctrl-C cancels the exchange, stops capture, attempts
local Episode finalization, and awaits endpoint and peer cleanup; it reports a
cancelled run rather than success.

The runner creates a fresh, in-memory signing key and two independent peer
keys. It exercises the real signed-credential handshake and encrypted transport
against its own isolated authority, **not DDS-issued production credentials**.
Only `127.0.0.1` listeners are opened. There is no login, discovery registration,
relay booking, wallet, task scheduling, or shared-environment fallback. No keys
or captured data are written to disk.

## Opt into a real microphone

```sh
cargo run --locked -p auki-component-volume-monitor --features microphone -- --microphone --seconds 10
```

This explicitly activates the default microphone on **A only**. B remains a
labelled synthetic source. The optional [CPAL](https://github.com/RustAudio/cpal/tree/v0.15.3)
adapter uses the device's actual default sample rate and channel count. It
converts supported native samples (`f32`, `i16`, `u16`, `i32`) to interleaved
`f32`, preserves channel order, and assembles fixed 10 ms blocks. There is no
resampling, automatic reconfiguration, or claim that native device encoding
matches SDK payload encoding. Unsupported rates/formats fail explicitly.

Allow microphone access in the OS when prompted. Linux builds with this feature
need ALSA development libraries. The default synthetic build needs no audio
device or audio-driver development package.

The callback feeds a bounded 32-block staging queue; a full queue or device
error fails capture visibly instead of silently dropping samples. The final
incomplete callback block, if any, is discarded when capture stops: it was never
published as an SDK observation. This adapter allocates per block and is a demo,
not a hard-real-time audio engine.

## What the metadata means

The Audio contract describes the **final SDK payload**, not the device's
intermediate representation. Synthetic sources advertise `synthetic_waveform`;
microphones advertise `microphone_signal`. The gauge is `kind: gauge`,
`observes: audio_rms_level`, `unit: dBFS`, with a real scalar `f64` payload.
RMS includes all channels equally, and silence is floored at -120 dBFS. This is
not calibrated dB SPL, perceptual loudness, or a statement about microphone gain.

Each source uses its own audio-sample clock: block zero is time zero and each
block advances 10 ms. Microphone times are derived from sample position, **not**
hardware capture timestamps or UTC. The clocks are separately named and are not
claimed to be synchronized across peers. The runtime's producer/Product
manifests and canonical hashes are used unchanged; the app invents no hashes.

The source configuration is fixed for the run. A format change requires a new
configured output and Product, termination notice, and explicit resubscription;
this demo does not attempt device hot-plug or reconfiguration.

## Boundaries

- All retention is **in memory**. Episodes keep every successfully derived local
  volume observation for this bounded run, then conclude. Closing the program
  discards them. No disk durability or crash recovery is claimed.
- The Episode is in the local runtime Catalog. The current network adapter
  exports Buffers only, so it does **not** advertise the Episode as remotely
  fetchable. Remote Episode fetching/persistence is separate follow-up work.
- Slow network consumers may see explicit retention gaps. Local metering checks
  its own input/capture errors; network delivery does not backpressure capture.
  A successfully imported observation is not a downstream processing receipt.
- Source Buffer closure ends subscriptions without inventing a producer failure.
  Transport errors terminate the run; no automatic reconnect or migration occurs.
  The host imposes an overall demo deadline (requested duration plus 10 seconds)
  and a three-second microphone-input deadline. These are application policies,
  not protocol heartbeats.
- No speaker playback, voice chat, compression, adaptive bitrate, cross-machine
  pairing, browser runtime, or production authentication UI is included. The
  local authority must not be turned into production trust or a public listener.

## Verify

```sh
cargo test --locked -p auki-component-volume-monitor
cargo test --locked -p auki-component-volume-monitor --all-features
cargo clippy --locked -p auki-component-volume-monitor --all-targets --all-features --no-deps -- -D warnings
```

Tests use synthetic blocks and ephemeral loopback peers. Even the all-features
tests do **not** open a microphone. They cover actual contracts and hashes,
local payload sharing, rolling eviction versus Episode retention, invalid input,
RMS arithmetic, bidirectional network delivery, raw audio transfer, conclusion,
and callback block assembly/overflow. Real microphone capture must be checked
manually with the explicit command above. The example is native-only; the
underlying Component protocol remains independently WASM-checkable.
