use auki_protocol::v1::frame::{decode_json_frame, encode_json_frame, encode_length};
use serde_json::Value;

const V1_JSON_FRAME_VECTORS: &str = include_str!("../vectors/v1_json_frames.json");

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn bytes_from_hex(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0, "hex string must have an even length");
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex byte"))
        .collect()
}

#[test]
fn locked_v1_json_frame_vectors_match_implementation() {
    let fixture: Value =
        serde_json::from_str(V1_JSON_FRAME_VECTORS).expect("valid frame vector fixture");
    assert_eq!(
        fixture["name"],
        Value::String("auki.protocol.v1.json_frame.vectors".to_owned())
    );

    let vectors = fixture["vectors"].as_array().expect("vectors array");
    assert!(!vectors.is_empty(), "expected at least one vector");

    for vector in vectors {
        let name = vector["name"].as_str().expect("vector name");
        let body = vector["body_utf8"].as_str().expect("body_utf8 string");
        let body_len = vector["body_len"].as_u64().expect("body_len u64");
        let prefix_hex = vector["prefix_hex"].as_str().expect("prefix_hex string");
        let frame_hex = vector["frame_hex"].as_str().expect("frame_hex string");

        assert_eq!(
            body.as_bytes().len() as u64,
            body_len,
            "{name}: body_len must count UTF-8 bytes"
        );

        let prefix = encode_length(body_len);
        assert_eq!(hex(&prefix), prefix_hex, "{name}: length prefix drifted");

        let mut frame = prefix;
        frame.extend_from_slice(body.as_bytes());
        assert_eq!(hex(&frame), frame_hex, "{name}: frame bytes drifted");
        assert_eq!(
            bytes_from_hex(frame_hex),
            frame,
            "{name}: frame_hex must match body_utf8 and prefix_hex"
        );

        let value: Value = serde_json::from_str(body).unwrap_or_else(|err| {
            panic!("{name}: body_utf8 must be valid JSON: {err}");
        });
        let encoded = encode_json_frame(&value, body_len).unwrap_or_else(|err| {
            panic!("{name}: encode_json_frame failed: {err}");
        });
        assert_eq!(
            encoded, frame,
            "{name}: encoder no longer emits the locked compact frame"
        );

        let (decoded, consumed) = decode_json_frame(&frame, body_len).unwrap_or_else(|err| {
            panic!("{name}: decode_json_frame failed: {err}");
        });
        assert_eq!(decoded, value, "{name}: decoded body drifted");
        assert_eq!(consumed, frame.len(), "{name}: consumed byte count drifted");
    }
}
