# Clean-room task: two-Peer volume monitoring

## Purpose

Test whether an unfamiliar developer or coding agent can build a truthful,
composable application from the proposed public vocabulary without reading or
copying the Camera fixture's implementation.

This is an API-usability test. Failure to complete it without modifying the SDK
is a result. The implementer must report missing abstractions rather than
inventing look-alike manifests, accessing private internals, or weakening the
requirements.

## Information available to the implementer

The clean-room implementer may use only:

- this task;
- generated Rust documentation for the public
  `auki-typed-dataflow-experiment` API;
- the compiler and test runner;
- the accepted decisions from
  `Design-Typed-Dataflow-Decision-Review.md`.

The implementer must not read:

- the experiment crate's module source;
- the Camera fixture or its tests;
- previous result documents;
- another implementation of this task.

## Application

Build an in-process simulation of two Peers, `peer-a` and `peer-b`. Each Peer
hosts these Components:

```text
Microphone Component
  output.audio: AudioBlock
       |- 60-second audio Buffer Product
       `- Volume Meter Component input.audio

Volume Meter Component
  output.level: GaugeObservation
       |- session-length volume Episode Product
       `- serialized observation by the other Peer
```

Each Peer must receive the other Peer's volume observations through the
serialized in-memory transport boundary. Production networking and real audio
hardware are deliberately out of scope.

## Typed payloads

Use an `AudioBlock` containing at least:

- interleaved `f32` samples;
- sample rate in hertz;
- channel count;
- frames per channel.

Use a `GaugeObservation` containing one `f64` value.

The Volume Meter computes RMS level as dBFS. Silence needs an explicit finite
floor rather than negative infinity. Do not label an uncalibrated digital
amplitude as dB SPL.

The level Output must truthfully describe:

```yaml
kind: gauge
observes: audio_level
datatype: float64
unit: dBFS
```

The audio Output contract must make its actual sample representation,
interleaving, sample rate, and channel configuration discoverable. If the
public contract type cannot express that truthfully, report the limitation.

## Retention

Treat one simulated AudioBlock as 10 milliseconds. The audio Buffer therefore
retains at most 6,000 blocks, representing 60 seconds.

The volume Episode begins with the Peer session and remains active until the
demo explicitly concludes the session. Its conclusion must name the final
timestamp. It must not silently evict earlier volume observations.

Memory is a valid storage location for this test. Do not claim disk durability.

## Catalog

Each Peer Catalog must expose:

- the Microphone Component Manifest;
- its current audio Output Manifest;
- the Volume Meter Component Manifest;
- its current level Output Manifest;
- the audio Buffer Product Manifest;
- the volume Episode Product Manifest.

Private implementation helpers must not appear in the Catalog. Products must
not be represented as Components.

## Required behavior

The demo and tests must prove:

1. typed Microphone output connects to the typed Volume Meter input;
2. incompatible payload types cannot connect;
3. both Peers independently produce audio and volume observations;
4. each audio Buffer evicts its oldest block after its 6,000-entry limit;
5. each volume Episode retains the whole simulated session and concludes once;
6. local audio Buffer retention and local Volume Meter consumption share the
   immutable AudioBlock sample allocation;
7. remote volume observations preserve values but cross a real serialization
   boundary;
8. dropping a remote observation handle stops subsequent delivery;
9. every observation references the Output Manifest that truthfully governs
   it;
10. Catalog entries remain Component and Product descriptions with distinct
    identities;
11. no production networking, Manager, heartbeat, Domain, Registry, or Log
    implementation is modified.

## Initial-pass constraint

The first attempt may add only the application crate. It may not change
`auki-typed-dataflow-experiment`.

If blocked, stop and report:

- the desired operation;
- the closest public API discovered;
- the missing or misleading abstraction;
- whether a workaround would violate the design;
- the smallest proposed SDK change.

Only after that report is reviewed may a second pass change the experimental
SDK.

## Deliverables

- application source;
- focused tests;
- a short attempt log including every clarification requested;
- public API changes, if a reviewed second pass authorizes them;
- final scoring using `CLEAN-ROOM-EVALUATION-RUBRIC.md`.

