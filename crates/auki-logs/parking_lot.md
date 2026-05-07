# Parking lot — auki-logs

---

## Per-entry checksums

Segment files have no per-entry checksum. A CBOR decode failure on non-truncation corruption surfaces as `Error::Cbor` rather than being attributable to a specific entry, and adjacent entries past the corrupt one stop being readable. Should we add a CRC32C per entry? Tradeoff: ~4 bytes/entry overhead (~0.4% on typical 1 KB payloads) vs. better diagnosis of mid-segment corruption.

## Reader streaming for unbounded captures

`LogReader::entries()` eagerly loads every entry across every segment. For long captures — especially with `retention_ns = 0` — this can be very large. Add a streaming iterator API (yields `Entry<T>` one at a time without buffering all segments), or leave it to consumers (renderer, analysis tools) to read individual segments themselves?

## Encoder-aware vs encoder-agnostic `Log<T>` post-migration

The [`auki-datatypes`](../auki-datatypes) migration changes segment payload encoding from CBOR-via-ciborium to prost (protobuf). Open: does `Log<T>` keep a serde-style encoder bound (`T: prost::Message` after migration), or does it become **encoding-agnostic** (consumer encodes bytes itself, `Log` only handles framing)?

Lean: encoding-agnostic. `auki-logs` is supposed to be format-neutral framing; bolting prost into its bound mixes concerns and locks out future encoders (Cap'n Proto, FlatBuffers, anything we haven't thought of). The encoding-agnostic shape is `Log<T>` where `T` provides `to_bytes() -> Vec<u8>` and `from_bytes(&[u8]) -> Result<Self>` via a small trait — consumers pick their encoder.

Decide in step 1 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md); the answer lands the new generic-bounds shape used by every subsequent step.
