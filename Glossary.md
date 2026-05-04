# Glossary

Definitions of key terms in the Auki SDK and the surrounding real-world-web protocol. This is a seed list — entries accrete as the SDK grows.

---

## Domain

A unique identifier applied as a tag to data, asserting that the data describes a specific physical space. The tag is what lets disparate data types — RGB video, point clouds, poses, detections — be grouped as describing the same place; without intent, an RGB clip is just video.

A Domain is *not* a scenegraph and *not* a coordinate system. A Domain has zero or more **scenegraphs** tagged with it; the **Domain Owner** designates one as the canonical **Map**.

When devices network on the real world web, they discover each other and form **clusters** around shared Domain IDs (a *domain-as-topic*). On disk, Domain membership rides on a data product as a `domain_membership` [TagClaim](tags.md), not as a path or filename — Domain is one kind of tag among many.

## Domain Owner

The entity that controls a Domain — concretely, the holder of the keypair whose pubkey hashes to the Domain ID (see [`tags.md`](tags.md)). Has authority to designate a scenegraph as the Map and to issue `domain_membership` TagClaims under this Domain.

## Domain ID

The identifier for a Domain. Derived as `hash(domain_owner_pubkey)` (see [`tags.md`](tags.md)). Used as the `tag_id` in `domain_membership` TagClaims and as the topic peers cluster around on the network.

**Domain ID, Scenegraph ID, and Session ID are three distinct identifiers** — they answer different questions, and none is derivable from another:

| Identifier      | Question                              | Derivation                       |
|-----------------|---------------------------------------|----------------------------------|
| Domain ID       | which place?                          | `hash(domain_owner_pubkey)`      |
| Scenegraph ID   | which structured map of that place?   | many per Domain; Owner picks one as the Map |
| Session ID      | which recording run?                  | per-daemon UUIDv4 minted at session start |

## Cluster

The runtime group of devices networking around a shared Domain ID — a *domain-as-topic*. When devices come online and want to participate in a Domain, they discover each other (via DHT, mDNS, or a circuit relay) and form a cluster. The transport is libp2p (see [`auki-network`](crates/auki-network)); the Domain ID is what gives the cluster a reason to exist.

Cluster formation lives in higher layers; the SDK provides primitives, not the network protocol itself.

## Scenegraph

A structured representation of the spatial data for a Domain — typed nodes (frames, sensors, clocks) connected by transform edges. Evaluable at a timestamp by composition along a transform path:

```
T_X_session(t) = T_body_session(t) ∘ T_X_body(t)
```

Many scenegraphs may be tagged with the same Domain ID; they may differ in coverage, resolution, contributing data sources, or age.

## Scenegraph ID

The identifier for a specific scenegraph. Distinct from Domain ID — multiple scenegraphs can share a Domain ID; the Domain Owner picks one as the canonical Map.

## Map

The canonical scenegraph designated by a Domain Owner. The default served when a peer asks for "the map" of a Domain without specifying a Scenegraph ID. One Map at a time per Domain, but many candidate scenegraphs.

## Session ID

The identifier for a recording session — a single span of capture activity by one daemon (BoosterApp, Sentinel, etc.). Minted as a fresh UUIDv4 at daemon startup, used both as the on-disk session directory name and as the `session_id` value carried in every manifest written during the run (see [`auki-session`](crates/auki-session)).

Orthogonal to Domain and Scenegraph: a Session ID identifies *when and by whom* data was captured, not *what it's about*. Tying a session's data products to a Domain happens after the fact via [TagClaim](tags.md).
