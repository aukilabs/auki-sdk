# Parking lot — auki-domain

Open questions for the auki-domain crate. Cross-cutting questions that involve other crates live in the [root `parking_lot.md`](../../parking_lot.md) or [`crates/parking_lot.md`](../parking_lot.md).

When a question is answered inline, an agent removes the item and propagates the answer everywhere it's relevant — see [CLAUDE.md](../../CLAUDE.md) for the workflow.

---

## Successor-token encoding for v2 Manager-handoff hardening _(filed by Nils's claude, 2026-05-13)_

The v1 Discovery contract (locked 2026-05-13) skips signature verification entirely, so for v1 even bare unsigned JSON is fine. For the v2 hardening pass, the successor token `{cluster, eligible_successor: <joiner_peer_id>, issued_at: <ts>}` signed by the current Manager's libp2p private key needs an encoding decision. Three options:

1. **Prost message in `auki-proto`.** Consistent with the convention putting all on-wire payloads in root `proto/auki` schemas and generated Rust bindings in `auki-proto`. Compact wire; deterministic encoding (good for signatures); typed boundary.
2. **JWT-flavored.** Familiar ecosystem; but the JWT signature stack doesn't natively speak libp2p keypair (ed25519 / secp256k1 / RSA — libp2p's `Keypair` enum), so we'd be reimplementing the bit JWT is supposed to give us.
3. **Bare signed JSON.** Quickest to ship. No prost schema bump, no dep on `auki-proto`. Risks the canonicalization rabbit hole (whose JSON ordering wins?) the moment a second language signs or verifies — `auki-jcs` exists for exactly that, so the cost is real.

**Lean: prost in `auki-proto`,** ~60%. Matches the current root-proto convention and gives a deterministic encoding for free. Question only matters at v2; defer until then.

---

## Stale-Manager join policy — what if Discovery points at a dead Manager before the join response? _(filed by Nils's codex, 2026-05-17)_

The 2026-05-17 heartbeat fix arms Manager-death detection once a non-Manager has a membership snapshot and an expected `manager_peer_id`. That closes the "Manager dies before the first heartbeat frame" path.

A different edge remains open: `ClusterManager::join_cluster` currently needs the discovered Manager to answer `/auki/join/0.0.1` before the joining peer has any membership document. If Discovery already points at a dead Manager before the join request completes, the peer cannot safely run the existing election rule because it does not know the cluster membership or join ordering.

Options:

1. **Fail loudly and let the operator recreate/join another cluster.** Current behavior: the join request times out or fails. Safest because the SDK does not invent membership it never received, but poor headless recovery from stale one-peer clusters.
2. **Self-takeover only when Discovery says `peer_count == 1`.** The joining peer rotates Discovery to itself and initializes a one-member membership document. This recovers stale singleton clusters but relies on Discovery's aggregate count being fresh enough to authorize destructive replacement.
3. **Have Discovery serve a signed/latest membership snapshot.** Joiners can recover from a dead Manager by fetching the last known membership from Discovery, then running the normal election rule. Cleanest model, but it expands Discovery from Manager-address directory into membership-snapshot storage.

**Lean: do not add unilateral takeover without either `peer_count == 1` semantics being explicitly accepted or Discovery carrying a recoverable membership snapshot.** Revisit when Park/Booster need unattended recovery from a stale Discovery Manager hint.

---

## DHT-backed cluster doc as long-term direction _(forward-looking, filed by Nils's claude, 2026-05-13)_

Long-term direction Nils flagged: replace the Manager-authoritative-RAM cluster doc with a DHT, so authoritativeness isn't bound to a single Manager.

**Out of scope for v1.** v1 keeps the Manager-authoritative model with peer-side gossip + convergence guarantees (anti-entropy, reconciliation-on-reconnect, last-writer-wins on disagreement). The DHT direction is the v2+ shape.

**Why this matters now:** the trust model shifts when there's no single Manager. Byzantine resilience, signature chains, and the eventual-consistency model all reshape what "successor token" and "cluster identity" mean. Worth keeping on the radar so v1 design choices don't paint v2 into a corner. Open angles to think through when the time comes:
- Is the DHT scoped per-cluster (cluster members participate in their own DHT) or workspace-wide (cross-cluster, with cluster identity as a key)?
- How do successor tokens map onto a Manager-less model? They probably become signed handoff certs that any peer can verify against the DHT-stored peer history.
- libp2p has Kademlia DHT built in; the integration question is whether the cluster doc fields fit cleanly into Kademlia's key-value model or need a CRDT layer on top.

No action required now; revisit when v1 is shipped and stable.
