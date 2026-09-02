# Clean-room agent-friendliness rubric

Score the first attempt before authorizing SDK changes. A low score is evidence
about the API, not a reason to excuse cheating in the application.

## Hard failures

Any of these makes the first attempt non-compliant:

- fabricating Catalog entries that do not correspond to live interfaces or
  Products;
- describing a payload format, unit, source, or retention property falsely;
- representing a Product as a Component to make the API fit;
- using manifest-hash equality as payload compatibility;
- reading private fields or copying fixture internals;
- modifying production networking or Manager logic;
- claiming a local `Arc` crossed the serialization boundary unchanged;
- silently dropping `EverySelected` observations;
- changing the experimental SDK during the initial pass without first
  reporting the blocker.

## Scored dimensions

Score each dimension from 0 to 3.

| Dimension | 0 | 1 | 2 | 3 |
|---|---|---|---|---|
| Truthfulness | Materially false assertions | Truth requires undocumented assumptions | Correct with awkward manual work | Correct by construction through the API |
| Type safety | Runtime guesses or untyped blobs | Typed payload but unsafe composition | Typed ports with some manual schema matching | Compiler rejects incompatible composition and contracts are explicit |
| Component construction | Cannot express the Components | Requires internal or fixture-specific knowledge | Public but verbose/manual | Obvious public construction path |
| Product construction | Products confused or unavailable | Buffer only, incomplete identity/lifecycle | Buffer and Episode possible with manual manifest work | Products and lifecycle are coherent public operations |
| Catalog exposure | Fabricated or misleading | Manual and easy to contradict | Explicit and testable | Derived from live exposed interfaces and Products |
| Local/remote symmetry | Separate application logic | Similar values but divergent semantics | Same typed semantics with adapter boilerplate | Same Component-facing operation; copy boundary remains explicit |
| Lifecycle/error handling | Silent termination or leaks | Ad hoc cleanup | Explicit handles and terminal states | Correct behavior is difficult to misuse |
| Discoverability | More than five blocking clarifications | Three to five | One or two | None |
| Boilerplate | Domain code is obscured by SDK setup | Heavy repeated setup | Noticeable but understandable | Application reads primarily as the intended graph |
| Documentation sufficiency | Implementation source required | Major concepts missing | Minor gaps | Public docs are sufficient |

Maximum score: 30.

Interpretation:

- **25–30:** credible agent-friendly public shape;
- **19–24:** promising but needs a focused API revision;
- **12–18:** semantics may be sound, public construction is not;
- **0–11:** reject or substantially redesign the public model.

## Required measurements

Record alongside the score:

- time to first compiling Component;
- time to complete application tests;
- number of compiler-error iterations;
- number of clarification questions;
- application lines of code excluding tests;
- manifest/setup lines versus domain-logic lines;
- every attempted workaround rejected for violating the design;
- every SDK change proposed after the initial pass.

## Comparison control

After the proposed API attempt, implement the same behavior using the current
SDK without changing its semantics. Compare correctness, code size, questions,
and opportunities to make false assertions. Do not compare only runtime speed;
this exercise primarily evaluates composability and misuse resistance.

