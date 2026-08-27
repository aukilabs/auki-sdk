---
name: auki-sdk-app-builder
description: Use when building public applications, demos, integrations, prototypes, robot producers, or tools with the Auki SDK; when code touches peer identity, authenticated Peer/Session/Domain lifecycle, registries, logs, resource catalogs, streams, geometry, routes, clocks, or payload contracts; or when deciding whether app code should use an SDK API versus an app-side adapter.
---

# Auki SDK App Builder

Use the Auki SDK as the product surface. Build applications on top of SDK capabilities instead of recreating the SDK inside the app.

## Core Rule

If the code is about how Auki peers authenticate and connect, describe capabilities, register metadata, open streams, encode/decode SDK payloads, manage clocks, or convert frames and transforms, inspect the SDK first.

Do not hand-roll Auki SDK concepts when an official SDK API, binding, example, or helper exists.

## SDK Mental Model

The Auki SDK handles shared distributed-system and spatial machinery for an application. The app should consume this machinery through public SDK APIs.

Current native app layering:

- Stable identity: `auki_identity` loads or reconstructs wallet seed material; a wallet-backed host passes the 32-byte seed from `Wallet::derive_child("peer/v1")` to `auki_p2p::Identity::from_ed25519_seed`. That `Identity` is the canonical key owner used by libp2p and DDS proofs.
- `auki_session::Peer`: long-lived peer identity + app identity + storage root + peer-level registries for sensors, frames, and detectors.
- `auki_session::Session`: one run / timeline born from `Peer::start_session()`. It owns the fresh `session_id`, the session clock registry, the auto-minted monotonic + UTC clocks, and live logs.
- `auki_protocols`: exact authenticated protocol IDs, versioned wire types, bounded framing, validation, and locked vectors. It owns no runtime or handlers.
- `auki_domain::Domain`: authenticated network presence for one `Peer` + `Session` in one exact DDS Domain UUID. It owns one P2P node, serves only the exact versions selected through default-none `ServedProtocols`, and leaves with an owned cleanup barrier.

Use the SDK for:

- Stable peer identity: seed loading/minting, wallet reconstruction, peer identity derivation, and representing the app in the Auki network.
- Peer/session lifecycle: `Peer::new(...)`, peer-level registry registration, `Peer::start_session()`, session clocks/logs, and `Domain::builder(...).join()`.
- Authenticated Domain lifecycle: installing host-fetched DDS authority, joining/leaving one Domain, reconnecting over explicit routes, and observing currently authenticated peers.
- Live resource discovery: reading what an expected authenticated peer currently offers through `/auki/auth/1/resources/0.2.0` (and retained catalog versions).
- Registries: content-addressed metadata for sensors, frames, clocks, payloads, and typed resources.
- Stream lifecycle: opening, accepting, producing, consuming, ending, and erroring streams.
- Typed payload contracts: using SDK-defined message wrappers and serialization instead of raw ad hoc bytes.
- Clocks and timestamps: declaring clock metadata and interpreting stream timestamps consistently.
- `auki-geometry`: coordinate frames, frame conventions, convention conversion, pose/transform composition, transform inversion, and spatial payload interpretation.
- Protocol compatibility: relying on SDK protocol versions and bindings rather than copying protocol details into app code.
- Inbound protocol policy: selecting each exact version the application really hosts. Compiling an `auki_protocols` feature or calling a client method does not install an inbound handler.

The app is responsible for:

- Product behavior, UI, workflows, and presentation.
- Choosing which SDK resources to request, produce, display, or reconcile.
- Acquiring and refreshing DDS verification keys and the local peer-bound
  credential; core Domain/P2P crates perform no DDS or DMS HTTP.
- Supplying exact-peer route hints from configuration, product state, or a
  host-owned discovery/DDS/DMS adapter. Routes and `known_peers()` observations
  never authorize a peer.
- Domain adaptation at the edge, such as reading a device/vendor API and publishing the result through SDK resources.
- User-facing error handling, observability, retries, and loading states.
- Small glue code around public SDK APIs.

## Native App Startup Shape

For a native app using the current split API, the startup order is:

1. Load or reconstruct stable identity.
2. Construct a `Peer`.
3. Register peer-level metadata on the `Peer`.
4. Start a `Session` from the `Peer`.
5. Register session logs on the `Session`.
6. Join a `Domain` only when network presence is needed; explicitly select each
   exact inbound protocol version the app serves.

Rust shape:

