# Security Profile V1

Status: draft production guardrails.

Last updated: 2026-05-22.

Related baseline:
[`baseline.md`](baseline.md).

## Purpose

This file summarizes the minimum security guardrails around the baseline for
production-like deployments.

The baseline is suitable for a first SDK implementation and configured/private
peer-to-peer relationships. It is not production-safe by itself.

## Security Verdict

The cryptographic structure is directionally sound: authority is rooted in the
transport-authenticated libp2p peer id, wallet-signed peer binding, verified
domain declarations and delegations, and local policy.

Discovery, offers, registry references, labels, timestamps, relays,
diagnostics, and status snapshots are not authority proofs.

The main risks are business-logic and operational risks: open admission, stale
authority, permissive offer handling, resource exhaustion, unsafe spatial
payload interpretation, and sensitive debug surfaces.

## Production Minimums

1. Production MUST use `whitelisted-only` or `app-policy` peer admission.
   Authorization mode `all` is for development and tests only.
2. Configured production peers SHOULD pin expected `peer_id` and expected
   `wallet_public_key`.
3. Peer binding freshness MUST be enforced. Recommended maximum age is 1 hour.
   Recommended future-issued tolerance is 5 minutes.
4. Long-lived peer relationships MUST have an authority deadline based on the
   earliest relevant expiry: peer binding freshness, delegation expiry,
   authorization material expiry, offer expiry where enforced, and local
   session lifetime.
5. When authority expires, implementations MUST mark affected domains and
   offers unusable, reject new Get/Subscribe attempts, and end or ignore
   affected active subscriptions.
6. Signed authority objects MUST NOT gain authority-changing optional fields in
   v1. Future authority extensions should use new `type` values.
7. Domain access policy and offer policy MUST be explicit. Never accept an
   offer merely because the peer is authorized to serve the domain.
8. Production deployments MUST define at least one offer-kind security profile
   before consuming spatial payloads.
9. Registry-reference hash verification proves byte integrity only. Trust in a
   registry entry MUST come from local policy or a profile-defined trust rule.
10. Implementations MUST cap frames, arrays, metadata, registry references,
    catalogs, inline canonical JSON, JSON nesting, signature work, active
    connections, and subscriptions.
11. Base64url decoding MUST be canonical: no padding, no alternate alphabet,
    correct decoded length, and no alternate spellings for the same bytes.
12. Ed25519 verification semantics MUST be specified consistently and covered by
    invalid and non-canonical test vectors.
13. Discovery and manually supplied addresses MUST pass dial policy. Do not
    auto-dial loopback, link-local, private, local-service, or expensive relay
    paths unless explicitly configured.
14. Status and debug surfaces are sensitive telemetry. Production endpoints must
    require local or admin access, redact sensitive topology by default, and cap
    retained history.
15. Remote errors, diagnostics, labels, metadata, and `retryable` flags are
    untrusted hints. They MUST NOT drive authority or uncontrolled retries.
16. Authorization material SHOULD be proof-of-possession or audience-bound.
    Bearer tokens MUST be redacted and expiry-enforced during active sessions.

## Suggested Default Limits

A production SDK should ship conservative defaults and require deployments to
raise them intentionally.

| Limit | Suggested default |
| --- | ---: |
| Handshake frame body | 64 KiB |
| Catalog response frame body | 512 KiB |
| Get response frame body | 1 MiB |
| Subscribe message frame body | 256 KiB |
| Declared domains per handshake | 16 |
| Delegations per handshake | 16 |
| Authorization material entries | 8 |
| Offers per catalog | 256 |
| Registry refs per offer | 16 |
| Inline canonical registry JSON | 16 KiB each |
| Metadata serialized size | 4 KiB |
| JSON nesting depth | 32 |
| Active subscriptions per peer | 32 |
| Active connections per peer id | local cap |
| Failed handshakes, catalog requests, Get, Subscribe, invalid messages | rate-limited |

Cheap checks should run before expensive checks: frame limit, JSON shape,
required fields, size/count/depth limits, base64url and PeerId shape, domain-id
recomputation, then canonicalization and signature verification.

## Known Baseline Limitations

V1 deliberately does not define online revocation, per-session wallet
challenges, deployment-audience scoped bindings, constrained delegations,
shared offer-kind profiles, reliable Subscribe delivery, replay/resume, or
large-object transfer.

These omissions are acceptable for a first configured/private SDK path.
Production deployments must compensate with short freshness windows, explicit
app policy, pinned identities, bounded sessions, strict offer profiles, and
operational controls.
