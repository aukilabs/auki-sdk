# Parking lot — auki-session

---

## TimeTransform log path encoding ambiguity

`timetransform_log_path` joins `from_id` and `to_id` with `__` after each is independently substituted (`/` → `__`). This means three distinct (from, to) pairs can collapse to the same filesystem path:

- `("a", "b/c/d")` → `a__b__c__d`
- `("a/b", "c/d")` → `a__b__c__d`
- `("a/b/c", "d")` → `a__b__c__d`

In practice, the SDK's recommended id convention is `<platform>-<machine_id>/<sensor_or_clock_name>` — exactly one `/` per id — which bounds this. But the encoding is fragile if ids ever drift from that shape. Worth a stricter scheme (separator that can't appear in ids, percent-encoding, etc.) before formalizing the layout to v1-stable.