```rust
let seed = auki_identity::load_or_mint_seed(&identity_seed_path)?;
let wallet = auki_identity::Wallet::from_seed(seed.to_vec())?;
let peer_seed: [u8; 32] = wallet
    .derive_child("peer/v1")
    .seed()
    .try_into()
    .expect("Wallet seeds are always 32 bytes");
let identity = auki_p2p::Identity::from_ed25519_seed(&peer_seed);
let peer_id = identity.peer_id().to_string();

let peer = auki_session::Peer::new(peer_id, app_id)
    .with_storage_root(storage_root);

let frame = peer.register_frame("head_left_camera_optical", FrameDef::ros_optical())?;
let sensor = peer.register_sensor("head_left_rgb", sensor_body)?;

let session = peer.start_session()?; // mints session_id + monotonic/UTC clocks
let clock = session.monotonic_clock();
let log = session.register_sensor_log(SensorLogSpec {
    sensor,
    clock,
    frame: Some(frame),
    head,
    segment_duration,
    retention,
})?;
```

`Session::new(peer_id, app_id)`, `Session::with_storage_root(...)`, sensor/frame/detector registration on `Session`, `Session::catalog()`, and `Session::join_domain(...)` are old-shape APIs. Do not use them in new app code unless pinned to an older SDK version that still exports them.

Catalog and network shape:

```rust
let rows = auki_domain::catalog_of(&peer, &session);

let config = auki_domain::DomainConfig::new(dds_domain_id, identity)
    .with_listen_addresses(listen_addresses)?;
let domain = auki_domain::Domain::builder(&peer, &session, config)
    .authority(dds_verification_keys, signed_p2p_credential)
    .served_protocols(auki_domain::ServedProtocols::none().with_resources_v2())
    .join()
    .await?;
let served_rows = domain.catalog()?;
domain.leave().await?;
```

Omit `served_protocols(...)` only for a client-only Domain that intentionally
accepts no built-in inbound application protocols. In Python, call the matching
exact methods such as `builder.serve_resources_v2()` and
`builder.serve_streams_v2()` before `await builder.join()`.

The current `auki-identity-swift` binding exposes `Wallet` only. The removed
Manager-era Swift network and browser packages exist only at the prior
`v0.0.60` tag and cannot join the authenticated Stage 1 runtime. Browser
support requires a future external authenticated-engine migration.

When docs, READMEs, examples, and exports disagree, prefer the current package exports, source-level public API, and tests for the SDK version being used.

## Cold-Start Workflow

Before implementing an SDK-based feature:

1. Identify the SDK version, target language/runtime, and package names in use.
2. Read the relevant official SDK docs, examples, package exports, and generated bindings.
3. Search the existing app for SDK usage patterns before adding a new abstraction.
4. Find the SDK primitive for each Auki concept involved: stable identity, peer, session, authenticated Domain, route, resource, registry entry, stream, clock, payload, frame, pose, or transform.
5. Implement with public SDK APIs first.
6. Validate against an SDK example, integration test, or live SDK behavior.

Prefer official examples over invented patterns. If the examples and exports disagree, trust the current SDK surface and verify with a small runnable probe.

## Stop Signs

Stop and inspect the SDK before writing code that implements any of these locally:

- A second P2P runtime, authentication handshake, or reconnect loop beside `Domain`.
- Treating route discovery, cached participant metadata, or connectivity as authorization. Hosts may discover dial hints externally, but only DDS-signed Domain credentials authorize protocol streams.
- A relay-booking service inside `Domain`. The product adapter selects and
  authorizes a provider and owns booking HTTP; canonical `auki-p2p::Node`
  manages the Circuit Relay v2 reservation, and only its confirmed complete
  circuit route enters distribution or a Domain's exact-peer dial hints.
- Direct construction of `Session` for current native SDKs; sessions are born from `Peer::start_session()`.
- Sensor, frame, or detector registration on `Session`; these are peer-level registries.
- Catalog serving or domain joining on `Session`; these are `auki-domain::Domain` responsibilities.
- A custom resource catalog or resource polling contract.
- A local registry or content-addressed hash format.
- A stream protocol, stream request envelope, stream accept path, or stream error path.
- Assuming a Domain serves a protocol merely because its wire types or client method are available.
- A custom payload wrapper for SDK transport.
- A clock model or timestamp normalization layer.
- Coordinate-frame convention conversion.
- Quaternion, matrix, or transform inversion/composition for SDK spatial data.
- A producer-to-render-frame conversion.

If the SDK provides the concept, use it. If the SDK lacks the concept, isolate the smallest app-side adapter and document the SDK gap.

## Resources And Registries

Treat the SDK resource catalog as the source of truth for what a peer can currently provide.

- Register frames, sensors, and detectors on `auki_session::Peer`.
- Start a `Session` from the peer; the SDK mints `session_id` and registers monotonic + UTC session clocks.
- Register extra clocks and owned logs on `auki_session::Session`.
- Build catalog rows through `auki_domain::catalog_of(&peer, &session)` and serve them through a `DomainBuilder` that explicitly selects the matching catalog version, not through app-owned catalog builders.
- Use SDK resource APIs to discover sensors, streams, pose resources, and other capabilities.
- Use resource IDs as stable handles for requestable resources.
- Use registry references and hashes to fetch immutable metadata instead of embedding guessed schemas in app code.
- Do not infer resource contracts from names when typed SDK metadata is available.
- Do not advertise or assume resources that cannot currently accept stream opens.

