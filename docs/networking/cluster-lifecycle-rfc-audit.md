# Cluster Lifecycle RFC Audit

## Blocking issues

- `cluster-lifecycle-backlog.md` → `SDK NTP / Clock-Sync Protocol` → `Spec slots` references stale RFC slots: `RFC-0022: Spatial Message Envelope`, `RFC-0025: Minimum Offer Kinds`, and existing clock references in `RFC-0020`, `RFC-0022`, `RFC-0023`, `RFC-0024`, and `RFC-0028`.
  - Recommended action: update the NTP/clock-sync slot references to the current RFC map: `RFC-0027: Spatial Message Envelope`, `RFC-0031: Minimum Offer Kinds`, plus current timestamp/clock touchpoints in `RFC-0023`, `RFC-0024`, `RFC-0027`, `RFC-0028`, `RFC-0029`, `RFC-0030`, and `RFC-0034`. Reserve a new owner RFC for clock sync instead of pointing at old slots.

- `cluster-lifecycle-backlog.md` → `Compatibility Fixtures And Test Vectors` → `Spec slots` lists old RFC numbers for nearly every concrete wire shape.
  - Recommended action: replace the stale list with the current owner RFCs: `RFC-0005: Peer Binding Schema`, `RFC-0007: Domain Declaration Schema`, `RFC-0008: Domain Delegation Schema`, `RFC-0024: Offer Catalog`, `RFC-0027: Spatial Message Envelope`, `RFC-0029: Get`, `RFC-0030: Subscribe`, and `RFC-0034: Status And Observability API`. Consider adding `RFC-0002`, `RFC-0003`, and `RFC-0032` because they own JSON encoding, signed bytes, and compatibility rules.

- `cluster-lifecycle-backlog.md` → `Later Deliverables` → `Discovery And Reachability` lists stale later-work RFC slots: `RFC-0011: Discovery Record Shape`, `RFC-0012: Discovery Data-Type Hints`, and `RFC-0018: Peer Graph Hints`.
  - Recommended action: update those references to `RFC-0015: Discovery Record Shape`, `RFC-0016: Discovery Data-Type Hints`, and `RFC-0022: Peer Graph Hints`.

## Should-fix issues before v1

- `cluster-lifecycle-specs.md` → `RFC-0012: Served Domain Set` → `Dynamic Updates (To Fill)` contains normative requirements even though the document says To Fill sections are not normative.
  - Recommended action: either move the dynamic-update requirements into a real owner RFC, or rewrite this subsection as non-normative backlog language. In particular, relocate or soften: “Any dynamic served-domain update MUST rerun…” and “implementations SHOULD treat served domain changes as requiring a reconnect…”.

- `cluster-lifecycle-specs.md` → `RFC-0009: Authority Chain Validation` duplicates detailed domain declaration and delegation validation rules already owned by `RFC-0007` and `RFC-0008`.
  - Recommended action: keep `RFC-0009` as the ordering/orchestration RFC and replace duplicated validation detail with references to the schema-owner RFCs. This reduces drift risk when declaration or delegation validation changes.

- `cluster-lifecycle-specs.md` → recurring clock/timestamp language in `RFC-0027`, `RFC-0029`, `RFC-0030`, and `RFC-0034` has no owner RFC for clock-sync/NTP semantics.
  - Recommended action: add a dedicated time-sync owner RFC or explicitly mark clock sync as out of v1 normative scope. The owner RFC should define peer clock offset, delay, sample freshness, failure handling, and whether time sync affects Offer/Get/Subscribe usability.

- `glossary.md` is missing or underspecifies recurring terms used by the filled RFCs: `peer binding freshness`, `accepted served domain set`, `offer kind`, `access mode`, `payload descriptor`, `registry reference`, `status snapshot`, `failure record`, `Get request`, `Subscribe accept`, `Subscribe reject`, `Subscribe end`, `sequence gap`, and clock-sync/NTP terms.
  - Recommended action: add descriptive, non-normative glossary entries for these terms after the owner RFCs are stable. Do not introduce new requirements in the glossary.

- `glossary.md` → `Offer Catalog` is too vague compared with `RFC-0024`.
  - Recommended action: update the term to describe a peer-to-peer snapshot of offers visible to a requester, not a signed authority object, with usability determined by `RFC-0026`.

- `cluster-lifecycle-backlog.md` → `Final AI / Expert Review` is now partially satisfied by this audit.
  - Recommended action: after this audit is accepted, remove or downgrade that backlog item to a narrower follow-up, such as “apply accepted audit fixes and rerun final consistency review”.

- `cluster-lifecycle-backlog.md` → `Compatibility Fixtures And Test Vectors` includes upgrade-rule deliverables that are partly solved by `RFC-0032: Protocol Versions Are Compatibility Contracts`.
  - Recommended action: downgrade the backlog item so it asks for fixtures/test vectors that exercise `RFC-0032`, not for re-defining the compatibility rules.

## Nice-to-have polish

- `cluster-lifecycle-specs.md` has no stale references to “Shared Rules”.
  - Recommended action: no document edit needed for this point.

- `cluster-lifecycle-specs.md` → `RFC-0015`, `RFC-0016`, and `RFC-0022` To Fill sections avoid RFC 2119 keywords.
  - Recommended action: keep them non-normative until filled; when filled, add owner RFC requirements and update the backlog references at the same time.

- `cluster-lifecycle-backlog.md` → `Suggested Finish Order` says “Add the missing time-sync RFC text” before the actual RFC slot exists.
  - Recommended action: name the intended owner RFC slot once chosen, so future edits do not reintroduce old slot references.

- `glossary.md` → `Spatial Data` includes “clocks” and “temporal state”, while the spec still treats clock sync as unresolved backlog work.
  - Recommended action: keep the broad term, but add separate clock/time-sync terms so implementers do not read “spatial data” as requiring NTP behavior in v1.

- `cluster-lifecycle-specs.md` → `RFC-0031: Minimum Offer Kinds` → `Future Kinds` uses a normative “Future kinds MUST preserve…” rule for later features.
  - Recommended action: this is acceptable as a guardrail, but consider rephrasing as a compatibility note if future offer kinds are meant to be entirely non-v1.
