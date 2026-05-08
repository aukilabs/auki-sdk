# Parking lot — auki-logs

---

## Per-entry checksums

Segment files have no per-entry checksum. A `LogPayload::decode` failure on non-truncation corruption surfaces as `Error::Payload` rather than being attributable to a specific entry, and adjacent entries past the corrupt one stop being readable. Should we add a CRC32C per entry? Tradeoff: ~4 bytes/entry overhead (~0.4% on typical 1 KB payloads) vs. better diagnosis of mid-segment corruption.

## Reader streaming for unbounded captures

`LogReader::entries()` eagerly loads every entry across every segment. For long captures — especially with `retention_ns = 0` — this can be very large. Add a streaming iterator API (yields `Entry<T>` one at a time without buffering all segments), or leave it to consumers (renderer, analysis tools) to read individual segments themselves?

