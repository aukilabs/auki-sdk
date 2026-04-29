//! RFC 8785 JSON Canonicalization Scheme — canonical UTF-8 bytes for content-addressed hashing.
//!
//! Thin wrapper over [`serde_jcs`]. Inputs are `serde_json::Value`s, which
//! cannot hold non-finite numbers, so canonicalization is infallible.

pub fn canonicalize(value: &serde_json::Value) -> Vec<u8> {
    serde_jcs::to_vec(value).expect("serde_json::Value cannot contain non-finite numbers")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn canon(v: serde_json::Value) -> String {
        String::from_utf8(canonicalize(&v)).expect("JCS output is valid UTF-8")
    }

    #[test]
    fn empty_object_and_array() {
        assert_eq!(canon(json!({})), "{}");
        assert_eq!(canon(json!([])), "[]");
    }

    #[test]
    fn primitives() {
        assert_eq!(canon(json!(null)), "null");
        assert_eq!(canon(json!(true)), "true");
        assert_eq!(canon(json!(false)), "false");
        assert_eq!(canon(json!(0)), "0");
        assert_eq!(canon(json!(-1)), "-1");
        assert_eq!(canon(json!(1.5)), "1.5");
    }

    #[test]
    fn object_keys_are_sorted() {
        // RFC 8785 §3.2.3: keys sorted by UTF-16 code-unit sequence.
        assert_eq!(
            canon(json!({"b": 1, "a": 2, "c": 3})),
            r#"{"a":2,"b":1,"c":3}"#
        );
        assert_eq!(
            canon(json!({"z": {"b": 1, "a": 2}, "a": []})),
            r#"{"a":[],"z":{"a":2,"b":1}}"#
        );
    }

    #[test]
    fn array_order_is_preserved() {
        assert_eq!(canon(json!([3, 1, 2])), "[3,1,2]");
    }

    #[test]
    fn control_chars_use_lowercase_hex_escapes() {
        // RFC 8785 §3.2.2.2: short escapes when available, else \u00xx (lowercase).
        assert_eq!(canon(json!("\u{0001}")), r#""\u0001""#);
        assert_eq!(canon(json!("\u{000f}")), r#""\u000f""#);
        assert_eq!(canon(json!("\t")), r#""\t""#);
        assert_eq!(canon(json!("\n")), r#""\n""#);
        assert_eq!(canon(json!("\"\\")), r#""\"\\""#);
    }

    #[test]
    fn non_ascii_passes_through_as_utf8() {
        // RFC 8785 §3.2.2.2: code points > U+007F are emitted as UTF-8, not escaped.
        assert_eq!(canon(json!("€")), "\"€\"");
        assert_eq!(canon(json!("汉")), "\"汉\"");
    }

    #[test]
    fn forward_slash_is_not_escaped() {
        // RFC 8785: "/" is preserved (unlike some JSON encoders that escape it).
        assert_eq!(canon(json!("a/b")), r#""a/b""#);
    }

    #[test]
    fn round_trip_is_stable() {
        // Sprint requirement: Value → bytes → re-parse → bytes must produce identical output.
        let v = json!({
            "z": [3, 2, 1, {"k": "v", "j": null}],
            "a": "hello",
            "m": 3.14,
            "t": true,
            "f": false,
            "n": null,
            "s": "with \"quotes\" and \\ backslash and / slash"
        });
        let first = canonicalize(&v);
        let reparsed: serde_json::Value =
            serde_json::from_slice(&first).expect("canonical bytes must parse as JSON");
        let second = canonicalize(&reparsed);
        assert_eq!(first, second);
    }
}
