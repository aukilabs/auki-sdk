# Public typed-dataflow volume monitor

This is the reviewed second pass of the clean-room task. It is deliberately a
separate crate and depends only on the public API of
`auki-typed-dataflow-experiment`.

It demonstrates:

```text
Microphone Component
  -> typed Audio Observable
       |- 6,000-block Buffer Product
       `- Volume Meter Component
            -> typed dBFS Gauge Observable
                 |- session Episode Product
                 `- serialized observer on another Peer
```

Run it with:

```sh
cargo run -p auki-typed-dataflow-volume-monitor
cargo test -p auki-typed-dataflow-volume-monitor --all-targets
cargo test -p auki-typed-dataflow-volume-monitor --doc
```

This is an in-process data-plane experiment. It does not use or modify
production networking, Manager, heartbeat, Domain, Registry, or Log code.