When a resource appears wrong, inspect the resource entry and registry entries first. Check the resource ID, variant, state, sensor or pose metadata, frame reference, clock reference, and payload type before changing app behavior.

## Robot Producers

When building an app that produces SDK resources for a robot, first build a resource inventory from the robot's APIs and documentation.

Do not rely on a single demo stream, UI-visible stream, or guessed sensor list. Investigate the robot/vendor API surface, device documentation, runtime frame names, sensor metadata, and available calibration/extrinsic methods before deciding what to register.

Register every SDK-relevant robot resource the producer is responsible for exposing:

- Image sensors: RGB, depth, stereo, fisheye, thermal, or other camera streams.
- Range sensors: lidar, radar, depth point clouds, sonar, and related point clouds.
- Joint and actuator state: joint encoders, grippers, end effectors, wheels, and other kinematic state.
- Pose and transform resources: fixed sensor extrinsics, live moving sensor poses, robot root poses, tool poses, and frame edges needed by consumers.
- Frames: body frames, optical frames, sensor frames, tool frames, world/map frames, and their conventions.
- Clocks: capture clocks and stream timestamp domains.
- Other robot capabilities the SDK supports and the app exposes, such as audio, IMU, force, or device state.

For each resource, confirm:

- The stable resource ID.
- The correct SDK resource variant and payload type.
- The registry metadata required by consumers.
- The frame ID, frame convention, and frame hash.
- The clock ID and timestamp semantics.
- Whether the resource is currently requestable.
- How stream open, first frame, no-signal, and end-of-stream behave.

Only advertise resources that can currently accept stream opens. If a robot resource is temporarily unavailable, omit it from the live catalog until it becomes requestable, while keeping its resource ID stable when it returns.

## Streams And Payloads

Use SDK stream APIs for stream lifecycle and SDK payload types for wire data.

- Open streams through SDK stream requests.
- Produce and consume typed SDK payloads.
- Preserve SDK timestamps and sequence metadata where available.
- Handle SDK stream errors explicitly instead of treating them as generic network failures.
- Avoid forwarding raw bytes unless the SDK payload contract says the payload is raw bytes.

If a consumer cannot decode a stream, verify the SDK payload wrapper and registry metadata before changing the producer's application logic.

## Auki Geometry

Use `auki-geometry` for SDK spatial math.

Do not hand-roll:

- Frame convention conversion.
- Axis flips between producer frames and render/world frames.
- Quaternion order conversion by guesswork.
- Transform composition or inversion.
- Sensor-frame to world/render-frame matrices.
- Pose interpretation for SDK spatial payloads.

When spatial data appears wrong:

1. Inspect the frame registry entries for all frames involved.
2. Confirm the declared frame conventions match the source data.
3. Confirm transform direction: which frame maps to which parent/root frame.
4. Use `auki-geometry` helpers for conversion, composition, and inversion.
5. Check timestamp alignment between sensor samples and pose/transform samples.
6. Only then consider domain-specific calibration or vendor-frame issues.

Do not fix spatial bugs by flipping signs or swapping axes locally unless you can point to the SDK or device contract that requires that conversion.

## Missing SDK Capabilities

When the SDK does not expose the capability the app needs:

- State the missing SDK capability precisely.
- Use the smallest app-side adapter that keeps the workaround isolated.
- Keep the adapter behind a narrow interface so it can be removed when the SDK surface exists.
- Avoid editing SDK internals, generated bindings, or private modules in the application repo.
- Do not create a parallel public contract that competes with the SDK.

Public app agents should not make SDK implementation changes as part of app work. Document the gap for SDK maintainers or the application owner.

## Implementation Checklist

Before coding:

- SDK version and runtime identified.
- Official docs/examples/exports inspected.
- Existing app SDK patterns inspected.
- Current lifecycle selected: identity -> `Peer` -> `Session` -> optional `Domain`.
- Exact inbound `ServedProtocols` selected, or default-none client-only behavior documented.
- Robot/vendor APIs and docs inspected when building a robot producer.
- SDK resource inventory completed for every exposed robot capability.
- SDK-owned responsibilities separated from app-owned responsibilities.
- `auki-geometry` considered for all frame, pose, and transform work.

Before finishing:

- No local replacement exists for an available SDK concept.
- Identity, peer/session/domain lifecycle, resource, registry, stream, clock, payload, and frame metadata paths use public SDK APIs.
- Peer-level metadata is registered on `Peer`; clocks/logs are registered on `Session`; catalog/network presence is handled by `Domain`.
- Each inbound protocol handler is an intentional exact-version opt-in; client operations are not confused with serving.
- Robot producer catalogs include all requestable SDK-relevant resources and required registry metadata.
- Any app-side workaround is isolated and documented as a missing SDK capability.
- Spatial math uses SDK geometry helpers or clearly justified device-specific calibration.
- A focused test, example, or live probe verifies the SDK integration path.
