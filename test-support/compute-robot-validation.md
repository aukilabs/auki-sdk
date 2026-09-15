# Compute + robot dev validation

## Scope and source

Validated SDK issue [#375](https://github.com/aukilabs/auki-sdk/issues/375)
against [#388](https://github.com/aukilabs/auki-sdk/pull/388), exact source
`95ef3b1cd2a2e5755bbeb47d323b44746de65ee4`, on 2026-09-15.
This follow-up adds examples and test support; it changes no Rust SDK implementation,
provider contract, deployment, or credentials. Its PR targets
`feat/domain-data-shared-credentials` so #388 need not be merged first.

The approved live scope was one retained dedicated compute node, one retained
Domain-assigned robot, and that Domain in dev. Worker IDs and credentials are omitted.
Every job used a unique run-specific capability, dedicated mode and one attempt.
Human authentication belonged to the operator; workers used their separate machine
credentials. No hardware commands, new provisioning, reassignment, staking, shared SQL
writes, merge, or deployment occurred.

Provider preflight observed DDS schema v0.14.6, DMS version 8088111, enabled robot
admission, and aligned dev endpoints/audiences. This is dated dev evidence, not a
compatibility guarantee for another environment.

## Live results

- **Machine authentication:** compute registration/SIWE and wallet-free robot startup
  succeeded through the SDK against real DDS/DMS.
- **HTTP example:** the actual `compute.py` and `robot.py` entrypoints completed the
  uppercase/inspection chain. The operator independently read the stored bytes,
  checked the inspection hash/length, and checked receipt executor identities.
- **P2P examples:** the actual entrypoints passed over both a localhost direct route
  and the dev DMS-booked libp2p relay. The authenticated Info response matched the
  expected partner and run. Independent DMS readback found `peer-verified` events.
  Both continuous worker processes exited with code 0 after SIGINT.
- **Renewal:** the lifecycle probe continued for 310 seconds, crossing the observed
  initial task-token lifetime of 180 seconds and the configured robot idle authority
  boundary. Task data reads continued, the task token rotated beyond its original
  expiry, and robot idle reads and subsequent authenticated peer exchanges succeeded.
- **Authority/lifetimes:** assigned-Domain robot idle reads passed; idle writes,
  deletes and wrong-Domain reads were denied. Retained task data/token access was
  fenced after termination. Compute task peers stopped; the robot peer survived
  between tasks and through a failed task.
- **Failure/cancellation:** both executor types produced intentional failures with
  independently verified failure reasons, executor receipts and artifact bytes.
  Both cancellation cases awaited handler cleanup and produced only the provider's
  cancellation audit receipt, not worker completion outputs.
- **Progress/events:** the lifecycle probe independently checked DMS progress,
  ordered events, completion/failure status and artifact metadata.

The final lifecycle run passed. Earlier probe attempts exposed harness mistakes:
submission before the first DMS availability poll, treating the Info requester
principal dictionary as a string, and retrieving a task peer after the task had
cleared its view. These were corrected using the provider/binding contracts, without
relaxing SDK authorization or changing SDK Rust code. All interrupted attempts were
included in cleanup auditing.

## Independent cleanup audit

The final read-only audit covered **7 run manifests / 15 jobs**: 10 completed,
3 failed (including intentional failure cases), and 2 canceled. Every task was
terminal with no retained executor reservation or task lease.

- 22 exact run-owned data names checked; zero remaining records.
- Zero active worker tasks, relay capacity holds or peer advertisements.
- Relevant relay bookings ended; robot retained authority was no longer active.
- Recorded worker processes were gone; process inspection found no probe workers.
- Original compute/robot capability sets restored through supported registration,
  then read back from DDS. Both workers were observed offline after presence drained.
- Worker records, identities and terminal job/receipt evidence were retained.

Sanitized detailed manifests remain in the operator's private host workspace; they
are deliberately not committed alongside credential/identity material.

## Local verification

Commands run from the repository root, with bounded Cargo build parallelism and
reduced debug info on the host:

```bash
CARGO_BUILD_JOBS=2 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --locked -p auki-auth -p auki-dms -p auki-domain-client -p auki-tasks -p auki-sdk --quiet
cargo build --locked -p auki-sdk --examples
cargo fmt --all -- --check
.venv/bin/python -m pytest core/bindings/python/auki-sdk-py/python_tests \
  core/examples/compute-robot test-support/test_compute_robot_operator.py -q
git diff --check
git diff --cached --check
```

The selected native suites/doctests and native example build passed. The final
combined Python suite passed **112 tests** (85 binding tests plus 27 example/operator
tests). The full Python binding was built with `maturin develop --locked
--manifest-path core/bindings/python/auki-sdk-py/Cargo.toml`; a minimal build omits
optional exports required by the full surface suite. Formatting and whitespace
checks passed.

**Lint limitation:** `cargo clippy --locked -p auki-sdk --all-targets -- -D warnings`
failed under the host Rust 1.98.1 toolchain on `unused_assignments` at
`core/auki-sdk/src/peer_runtime.rs:977` and `result_large_err` diagnostics in
`bootstrap.rs` / `peer_runtime.rs`. These are inherited from the exact #388 head:
this follow-up changes no Rust source, Cargo manifest or lockfile. No lint allowances
or unrelated Rust repairs were added. Do not describe strict Clippy as green.

Independent review found and reproduced an operator SIGTERM issue. The focused repair
added pre-mutation audit persistence and controlled signal cleanup; its subprocess
regression passed. Final independent review passed with 27 tests and no further
consequential finding on the frozen implementation.

## Coverage boundaries

Unassigned robots and forged/expired/wrong-audience credential variants remain local
fixture coverage; the retained robot was not reassigned and no extra fixtures were
provisioned. Fencing retained SDK handles does not imply instant remote revocation of
a copied bearer token. Native Rust examples were built, while live worker commands
used Python bindings exercising the native Rust runtime. WASM/browser, Swift, Expo,
other operating systems, physical robot hardware and production were not live-tested.

See [the runnable example](../core/examples/compute-robot/README.md) for setup and the
opt-in operator command. DMS availability requires a worker poll, not just successful
DDS startup; the operator waits using read-only estimates and does not replay job
creation POSTs.
