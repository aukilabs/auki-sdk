# Parking lot — auki-layout

(Renamed from `auki-session` 2026-05-08.)

---

## TimeTransform log path encoding ambiguity

`timetransform_log_path` joins `from_id` and `to_id` with `__` after each is independently substituted (`/` → `__`). This means three distinct (from, to) pairs can collapse to the same filesystem path:

- `("a", "b/c/d")` → `a__b__c__d`
- `("a/b", "c/d")` → `a__b__c__d`
- `("a/b/c", "d")` → `a__b__c__d`

In practice, the SDK's recommended id convention is `<platform>-<machine_id>/<sensor_or_clock_name>` — exactly one `/` per id — which bounds this. But the encoding is fragile if ids ever drift from that shape. Worth a stricter scheme (separator that can't appear in ids, percent-encoding, etc.) before formalizing the layout to v1-stable.

---

## Resolved 2026-05-08: crate renamed `auki-session` → `auki-layout`

The crate name vs. scope mismatch is resolved by renaming. `auki-session` is now reserved for the future Rust counterpart of [`auki-session-py`](../auki-session-py)'s in-process runtime surface (per the [root `Session.open` Propagate item](../../parking_lot.md)); this crate becomes `auki-layout` and owns *only* the on-disk layout contract — paths, lifecycle convention, ID encoding. Mechanical scope: workspace member entry, Cargo metadata, [`auki-registry`](../auki-registry)'s path-dep + 6 `auki_session::` → `auki_layout::` call sites, and doc cross-references across the workspace. No behaviour change.
