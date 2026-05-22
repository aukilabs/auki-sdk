# Tags — signed claims attached to data products

> **Status: WIP — working draft, not yet a committed spec.** Schemas, claim
> types, and authority rules in this document are subject to change. No SDK
> code consumes or produces TagClaims yet. v0 leaves signatures empty; the
> identity layer (`auki-identity`, planned) fills them in.

## Purpose

A *tag* is a signed assertion attached to a data product, expressing that the data is associated with some externally-meaningful identity — a domain, an anchor, a contributor. The protocol uses the same primitive for all of these: a `TagClaim`, signed by the issuer's wallet, attached to the data via an append-only sidecar file.

This factoring lets the SDK stop encoding domain membership in the filesystem tree. Domain becomes one kind of tag among many. Same primitive supports anchor citations, contribution credits, and future claim types we haven't named.

## `TagClaim` — v0 schema

```
TagClaim {
  // ── What's being claimed ──────────────────────────────
  data_id:      Hash,         // hash of the data product's manifest; identifies WHICH data
  tag_id:       Bytes,        // the thing being associated (domain_id, anchor_id, contributor_id, ...)
  claim_type:   string,       // tagged-enum discriminant; tells the verifier how to interpret tag_id

  // ── Who's making the claim ───────────────────────────
  issuer:       PubKey,       // public key of the issuing wallet
  issued_at_ns: i64,          // claim timestamp (nanoseconds, integrator's clock)

  // ── Cryptographic envelope ───────────────────────────
  signature:    Option<Bytes>,  // None for v0 (unsigned); Some(...) once identity layer lands
  schema_version: u32,          // 1
}
```

## Where it lives on disk

Tags do **not** live inside `log_manifest.json`. The auki-logs manifest is written once and treated as immutable; mutating it to add tags would break that invariant and force readers to re-canonicalize / re-hash on every change.

Instead, claims accumulate in a sibling file:

```
<log_root>/
├── log_manifest.json           ← immutable, written once
├── tags.jsonl              ← append-only, one TagClaim per line
└── segments/<padded-ns>.seg
```

JSONL means appending a new tag is a single-line write — no read-modify-rewrite, no locking, atomic on any sane filesystem. Park-on-receipt becomes "append a line to my local copy's `tags.jsonl`," nothing more. The manifest's `data_id` (its hash) doesn't change when tags are added.

## Claim types

Snake-case strings, extensible. Receivers parse known types and ignore unknown ones (forward-compatible).

| `claim_type`           | `tag_id` references                              | Typical issuer                       |
|------------------------|--------------------------------------------------|--------------------------------------|
| `domain_membership`    | a `domain_id` (hash of domain owner's pubkey)    | domain owner, or a delegated peer    |
| `anchor_citation`      | an `anchor_id` (hash of an anchor record)        | the data producer (observed it)      |
| `contribution_credit`  | a `contributor_id` (e.g. a detector developer)   | the app using the BYO component      |
| `revoke`               | the hash of a prior `TagClaim` being revoked     | the original issuer (only)           |

## Verification (when identity lands)

For each `TagClaim` attached to a data product:

1. **`data_id` matches** the manifest hash this claim is attached to. Local check, no network.
2. (If `signature.is_some()`) **Signature verifies** against `issuer` over the canonical-JCS bytes of `(data_id, tag_id, claim_type, issuer, issued_at_ns)`. The signature field is not part of the signed material.
3. **Issuer authority for this `claim_type`** — is the issuer authorized? E.g. for `domain_membership`, is `issuer` either the domain owner OR holds a delegation cert from them? This is **application-layer rule**, not protocol-enforced. The protocol provides verifiable facts; the convention layer decides what to trust.

## Who can apply a tag

Three points on the authority spectrum (see also: design discussion in conversations leading to this draft):

1. **Strict** — only the tag owner (e.g. domain owner) can apply the tag. Cryptographically clean: only the owner has the signing key. Practical pattern: contributors send untagged data to the cluster; owner tags on receipt. Default starting point.
2. **Delegated** — owner issues signed capability tokens authorizing specific peers. Same shape as PKI intermediates or OAuth scopes. Layers on additively when the workflow needs it (offline pre-tagging, etc.).
3. **Mandatory-on-membership** — cluster admission grants implicit tag-applying capability. Producer signs with their own key + an "I was admitted" attestation. Distributes signing load; trade-off is rigidity at the producer side.

The schema supports any of these; which authority model an ecosystem chooses is a convention layer decision.

## Concrete example (one line of `tags.jsonl`)

```json
{"data_id":"a3f2…","tag_id":"7e5c7d3bfc1a4e0a9d8b2c6f4e1d8a3b","claim_type":"domain_membership","issuer":"04a8…","issued_at_ns":1745000030500000000,"signature":null,"schema_version":1}
```

(Wrapped here for readability; on disk it's a single line.)

## Open questions

Cross-cutting design decisions for TagClaims (revocation semantics, `tag_id` derivation, set-scoped claims, propagation across derived data, self-hash) are tracked on the [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5).

Implementation note (not an open question): `signature` is null for v0 by design — the identity layer (`auki-identity`, planned) fills it in once it lands. Other fields are signature-aware so that addition is non-breaking.

## Out of scope (for v0)

- Wire transport (gossip vs. direct exchange vs. central registry).
- Trust / signing implementation (deferred to `auki-identity`).
- Authority enforcement at the protocol level (which keys can issue which `claim_type` is convention).
- Reward / payment routing (it's the convention layer's job, even though `contribution_credit` claims are the substrate).
- Tag-based query / indexing APIs (consumers build their own).
