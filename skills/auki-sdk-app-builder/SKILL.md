---
name: auki-sdk-app-builder
description: Use when building public applications, demos, integrations, prototypes, robot producers, or tools with the Auki SDK. Guides your agent to understand what the SDK provides, inspect official SDK docs/examples/package exports before implementation, prefer public SDK APIs for resources, registries, streams, geometry, discovery, clocks, and payload contracts, identify and register robot resources from device APIs/docs, avoid hand-rolling SDK protocols or patching SDK internals in app code, and document SDK capability gaps clearly.
---

# Auki SDK App Builder

Use the Auki SDK as the product surface. Build applications on top of SDK capabilities instead of recreating the SDK inside the app.

## Core Rule

If the code is about how Auki peers find each other, join clusters, describe capabilities, register metadata, open streams, encode/decode SDK payloads, manage clocks, or convert frames and transforms, inspect the SDK first.

Do not hand-roll Auki SDK concepts when an official SDK API, binding, example, or helper exists.

## SDK Mental Model

The Auki SDK handles shared distributed-system and spatial machinery for an application. The app should consume this machinery through public SDK APIs.

Use the SDK for:

- Peer identity: representing the app in the Auki network.
- Discovery and cluster membership: finding peers, joining/leaving clusters, reconnecting, and observing participants.
- Live resource discovery: reading what a peer currently offers through the SDK resource catalog, including `/auki/resources`.
- Registries: content-addressed metadata for sensors, frames, clocks, payloads, and typed resources.
- Stream lifecycle: opening, accepting, producing, consuming, ending, and erroring streams.
- Typed payload contracts: using SDK-defined message wrappers and serialization instead of raw ad hoc bytes.
- Clocks and timestamps: declaring clock metadata and interpreting stream timestamps consistently.
- `auki-geometry`: coordinate frames, frame conventions, convention conversion, pose/transform composition, transform inversion, and spatial payload interpretation.
- Protocol compatibility: relying on SDK protocol versions and bindings rather than copying protocol details into app code.

The app is responsible for:

- Product behavior, UI, workflows, and presentation.
- Choosing which SDK resources to request, produce, display, or reconcile.
- Domain adaptation at the edge, such as reading a device/vendor API and publishing the result through SDK resources.
- User-facing error handling, observability, retries, and loading states.
- Small glue code around public SDK APIs.

## Cold-Start Workflow

Before implementing an SDK-based feature:

1. Identify the SDK version, target language/runtime, and package names in use.
2. Read the relevant official SDK docs, examples, package exports, and generated bindings.
3. Search the existing app for SDK usage patterns before adding a new abstraction.
4. Find the SDK primitive for each Auki concept involved: resource, registry entry, stream, clock, payload, peer, cluster, frame, pose, or transform.
5. Implement with public SDK APIs first.
6. Validate against an SDK example, integration test, or live SDK behavior.

Prefer official examples over invented patterns. If the examples and exports disagree, trust the current SDK surface and verify with a small runnable probe.

## Stop Signs

Stop and inspect the SDK before writing code that implements any of these locally:

- A peer discovery mechanism.
- A cluster join/reconnect loop.
- A custom resource catalog or resource polling contract.
- A local registry or content-addressed hash format.
- A stream protocol, stream request envelope, stream accept path, or stream error path.
- A custom payload wrapper for SDK transport.
- A clock model or timestamp normalization layer.
- Coordinate-frame convention conversion.
- Quaternion, matrix, or transform inversion/composition for SDK spatial data.
- A producer-to-render-frame conversion.

If the SDK provides the concept, use it. If the SDK lacks the concept, isolate the smallest app-side adapter and document the SDK gap.

## Resources And Registries

Treat the SDK resource catalog as the source of truth for what a peer can currently provide.

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
- Robot/vendor APIs and docs inspected when building a robot producer.
- SDK resource inventory completed for every exposed robot capability.
- SDK-owned responsibilities separated from app-owned responsibilities.
- `auki-geometry` considered for all frame, pose, and transform work.

Before finishing:

- No local replacement exists for an available SDK concept.
- Resource, registry, stream, clock, payload, and frame metadata paths use public SDK APIs.
- Robot producer catalogs include all requestable SDK-relevant resources and required registry metadata.
- Any app-side workaround is isolated and documented as a missing SDK capability.
- Spatial math uses SDK geometry helpers or clearly justified device-specific calibration.
- A focused test, example, or live probe verifies the SDK integration path.
