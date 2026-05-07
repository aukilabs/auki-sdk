# Parking lot — auki-session

---

## TimeTransform log path encoding ambiguity

`timetransform_log_path` joins `from_id` and `to_id` with `__` after each is independently substituted (`/` → `__`). This means three distinct (from, to) pairs can collapse to the same filesystem path:

- `("a", "b/c/d")` → `a__b__c__d`
- `("a/b", "c/d")` → `a__b__c__d`
- `("a/b/c", "d")` → `a__b__c__d`

In practice, the SDK's recommended id convention is `<platform>-<machine_id>/<sensor_or_clock_name>` — exactly one `/` per id — which bounds this. But the encoding is fragile if ids ever drift from that shape. Worth a stricter scheme (separator that can't appear in ids, percent-encoding, etc.) before formalizing the layout to v1-stable.

---

## Crate name vs scope mismatch — `auki-session` is path helpers today _(filed by Dobby, 2026-05-08)_

The crate is named `auki-session` but currently exports only path-construction helpers: `registries_root`, `sensor_entry_path`, `clock_entry_path`, `frame_entry_path`, `session_root`, `timetransform_log_path`, `sensorlog_path`, `poselog_path`, `id_to_segment`. There is no `Session` type, no lifecycle, no clock binding. Today's daemons construct sessions by convention (`session_id` is a UUIDv4 the integrator mints at boot, threaded through every manifest by hand) and the root [`README.md`](../../README.md) "What's implemented today" section explicitly lists *"A `Session` abstraction tying clock + sensor-id minting + recording lifecycle together"* as not-yet-implemented.

The mismatch will bite when the real `Session` lands. The root parking-lot's [`Session.open` Propagate item](../../parking_lot.md) already specs `auki_session::Session::open(app_root, *, app_id, app_instance, session_id=None) -> Session` — but `Session` and `session_root()` (the path helper) cannot share a crate without a name collision *and* a documentation problem: a crate that exports both a runtime object and the unrelated path helpers becomes the same kind of grab-bag PR #55 just split `auki-registry` out of.

Two forward paths:

1. **Rename now to `auki-paths`.** Cheap while the crate has zero in-workspace consumers calling `auki_session::*`. Reserves `auki-session` for the runtime abstraction. The Cargo workspace member entry, the `auki-session-py` cross-references, and the workspace-internal path-dep edges are the entire scope.
2. **Footnote the API-surface row in the root README.** Mirror the pattern PR #55 introduced for `auki-registry` ("**Log payload types departing**…"). E.g.: *"`auki-session` | path helpers today; **`Session` runtime abstraction pending** per [root parking-lot](parking_lot.md)."* Cheaper, lossier — when the runtime lands the path helpers either move out (back to (1) but later, with consumers) or stay and the crate becomes a permanent two-thing crate.

Lean: (1) now, while the cost is zero. The `auki-session-py` crate already exists as scaffolding for the future Python `Session` surface — keeping the name available for the Rust counterpart it will wrap is consistent with that crate's own README.

Surfacing for Nils to pick. Not gating any in-flight work.
