# For SDK Consumers

This section is for engineers building products on top of the Auki SDK. If you're integrating the SDK into:

- **Booster** — the robot operator-facing visualizer / control surface
- **Park** — the desktop / browser visualizer + scenegraph composition
- **Galbot ROS adapters** — publishing pose, camera, point-cloud streams from a ROS environment
- **Your own custom robot data plane consumer**

…you're in the right place.

## Where to start

- [Quickstart](Quickstart) — boot a peer, register a sensor and log, inspect the catalog
- [Concept: peer-owned logs](Concept-Peer-Owned-Logs) — the SDK's core data abstraction (source/writer split, materialization)

More concept primers and recipes coming as follow-up cards land:

- The three IDs (peer / app / session)
- Materialization (how Park copies Galbot's stream)
- Register a sensor and publish frames (recipe)
- Consume another peer's stream (recipe)
- Materialize a remote log with custom retention (recipe)
- Migrate from pre-#216 SDK to v0.0.53+ (recipe)
- Troubleshooting common errors

## API surface

Apps use `auki-session` as the entry point. See:

- [auki-session crate README](https://github.com/aukilabs/auki-sdk/blob/develop/crates/auki-session/README.md) — Rust API
- [auki-session-py README](https://github.com/aukilabs/auki-sdk/blob/develop/bindings/python/auki-session-py/README.md) — Python API

## Release history

- [Release history](Release-History) — what changed between SDK versions, what to expect when bumping pins *(stub)*

For the immediate "what does this version break?" answer, the annotated git tag is authoritative: `git show v0.0.53` etc.
