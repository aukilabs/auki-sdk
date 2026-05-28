# Concept: Peer-Owned Logs

*Coming soon — content to be drafted in a follow-up card.*

This page will explain the core SDK invariant: a peer owns data products, a data product has exactly one canonical `peer_id`, and materialized copies preserve that `peer_id`. It'll cover what a "log" actually is, the source/writer split on manifests, and how materialization works without `materialized_by_peer_id` style ownership leaks.

Until then, see:

- [#216 design spec](https://github.com/aukilabs/auki-sdk/blob/develop/docs/superpowers/specs/2026-05-27-216-schema-and-api-placement-design.md) — the locked schema and rationale
- [`dataproducts.md`](https://github.com/aukilabs/auki-sdk/blob/develop/dataproducts.md) — the resource catalog reference

[← Back to: For SDK Consumers](For-SDK-Consumers)
