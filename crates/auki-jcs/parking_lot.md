# Parking lot — auki-jcs

---

## `serde_jcs` upstream vendoring strategy

This crate is a thin wrapper over the [`serde_jcs`](https://crates.io/crates/serde_jcs) crate. If upstream becomes unmaintained or diverges from RFC 8785, every hash in the SDK changes. Should we vendor `serde_jcs` directly, pin to a specific version with a documented audit, or leave it as a normal dependency?
