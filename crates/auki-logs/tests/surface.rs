use auki_logs::{
    BytesPayload, Entry, Log, canonical_manifest_json_str, decode_segment_bytes,
    encode_segment_bytes,
};
use serde_json::json;

#[test]
fn rust_root_api_remains_source_compatible() {
    let canonical = canonical_manifest_json_str(
        r#"{"retention_ns":3000000000,"segment_duration_ns":1000000000,"kind":"test"}"#,
    )
    .unwrap();
    assert_eq!(
        canonical,
        r#"{"kind":"test","retention_ns":3000000000,"segment_duration_ns":1000000000}"#
    );

    let entries = vec![Entry {
        timestamp_ns: 100,
        payload: BytesPayload {
            bytes: b"hello".to_vec(),
        },
    }];
    let segment = encode_segment_bytes(0, &entries).unwrap();
    let decoded = decode_segment_bytes(&segment).unwrap();
    assert_eq!(decoded[0].timestamp_ns, 100);
    assert_eq!(decoded[0].payload.bytes, b"hello");

    let dir = tempfile::tempdir().unwrap();
    let mut log = Log::<BytesPayload>::open(
        dir.path(),
        json!({
            "kind": "test",
            "segment_duration_ns": 1_000_000_000i64,
            "retention_ns": 3_000_000_000i64,
        }),
    )
    .unwrap();
    log.append(
        42,
        &BytesPayload {
            bytes: vec![1, 2, 3],
        },
    )
    .unwrap();
    drop(log);

    let reader = Log::<BytesPayload>::read(dir.path()).unwrap();
    let entries = reader.entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].payload.bytes, vec![1, 2, 3]);
}
