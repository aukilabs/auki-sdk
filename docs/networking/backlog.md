# Peer-To-Peer Cluster Backlog

Status: implementation and follow-up work queue. This file is not part of the
protocol requirements.

Owner: TBD.

Last updated: 2026-05-26.

Related baseline:
[`baseline.md`](baseline.md).

Related drafts:
[`drafts.md`](drafts.md).

Related glossary:
[`glossary.md`](glossary.md).

## Purpose

Track the work that follows the draft implementable baseline.

The protocol requirements live in `baseline.md`. Future extension sketches live
in `drafts.md`. This file is only the work queue.

## Current State

The first RFC-shaped SDK foundation now exists in `auki-protocol` and
`auki-p2p`.

`auki-protocol` owns the pure v1 protocol surface: JSON frames, signed peer and
domain authority objects, lifecycle handshakes, offer catalogs, Get,
Subscribe, spatial messages, status snapshots, and locked vectors.

`auki-p2p` owns the clean libp2p runtime path: configured peer dialing,
lifecycle authorization, local domain and offer registration, offer loading,
Get and Subscribe consumers/providers, relationship status, and a full
two-peer smoke flow. It does not reuse `NetworkRuntime`, Discovery, Manager
election, or legacy cluster membership semantics.

Discovery remains optional and non-authoritative. The next work is proving the
new path against real SDK app surfaces without breaking the shipped
`auki-network` / `auki-domain` runtime.

## Next Work

Build migration adapters and one app-facing proof without replacing shipped
legacy behavior.

Suggested next vertical slice: browser-first Sentinel preview.

1. Define a small RFC offer profile for Sentinel's live RGB preview:
   `subscribe` access, JPEG payload bytes in `auki.spatial_message.v1`, and
   Sensor / Clock / Frame registry references.
2. Add a producer adapter that maps Sentinel's existing preview latch to an
   `AukiNode` Subscribe provider.
3. Add a browser-compatible consumer experiment that uses `auki-protocol`
   frames and browser libp2p transports instead of Park's Rust HTTP cache.
4. For the localhost MVP, have the browser dial Sentinel and open Subscribe;
   Sentinel then sends preview frames directly on that stream.
5. Keep Park's current backend path as legacy compatibility while the browser
   peer path is proven.

Transport questions for the browser slice:

- Which browser transport is selected first: WebRTC Direct, WebTransport, or
  WebSocket?
- Is any relay/signaling service required only for setup, or do frame bytes
  continue to flow through it?
- What multiaddr shape should Sentinel advertise for a localhost/browser MVP?
- Does `auki-p2p` need a browser-specific crate/feature, or should the browser
  experiment pair `auki-protocol` with a JS/libp2p runtime first?

## Interop/Test Work

Create these from the first SDK implementation, so the examples match running
code:

- compatibility examples;
- expected results;
- validation transcripts;
- fixture JSON;
- signed-object test vectors;
- frame encoding examples;
- machine-readable schema aids where they are useful.

Suggested coverage:

- JSON frame encoding and JSON wire conventions;
- signed-object canonicalization;
- peer bindings;
- domain id derivation;
- domain declarations;
- domain delegations;
- authority-chain validation;
- lifecycle handshake;
- Offer Catalog;
- Get;
- Subscribe;
- spatial message envelopes;
- error objects;
- status snapshots;
- time fields.

## Drafts To Pull Forward When Needed

Detailed draft text lives in [`drafts.md`](drafts.md).

Pull these forward only when product or implementation work needs them:

- Dynamic Served-Domain Updates
- Discovery Record Shape
- Discovery Data-Type Hints
- Peer Graph Hints
- Concrete Clock-Sync Protocol
- Shared Offer-Kind Profiles

These do not block the baseline configured/manual peer-to-peer path.

## Later / Product-Scope Work

Keep these parked unless product scope pulls them forward:

- DHT-style discovery.
- Relay milestone expansion beyond `RFC-0017` and `RFC-0018`.
- Shared offer-kind profiles and payload schemas.
- Chunking, replay, resume, reliable history, and large object transfer.
- Implementation migration notes, if needed after the baseline implementation
  exists.
