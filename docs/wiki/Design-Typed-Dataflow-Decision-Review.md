# Typed dataflow experiment: decision review

> **Purpose:** decide which ideas have earned another implementation round.
> This is not a request to approve the prototype code as production code.

The experiment now spans three stacked reviews:

1. typed ports, Buffer, Episode, and pump mechanics;
2. Component, Observable, Operable, Output, Product, and Catalog semantics;
3. observation lifecycle, failure isolation, and shared scheduling.

The useful question is no longer “do we like the design?” It is: **which
individual propositions are supported by evidence, which need revision, and
which should be discarded?**

Use `Accept`, `Revise`, or `Reject` for each proposition. `Accept` means accept
the semantic rule, not the current Rust implementation or exact API spelling.

## Proposed dispositions

| # | Proposition | Proposed | Evidence and qualification |
|---:|---|---|---|
| 1 | A Peer is the authenticated runtime and authority boundary; a Component is a hosted unit of behavior. | **Accept** | This separates who participates from what behavior is composable. It does not require one process or one robot per Peer. |
| 2 | Components connect through named, typed inputs and outputs. | **Accept** | Compile-time incompatibility works. Static connections can compile near a direct call; dynamic connections have measurable but understandable overhead. Exact `input`/`output` syntax remains provisional. |
| 3 | Observable means a Component can show typed data; Operable means another Component can intentionally cause typed configuration or execution. | **Accept** | These names describe direction and authority without encoding one access mode. A Component may expose either, both, or neither. |
| 4 | Component identity is stable behavior identity; Output identity names one immutable configured production contract. | **Accept** | Camera reconfiguration showed why changing resolution must replace the Output without replacing the addressable Camera Component. |
| 5 | Compatibility is determined by standardized typed fields and schemas, not manifest-hash equality. | **Accept** | Hash equality is too brittle for independently authored compatible producers. A hash identifies the exact manifest bytes; it is not a substitute for compatibility matching. |
| 6 | Component Manifests, Output Manifests, and Product Manifests are distinct fetchable Catalog descriptions. | **Accept** | They describe different identities and lifecycles. The current JSON hashing is experimental and must not be accepted as the canonical format. |
| 7 | A Product does not become a Component merely because both support similarly named requests. | **Accept** | A Component is behavior; a Buffer, Episode, or Artifact is produced data. Sharing `latest` or `time range` vocabulary does not erase that distinction. |
| 8 | A Buffer is optional retention attached to an Output, not mandatory plumbing between Components. | **Accept** | Direct delivery is materially cheaper. Retention is valuable when history, promotion, or replay is required, but imposing it everywhere has not earned its cost. |
| 9 | An Episode is a deliberately retained interval with an explicit conclusion; a Buffer is a bounded evicting Product. | **Revise** | The semantic distinction is useful, but Episode Product Manifests, active lifecycle, persistence, and promotion storage are not yet coherent enough to accept the implementation. |
| 10 | Observation selection, delivery policy, and execution policy are independent. | **Accept** | `latest_existing` versus `follow_new`, `EverySelected` versus `CoalesceLatest`, and inline versus scheduled execution answer three different questions. Combining them caused earlier ambiguity. |
| 11 | Local and transported composition should preserve the same typed semantics while allowing different ownership and copy behavior. | **Accept** | Local observers can share immutable storage; serialization necessarily creates a copy. The transport boundary must report rather than conceal that difference. |
| 12 | Scheduler choice is private runtime policy, not part of an Observable contract. | **Accept** | A fixed pool avoids one thread per relationship, but the measured throughput/latency tradeoff proves that the current pool is not a universal answer. |
| 13 | Returned Component errors make a relationship explicitly failed; callback panics are contained at the Component boundary. | **Revise** | Explicit returned errors are sound. `catch_unwind` worked for the Rust fixture, but panic behavior across FFI, process-abort configurations, poisoned state, and internal runtime bugs needs a narrower production rule. |
| 14 | A contract-affecting reconfiguration emits an explicit transition; pinned observers stop and follow-current observers may deliberately cross it. | **Accept** | Tests prevent consumers and Buffer Products from silently mixing observations governed by different Output Manifests. |
| 15 | Catalog visibility is explicit exposure, not proof that a Component is currently producing or that a Product is durable. | **Accept** | Discoverability and momentary runtime health are separate. Dynamic availability still needs a defined inventory/health mechanism. |
| 16 | Payload contracts describe the final emitted SDK payload truthfully. | **Accept** | Source hardware may also be described, but it must not be confused with the bytes and typed fields actually emitted. Automated media validation remains out of scope. |
| 17 | The present public API is ready for unfamiliar developers or agents. | **Reject pending evidence** | The Camera fixture proves internal semantics, but there is no generic public Component/Output construction path. A clean-room application test is required before making an agent-friendliness claim. |

The initial public-API feasibility probe has now confirmed proposition 17's
rejection: the Catalog accepts Component and Output descriptions before any
corresponding generic live interface can be constructed. See
[`CLEAN-ROOM-FIRST-ATTEMPT.md`](../../experiments/typed-dataflow/CLEAN-ROOM-FIRST-ATTEMPT.md).

## Explicit non-decisions

This review does not decide:

- the production networking or Manager architecture;
- canonical manifest serialization and hashing;
- authentication and authorization policy;
- Domain assignment;
- a global executor or async runtime;
- storage layout, chunking, or disk policy;
- timestamp normalization and time-transform policy;
- whether one physical device should be represented by one Component or
  several smaller Components.

Those questions should not be answered accidentally through prototype code.

## Reviewer response

Copy this section into the review discussion:

```text
1 Peer / Component boundary:                 Accept | Revise | Reject
2 Typed named ports:                         Accept | Revise | Reject
3 Observable / Operable:                     Accept | Revise | Reject
4 Stable Component / immutable Output:       Accept | Revise | Reject
5 Typed compatibility, not hash equality:    Accept | Revise | Reject
6 Three Manifest identities:                 Accept | Revise | Reject
7 Products remain distinct from Components:  Accept | Revise | Reject
8 Buffer is optional retention:              Accept | Revise | Reject
9 Buffer / Episode lifecycle:                Accept | Revise | Reject
10 Selection / delivery / execution split:   Accept | Revise | Reject
11 Same semantics, different copy boundary:  Accept | Revise | Reject
12 Scheduler is runtime policy:              Accept | Revise | Reject
13 Error and panic boundary:                  Accept | Revise | Reject
14 Explicit Output transitions:              Accept | Revise | Reject
15 Exposure versus availability:             Accept | Revise | Reject
16 Truthful final-payload contracts:          Accept | Revise | Reject
17 Public API agent-friendliness:             Accept | Revise | Reject

Required revisions:

Questions that block the next experiment:
```

## Evidence

- [`RESULTS.md`](../../experiments/typed-dataflow/RESULTS.md)
- [`RESULTS-OBSERVABLE-OPERABLE.md`](../../experiments/typed-dataflow/RESULTS-OBSERVABLE-OPERABLE.md)
- [`RESULTS-OBSERVATION-REQUESTS.md`](../../experiments/typed-dataflow/RESULTS-OBSERVATION-REQUESTS.md)
- [`RESULTS-DATAFLOW-STRESS.md`](../../experiments/typed-dataflow/RESULTS-DATAFLOW-STRESS.md)
- [`CLEAN-ROOM-FIRST-ATTEMPT.md`](../../experiments/typed-dataflow/CLEAN-ROOM-FIRST-ATTEMPT.md)
