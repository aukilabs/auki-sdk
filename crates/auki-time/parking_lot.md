# Parking lot — auki-time

---

## Future `TimeTransformSource` variants

`TimeTransformSource::LocalClockRead` is the only variant today. The README hints at future `HeartbeatExchange` (network-based) and possibly `Gps`. Some of these can't reliably detect discontinuities locally — `HeartbeatExchange` round-trip noise can dwarf real clock steps. The README notes that `discontinuous: bool` may need to widen to `Option<bool>` (false = known continuous, null = this source can't tell). Confirm direction before any new variant lands, since CBOR-with-serde-defaults tolerance affects the migration story.
